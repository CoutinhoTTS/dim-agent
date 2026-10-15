use super::*;
use dim_agent_capabilities::methods::{Interruption, MethodDecl, SessionMethods};

/// Methods contract A.1: assembly fails for a recorded Session namespace that
/// could shadow a built-in or share a name with a configured management
/// namespace; one under `session` that does neither loads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_namespaces_overlapping_builtins_or_management_fail_assembly() {
    for (namespace, reason) in [
        ("session", "invalid Session namespace session"),
        ("session.mcp", "invalid Session namespace session.mcp"),
        (
            "session.permission",
            "invalid Session namespace session.permission",
        ),
        (
            "session.create.draft",
            "invalid Session namespace session.create.draft",
        ),
        ("agent.notes", "invalid Session namespace agent.notes"),
        ("Shell", "invalid Session namespace Shell"),
    ] {
        let error = try_launch_with(Arc::new(MockTools), ServerConfig::default(), |builder| {
            builder.session_methods(namespace);
        })
        .await
        .err()
        .unwrap_or_else(|| panic!("{namespace} loaded"));
        assert!(error.to_string().contains(reason), "{namespace}: {error}");
    }
    use std::os::unix::fs::PermissionsExt;
    let home = std::path::PathBuf::from("/tmp").join(format!("ds-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    for namespace in ["mcp", "account.codex.device"] {
        let error = try_launch_with(
            Arc::new(MockTools),
            ServerConfig {
                product_home: Some(home.clone()),
                target_home: Some(home.join("rust")),
                ..Default::default()
            },
            |builder| {
                builder.session_methods(namespace);
            },
        )
        .await
        .err()
        .unwrap_or_else(|| panic!("{namespace} loaded"));
        assert!(
            error.to_string().contains(&format!(
                "Session namespace {namespace} overlaps management namespace"
            )),
            "{namespace}: {error}"
        );
    }
    let (server, _) = launch_with(Arc::new(MockTools), ServerConfig::default(), |builder| {
        builder.session_methods("session.echo");
    })
    .await;
    server.shutdown();
    server.join().await.unwrap();
}

/// Contributes the `notes` Session methods from outside App Server, as any
/// plugin does: it records its namespace on the assembly and provides
/// `SessionMethods` from a child plugin, which a test restarts to reload the
/// provider alone. Rpc never names it.
#[derive(Clone, Default)]
struct Notes(Arc<NotesState>);
#[derive(Default)]
struct NotesState {
    /// The provider of each Session, most recent last.
    views: std::sync::Mutex<Vec<rutis::FiberView>>,
    /// Holds the next provider start until notified.
    hold: std::sync::Mutex<Option<Arc<tokio::sync::Notify>>>,
    /// `notes.wait` started, and later settled after its token fired.
    entered: tokio::sync::Notify,
    settled: tokio::sync::Notify,
}
impl Notes {
    fn install(&self, builder: &mut AgentBuilder) {
        builder.session_methods("notes");
        let notes = self.clone();
        builder.session_plugin(move |scope| NotesOwner {
            scope: *scope,
            notes: notes.clone(),
        });
    }
    /// The next provider start waits until the returned gate is notified.
    fn hold_next(&self) -> Arc<tokio::sync::Notify> {
        let gate = Arc::new(tokio::sync::Notify::new());
        *self.0.hold.lock().unwrap() = Some(gate.clone());
        gate
    }
    /// Restarts the provider of the most recently created Session.
    async fn restart(&self) {
        let view = self.0.views.lock().unwrap().last().cloned().unwrap();
        view.restart().await.unwrap();
    }
}
struct NotesOwner {
    scope: dim_agent_runtime::SessionScope,
    notes: Notes,
}
impl TypedPlugin for NotesOwner {
    type Deps = ();
    fn name(&self) -> &str {
        "NotesOwner"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (): Self::Deps,
    ) -> rutis::BoxFuture<'a, Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            let view = ctx.plugin(capability_plugin(NotesProvider {
                scope: self.scope,
                notes: self.notes.clone(),
            }));
            self.notes.0.views.lock().unwrap().push(view);
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for NotesOwner {
    fn keys(&self) -> KeysOf<Self> {}
}
struct NotesProvider {
    scope: dim_agent_runtime::SessionScope,
    notes: Notes,
}
impl TypedPlugin for NotesProvider {
    type Deps = ();
    fn name(&self) -> &str {
        "Notes"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (): Self::Deps,
    ) -> rutis::BoxFuture<'a, Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            let hold = self.notes.0.hold.lock().unwrap().take();
            if let Some(gate) = hold {
                gate.notified().await;
            }
            let state = self.notes.0.clone();
            SessionMethods::provide(
                ctx,
                &self.scope,
                "notes",
                vec![
                    MethodDecl::new("notes.echo", false, Interruption::Drain),
                    MethodDecl::new("notes.wait", true, Interruption::Report),
                ],
                Arc::new(move |method, params, cancel| {
                    let state = state.clone();
                    Box::pin(async move {
                        if method == "notes.wait" {
                            state.entered.notify_one();
                            cancel.cancelled().await;
                            state.settled.notify_one();
                        }
                        Ok(json!({"method": method, "params": params}))
                    })
                }),
            )?;
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for NotesProvider {
    fn keys(&self) -> KeysOf<Self> {}
}
fn app_error(reply: &Value) -> &str {
    reply["error"]["data"]["appError"]
        .as_str()
        .unwrap_or_default()
}

/// Methods contract A.1 and Appendix C: Rpc routes a name inside a recorded
/// Session namespace through the Session its `sessionId` names; every other
/// name is `unknown_method` before its params are read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn contributed_session_methods_dispatch_through_the_session_they_name() {
    let notes = Notes::default();
    let (server, _) = launch_with(Arc::new(MockTools), ServerConfig::default(), |builder| {
        notes.install(builder)
    })
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let created = client.rpc("session.create", create(json!([]))).await;
    let sid = created["result"]["sessionId"].as_str().unwrap().to_owned();
    // The provider receives the params as sent, `sessionId` included.
    let params = json!({"sessionId": sid, "branchId": "main", "text": "hi"});
    let reply = client.rpc("notes.echo", params.clone()).await;
    assert_eq!(
        reply["result"],
        json!({"method": "notes.echo", "params": params})
    );
    // This profile serves Session methods to every transport, as built-ins.
    let mut remote = Client::new_version(&server, true, 3).await;
    assert_ok(&remote.rpc("notes.echo", json!({"sessionId": sid})).await);

    // Outside every recorded namespace, or outside the grammar, a name is
    // unknown whatever its params.
    for (method, params) in [
        ("other.echo", json!({"sessionId": sid})),
        ("other.echo", json!([1])),
        ("notes.Echo", json!({})),
        ("session.notes", json!({"sessionId": sid})),
    ] {
        let reply = client.rpc(method, params).await;
        assert_eq!(reply["error"]["code"], -32601, "{method}: {reply}");
        assert_eq!(app_error(&reply), "unknown_method", "{method}");
    }
    // A candidate must name its Session.
    for params in [json!({}), json!({"sessionId": 7}), json!([sid])] {
        let reply = client.rpc("notes.echo", params).await;
        assert_eq!(reply["error"]["code"], -32602, "{reply}");
    }
    // A listed namespace whose provider does not declare the name.
    for method in ["notes", "notes.missing"] {
        let reply = client.rpc(method, json!({"sessionId": sid})).await;
        assert_eq!(app_error(&reply), "unknown_method", "{method}");
    }
    let unknown = client
        .rpc(
            "notes.echo",
            json!({"sessionId": uuid::Uuid::new_v4().to_string()}),
        )
        .await;
    assert_eq!(app_error(&unknown), "session_closed");
    // Session methods are protocol v3, and a Session belongs to the version
    // that created it.
    let mut v2 = Client::new_version(&server, false, 2).await;
    let reply = v2.rpc("notes.echo", json!({"sessionId": sid})).await;
    assert_eq!(app_error(&reply), "unknown_method");
    let created = v2.rpc("session.create", create(json!([]))).await;
    let other = created["result"]["sessionId"].as_str().unwrap();
    let reply = client.rpc("notes.echo", json!({"sessionId": other})).await;
    assert_eq!(app_error(&reply), "session_protocol_version");

    assert_ok(&client.rpc("session.close", json!({"sessionId": sid})).await);
    let closed = client.rpc("notes.echo", json!({"sessionId": sid})).await;
    assert_eq!(app_error(&closed), "session_closed");
    server.shutdown();
    server.join().await.unwrap();
}

/// A reloading provider affects only its namespace: admitted `Report` calls
/// are interrupted, and a recorded namespace without an active provider is
/// retryable until the next generation serves it. Closing the Session
/// interrupts its calls the same way.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reloading_or_closed_session_namespace_answers_as_specified() {
    let notes = Notes::default();
    let (server, _) = launch_with(Arc::new(MockTools), ServerConfig::default(), |builder| {
        notes.install(builder)
    })
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let created = client.rpc("session.create", create(json!([]))).await;
    let sid = created["result"]["sessionId"].as_str().unwrap().to_owned();
    let wait = client.send("notes.wait", json!({"sessionId": sid})).await;
    notes.0.entered.notified().await;
    let gate = notes.hold_next();
    let restart = tokio::spawn({
        let notes = notes.clone();
        async move { notes.restart().await }
    });
    let interrupted = client.response(wait).await;
    assert_eq!(app_error(&interrupted), "operation_interrupted");
    notes.0.settled.notified().await;
    let reloading = client.rpc("notes.echo", json!({"sessionId": sid})).await;
    assert_eq!(reloading["error"]["code"], -32000);
    assert_eq!(app_error(&reloading), "service_reloading");
    // Rpc and the Session's built-ins are unaffected.
    assert_ok(&client.rpc("daemon.ping", json!({})).await);
    gate.notify_one();
    restart.await.unwrap();
    assert_ok(&client.rpc("notes.echo", json!({"sessionId": sid})).await);

    let wait = client.send("notes.wait", json!({"sessionId": sid})).await;
    notes.0.entered.notified().await;
    let mut other = Client::new_version(&server, false, 3).await;
    assert_ok(&other.rpc("session.close", json!({"sessionId": sid})).await);
    assert_eq!(
        app_error(&client.response(wait).await),
        "operation_interrupted"
    );
    let closed = client.rpc("notes.echo", json!({"sessionId": sid})).await;
    assert_eq!(app_error(&closed), "session_closed");
    server.shutdown();
    server.join().await.unwrap();
}

/// Rpc acquires the Session through the index, which restores one that is
/// not loaded, before it dispatches.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_method_restores_an_unloaded_session() {
    let directory = std::env::temp_dir().join(format!("dim-methods-{}", uuid::Uuid::new_v4()));
    let config = ServerConfig {
        database_path: Some(directory.join("dimcode.sqlite")),
        ..Default::default()
    };
    let notes = Notes::default();
    let (server, _) = launch_with(Arc::new(MockTools), config.clone(), |builder| {
        notes.install(builder)
    })
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let created = client.rpc("session.create", create(json!([]))).await;
    let sid = created["result"]["sessionId"].as_str().unwrap().to_owned();
    drop(client);
    server.shutdown();
    server.join().await.unwrap();

    let (server, agent) = launch_with(Arc::new(MockTools), config, |builder| {
        notes.install(builder)
    })
    .await;
    assert!(agent.get_session(&sid).is_none());
    let mut client = Client::new_version(&server, false, 3).await;
    assert_ok(&client.rpc("notes.echo", json!({"sessionId": sid})).await);
    assert!(agent.get_session(&sid).is_some());
    drop(client);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
