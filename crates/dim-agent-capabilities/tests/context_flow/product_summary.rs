//! The product summary strategy with the product window facts: every
//! summarizer request fits the model's input budget as `count_tokens`
//! counts it, also for CJK-heavy history (ContextCompaction §5.1, §5.2).
use super::*;
use dim_agent_capabilities::{
    agent_loop::AgentLoop, context_model::ProductContextModel,
    summary_strategy::ProductSummaryStrategyPlugin,
};

const WINDOW: u64 = 4_000;
const OUTPUT: u64 = 100;

/// Records summarizer requests; answers every request with short text.
#[derive(Default)]
struct Recorder(std::sync::Mutex<Vec<Arc<ModelRequest>>>);
impl ModelDriver for Recorder {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        _: ModelConnection,
        _: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        let summarizer = matches!(request.system.as_deref(),
            Some(SystemPrompt::Text(text)) if text.starts_with("You summarize a coding session"));
        if summarizer {
            self.0.lock().unwrap().push(request);
        }
        let text = if summarizer { "摘要" } else { "好" };
        Box::pin(async move {
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "reply".into(),
                }),
                Ok(ModelChunk::Text(text.into())),
                Ok(ModelChunk::MessageStop {
                    message_id: "reply".into(),
                    reason: ModelStopReason::EndTurn,
                }),
            ])) as ModelStream)
        })
    }
}
struct Local;
impl ResolveModelConnection for Local {
    fn resolve(
        &self,
        _: Arc<ModelRequest>,
        _: CancellationToken,
    ) -> HostFuture<'_, ResolvedModelConnection> {
        Box::pin(async {
            Ok(ResolvedModelConnection {
                connection: ModelConnection::Local,
                failure_policy: None,
                recovery: None,
            })
        })
    }
}

fn cjk(count: usize) -> String {
    "压缩上下文".chars().cycle().take(count).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_summary_request_fits_the_window_for_cjk_history() {
    let recorder = Arc::new(Recorder::default());
    let root = Ctx::root().unwrap();
    root.provide(ModelHost(recorder.clone())).unwrap();
    root.provide(ProviderConnectionResolver(Arc::new(Local)))
        .unwrap();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder
        .session_value(Arc::new(ModelHost(recorder.clone())))
        .session_value(Arc::new(ToolHost(Arc::new(MockTools))))
        .session_value(Arc::new(HistoryRenderer::default()))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(Some(Arc::new(
            ProductContextModel,
        )))))
        .session_plugin(ProductSummaryStrategyPlugin::new)
        .session_plugin(DefaultPrepareModelPlugin::new);
    let snapshot: ModelCapabilitySnapshot = serde_json::from_value(serde_json::json!({
        "target": {"providerId": "local", "modelId": "small"},
        "capabilities": {"modalities": ["text"], "toolCalling": true, "streaming": true,
            "structuredOutput": false, "vision": false, "audio": false, "reasoning": false,
            "contextWindow": WINDOW, "maxOutputTokens": OUTPUT}
    }))
    .unwrap();
    defaults::install_with_prompt(
        &mut builder,
        Arc::new(EmptyPromptHost),
        Some(ConfigureUpdate {
            sources: Vec::new(),
            model: Some(ModelSelection {
                provider_id: "local".into(),
                model_id: "small".into(),
                capability_snapshot: Some(snapshot),
                config: None,
                metadata: None,
            }),
        }),
    );
    let agent = builder.build_in(&root);
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    // One message larger than the whole window: budget recovery compacts
    // it, and the summarizer must truncate it. Then turns that fill the
    // window again, and a manual compaction.
    for text in [cjk(6_000), cjk(1_800), cjk(1_800), cjk(1_800)] {
        session
            .submit(text, InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
    }
    let command = session
        .service::<AgentLoop>("main")
        .unwrap()
        .compact()
        .unwrap();
    let events = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let events = session.history("main", None).await.unwrap();
            if events.iter().any(|event| {
                matches!(&event.kind,
                EventKind::ContextCompaction(data) if data.command == Some(command))
            }) {
                break events;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let requests = recorder.0.lock().unwrap().clone();
    assert!(!requests.is_empty());
    for request in &requests {
        let model = ProductContextModel
            .resolve(request.clone(), CancellationToken::new())
            .await
            .unwrap();
        let tokens = model.count_tokens(request);
        assert!(
            tokens <= (WINDOW - OUTPUT) as f64,
            "summary request of {tokens} tokens exceeds the input budget"
        );
        assert_eq!(model.output_tokens(), OUTPUT as f64);
    }
    // The oversized CJK message was truncated, not sent whole.
    assert!(events.iter().any(|event| matches!(&event.kind,
        EventKind::ContextCompaction(ContextCompactionData {
            outcome: ContextCompactionResult::Completed { diagnostics, .. }, ..
        }) if diagnostics.strategy.details["compactedToolPayloadCount"] == 1)));
    agent.close().await.unwrap();
    root.shutdown().await.unwrap();
}
