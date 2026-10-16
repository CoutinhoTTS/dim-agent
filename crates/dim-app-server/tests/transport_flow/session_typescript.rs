use super::*;
use dim_agent_capabilities::{
    adapters::{DefaultPrepareModelPlugin, EchoModel},
    history::HistoryRenderer,
    host::{
        CompactionStrategyHost, ContextModelHost, IdentityContext, ModelChunk, ModelConnection,
        ModelDriver, ModelHost, ModelStopReason, ModelStream, PrepareContextHost,
    },
    methods::SessionMethods,
    tool_registry::definition,
};
use dim_agent_runtime::SessionScope;
use dim_agent_types::{MessageContent, MessagePart, ModelRequest, Role};
use dim_app_server::session_typescript::SessionTypeScriptConfig;
use std::path::PathBuf;

fn fixture() -> SessionTypeScriptConfig {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    SessionTypeScriptConfig {
        node: std::env::var_os("DIM_TEST_NODE")
            .expect("DIM_TEST_NODE")
            .into(),
        js_runtime: std::env::var_os("DIM_TEST_JS_RUNTIME")
            .expect("DIM_TEST_JS_RUNTIME")
            .into(),
        anchor: root.join("runtime/package.json"),
        rows: vec![json!({
            "id": "typescript", "name": root.join("runtime/fixtures/session-part.ts"),
            "provides": serde_json::from_str::<Value>(include_str!("../../../../runtime/fixtures/session-part.json")).unwrap(),
            "config": {},
        })],
    }
}

// This host explicitly requires the fixture's method provider, so the
// Active-at-first-read scenario does not depend on scheduler timing.
struct ActiveFixture(SessionScope);
impl TypedPlugin for ActiveFixture {
    type Deps = (rutis::KeyedGate<SessionMethods>,);
    fn name(&self) -> &str {
        "ActiveTypeScriptFixture"
    }
    fn apply<'a>(
        &'a self,
        _: &'a rutis::Ctx,
        _: Self::Deps,
    ) -> rutis::BoxFuture<'a, Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async { Ok(rutis::Effect::Done) })
    }
}
impl Capability for ActiveFixture {
    fn keys(&self) -> KeysOf<Self> {
        (rutis::DepKey::dynamic("session.typescript").instance(self.0.instance),)
    }
}

struct CheckedModel(tokio::sync::mpsc::UnboundedSender<Vec<String>>);
impl ModelDriver for CheckedModel {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        self.0
            .send(
                request
                    .tools
                    .as_deref()
                    .into_iter()
                    .flatten()
                    .map(|tool| tool.name.clone())
                    .collect(),
            )
            .unwrap();
        let batch = request.messages.last().is_some_and(|message| {
            message.role == Role::User
                && match &message.content {
                    MessageContent::Text(text) => text == "batch",
                    MessageContent::Parts(parts) => parts.iter().any(
                        |part| matches!(part, MessagePart::Text { text, .. } if text == "batch"),
                    ),
                }
        });
        if batch {
            return Box::pin(async {
                let chunks = vec![
                    ModelChunk::MessageStart {
                        message_id: "batch".into(),
                    },
                    ModelChunk::ToolStart {
                        id: "fast".into(),
                        name: "ts_echo".into(),
                    },
                    ModelChunk::ToolArguments {
                        id: "fast".into(),
                        name: "ts_echo".into(),
                        delta: r#"{"text":"fast"}"#.into(),
                    },
                    ModelChunk::ToolUse(ToolCall {
                        id: "fast".into(),
                        name: "ts_echo".into(),
                        input: serde_json::from_value(json!({"text":"fast"})).unwrap(),
                    }),
                    ModelChunk::ToolStart {
                        id: "slow".into(),
                        name: "native_slow".into(),
                    },
                    ModelChunk::ToolArguments {
                        id: "slow".into(),
                        name: "native_slow".into(),
                        delta: "{}".into(),
                    },
                    ModelChunk::ToolUse(ToolCall {
                        id: "slow".into(),
                        name: "native_slow".into(),
                        input: Default::default(),
                    }),
                    ModelChunk::MessageStop {
                        message_id: "batch".into(),
                        reason: ModelStopReason::ToolUse,
                    },
                ];
                Ok(Box::pin(futures_util::stream::iter(chunks.into_iter().map(Ok))) as ModelStream)
            });
        }
        EchoModel.invoke(request, connection, cancel)
    }
}

struct SlowTools(Arc<tokio::sync::Notify>);
impl ToolDriver for SlowTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        let mut tools = MockTools.definitions();
        tools.push(definition(
            "native_slow",
            "Held batch tool",
            json!({"type":"object"}),
        ));
        tools
    }
    fn execute(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        if call.name != "native_slow" {
            return MockTools.execute(call, context, cancel);
        }
        Box::pin(async move {
            self.0.notified().await;
            assert!(
                !cancel.is_cancelled(),
                "another tool's normal completion cancelled this batch"
            );
            Ok(ToolResult::text("native slow completed"))
        })
    }
}

async fn start(
    mut config: ServerConfig,
    active: bool,
) -> (
    RunningServer,
    tokio::sync::mpsc::UnboundedReceiver<Vec<String>>,
    Arc<tokio::sync::Notify>,
) {
    config.unix_path = Some(std::env::temp_dir().join(format!(
        "dm-ts-{}/server.sock",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )));
    let (builder, recv, release) = setup(active);
    let server = serve(builder, config).await.unwrap();
    (server, recv, release)
}

fn setup(
    active: bool,
) -> (
    AgentBuilder,
    tokio::sync::mpsc::UnboundedReceiver<Vec<String>>,
    Arc<tokio::sync::Notify>,
) {
    let (send, recv) = tokio::sync::mpsc::unbounded_channel();
    let release = Arc::new(tokio::sync::Notify::new());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::install(&mut builder);
    builder
        .session_value(Arc::new(HistoryRenderer::default()))
        .session_value(Arc::new(ModelHost(Arc::new(CheckedModel(send)))))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_value(Arc::new(CompactionStrategyHost(None)))
        .session_plugin(DefaultPrepareModelPlugin::new);
    defaults::tool_host(&mut builder, Arc::new(SlowTools(release.clone())));
    if active {
        builder.session_plugin(|scope| ActiveFixture(*scope));
    }
    (builder, recv, release)
}

async fn new_session(client: &mut Client, tools: Value) -> String {
    let mut params = create(tools);
    params["permissionPolicy"]["allowedTools"] = json!(["ts_echo", "native_slow", "echo", "wait"]);
    let created = client.rpc("session.create", params).await;
    assert_ok(&created);
    created["result"]["sessionId"].as_str().unwrap().into()
}

async fn submit(client: &mut Client, session: &str, text: &str) {
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":session,"branchId":"main","text":text}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc(
                "session.idle",
                json!({"sessionId":session,"branchId":"main"}),
            )
            .await,
    );
}

async fn started(client: &mut Client, id: u64) {
    loop {
        let notice = client.notice("management.progress").await;
        if notice["params"]["id"] == id && notice["params"]["data"]["phase"] == "started" {
            return;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_session_typescript_tools_methods_and_generation_lifetimes() {
    let (server, mut requests, release) = start(
        ServerConfig {
            session_typescript: Some(fixture()),
            ..Default::default()
        },
        true,
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let first = new_session(&mut client, json!(["ts_echo", "native_slow", "echo"])).await;
    let second = new_session(&mut client, json!(["ts_echo", "echo"])).await;
    submit(&mut client, &first, r#"tool:ts_echo {"text":"first"}"#).await;
    assert!(requests.recv().await.unwrap().contains(&"ts_echo".into()));
    let status = client
        .rpc("session.typescript.status", json!({"sessionId":first}))
        .await;
    assert_eq!(status["result"]["calls"], 1);
    let other = client
        .rpc("session.typescript.status", json!({"sessionId":second}))
        .await;
    assert_eq!(other["result"]["calls"], 0);
    assert_eq!(status["result"]["pid"], other["result"]["pid"]);
    assert_ne!(
        status["result"]["generation"],
        other["result"]["generation"]
    );
    let listed = client
        .rpc("session.methods", json!({"sessionId":first}))
        .await;
    assert!(
        listed["result"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decl| decl["name"] == "session.typescript.drain")
    );

    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":first,"branchId":"main","text":"batch"}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc("session.typescript.wait_fast", json!({"sessionId":first}))
            .await,
    );
    release.notify_one();
    assert_ok(
        &client
            .rpc("session.idle", json!({"sessionId":first,"branchId":"main"}))
            .await,
    );
    let session = server.agent().unwrap().get_session(&first).unwrap();
    let history = session.history("main", None).await.unwrap();
    let results: Vec<_> = history
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ToolResult(result) => Some(result),
            _ => None,
        })
        .collect();
    assert!(results.iter().any(|batch| {
        batch.results.iter().any(|result| {
            result.call_id == "slow"
                && result.result.content
                    == dim_agent_types::ToolContent::Text("native slow completed".into())
        })
    }));
    let first_result = &results[0].results[0].result;
    let dim_agent_types::ToolContent::Text(text) = &first_result.content else {
        panic!("text result")
    };
    let call: Value = serde_json::from_str(text).unwrap();
    assert_eq!(call["sessionId"], first);
    assert_eq!(call["branchId"], "main");
    assert_eq!(call["arguments"]["text"], "first");
    assert!(!call["callId"].as_str().unwrap().is_empty());
    assert!(!call["turnId"].as_str().unwrap().is_empty());
    assert!(call["config"]["cwd"].is_string());

    let error = client
        .rpc("session.typescript.error", json!({"sessionId":first}))
        .await;
    assert_eq!(error["error"]["code"], -32011);
    assert_eq!(
        error["error"]["data"]["appError"],
        format!("owned_{}", status["result"]["generation"].as_str().unwrap())
    );
    assert_eq!(
        client
            .rpc("session.typescript.malformed", json!({"sessionId":first}))
            .await["error"]["code"],
        -32603
    );
    submit(&mut client, &first, r#"tool:ts_echo {"text":"malformed"}"#).await;
    assert!(session.history("main", None).await.unwrap().iter().any(|event| matches!(&event.kind, EventKind::ToolResult(batch) if batch.results.iter().any(|result| result.result.is_error == Some(true)))));

    assert_ok(&client.rpc("session.submit", json!({"sessionId":first,"branchId":"main","text":r#"tool:ts_echo {"text":"cancel"}"#})).await);
    assert_ok(
        &client
            .rpc("session.typescript.wait_tool", json!({"sessionId":first}))
            .await,
    );
    assert_ok(
        &client
            .rpc("session.stop", json!({"sessionId":first,"branchId":"main"}))
            .await,
    );
    assert_ok(
        &client
            .rpc("session.idle", json!({"sessionId":first,"branchId":"main"}))
            .await,
    );

    let retained = session
        .session_named_service::<SessionMethods>("session.typescript")
        .unwrap();
    let report = client
        .send("session.typescript.report", json!({"sessionId":first}))
        .await;
    started(&mut client, report).await;
    let drain = client
        .send("session.typescript.drain", json!({"sessionId":first}))
        .await;
    started(&mut client, drain).await;
    let mut closer = Client::new_version(&server, false, 3).await;
    assert_ok(
        &closer
            .rpc("session.close", json!({"sessionId":first}))
            .await,
    );
    assert_eq!(
        client.response(report).await["error"]["data"]["appError"],
        "operation_interrupted"
    );
    let drained = client.response(drain).await;
    assert_eq!(drained["result"]["cancelled"], true);
    assert_eq!(drained["result"]["alive"], true);
    assert_eq!(
        retained
            .accept(
                "session.typescript.status".into(),
                json!({}),
                CancellationToken::new(),
                |_| {}
            )
            .err()
            .unwrap()
            .data
            .unwrap()
            .app_error,
        "service_reloading"
    );
    submit(&mut client, &second, r#"tool:ts_echo {"text":"second"}"#).await;
    assert_eq!(
        client
            .rpc("session.typescript.status", json!({"sessionId":second}))
            .await["result"]["calls"],
        1
    );
    server.shutdown();
    server.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_session_typescript_restore_fork_delete_and_disabled_parts() {
    let home = std::env::temp_dir().join(format!(
        "dm-ts-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    let config = ServerConfig {
        database_path: Some(home.join("sessions.sqlite")),
        trace_directory: home.join("trace"),
        session_typescript: Some(fixture()),
        ..Default::default()
    };
    let (server, _requests, _) = start(config.clone(), true).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let session_id = new_session(&mut client, json!(["ts_echo", "echo"])).await;
    submit(&mut client, &session_id, r#"tool:ts_echo {"text":"saved"}"#).await;
    drop(client);
    server.shutdown();
    server.join().await.unwrap();

    let (server, mut requests, _) = start(config.clone(), true).await;
    assert!(server.agent().unwrap().get_session(&session_id).is_none());
    let mut client = Client::new_version(&server, false, 3).await;
    assert_ok(
        &client
            .rpc("session.typescript.status", json!({"sessionId":session_id}))
            .await,
    );
    submit(
        &mut client,
        &session_id,
        r#"tool:ts_echo {"text":"restored"}"#,
    )
    .await;
    assert!(requests.recv().await.unwrap().contains(&"ts_echo".into()));
    assert_eq!(
        client
            .rpc("session.typescript.status", json!({"sessionId":session_id}))
            .await["result"]["calls"],
        1
    );
    let source = server.agent().unwrap().get_session(&session_id).unwrap();
    let forked = client
        .rpc(
            "session.fork",
            json!({"sessionId":session_id,"at":source.sequence("main").unwrap()}),
        )
        .await;
    assert_ok(&forked);
    let fork_id = forked["result"]["sessionId"].as_str().unwrap();
    let source_status = client
        .rpc("session.typescript.status", json!({"sessionId":session_id}))
        .await;
    let fork_status = client
        .rpc("session.typescript.status", json!({"sessionId":fork_id}))
        .await;
    assert_ok(&fork_status);
    assert_eq!(source_status["result"]["pid"], fork_status["result"]["pid"]);
    assert_ne!(
        source_status["result"]["generation"],
        fork_status["result"]["generation"]
    );
    assert_eq!(fork_status["result"]["calls"], 0);
    submit(&mut client, fork_id, r#"tool:ts_echo {"text":"forked"}"#).await;
    assert_eq!(
        client
            .rpc("session.typescript.status", json!({"sessionId":fork_id}))
            .await["result"]["calls"],
        1
    );
    let fork = server.agent().unwrap().get_session(fork_id).unwrap();
    let history = fork.history("main", None).await.unwrap();
    let result = history
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ToolResult(batch) => batch.results.first(),
            _ => None,
        })
        .unwrap();
    let dim_agent_types::ToolContent::Text(text) = &result.result.content else {
        panic!("text result")
    };
    assert_eq!(
        serde_json::from_str::<Value>(text).unwrap()["sessionId"],
        fork_id
    );
    let drain = client
        .send("session.typescript.drain", json!({"sessionId":fork_id}))
        .await;
    started(&mut client, drain).await;
    let mut deleter = Client::new_version(&server, false, 3).await;
    assert_ok(
        &deleter
            .rpc("session.delete", json!({"sessionId":fork_id}))
            .await,
    );
    assert_eq!(client.response(drain).await["result"]["alive"], true);
    assert_eq!(
        client
            .rpc("session.typescript.status", json!({"sessionId":fork_id}))
            .await["error"]["data"]["appError"],
        "session_closed"
    );
    assert_eq!(
        client
            .rpc("session.typescript.status", json!({"sessionId":session_id}))
            .await["result"]["calls"],
        1
    );
    drop(client);
    server.shutdown();
    server.join().await.unwrap();

    let mut config = config;
    config.session_typescript.as_mut().unwrap().rows[0]["disabled"] = json!(true);
    let (server, mut requests, _) = start(config, false).await;
    let mut client = Client::new_version(&server, false, 3).await;
    submit(&mut client, &session_id, r#"tool:echo {"text":"native"}"#).await;
    assert!(!requests.recv().await.unwrap().contains(&"ts_echo".into()));
    assert_eq!(
        client
            .rpc("session.typescript.status", json!({"sessionId":session_id}))
            .await["error"]["data"]["appError"],
        "unknown_method"
    );
    assert!(
        !server
            .diagnostics()
            .plugins
            .iter()
            .any(|part| part.name.ends_with("runtime/fixtures/session-part.ts"))
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

type Control = (SessionScope, rutis::Ctx, Arc<rutis_loader::Loader>);
struct ControlledGate(SessionScope, tokio::sync::mpsc::UnboundedSender<Control>);
impl TypedPlugin for ControlledGate {
    type Deps = (
        Arc<rutis_loader::Loader>,
        rutis::KeyedGate<rutis_loader::RuntimeRows>,
    );
    fn name(&self) -> &str {
        "ControlledTypeScriptDependency"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (loader, _): Self::Deps,
    ) -> rutis::BoxFuture<'a, Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            self.1.send((self.0, ctx.clone(), loader)).unwrap();
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for ControlledGate {
    fn keys(&self) -> KeysOf<Self> {
        // Observe the settled initial metadata, rather than a fiber replaced
        // by RuntimeRows' offline-to-online refresh (as in the C4 spike).
        ((), rutis::DepKey::dynamic("session-node"))
    }
}

struct GateValue;
impl rutis_bridge::session::HostDispatch for GateValue {
    fn invoke(&self, _: &str, _: rutis_bridge::session::Value) -> rutis_bridge::session::Reply {
        Err(rutis_bridge::session::Error::Value(
            "fixture dependency is not a tool".into(),
        ))
    }
    fn methods(&self) -> Option<Value> {
        Some(json!({"declaration":"sync", "execute":"async"}))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_session_typescript_pending_parts_keep_native_sessions_available() {
    let mut parts = fixture();
    let mut gate = parts.rows.remove(0);
    let entry = PathBuf::from(gate["name"].as_str().unwrap());
    gate["id"] = json!("disabled-gate");
    gate["disabled"] = json!(true);
    parts.rows.push(json!({"id":"typescript", "name":entry.with_file_name("session-late.ts"), "provides":serde_json::from_str::<Value>(include_str!("../../../../runtime/fixtures/session-late.json")).unwrap()}));
    parts.rows.push(gate);
    let (mut builder, mut requests, _) = setup(false);
    let (send, mut controls) = tokio::sync::mpsc::unbounded_channel();
    builder.session_plugin(move |scope| ControlledGate(*scope, send.clone()));
    let server = serve(
        builder,
        ServerConfig {
            unix_path: Some(std::env::temp_dir().join(format!(
                "dm-ts-{}/server.sock",
                &uuid::Uuid::new_v4().simple().to_string()[..8]
            ))),
            session_typescript: Some(parts),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut client = Client::new_version(&server, false, 3).await;
    let session = new_session(&mut client, json!(["late", "echo"])).await;
    let (scope, ctx, loader) = controls.recv().await.unwrap();
    let view = loader
        .entries()
        .into_iter()
        .find(|entry| {
            entry.id == "typescript"
                && entry.instance.as_ref().is_some_and(|instance| {
                    instance.chain.iter().any(|(_, id)| *id == scope.instance)
                })
        })
        .unwrap()
        .view
        .unwrap();
    assert_eq!(view.state().state, rutis::FiberState::Pending);
    submit(&mut client, &session, r#"tool:echo {"text":"before"}"#).await;
    assert!(!requests.recv().await.unwrap().contains(&"late".into()));
    assert!(
        !server
            .diagnostics()
            .plugins
            .iter()
            .any(|part| part.name.ends_with("runtime/fixtures/session-part.ts"))
    );
    ctx.provide_as::<dyn rutis_bridge::session::HostDispatch>(
        rutis_bridge::session::host_key_in("toolContribution:ts_echo", scope.instance),
        Arc::new(GateValue),
    )
    .unwrap();
    let mut state = view.watch();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if state.borrow().state == rutis::FiberState::Active {
                break;
            }
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|error| {
        let current = loader
            .entries()
            .into_iter()
            .filter(|entry| entry.id == "typescript")
            .map(|entry| {
                (
                    entry.instance,
                    entry.status,
                    entry.view.map(|view| view.state()),
                )
            })
            .collect::<Vec<_>>();
        panic!(
            "late provider: {error}; view={:?}; current={current:?}",
            view.state()
        )
    });
    assert_eq!(
        client
            .rpc("session.late.status", json!({"sessionId":session}))
            .await["result"],
        "active"
    );
    submit(&mut client, &session, r#"tool:echo {"text":"after"}"#).await;
    while let Ok(names) = requests.try_recv() {
        assert!(!names.contains(&"late".into()));
    }
    server.shutdown();
    server.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_session_typescript_rejects_invalid_boundaries_without_disabling_native_tools() {
    let mut parts = fixture();
    parts.rows[0]["config"] = json!({"badDeclaration":true});
    let (server, mut requests, _) = start(
        ServerConfig {
            session_typescript: Some(parts),
            ..Default::default()
        },
        true,
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let session = new_session(&mut client, json!(["ts_echo", "echo"])).await;
    submit(&mut client, &session, r#"tool:echo {"text":"native"}"#).await;
    assert!(!requests.recv().await.unwrap().contains(&"ts_echo".into()));
    assert_ok(
        &client
            .rpc("session.typescript.status", json!({"sessionId":session}))
            .await,
    );
    server.shutdown();
    server.join().await.unwrap();

    for bad_methods in [json!(true), json!("unsupported"), Value::Null] {
        let mut parts = fixture();
        if !bad_methods.is_null() {
            parts.rows[0]["config"] = json!({"badMethods":bad_methods});
        } else {
            parts.rows[0]["provides"]["toolContribution:ghost"] =
                json!({"declaration":"sync", "execute":"async"});
        }
        let (mut builder, mut requests, _) = setup(false);
        let (send, mut controls) = tokio::sync::mpsc::unbounded_channel();
        builder.session_plugin(move |scope| ControlledGate(*scope, send.clone()));
        let server = serve(
            builder,
            ServerConfig {
                unix_path: Some(std::env::temp_dir().join(format!(
                    "dm-ts-{}/server.sock",
                    &uuid::Uuid::new_v4().simple().to_string()[..8]
                ))),
                session_typescript: Some(parts),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut client = Client::new_version(&server, false, 3).await;
        let session = new_session(&mut client, json!(["ts_echo", "echo"])).await;
        let (scope, _, loader) = controls.recv().await.unwrap();
        let entry = loader
            .entries()
            .into_iter()
            .find(|entry| {
                entry.id == "typescript"
                    && entry.instance.as_ref().is_some_and(|instance| {
                        instance.chain.iter().any(|(_, id)| *id == scope.instance)
                    })
            })
            .unwrap();
        if !bad_methods.is_null() {
            let view = entry.view.unwrap();
            let mut state = view.watch();
            tokio::time::timeout(Duration::from_secs(5), async {
                while state.borrow().state != rutis::FiberState::Failed {
                    state.changed().await.unwrap();
                }
            })
            .await
            .unwrap();
            assert_eq!(
                client
                    .rpc("session.typescript.status", json!({"sessionId":session}))
                    .await["error"]["data"]["appError"],
                "service_reloading"
            );
        } else {
            assert!(
                matches!(entry.status, rutis_loader::EntryStatus::Unresolved(ref error) if error.to_string().contains("provides differs")),
                "{:?}",
                entry.status
            );
        }
        submit(&mut client, &session, r#"tool:echo {"text":"native"}"#).await;
        assert!(!requests.recv().await.unwrap().contains(&"ts_echo".into()));
        server.shutdown();
        server.join().await.unwrap();
    }

    for (change, expected) in [
        (
            json!({"toolContribution:echo":{"declaration":"sync","execute":"async"}}),
            "collides",
        ),
        (
            json!({"toolContribution:ts_echo":{"declaration":"async","execute":"async"}}),
            "method modes",
        ),
        (
            json!({"sessionMethods:session.create":{"describe":"async","call":"async"}}),
            "invalid Session namespace",
        ),
        (
            json!({"sessionMethods:session.delete":{"describe":"async","call":"async"}}),
            "invalid Session namespace",
        ),
        (
            json!({"sessionMethods:session.fork":{"describe":"async","call":"async"}}),
            "invalid Session namespace",
        ),
        (
            json!({"sessionMethods:session.input":{"describe":"async","call":"async"}}),
            "duplicate Session method namespace",
        ),
    ] {
        let mut parts = fixture();
        parts.rows[0]["disabled"] = json!(true);
        parts.rows[0]["provides"] = change;
        let (builder, _, _) = setup(false);
        let error = serve(
            builder,
            ServerConfig {
                session_typescript: Some(parts),
                ..Default::default()
            },
        )
        .await
        .err()
        .expect("invalid structural input must fail");
        assert!(error.to_string().contains(expected), "{error}");
    }

    let mut parts = fixture();
    let mut repeated = parts.rows[0].clone();
    repeated["id"] = json!("different-row");
    repeated["provides"] = json!({});
    parts.rows.push(repeated);
    let (builder, _, _) = setup(false);
    let error = serve(
        builder,
        ServerConfig {
            session_typescript: Some(parts),
            ..Default::default()
        },
    )
    .await
    .err()
    .expect("inconsistent module snapshots must fail");
    assert!(
        error.to_string().contains("same provides snapshot"),
        "{error}"
    );
}
