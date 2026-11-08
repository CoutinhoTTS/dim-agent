//! The `codemode` tool through the real Agent runtime: the model calls it,
//! the program calls `echo` through the branch's pinned ToolBinding and
//! Approval, and the outer `step/tool-result` carries the program's value.
use dim_agent_capabilities::{
    adapters::{DefaultPrepareModelPlugin, MockTools},
    defaults,
    history::HistoryRenderer,
    host::*,
};
use dim_agent_runtime::{
    AgentBuilder, AgentConfig, Feature, MemoryTimelineStore, Session, SessionConfig,
    testing::{MinimalHost, branch_events},
};
use dim_agent_types::*;
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Script {
    batches: Mutex<VecDeque<Vec<ToolCall>>>,
    requests: Mutex<Vec<Arc<ModelRequest>>>,
}

impl Script {
    fn push(&self, calls: &[(&str, &str, serde_json::Value)]) {
        let calls = calls
            .iter()
            .map(|(name, id, input)| ToolCall {
                id: (*id).into(),
                name: (*name).into(),
                input: input.as_object().unwrap().clone(),
            })
            .collect();
        self.batches.lock().unwrap().push_back(calls);
    }
}

impl ModelDriver for Script {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        _: ModelConnection,
        _: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            let id = format!("m{}", self.requests.lock().unwrap().len());
            let mut chunks = vec![Ok(ModelChunk::MessageStart {
                message_id: id.clone(),
            })];
            let reason = match self.batches.lock().unwrap().pop_front() {
                Some(calls) => {
                    for call in calls {
                        chunks.push(Ok(ModelChunk::ToolStart {
                            id: call.id.clone(),
                            name: call.name.clone(),
                        }));
                        chunks.push(Ok(ModelChunk::ToolUse(call)));
                    }
                    ModelStopReason::ToolUse
                }
                None => {
                    chunks.push(Ok(ModelChunk::Text("done".into())));
                    chunks.push(Ok(ModelChunk::TextEnd));
                    ModelStopReason::EndTurn
                }
            };
            chunks.push(Ok(ModelChunk::MessageStop {
                message_id: id,
                reason,
            }));
            Ok(Box::pin(futures_util::stream::iter(chunks)) as ModelStream)
        })
    }
}

struct Fixture {
    agent: MinimalHost,
    model: Arc<Script>,
}

impl Fixture {
    async fn new() -> Self {
        let model = Arc::new(Script::default());
        let mut builder = AgentBuilder::new(AgentConfig::default());
        builder
            .timeline_store(Arc::new(MemoryTimelineStore::default()))
            .session_value(Arc::new(HistoryRenderer::default()))
            .process_value(Arc::new(ModelHost(model.clone())))
            .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
            .session_value(Arc::new(ContextModelHost(None)))
            .session_value(Arc::new(CompactionStrategyHost(None)))
            .require_feature(
                Feature::new("PrepareModelHost").session("PrepareModelHost", |names| {
                    Ok(Box::new(DefaultPrepareModelPlugin::new(names)?))
                }),
            );
        defaults::install(&mut builder);
        defaults::tool_host(&mut builder, Arc::new(MockTools));
        dim_codemode::install_required(&mut builder);
        let agent = MinimalHost::start(&builder).await.unwrap();
        Self { agent, model }
    }

    async fn session(&self) -> Session {
        self.session_with_policy(PermissionPolicy::default()).await
    }

    async fn session_with_policy(&self, permission_policy: PermissionPolicy) -> Session {
        self.agent
            .create_session(SessionConfig {
                tools: Some(vec!["codemode".into(), "echo".into()]),
                interactive: false,
                permission_policy,
                ..SessionConfig::default()
            })
            .await
            .unwrap()
    }

    async fn shutdown(self) {
        self.agent.close().await.unwrap();
    }
}

async fn result(session: &Session, call_id: &str) -> ToolResult {
    tokio::time::timeout(Duration::from_secs(10), session.idle("main"))
        .await
        .unwrap()
        .unwrap();
    let events = branch_events(session, "main", ..).await.unwrap();
    events
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::ToolResult(data) => data
                .results
                .iter()
                .find(|result| result.call_id == call_id)
                .map(|result| result.result.clone()),
            _ => None,
        })
        .expect("the codemode result is committed")
}

fn text(result: &ToolResult) -> &str {
    match &result.content {
        ToolContent::Text(text) => text,
        ToolContent::Blocks(_) => panic!("expected text"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_program_calls_a_pinned_tool_and_the_outer_result_carries_its_value() {
    let fixture = Fixture::new().await;
    let session = fixture.session().await;
    let code = r#"const value = await tools.call("echo", {text: "hello"}); return {content: value.content};"#;
    fixture
        .model
        .push(&[("codemode", "c1", json!({ "code": code }))]);
    session
        .submit("run".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    let result = result(&session, "c1").await;
    assert_eq!(text(&result), r#"{"content":"hello"}"#);
    assert_eq!(result.is_error, None);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_denied_child_is_a_catchable_program_error() {
    let fixture = Fixture::new().await;
    let session = fixture
        .session_with_policy(PermissionPolicy {
            disallowed_tools: vec!["echo".into()],
            ..PermissionPolicy::default()
        })
        .await;
    let code = r#"try { await tools.call("echo", {text: "x"}); return "allowed"; } catch (error) { return error.message; }"#;
    fixture
        .model
        .push(&[("codemode", "c1", json!({ "code": code }))]);
    session
        .submit("run".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    let result = result(&session, "c1").await;
    let value: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    assert!(
        value.as_str().unwrap().contains("echo"),
        "the denial names the tool: {value}"
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_sessions_with_main_branches_keep_their_own_runners() {
    let fixture = Fixture::new().await;
    let first = fixture.session().await;
    let second = fixture.session().await;
    fixture
        .model
        .push(&[("codemode", "c1", json!({ "code": "return \"first\";" }))]);
    first
        .submit("run first".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    let first_result = result(&first, "c1").await;
    fixture
        .model
        .push(&[("codemode", "c2", json!({ "code": "return \"second\";" }))]);
    second
        .submit("run second".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    let second_result = result(&second, "c2").await;
    assert_eq!(text(&first_result), "\"first\"");
    assert_eq!(text(&second_result), "\"second\"");
    fixture.shutdown().await;
}
