use axum::{
    Json, Router, body::Body, extract::State, http::HeaderMap, response::Response, routing::post,
};
use dim_agent_aimux::{AimuxModels, AimuxPlugin, ConfiguredModelsPlugin, config::ModelConfigFile};
use dim_agent_capabilities::{adapters::MockTools, defaults, history::HistoryRenderer, host::*};
use dim_agent_runtime::*;
use dim_agent_types::*;
use futures_util::StreamExt;
use rutis::host::CtxHost;
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
struct Endpoint {
    requests: Arc<Mutex<Vec<Value>>>,
    authorizations: Arc<Mutex<Vec<Option<String>>>>,
}
struct TestServer {
    endpoint: Endpoint,
    url: String,
    cancel: CancellationToken,
    join: tokio::task::JoinHandle<()>,
}
impl TestServer {
    async fn start() -> Self {
        let endpoint = Endpoint::default();
        let router = Router::new()
            .route("/v1/chat/completions", post(respond))
            .with_state(endpoint.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let join = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await
                .unwrap();
        });
        Self {
            endpoint,
            url,
            cancel,
            join,
        }
    }
    fn models(&self) -> Arc<AimuxModels> {
        Arc::new(AimuxModels::from_config(configuration(&self.url)).unwrap())
    }
    async fn close(self) {
        self.cancel.cancel();
        self.join.await.unwrap();
    }
}
fn configuration(url: &str) -> ModelConfigFile {
    serde_json::from_value(json!({"defaultModel":{"providerId":"local","modelId":"test-model"},"providers":[{"id":"local","kind":"openai-compatible","baseUrl":url,"models":[{"id":"test-model","config":{"maxTokens":4096,"temperature":0}}]}]})).unwrap()
}
fn frame(delta: Value, finish: Value) -> String {
    format!(
        "data: {}\n\n",
        json!({"id":"remote-id","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    )
}
async fn respond(
    State(endpoint): State<Endpoint>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    endpoint.requests.lock().unwrap().push(request.clone());
    endpoint.authorizations.lock().unwrap().push(
        headers
            .get("authorization")
            .map(|value| value.to_str().unwrap().to_owned()),
    );
    let messages = request["messages"].as_array().unwrap();
    let last = messages.last().unwrap();
    let prompt = last["content"].as_str().unwrap_or("");
    if prompt == "reject401" {
        return Response::builder()
            .status(401)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"error":{"message":"Expired credential","code":"token_expired"}}"#,
            ))
            .unwrap();
    }
    let (data, hold) = if last["role"] == "tool" {
        (
            frame(json!({"content":"tool completed"}), json!("stop")) + "data: [DONE]\n\n",
            false,
        )
    } else if prompt == "tool" || prompt == "unauthorized" || prompt == "length" {
        let name = if prompt == "unauthorized" {
            "forbidden"
        } else {
            "echo"
        };
        let mut data = frame(
            json!({"tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":name,"arguments":"{\"text\":"}}]}),
            Value::Null,
        );
        data += &frame(
            json!({"tool_calls":[{"index":0,"function":{"arguments":"\"result text\"}"}}]}),
            Value::Null,
        );
        data += &frame(
            json!({}),
            json!(if prompt == "length" {
                "length"
            } else {
                "tool_calls"
            }),
        );
        data += "data: [DONE]\n\n";
        (data, false)
    } else if prompt == "wait" {
        (frame(json!({"content":"started"}), Value::Null), true)
    } else if prompt == "truncated" {
        (frame(json!({"content":"partial"}), Value::Null), false)
    } else {
        let mut data = frame(json!({"reasoning_content":"thinking"}), Value::Null);
        data += &frame(json!({"content":"hello"}), json!("stop"));
        data += &format!(
            "data: {}\n\n",
            json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":2}}})
        );
        data += "data: [DONE]\n\n";
        (data, true)
    };
    let body = Body::from_stream(async_stream::stream! {
        yield Ok::<_, Infallible>(data);
        if hold { std::future::pending::<()>().await; }
    });
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(body)
        .unwrap()
}
fn builder(models: Arc<AimuxModels>, tools: Arc<dyn ToolDriver>) -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig {
        model: models.default_model().clone(),
        ..Default::default()
    });
    defaults::install_with_process_model(&mut builder);
    builder
        .session_value(Arc::new(HistoryRenderer::default()))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_value(Arc::new(CompactionStrategyHost(None)))
        .session_value(Arc::new(ToolHost(tools)));
    dim_agent_aimux::install(&mut builder);
    builder
}
struct ModelAgent {
    agent: Agent,
    root: rutis::Ctx,
}
impl std::ops::Deref for ModelAgent {
    type Target = Agent;
    fn deref(&self) -> &Agent {
        &self.agent
    }
}
impl ModelAgent {
    async fn close(&self) -> dim_agent_runtime::Result<()> {
        self.agent.close().await?;
        self.root.shutdown().await.map_err(Into::into)
    }
}
fn model_agent(models: Arc<AimuxModels>, custom: Option<AgentBuilder>) -> ModelAgent {
    let root = rutis::Ctx::root().unwrap();
    root.plugin(AimuxPlugin);
    root.plugin(ConfiguredModelsPlugin(models.clone()));
    let agent = custom
        .unwrap_or_else(|| builder(models, Arc::new(MockTools)))
        .build_in(&root);
    ModelAgent { agent, root }
}
fn session_config() -> SessionConfig {
    SessionConfig {
        model: ModelSelection {
            provider_id: "local".into(),
            model_id: "test-model".into(),
            capability_snapshot: None,
            config: None,
            metadata: None,
        },
        system_prompt: "system instructions".into(),
        tools: Some(vec!["echo".into()]),
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_requests_use_connection_snapshots_and_release_retained_streams_on_close() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let first = TestServer::start().await;
        let second = TestServer::start().await;
        let root = rutis::Ctx::root().unwrap();
        root.plugin(AimuxPlugin).await.unwrap();
        let host = root.get::<ModelHost>().unwrap();
        let request = |text: &str| Arc::new(ModelRequest::new("logical-provider", "test-model", vec![Message::text(Role::User, text)]));
        let connection = |server: &TestServer, key: &str| ModelConnection::Http(HttpModelConnection {
 api_key: None,
            network: Default::default(),
            protocol: ModelProtocol::OpenaiCompatible, base_url: server.url.clone(),
            bearer: Some(key.to_owned()), headers: Default::default(), preset: None,
        });
        for (server, key) in [(&first, "first-secret"), (&second, "second-secret")] {
            let mut stream = host.0.invoke(request("text"), connection(server, key), CancellationToken::new()).await.unwrap();
            while let Some(chunk) = stream.next().await { chunk.unwrap(); }
            assert_eq!(*server.endpoint.authorizations.lock().unwrap(), [Some(format!("Bearer {key}"))]);
            assert!(!server.endpoint.requests.lock().unwrap()[0].to_string().contains(key));
        }
        let failure = host.0.invoke(request("reject401"), connection(&second, "expired-secret"), CancellationToken::new()).await.err().unwrap();
        assert!(matches!(&failure, HostError::Failed { kind: HostErrorKind::Http { status: Some(401), code: Some(code) }, .. } if code == "token_expired"));
        assert!(!failure.to_string().contains("expired-secret"));
        let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = unused.local_addr().unwrap();
        drop(unused);
        let failure = host.0.invoke(request("text"), ModelConnection::Http(HttpModelConnection {
 api_key: None,
            network: Default::default(),
            protocol: ModelProtocol::OpenaiCompatible,
            base_url: format!("http://user:url-secret@{address}/v1?api-key=query-secret"),
            headers: Default::default(), bearer: None, preset: None,
        }), CancellationToken::new()).await.err().unwrap();
        assert!(!failure.to_string().contains("query-secret"));
        assert!(!failure.to_string().contains("url-secret"));
        let failure = host.0.invoke(request("text"), ModelConnection::Http(HttpModelConnection {
 api_key: None,
            network: Default::default(),
            protocol: ModelProtocol::OpenaiCompatible, base_url: first.url.clone(),
            headers: Default::default(), bearer: Some("header-secret\n".into()), preset: None,
        }), CancellationToken::new()).await.err().unwrap();
        assert!(!failure.to_string().contains("header-secret"));
        let mut unpolled = host.0.invoke(request("wait"), connection(&first, "unpolled"), CancellationToken::new()).await.unwrap();
        let mut partial = host.0.invoke(request("wait"), connection(&second, "partial"), CancellationToken::new()).await.unwrap();
        assert!(matches!(partial.next().await.unwrap().unwrap(), ModelChunk::MessageStart { .. }));
        assert!(matches!(partial.next().await.unwrap().unwrap(), ModelChunk::Text(_)));
        root.shutdown().await.unwrap();
        for stream in [&mut unpolled, &mut partial] {
            assert!(matches!(stream.next().await, Some(Err(HostError::Failed { kind: HostErrorKind::OwnerClosed, .. }))));
            assert!(stream.next().await.is_none());
        }
        assert!(matches!(host.0.invoke(request("text"), connection(&first, "old"), CancellationToken::new()).await,
            Err(HostError::Failed { kind: HostErrorKind::OwnerClosed, .. })));
        let next = rutis::Ctx::root().unwrap();
        next.plugin(AimuxPlugin).await.unwrap();
        let mut stream = next.get::<ModelHost>().unwrap().0.invoke(request("text"), connection(&first, "new"), CancellationToken::new()).await.unwrap();
        while let Some(chunk) = stream.next().await { chunk.unwrap(); }
        next.shutdown().await.unwrap();
        first.close().await; second.close().await;
    }).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_http_text_usage_tools_and_continuation_follow_agent_events() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let server = TestServer::start().await;
        let agent = model_agent(server.models(), None);
        let session = agent.create_session(session_config()).await.unwrap();
        session.submit("text".into(), InputMode::Queue, "main").await.unwrap();
        session.idle("main").await.unwrap();
        let events = session.history("main", None).await.unwrap();
        assert!(events.iter().any(|e| matches!(&e.kind, EventKind::Thinking(data) if data.delta == "thinking")));
        assert!(events.iter().any(|e| matches!(&e.kind, EventKind::Text(data) if data.delta == "hello")));
        assert!(events.iter().any(|e| matches!(&e.kind, EventKind::ModelUsage(data) if data.usage.prompt_tokens == 10. && data.usage.total_tokens == 13. && data.usage.cache_read_tokens == Some(2.))));
        let call = events.iter().find_map(|e| if let EventKind::ModelCall(call) = &e.kind { Some(call) } else { None }).unwrap();
        let metadata = call.request.metadata.as_ref().unwrap();
        assert_eq!(metadata.session_id.as_deref(), Some(session.id()));
        assert_eq!(metadata.turn_id.as_deref(), Some(call.turn_id.as_str()));
        assert_eq!(metadata.extensions["requestId"], call.request_id);
        assert!(metadata.run_id.is_none());
        session.submit("tool".into(), InputMode::Queue, "main").await.unwrap();
        session.idle("main").await.unwrap();
        let events = session.history("main", None).await.unwrap();
        assert!(events.iter().any(|e| matches!(&e.kind, EventKind::ToolResult(data) if data.results[0].result.content == ToolContent::Text("result text".into()))));
        assert!(events.iter().any(|e| matches!(&e.kind, EventKind::Text(data) if data.delta == "tool completed")));
        assert!(matches!(events.last().unwrap().kind, EventKind::TurnEnd(TurnEndData { outcome: TurnOutcome::Completed { .. }, .. })));
        let requests = server.endpoint.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0]["model"], "test-model");
        assert_eq!(requests[0]["temperature"].as_f64(), Some(0.));
        assert_eq!(
            requests[0]["messages"][0]["content"],
            format!(
                "system instructions\n\n{}",
                include_str!("../../dim-agent-capabilities/src/input-guide.txt").trim_end()
            )
        );
        assert_eq!(requests[2]["messages"].as_array().unwrap().last().unwrap()["content"], "result text");

        let mut explicit = session_config();
        explicit.model.config = Some(ModelConfig { temperature: Some(0.375), ..Default::default() });
        explicit.model.metadata = Some(serde_json::from_value(json!({
            "sessionId": "stale-session", "turnId": "stale-turn", "requestId": "stale-request",
            "runId": "caller-run", "traceId": "caller-trace", "experiment": "selected"
        })).unwrap());
        let selected = agent.create_session(explicit).await.unwrap();
        selected.submit("text".into(), InputMode::Queue, "main").await.unwrap();
        selected.idle("main").await.unwrap();
        let events = selected.history("main", None).await.unwrap();
        let call = events.iter().find_map(|e| if let EventKind::ModelCall(call) = &e.kind { Some(call) } else { None }).unwrap();
        assert_eq!(call.request.config.as_ref().unwrap().temperature, Some(0.375));
        assert_eq!(call.request.config.as_ref().unwrap().max_tokens, None, "An explicit model configuration must not inherit catalog fields");
        let metadata = call.request.metadata.as_ref().unwrap();
        assert_eq!(metadata.session_id.as_deref(), Some(selected.id()));
        assert_eq!(metadata.turn_id.as_deref(), Some(call.turn_id.as_str()));
        assert_eq!(metadata.extensions["requestId"], call.request_id);
        assert_eq!(metadata.run_id.as_deref(), Some("caller-run"));
        assert_eq!(metadata.trace_id.as_deref(), Some("caller-trace"));
        assert_eq!(metadata.extensions["experiment"], "selected");
        let requests = server.endpoint.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[3]["temperature"].as_f64(), Some(0.375));
        assert!(requests[3].get("max_tokens").is_none());
        agent.close().await.unwrap(); server.close().await;
    }).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupt_releases_native_request_and_keeps_other_branches_usable() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let server = TestServer::start().await; let agent = model_agent(server.models(), None);
        let session = agent.create_session(session_config()).await.unwrap();
        session.submit("wait".into(), InputMode::Queue, "main").await.unwrap();
        // Streaming text is progress until its block ends (agent-029).
        while !session.get::<dim_agent_runtime::ModelProgress>("main").unwrap().is_some_and(|progress| !progress.text.is_empty()) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let branch = session.branch("main".into(), 0, BranchOptions { prompt: "text".into(), source: BranchSource::Host { operation_id: "branch".into() } }).await.unwrap();
        session.stop("main", "user stop".into()).await.unwrap();
        session.idle(&branch).await.unwrap();
        assert!(session.history("main", None).await.unwrap().iter().any(|e| matches!(&e.kind, EventKind::ModelOutputEnd(data) if matches!(data.outcome, ModelOutputOutcome::Cancelled { .. }))));
        assert!(session.history(&branch, None).await.unwrap().iter().any(|e| matches!(&e.kind, EventKind::Text(data) if data.delta == "hello")));
        session.submit("text".into(), InputMode::Queue, "main").await.unwrap();
        session.idle("main").await.unwrap();
        agent.close().await.unwrap(); server.close().await;
    }).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_native_outputs_do_not_execute_tools_or_invent_usage() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let server = TestServer::start().await; let agent = model_agent(server.models(), None);
        for prompt in ["unauthorized", "truncated", "length"] {
            let session = agent.create_session(session_config()).await.unwrap();
            session.submit(prompt.into(), InputMode::Queue, "main").await.unwrap();
            session.idle("main").await.unwrap();
            let events = session.history("main", None).await.unwrap();
            assert!(events.iter().any(|e| matches!(&e.kind, EventKind::ModelOutputEnd(data) if matches!(data.outcome, ModelOutputOutcome::Failed { .. }))), "{prompt}: {events:?}");
            // Calls a failed output started are closed, never dispatched (agent-029).
            assert!(!events.iter().any(|e| matches!(&e.kind, EventKind::ToolCall(_) | EventKind::ModelUsage(_)) || matches!(&e.kind, EventKind::ToolResult(data) if data.call_event_id.is_some())), "{prompt}: {events:?}");
            session.close().await.unwrap();
        }
        agent.close().await.unwrap(); server.close().await;
    }).await.unwrap();
}

#[tokio::test]
async fn model_configuration_rejects_unknown_models_before_session_start() {
    let server = TestServer::start().await;
    let agent = model_agent(server.models(), None);
    let mut config = session_config();
    config.model.model_id = "not-registered".into();
    assert!(agent.create_session(config).await.is_err());
    assert!(agent.sessions().is_empty());
    assert!(server.endpoint.requests.lock().unwrap().is_empty());
    agent.close().await.unwrap();
    server.close().await;
}

struct OverrideRequest {
    remove_tools: bool,
    timeline: rutis::host::Key<Timeline>,
}
impl rutis::host::DynPlugin for OverrideRequest {
    fn name(&self) -> &str {
        "test.final-model-request"
    }
    fn injects(&self) -> Vec<rutis::host::TypeKey> {
        vec![self.timeline.erase()]
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
    ) -> rutis::host::BoxFuture<'a, rutis::Result<rutis::host::Effect>> {
        Box::pin(async move {
            let timeline = ctx.require_at(&self.timeline)?;
            let remove_tools = self.remove_tools;
            timeline.before(ctx, self.name(), &["model/call"], move |mut event| {
                let EventKind::ModelCall(call) = &mut event.kind else {
                    unreachable!()
                };
                let request = Arc::make_mut(&mut call.request);
                request.provider_id = "selected".into();
                request.model_id = "selected-model".into();
                request.system = Some(Arc::new(SystemPrompt::Text("final system".into())));
                request.config = Some(ModelConfig {
                    max_tokens: Some(17.),
                    temperature: Some(0.),
                    ..Default::default()
                });
                if remove_tools {
                    request.tools = Some(Arc::new(Vec::new()));
                }
                Ok(event)
            })?;
            Ok(rutis::host::Effect::Done)
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn before_committed_request_controls_model_settings_and_tool_admission() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let original = TestServer::start().await; let selected = TestServer::start().await;
        let mut config = configuration(&original.url);
        config.providers.push(dim_agent_aimux::config::ProviderConfig {
            id: "selected".into(), kind: dim_agent_aimux::config::ProviderKind::OpenaiCompatible,
            api_key_env: None, base_url: selected.url.clone(), headers: Default::default(), compatible: None,
            models: vec![dim_agent_aimux::config::ModelEntry { id: "selected-model".into(), config: ModelConfig { max_tokens: Some(99.), temperature: Some(0.8), ..Default::default() } }],
        });
        let models = Arc::new(AimuxModels::from_config(config).unwrap());
        for remove_tools in [false, true] {
            let mut builder = builder(models.clone(), Arc::new(MockTools));
            builder.require_feature(Feature::new("test.final-model-request").branch("test.final-model-request", move |names| Ok(Box::new(OverrideRequest { remove_tools, timeline: names.key()? }))));
            let agent = model_agent(models.clone(), Some(builder)); let session = agent.create_session(session_config()).await.unwrap();
            session.submit("tool".into(), InputMode::Queue, "main").await.unwrap();
            session.idle("main").await.unwrap();
            let events = session.history("main", None).await.unwrap();
            if remove_tools {
                assert!(!events.iter().any(|event| matches!(event.kind, EventKind::ToolCall(_) | EventKind::ToolResult(_))));
                assert!(events.iter().any(|event| matches!(&event.kind, EventKind::ModelOutputEnd(end) if matches!(end.outcome, ModelOutputOutcome::Failed { .. }))));
            } else {
                assert!(events.iter().any(|event| matches!(&event.kind, EventKind::Text(text) if text.delta == "tool completed")));
            }
            let requests = std::mem::take(&mut *selected.endpoint.requests.lock().unwrap());
            assert_eq!(requests.len(), if remove_tools { 1 } else { 2 });
            for request in requests {
                assert_eq!(request["model"], "selected-model");
                assert_eq!(request["messages"][0]["content"], "final system");
                assert_eq!(request["max_tokens"], 17);
                assert_eq!(request["temperature"].as_f64(), Some(0.));
                assert_eq!(request.get("tools").is_none(), remove_tools);
            }
            agent.close().await.unwrap();
        }
        assert!(original.endpoint.requests.lock().unwrap().is_empty());
        original.close().await; selected.close().await;
    }).await.unwrap();
}

#[path = "model_flow/native.rs"]
mod native;
