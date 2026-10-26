//! Public Session flows through MCP, ToolBinding, Approval and ToolExecutor.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::post,
};
use dim_agent_capabilities::{
    adapters::{DefaultPrepareModelPlugin, MockTools},
    defaults,
    history::HistoryRenderer,
    host::*,
    tool_registry::ToolSourcesView,
};
use dim_agent_runtime::*;
use dim_agent_types::*;
use dim_app_server::{
    mcp::{self, McpChange, McpPlugin, McpService, McpSession, McpSourceState},
    settings::{SettingsPlugin, SettingsScope, SettingsService},
};
use rutis::Ctx;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    convert::Infallible,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;

struct Remote {
    calls: Mutex<Vec<String>>,
    initializes: AtomicUsize,
    deletes: AtomicUsize,
    invalid: AtomicBool,
    listing: Notify,
    hold: AtomicBool,
    release: Notify,
    revision: AtomicUsize,
    events: broadcast::Sender<String>,
}
impl Remote {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::default(),
            initializes: AtomicUsize::new(0),
            deletes: AtomicUsize::new(0),
            invalid: AtomicBool::new(false),
            listing: Notify::new(),
            hold: AtomicBool::new(false),
            release: Notify::new(),
            revision: AtomicUsize::new(0),
            events: broadcast::channel(16).0,
        })
    }
}
async fn rpc(
    State(remote): State<Arc<Remote>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(id) = body.get("id") else {
        return StatusCode::ACCEPTED.into_response();
    };
    let result = match body["method"].as_str().unwrap() {
        "initialize" => {
            remote.initializes.fetch_add(1, Ordering::SeqCst);
            json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{"listChanged":true}},"serverInfo":{"name":"session-test","version":"1"}})
        }
        "tools/list" => {
            if remote.hold.load(Ordering::SeqCst)
                && headers.get("x-generation").is_some_and(|h| h == "slow")
            {
                remote.listing.notify_one();
                remote.release.notified().await;
            }
            if remote.invalid.load(Ordering::SeqCst) {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            json!({"tools":[
                {"name":"lookup/item","description":format!("Subtract two numbers, revision {}", remote.revision.load(Ordering::SeqCst)),"inputSchema":{"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"]}},
                {"name":"lookup_item","description":"A distinct original name","inputSchema":{"type":"object"}}
            ]})
        }
        "tools/call" => {
            remote
                .calls
                .lock()
                .unwrap()
                .push(body["params"]["name"].as_str().unwrap().into());
            let a = body["params"]["arguments"]["a"].as_i64().unwrap();
            let b = body["params"]["arguments"]["b"].as_i64().unwrap();
            let generation = headers.get("x-generation").unwrap().to_str().unwrap();
            json!({"content":[{"type":"text","text":format!("{generation}:{}", a-b)}]})
        }
        _ => panic!("unexpected method: {body}"),
    };
    let mut response = Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response();
    response
        .headers_mut()
        .insert("mcp-session-id", "fixture".parse().unwrap());
    response
}
async fn delete(State(remote): State<Arc<Remote>>) -> StatusCode {
    remote.deletes.fetch_add(1, Ordering::SeqCst);
    StatusCode::NO_CONTENT
}
async fn events(State(remote): State<Arc<Remote>>) -> impl IntoResponse {
    Sse::new(futures_util::stream::unfold(
        remote.events.subscribe(),
        |mut rx| async move {
            rx.recv()
                .await
                .ok()
                .map(|data| (Ok::<_, Infallible>(Event::default().data(data)), rx))
        },
    ))
}

enum Action {
    Call {
        pause: bool,
        name: Option<String>,
    },
    Text,
    /// Ignores cancellation until released, so a Session close waits for it.
    Hold,
}
#[derive(Default)]
struct Model {
    actions: Mutex<VecDeque<Action>>,
    requests: Mutex<Vec<Arc<ModelRequest>>>,
    entered: Notify,
    release: Notify,
}
impl ModelDriver for Model {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        _: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        let action = self
            .actions
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Action::Text);
        self.requests.lock().unwrap().push(request.clone());
        Box::pin(async move {
            let (chunks, reason) = match action {
                Action::Call { pause, name } => {
                    let name = name.unwrap_or_else(|| {
                        request
                            .tools
                            .as_ref()
                            .unwrap()
                            .iter()
                            .find(|t| {
                                t.metadata
                                    .as_ref()
                                    .and_then(|m| m.mcp_original_tool_name.as_deref())
                                    == Some("lookup/item")
                            })
                            .unwrap()
                            .name
                            .clone()
                    });
                    if pause {
                        self.entered.notify_one();
                        tokio::select! { _ = cancel.cancelled() => return Err(HostError::Cancelled("test".into())), _ = self.release.notified() => {} }
                    }
                    let id = uuid::Uuid::new_v4().to_string();
                    (
                        vec![
                            ModelChunk::ToolStart {
                                id: id.clone(),
                                name: name.clone(),
                            },
                            ModelChunk::ToolUse(ToolCall {
                                id,
                                name,
                                input: json!({"a":17,"b":9}).as_object().unwrap().clone(),
                            }),
                        ],
                        ModelStopReason::ToolUse,
                    )
                }
                Action::Text => (
                    vec![ModelChunk::Text("done".into()), ModelChunk::TextEnd],
                    ModelStopReason::EndTurn,
                ),
                Action::Hold => {
                    self.entered.notify_one();
                    self.release.notified().await;
                    (
                        vec![ModelChunk::Text("done".into()), ModelChunk::TextEnd],
                        ModelStopReason::EndTurn,
                    )
                }
            };
            let mut stream = vec![Ok(ModelChunk::MessageStart {
                message_id: "message".into(),
            })];
            stream.extend(chunks.into_iter().map(Ok));
            stream.push(Ok(ModelChunk::MessageStop {
                message_id: "message".into(),
                reason,
            }));
            Ok(Box::pin(futures_util::stream::iter(stream)) as ModelStream)
        })
    }
}

struct Env {
    root: Ctx,
    publication: mcp_host::Host,
    home: PathBuf,
    agent: Agent,
    model: Arc<Model>,
    directory: Arc<McpService>,
    remote: Arc<Remote>,
    url: String,
    stop: CancellationToken,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}
#[path = "fixtures/mcp_host.rs"]
mod mcp_host;
impl Env {
    async fn new() -> Self {
        Self::with(|_, _| {}).await
    }
    /// `configure` adds Session capabilities after the MCP Session part.
    async fn with(configure: impl FnOnce(&mut AgentBuilder, &Arc<Remote>)) -> Self {
        let remote = Remote::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let stop = CancellationToken::new();
        let server = tokio::spawn(
            axum::serve(
                listener,
                Router::new()
                    .route("/mcp", post(rpc).get(events).delete(delete))
                    .with_state(remote.clone()),
            )
            .with_graceful_shutdown(stop.clone().cancelled_owned())
            .into_future(),
        );
        let home = std::env::temp_dir().join(format!("dim-mcp-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(home.join("project")).unwrap();
        let home = home.canonicalize().unwrap();
        std::fs::write(
            home.join("mcp.json"),
            json!({"mcpServers":{"docs":configuration(&url, "one")}}).to_string(),
        )
        .unwrap();
        let root = Ctx::root().unwrap();
        // The management table the process providers register with.
        root.plugin(
            dim_app_server::methods::ManagementMethodsPlugin::typed(&dim_app_server::catalog())
                .unwrap(),
        );
        let (publication, _) = mcp_host::Host::start(&root).await;
        let views = vec![
            root.plugin(rutis::Typed::new(
                dim_agent_capabilities::file_storage::FileStoragePlugin::new(vec![home.clone()]),
            )),
            root.plugin(
                SettingsPlugin::new(0, vec![])
                    .typed(&dim_app_server::catalog())
                    .unwrap(),
            ),
        ];
        wait_active(
            &root,
            &views,
            Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        root.get::<SettingsService>()
            .unwrap()
            .authorize_project(home.join("project"), &CancellationToken::new())
            .await
            .unwrap();
        let model = Arc::new(Model::default());
        let mut builder = AgentBuilder::new(Default::default());
        builder
            .session_value(Arc::new(HistoryRenderer::default()))
            .process_value(Arc::new(ModelHost(model.clone())))
            .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
            .session_value(Arc::new(ContextModelHost(None)))
            .session_value(Arc::new(CompactionStrategyHost(None)))
            .require_feature(
                Feature::new("PrepareModelHost").session("PrepareModelHost", |names| {
                    names.part(DefaultPrepareModelPlugin::new(names)?)
                }),
            );
        defaults::tool_host(&mut builder, Arc::new(MockTools));
        defaults::install_with_prompt(
            &mut builder,
            Arc::new(EmptyPromptHost),
            Some(ConfigureUpdate {
                sources: vec![],
                model: Some(SessionConfig::default().model),
            }),
        );
        mcp::install(&mut builder);
        // The Agent's own plugin: it registers McpSession with the Agent.
        builder.plugin(|names| names.part(McpPlugin::new(0)));
        configure(&mut builder, &remote);
        let agent = builder.build_in(&root);
        // The directory runs before any Session does.
        agent.start().await.unwrap();
        let directory = root.get::<McpService>().unwrap();
        Self {
            root,
            publication,
            home,
            agent,
            model,
            directory,
            remote,
            url,
            stop,
            server,
        }
    }
    async fn session(&self, project: bool, preset: PermissionPreset) -> Session {
        self.agent
            .create_session(SessionConfig {
                cwd: self
                    .home
                    .join(if project { "project" } else { "other" })
                    .to_string_lossy()
                    .into(),
                tools: Some(vec![]),
                permission_policy: PermissionPolicy {
                    preset,
                    ..Default::default()
                },
                ..Default::default()
            })
            .await
            .unwrap()
    }
    async fn update(&self, source: SettingsScope, id: &str, change: McpChange) {
        let cancel = CancellationToken::new();
        let catalog = self.directory.catalog(Some(1), &cancel).await.unwrap();
        let version = if source == SettingsScope::User {
            catalog.user_revision
        } else {
            catalog.project_revision.unwrap()
        };
        self.directory
            .update(source, version, id.into(), change, &cancel)
            .await
            .unwrap();
    }
    async fn turn(&self, session: &Session, action: Action) {
        // A rejected tool can end its turn without requesting the text step.
        *self.model.actions.lock().unwrap() = VecDeque::from([action, Action::Text]);
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), session.idle("main"))
            .await
            .unwrap()
            .unwrap();
    }
    async fn close(self) {
        self.agent.close().await.unwrap();
        self.publication.close().await;
        self.root.shutdown().await.unwrap();
        self.stop.cancel();
        self.server.await.unwrap().unwrap();
        std::fs::remove_dir_all(self.home).unwrap();
    }
}
/// Fails Session assembly after McpSession applied, once discovery started
/// during assembly would have reached the server.
struct FailAfterMcp {
    remote: Arc<Remote>,
}
impl rutis::TypedPlugin for FailAfterMcp {
    type Deps = rutis::KeyedGate<McpSession>;
    fn name(&self) -> &str {
        "FailAfterMcp"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        _: Self::Deps,
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            let _ = tokio::time::timeout(Duration::from_millis(500), async {
                while self.remote.initializes.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            Err(rutis::CordisError::PluginFailed(
                "assembly failed after McpSession".into(),
            ))
        })
    }
}

fn configuration(url: &str, generation: &str) -> Value {
    json!({"type":"http","url":url,"headers":{"X-Generation":generation}})
}
async fn ready(session: &Session) -> String {
    let mcp = session.session_service::<McpSession>().unwrap();
    let mut changed = mcp.subscribe();
    let version = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(McpSourceState::Ready { version }) =
                changed.borrow_and_update().servers.get("docs")
            {
                return version.clone();
            }
            changed.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    adopted(session, Some(&version)).await;
    version
}
async fn adopted(session: &Session, version: Option<&str>) {
    let registry = session.session_service::<ToolSourcesView>().unwrap();
    let mut changed = registry.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = changed
                .borrow_and_update()
                .0
                .get("mcp:docs")
                .and_then(|s| s.adopted.clone());
            if current.as_deref() == version {
                break;
            }
            changed.changed().await.unwrap();
        }
        // Registry acceptance precedes the branch's persisted adoption.
        loop {
            let events = session.history("main", None).await.unwrap();
            if events.iter().rev().find_map(|e| match &e.kind {
                EventKind::ContextConfigured(c) => Some(
                    c.settings
                        .tool_sources
                        .iter()
                        .find(|s| s.key == "mcp:docs")
                        .map(|s| s.version.as_str()),
                ),
                _ => None,
            }) == Some(version)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn results(session: &Session) -> Vec<String> {
    session
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::ToolResult(r) => Some(serde_json::to_string(&r.results).unwrap()),
            _ => None,
        })
        .collect()
}

async fn temporary(
    session: &Session,
    method: &str,
    mut params: Value,
) -> std::result::Result<Value, dim_agent_capabilities::methods::RpcError> {
    params["sessionId"] = json!(session.id());
    session
        .session_service::<dim_agent_capabilities::methods::SessionMethods>()
        .unwrap()
        .provider(method)
        .unwrap()
        .accept(method.into(), params, CancellationToken::new(), |_| {})
        .unwrap()
        .reply
        .await
        .unwrap()
}

#[tokio::test]
async fn temporary_mcp_binds_calls_isolates_and_disposes_without_settings_writes() {
    let env = Env::new().await;
    let first = env.session(false, PermissionPreset::FullAccess).await;
    let second = env.session(false, PermissionPreset::FullAccess).await;
    ready(&first).await;
    ready(&second).await;
    let settings = std::fs::read(env.home.join("mcp.json")).unwrap();
    let binding = uuid::Uuid::new_v4().to_string();
    let input =
        json!({"bindingId":binding,"servers":{"client":configuration(&env.url,"temporary")}});
    temporary(&first, "session.mcp_temporary.attach", input.clone())
        .await
        .unwrap();
    let initialized = env.remote.initializes.load(Ordering::SeqCst);
    temporary(&first, "session.mcp_temporary.attach", input)
        .await
        .unwrap();
    assert_eq!(env.remote.initializes.load(Ordering::SeqCst), initialized);
    env.turn(&first, Action::Text).await;
    let request = env.model.requests.lock().unwrap().last().unwrap().clone();
    let native_id = format!("temporary:{binding}:client");
    let tool = request
        .tools
        .as_ref()
        .unwrap()
        .iter()
        .find(|tool| {
            tool.metadata.as_ref().is_some_and(|metadata| {
                metadata.mcp_server_id.as_deref() == Some(&native_id)
                    && metadata.mcp_original_tool_name.as_deref() == Some("lookup/item")
            })
        })
        .expect("the very first request must include the bound MCP tool")
        .name
        .clone();
    env.turn(
        &first,
        Action::Call {
            pause: false,
            name: Some(tool),
        },
    )
    .await;
    assert!(
        results(&first)
            .await
            .last()
            .unwrap()
            .contains("temporary:8")
    );
    let sources = first.session_service::<ToolSourcesView>().unwrap();
    let (key, version) = sources
        .get()
        .0
        .into_iter()
        .find(|(key, _)| key.starts_with("temporary_mcp_"))
        .unwrap();
    let mut changes = sources.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        while env.remote.events.receiver_count() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    env.remote.revision.store(1, Ordering::SeqCst);
    env.remote
        .events
        .send(json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}).to_string())
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while changes
            .borrow_and_update()
            .0
            .get(&key)
            .is_none_or(|state| state.adopted == version.adopted)
        {
            changes.changed().await.unwrap();
        }
    })
    .await
    .expect("temporary tools must adopt list changes");
    env.turn(&first, Action::Text).await;
    assert!(
        env.model
            .requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .tools
            .as_ref()
            .unwrap()
            .iter()
            .any(|tool| {
                tool.metadata
                    .as_ref()
                    .is_some_and(|metadata| metadata.mcp_server_id.as_deref() == Some(&native_id))
                    && tool.description.contains("revision 1")
            })
    );
    env.turn(&second, Action::Text).await;
    assert!(
        env.model
            .requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .tools
            .as_ref()
            .unwrap()
            .iter()
            .all(|tool| !tool
                .metadata
                .as_ref()
                .is_some_and(|metadata| metadata.mcp_server_id.as_deref() == Some(&native_id)))
    );
    let mut conflict =
        json!({"bindingId":binding,"servers":{"client":configuration(&env.url,"other")}});
    assert_eq!(
        temporary(&first, "session.mcp_temporary.attach", conflict.take())
            .await
            .unwrap_err()
            .data
            .unwrap()
            .app_error,
        "mcp_binding_conflict"
    );
    temporary(
        &first,
        "session.mcp_temporary.renew",
        json!({"bindingId":binding}),
    )
    .await
    .unwrap();
    let deleted = env.remote.deletes.load(Ordering::SeqCst);
    temporary(
        &first,
        "session.mcp_temporary.detach",
        json!({"bindingId":binding}),
    )
    .await
    .unwrap();
    temporary(
        &first,
        "session.mcp_temporary.detach",
        json!({"bindingId":binding}),
    )
    .await
    .unwrap();
    assert_eq!(
        env.remote.deletes.load(Ordering::SeqCst),
        deleted + 1,
        "binding disposal closes its HTTP connection despite retained model requests"
    );
    env.turn(&first, Action::Text).await;
    assert!(
        env.model
            .requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .tools
            .as_ref()
            .unwrap()
            .iter()
            .all(|tool| !tool
                .metadata
                .as_ref()
                .is_some_and(|metadata| metadata.mcp_server_id.as_deref() == Some(&native_id)))
    );
    assert_eq!(std::fs::read(env.home.join("mcp.json")).unwrap(), settings);
    assert!(!env.home.join("project/.mcp.json").exists());
    env.close().await;
}

#[tokio::test]
async fn temporary_mcp_failed_setup_cleans_up_and_an_abandoned_binding_expires() {
    let env = Env::new().await;
    let session = env.session(false, PermissionPreset::FullAccess).await;
    ready(&session).await;
    let failed_id = uuid::Uuid::new_v4().to_string();
    let refused = temporary(&session,"session.mcp_temporary.attach",json!({"bindingId":failed_id,"servers":{"client":configuration("http://127.0.0.1:1/mcp","unused")}})).await.unwrap_err();
    assert_eq!(refused.data.unwrap().app_error, "mcp_binding_failed");
    assert_eq!(
        temporary(
            &session,
            "session.mcp_temporary.renew",
            json!({"bindingId":failed_id})
        )
        .await
        .unwrap_err()
        .data
        .unwrap()
        .app_error,
        "mcp_binding_missing"
    );
    let binding = uuid::Uuid::new_v4().to_string();
    let input =
        json!({"bindingId":binding,"servers":{"client":configuration(&env.url,"temporary")}});
    temporary(&session, "session.mcp_temporary.attach", input.clone())
        .await
        .unwrap();
    let deleted = env.remote.deletes.load(Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(35), async {
        while env.remote.deletes.load(Ordering::SeqCst) == deleted {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("abandoned MCP resources must expire");
    assert_eq!(
        temporary(
            &session,
            "session.mcp_temporary.renew",
            json!({"bindingId":binding})
        )
        .await
        .unwrap_err()
        .data
        .unwrap()
        .app_error,
        "mcp_binding_missing"
    );
    temporary(&session, "session.mcp_temporary.attach", input)
        .await
        .unwrap();
    let deleted = env.remote.deletes.load(Ordering::SeqCst);
    session.close().await.unwrap();
    assert_eq!(
        env.remote.deletes.load(Ordering::SeqCst),
        deleted + 2,
        "closing the Session disposes both configured and temporary connections"
    );
    env.close().await;
}

async fn state(session: &Session, expected: McpSourceState) {
    let mut state = session.session_service::<McpSession>().unwrap().subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        while state.borrow_and_update().servers.get("docs") != Some(&expected) {
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn project_discovery_does_not_connect_until_the_user_enables_it() {
    let env = Env::new().await;
    // The project file asks to be enabled; ADR 003 D3 ignores that.
    let mut project = configuration(&env.url, "project");
    project["enabled"] = json!(true);
    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::Put(serde_json::from_value(project).unwrap()),
    )
    .await;
    let file = env.home.join("project/.mcp.json");
    let written = std::fs::read(&file).unwrap();
    let session = env.session(true, PermissionPreset::FullAccess).await;
    state(&session, McpSourceState::Disabled).await;
    assert_eq!(
        env.remote.initializes.load(Ordering::SeqCst),
        0,
        "opening a workspace ran its MCP server"
    );

    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::SetEnabled(true),
    )
    .await;
    let enabled = ready(&session).await;
    assert_eq!(env.remote.initializes.load(Ordering::SeqCst), 1);
    // The choice is the user's, recorded outside the shared project file.
    assert_eq!(std::fs::read(&file).unwrap(), written);
    // A project-scope edit reaches the open Session through the Settings
    // event alone: nothing else wakes it for a change that withdraws nothing.
    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::Put(serde_json::from_value(configuration(&env.url, "edited")).unwrap()),
    )
    .await;
    let mcp = session.session_service::<McpSession>().unwrap();
    let mut state = mcp.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(state.borrow_and_update().servers.get("docs"), Some(McpSourceState::Ready { version }) if version != &enabled) { break; }
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert_eq!(env.remote.initializes.load(Ordering::SeqCst), 2);
    env.close().await;
}

#[tokio::test]
async fn enabled_project_entry_serves_tools_and_disabling_stops_it_in_every_session() {
    let env = Env::new().await;
    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::Put(serde_json::from_value(configuration(&env.url, "project")).unwrap()),
    )
    .await;
    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::SetEnabled(true),
    )
    .await;
    let session = env.session(true, PermissionPreset::FullAccess).await;
    ready(&session).await;
    env.turn(
        &session,
        Action::Call {
            pause: false,
            name: None,
        },
    )
    .await;
    assert!(
        results(&session)
            .await
            .last()
            .unwrap()
            .contains("project:8")
    );
    let other = env.session(true, PermissionPreset::FullAccess).await;
    ready(&other).await;
    assert_eq!(env.remote.initializes.load(Ordering::SeqCst), 2);

    *env.model.actions.lock().unwrap() = VecDeque::from([
        Action::Call {
            pause: true,
            name: None,
        },
        Action::Text,
    ]);
    session
        .submit("pause".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), env.model.entered.notified())
        .await
        .unwrap();
    // Disabling stops the server in both Sessions before it returns,
    // including the connection that the paused request has already bound.
    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::SetEnabled(false),
    )
    .await;
    assert_eq!(
        env.remote.deletes.load(Ordering::SeqCst),
        2,
        "disabling returned before every Session's generation closed"
    );
    state(&other, McpSourceState::Disabled).await;
    adopted(&other, None).await;
    env.model.release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), session.idle("main"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        env.remote.calls.lock().unwrap().len(),
        1,
        "a disabled server received a call"
    );
    assert!(results(&session).await.last().unwrap().contains("isError"));
    state(&session, McpSourceState::Disabled).await;
    env.close().await;
}

#[tokio::test]
async fn closing_one_session_drains_only_its_connections_with_retained_requests() {
    let env = Env::new().await;
    let first = env.session(false, PermissionPreset::FullAccess).await;
    let second = env.session(false, PermissionPreset::FullAccess).await;
    ready(&first).await;
    ready(&second).await;
    assert_eq!(env.remote.initializes.load(Ordering::SeqCst), 2);
    env.turn(
        &first,
        Action::Call {
            pause: false,
            name: None,
        },
    )
    .await;
    // Model keeps the request (and its tool binding) alive after Session close.
    let retained = env.model.requests.lock().unwrap()[0].clone();
    first.close().await.unwrap();
    assert_eq!(env.remote.deletes.load(Ordering::SeqCst), 1);
    assert!(first.session_service::<McpSession>().is_err());
    env.turn(
        &second,
        Action::Call {
            pause: false,
            name: None,
        },
    )
    .await;
    assert!(results(&second).await.last().unwrap().contains("one:8"));
    assert_eq!(
        env.remote.initializes.load(Ordering::SeqCst),
        2,
        "closing a sibling must not reconnect this Session"
    );
    assert_eq!(env.remote.deletes.load(Ordering::SeqCst), 1);
    drop(retained);
    env.close().await;
}

#[tokio::test]
async fn bound_model_request_keeps_old_connection_while_next_request_uses_replacement() {
    let env = Env::new().await;
    let session = env.session(false, PermissionPreset::FullAccess).await;
    let v1 = ready(&session).await;
    env.model.actions.lock().unwrap().extend([
        Action::Call {
            pause: true,
            name: None,
        },
        Action::Text,
    ]);
    session
        .submit("go".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), env.model.entered.notified())
        .await
        .unwrap();
    let declarations = env.model.requests.lock().unwrap()[0].tools.clone().unwrap();
    assert_eq!(declarations.len(), 2);
    assert_ne!(declarations[0].name, declarations[1].name);
    assert!(declarations.iter().all(|t| {
        t.name.len() <= 64
            && t.name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    }));
    env.update(
        SettingsScope::User,
        "docs",
        McpChange::Put(serde_json::from_value(configuration(&env.url, "two")).unwrap()),
    )
    .await;
    let mcp = session.session_service::<McpSession>().unwrap();
    let mut state = mcp.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(state.borrow_and_update().servers.get("docs"), Some(McpSourceState::Ready { version }) if version != &v1) { break; }
            state.changed().await.unwrap();
        }
    }).await.unwrap();
    ready(&session).await;
    assert_eq!(
        env.remote.deletes.load(Ordering::SeqCst),
        0,
        "the bound request owns V1"
    );
    env.model.release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), session.idle("main"))
        .await
        .unwrap()
        .unwrap();
    assert!(results(&session).await[0].contains("one:8"));
    env.turn(
        &session,
        Action::Call {
            pause: false,
            name: None,
        },
    )
    .await;
    assert!(results(&session).await.last().unwrap().contains("two:8"));
    assert_eq!(
        *env.remote.calls.lock().unwrap(),
        vec!["lookup/item", "lookup/item"]
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while env.remote.deletes.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    env.close().await;
}

#[tokio::test]
async fn project_visibility_disable_refresh_failure_and_permission_use_real_sources() {
    let env = Env::new().await;
    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::Put(
            serde_json::from_value(json!({"type":"http","url":env.url,"enabled":false})).unwrap(),
        ),
    )
    .await;
    let user = env.session(false, PermissionPreset::FullAccess).await;
    ready(&user).await;
    let project = env.session(true, PermissionPreset::FullAccess).await;
    let view = project.session_service::<McpSession>().unwrap();
    let mut state = view.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if state.borrow_and_update().servers.get("docs") == Some(&McpSourceState::Disabled) {
                break;
            }
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    env.update(SettingsScope::Project(1), "docs", McpChange::Remove)
        .await;
    ready(&project).await;
    assert_eq!(
        env.remote.initializes.load(Ordering::SeqCst),
        2,
        "Sessions own distinct user connections"
    );
    let denied = env.session(false, PermissionPreset::ReadOnly).await;
    ready(&denied).await;
    env.turn(
        &denied,
        Action::Call {
            pause: false,
            name: None,
        },
    )
    .await;
    assert!(
        env.remote.calls.lock().unwrap().is_empty(),
        "unknown MCP risk must not bypass Approval"
    );
    assert!(results(&denied).await.last().unwrap().contains("isError"));
    let old_name = env.model.requests.lock().unwrap()[0]
        .tools
        .as_ref()
        .unwrap()[0]
        .name
        .clone();
    env.update(SettingsScope::User, "docs", McpChange::SetEnabled(false))
        .await;
    adopted(&user, None).await;
    env.turn(
        &user,
        Action::Call {
            pause: false,
            name: Some(old_name),
        },
    )
    .await;
    assert!(env.remote.calls.lock().unwrap().is_empty());
    assert!(
        env.model
            .requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .tools
            .as_ref()
            .unwrap()
            .is_empty()
    );
    assert!(user.history("main", None).await.unwrap().iter().any(|event| matches!(
        &event.kind, EventKind::ModelOutputEnd(end)
        if matches!(&end.outcome, ModelOutputOutcome::Failed { error, .. } if error.contains("Tool was not offered"))
    )));
    env.update(SettingsScope::User, "docs", McpChange::SetEnabled(true))
        .await;
    ready(&user).await;
    env.remote.invalid.store(true, Ordering::SeqCst);
    user.session_service::<McpSession>()
        .unwrap()
        .refresh()
        .unwrap();
    adopted(&user, None).await;
    assert!(matches!(
        user.session_service::<McpSession>()
            .unwrap()
            .state()
            .servers["docs"],
        McpSourceState::Failed { .. }
    ));
    env.remote.invalid.store(false, Ordering::SeqCst);
    user.session_service::<McpSession>()
        .unwrap()
        .refresh()
        .unwrap();
    ready(&user).await;
    env.turn(
        &user,
        Action::Call {
            pause: false,
            name: None,
        },
    )
    .await;
    assert!(results(&user).await.last().unwrap().contains("one:8"));
    let connections = env.remote.initializes.load(Ordering::SeqCst);
    project.close().await.unwrap();
    assert!(view.state().closed);
    assert!(view.refresh().is_err());
    env.turn(
        &user,
        Action::Call {
            pause: false,
            name: None,
        },
    )
    .await;
    assert!(results(&user).await.last().unwrap().contains("one:8"));
    assert_eq!(
        env.remote.initializes.load(Ordering::SeqCst),
        connections,
        "closing one Session must not interrupt another Session's connection"
    );
    env.close().await;
}

#[tokio::test]
async fn unreadable_catalog_preserves_adopted_tools_until_a_valid_update() {
    let env = Env::new().await;
    let session = env.session(true, PermissionPreset::FullAccess).await;
    let view = session.session_service::<McpSession>().unwrap();
    let mut version = ready(&session).await;
    for (path, disconnect) in [
        (env.home.join("mcp.json"), false),
        (env.home.join("project/.mcp.json"), true),
    ] {
        let original = std::fs::read(&path).ok();
        std::fs::write(&path, "{ invalid json").unwrap();
        let mut state = view.subscribe();
        view.refresh().unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if state.borrow_and_update().catalog_error.is_some() {
                    break;
                }
                state.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(
            view.state().servers.get("docs"),
            Some(&McpSourceState::Ready {
                version: version.clone()
            }),
            "a catalog read error is not an empty or disabled source"
        );
        adopted(&session, Some(&version)).await;
        env.turn(
            &session,
            Action::Call {
                pause: false,
                name: None,
            },
        )
        .await;
        assert!(results(&session).await.last().unwrap().contains("one:8"));
        assert_eq!(env.remote.initializes.load(Ordering::SeqCst), 1);

        if disconnect {
            // Preserving sources must also preserve their failure subscription;
            // a broken catalog does not keep a dead connection callable.
            env.directory
                .disconnect(
                    SettingsScope::User,
                    "docs".into(),
                    &CancellationToken::new(),
                )
                .await
                .unwrap();
            adopted(&session, None).await;
            assert!(matches!(
                view.state().servers["docs"],
                McpSourceState::Failed { .. }
            ));
            assert!(view.state().catalog_error.is_some());
        }
        if let Some(original) = original {
            std::fs::write(&path, original).unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        view.refresh().unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if state.borrow_and_update().catalog_error.is_none()
                    && matches!(view.state().servers.get("docs"), Some(McpSourceState::Ready { version: current }) if current != &version)
                {
                    break;
                }
                state.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        version = ready(&session).await;
    }
    // A successfully read empty directory really removes the source.
    std::fs::write(env.home.join("mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
    view.refresh().unwrap();
    adopted(&session, None).await;
    assert!(view.state().catalog_error.is_none());
    assert!(view.state().servers.is_empty());
    env.close().await;
}

#[tokio::test]
async fn disabling_an_entry_whose_project_file_is_unreadable_withdraws_it() {
    let env = Env::new().await;
    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::Put(serde_json::from_value(configuration(&env.url, "project")).unwrap()),
    )
    .await;
    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::SetEnabled(true),
    )
    .await;
    let record = env.home.join("mcp-projects.json");
    let granted = std::fs::read(&record).unwrap();
    let session = env.session(true, PermissionPreset::FullAccess).await;
    let view = session.session_service::<McpSession>().unwrap();
    ready(&session).await;

    let file = env.home.join("project/.mcp.json");
    let written = std::fs::read(&file).unwrap();
    std::fs::write(&file, "{ invalid json").unwrap();
    let mut changes = view.subscribe();
    view.refresh().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while changes.borrow_and_update().catalog_error.is_none() {
            changes.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    ready(&session).await;
    // The user's choice does not depend on the file: the running server
    // stops before the call returns, and the Session reports it disabled.
    env.directory
        .update(
            SettingsScope::Project(1),
            "unreadable".into(),
            "docs".into(),
            McpChange::SetEnabled(false),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        env.remote.deletes.load(Ordering::SeqCst),
        1,
        "a disabled server kept running while its project file was unreadable"
    );
    state(&session, McpSourceState::Disabled).await;
    adopted(&session, None).await;
    assert_eq!(std::fs::read(&file).unwrap(), b"{ invalid json");

    // Another process enables it again; once the file is readable, the Session
    // follows the publisher without its own refresh and connects again.
    std::fs::write(&record, granted).unwrap();
    std::fs::write(&file, written).unwrap();
    env.directory
        .catalog(Some(1), &CancellationToken::new())
        .await
        .unwrap();
    ready(&session).await;
    assert!(view.state().catalog_error.is_none());
    assert_eq!(env.remote.initializes.load(Ordering::SeqCst), 2);
    env.close().await;
}

#[tokio::test]
async fn a_closing_session_starts_no_connection() {
    let env = Env::new().await;
    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::Put(serde_json::from_value(configuration(&env.url, "project")).unwrap()),
    )
    .await;
    let session = env.session(true, PermissionPreset::FullAccess).await;
    state(&session, McpSourceState::Disabled).await;
    // A held turn keeps the close waiting for its branch, with every Session
    // capability still loaded.
    *env.model.actions.lock().unwrap() = VecDeque::from([Action::Hold]);
    session
        .submit("hold".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), env.model.entered.notified())
        .await
        .unwrap();
    let closing = session.close();
    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::SetEnabled(true),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        env.remote.initializes.load(Ordering::SeqCst),
        0,
        "a closing Session connected an MCP server"
    );
    env.model.release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), closing)
        .await
        .unwrap()
        .unwrap();
    env.close().await;
}

#[tokio::test]
async fn slow_source_isolated_notifications_adopt_and_disconnect_withdraws_without_replay() {
    let env = Env::new().await;
    env.remote.hold.store(true, Ordering::SeqCst);
    env.update(
        SettingsScope::User,
        "slow",
        McpChange::Put(serde_json::from_value(configuration(&env.url, "slow")).unwrap()),
    )
    .await;
    let session = tokio::time::timeout(
        Duration::from_secs(2),
        env.session(false, PermissionPreset::FullAccess),
    )
    .await
    .unwrap();
    let v1 = ready(&session).await;
    tokio::time::timeout(Duration::from_secs(5), env.remote.listing.notified())
        .await
        .unwrap();
    let view = session.session_service::<McpSession>().unwrap();
    assert_eq!(view.state().servers["slow"], McpSourceState::Connecting);
    env.turn(
        &session,
        Action::Call {
            pause: false,
            name: None,
        },
    )
    .await;
    assert!(results(&session).await[0].contains("one:8"));

    // A completed list from a cancelled worker must not republish a disabled source.
    env.update(SettingsScope::User, "slow", McpChange::SetEnabled(false))
        .await;
    let mut status = view.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if status.borrow_and_update().servers.get("slow") == Some(&McpSourceState::Disabled) {
                break;
            }
            status.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    env.remote.release.notify_one();
    assert_eq!(
        ready(&session).await,
        v1,
        "another source's configuration must not replace docs"
    );

    tokio::time::timeout(Duration::from_secs(5), async {
        while env.remote.events.receiver_count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    env.remote.revision.store(1, Ordering::SeqCst);
    env.remote
        .events
        .send(json!({"jsonrpc":"2.0","method":"notifications/tools/list_changed"}).to_string())
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async { loop {
        if matches!(status.borrow_and_update().servers.get("docs"), Some(McpSourceState::Ready { version }) if version != &v1) { break; }
        status.changed().await.unwrap();
    }}).await.unwrap();
    ready(&session).await;
    env.turn(
        &session,
        Action::Call {
            pause: false,
            name: None,
        },
    )
    .await;
    let declarations = env
        .model
        .requests
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .tools
        .clone()
        .unwrap();
    assert_eq!(declarations.len(), 2, "cancelled slow list must not return");
    assert!(declarations[0].description.contains("revision 1"));
    let calls = env.remote.calls.lock().unwrap().len();
    env.directory
        .disconnect(
            SettingsScope::User,
            "docs".into(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    adopted(&session, None).await;
    assert!(matches!(
        view.state().servers["docs"],
        McpSourceState::Failed { .. }
    ));
    assert_eq!(env.remote.calls.lock().unwrap().len(), calls);
    view.refresh().unwrap();
    ready(&session).await;
    env.turn(
        &session,
        Action::Call {
            pause: false,
            name: None,
        },
    )
    .await;
    assert_eq!(env.remote.calls.lock().unwrap().len(), calls + 1);
    env.close().await;
}

#[tokio::test]
async fn an_unpublished_session_never_starts_discovery() {
    let env = Env::with(|builder, remote| {
        let remote = remote.clone();
        builder.require_feature(Feature::new("FailAfterMcp").session(
            "FailAfterMcp",
            move |names| {
                names.part(FailAfterMcp {
                    remote: remote.clone(),
                })
            },
        ));
    })
    .await;
    let created = env
        .agent
        .create_session(SessionConfig {
            cwd: env.home.join("other").to_string_lossy().into(),
            tools: Some(vec![]),
            ..Default::default()
        })
        .await;
    let error = created.err().expect("assembly fails").to_string();
    assert!(
        error.contains("assembly failed after McpSession"),
        "{error}"
    );
    // An enabled user server is eligible; only publication may start discovery.
    assert_eq!(env.remote.initializes.load(Ordering::SeqCst), 0);
    env.close().await;
}

#[tokio::test]
async fn a_shadowed_user_server_closes_its_unused_connection() {
    let env = Env::new().await;
    let session = env.session(true, PermissionPreset::FullAccess).await;
    ready(&session).await;
    // The branch adopts the user tools, so a binding holds the connection.
    env.turn(&session, Action::Text).await;
    assert_eq!(env.remote.initializes.load(Ordering::SeqCst), 1);
    // A project entry shadows the user server even while disabled.
    let mut shadow = configuration(&env.url, "project");
    shadow["enabled"] = json!(false);
    env.update(
        SettingsScope::Project(1),
        "docs",
        McpChange::Put(serde_json::from_value(shadow).unwrap()),
    )
    .await;
    let mut state = session.session_service::<McpSession>().unwrap().subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !matches!(
            state.borrow_and_update().servers.get("docs"),
            Some(McpSourceState::Disabled)
        ) {
            state.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    // The next request adopts the tool set without it, releasing the binding;
    // the user generation is still current but now unused, so it closes.
    env.turn(&session, Action::Text).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while env.remote.deletes.load(Ordering::SeqCst) < 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the unused user connection closes");
    assert_eq!(env.remote.initializes.load(Ordering::SeqCst), 1);
    env.close().await;
}
