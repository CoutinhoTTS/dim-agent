//! #2152: a line cut at 0 answers the context methods like any other line.
use super::*;
use dim_agent_capabilities::{
    adapters::{DefaultPrepareModelPlugin, EchoModel},
    context_methods::ContextMethodsPlugin,
    context_model::window_facts,
    history::HistoryRenderer,
    host::{
        CompactionInput, CompactionStrategy, CompactionStrategyHost, ContextModel,
        ContextModelHost, IdentityContext, ModelHost, PrepareContextHost, ResolveContextModel,
        StrategyDiagnostics, StrategyOutcome,
    },
};
use dim_agent_runtime::ConfigureUpdate;
use dim_agent_types::{CompactionStrategyRecord, JsonObject, ModelSelection};

struct Window;
impl ResolveContextModel for Window {
    fn resolve(
        &self,
        request: Arc<dim_agent_types::ModelRequest>,
        _: CancellationToken,
    ) -> HostFuture<'_, Arc<dyn ContextModel>> {
        Box::pin(async move { Ok(window_facts(16_000, 200., &request)) })
    }
}

/// A strategy that always answers `skipped`: the test needs the step to run,
/// not a replacement.
struct KeepsHistory;
impl CompactionStrategy for KeepsHistory {
    fn name(&self) -> &str {
        "keeps-history"
    }
    fn replacement_size(&self, _: &dyn ContextModel) -> f64 {
        0.
    }
    fn compact(&self, _: CompactionInput, _: CancellationToken) -> HostFuture<'_, StrategyOutcome> {
        Box::pin(async {
            Ok(StrategyOutcome::Skipped {
                reason: "the fixture keeps every entry".into(),
                diagnostics: StrategyDiagnostics {
                    model_call_count: 0,
                    model_usage: None,
                    record: CompactionStrategyRecord {
                        kind: "keeps-history".into(),
                        details: JsonObject::new(),
                    },
                },
            })
        })
    }
}

async fn launch_context() -> RunningServer {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.timeline_store(dim_agent_runtime::TimelineStoreBinding::new(Arc::new(
        dim_agent_runtime::MemoryTimelineStore::default(),
    )));
    defaults::install(&mut builder);
    builder
        .session_value(Arc::new(HistoryRenderer::default()))
        .process_value(Arc::new(ModelHost(Arc::new(EchoModel))))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(Some(Arc::new(Window)))))
        .session_value(Arc::new(CompactionStrategyHost(Some(Arc::new(
            KeepsHistory,
        )))))
        .require_feature(
            Feature::new("PrepareModelHost").session("PrepareModelHost", |names| {
                Ok(Box::new(DefaultPrepareModelPlugin::new(names)?))
            }),
        )
        .require_feature(
            Feature::new("ContextMethods").session("ContextMethods", |names| {
                Ok(Box::new(ContextMethodsPlugin::new(names)?))
            }),
        );
    defaults::tool_host(&mut builder, Arc::new(MockTools));
    serve(
        builder,
        ServerConfig {
            unix_path: Some(std::path::PathBuf::from("/tmp").join(format!(
                "dim-server-test-{}/server.sock",
                uuid::Uuid::new_v4()
            ))),
            ..ServerConfig::default()
        },
    )
    .await
    .unwrap()
}

async fn settle(client: &mut Client, session: &str, branch: &str) {
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":session,"branchId":branch,"text":"one turn"}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc(
                "session.idle",
                json!({"sessionId":session,"branchId":branch}),
            )
            .await,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_line_cut_at_zero_answers_context_usage_and_compacts() {
    let server = launch_context().await;
    let agent = server.agent().unwrap();
    let mut client = Client::new_version(&server, false, 3).await;
    let created = client
        .rpc(
            "session.create",
            json!({
                "cwd": std::env::temp_dir(),
                "tools": ["branch"],
                "permissionPolicy": {"allowedTools": ["branch"]}
            }),
        )
        .await;
    assert_ok(&created);
    let sid = created["result"]["sessionId"].as_str().unwrap().to_owned();
    let session = agent.get_session(&sid).unwrap();
    session
        .configure(
            ConfigureUpdate {
                model: Some(ModelSelection {
                    provider_id: "mock".into(),
                    model_id: "deterministic".into(),
                    capability_snapshot: None,
                    config: None,
                    metadata: None,
                }),
                ..Default::default()
            },
            "main",
        )
        .await
        .unwrap();
    settle(&mut client, &sid, "main").await;
    let opened = client
        .rpc(
            "session.branch_create",
            json!({"sessionId":sid,"parentBranchId":"main","at":0,"prompt":"work on the line"}),
        )
        .await;
    assert_ok(&opened);
    let line = opened["result"]["branch"]["branchId"]
        .as_str()
        .unwrap()
        .to_owned();
    settle(&mut client, &sid, &line).await;
    let usage = client
        .rpc(
            "session.context.usage",
            json!({"sessionId":sid,"branchId":line}),
        )
        .await;
    assert_ok(&usage);
    assert_eq!(usage["result"]["contextWindow"], 16_000.0, "{usage}");
    let compact = client
        .rpc(
            "session.context.compact",
            json!({"sessionId":sid,"branchId":line}),
        )
        .await;
    assert_ok(&compact);
    assert_eq!(compact["result"]["status"], "skipped", "{compact}");
}
