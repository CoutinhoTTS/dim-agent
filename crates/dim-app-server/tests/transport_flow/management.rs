use super::*;
use dim_app_server::methods::{Handler, Interruption, MethodDecl, MethodLevel, Methods};

#[derive(Clone, Default)]
struct Fixture {
    views: Arc<std::sync::Mutex<Vec<rutis::FiberView>>>,
    management: Arc<std::sync::Mutex<Option<Arc<Methods>>>>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    committed: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
}
struct Owner(Fixture);
impl TypedPlugin for Owner {
    type Deps = (Arc<Methods>,);
    fn name(&self) -> &str {
        "ManagementFixture"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (management,): Self::Deps,
    ) -> rutis::BoxFuture<'a, Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            *self.0.management.lock().unwrap() = Some(management);
            let first = ctx.plugin(capability_plugin(Contributor(self.0.clone(), true)));
            ctx.plugin(capability_plugin(Contributor(self.0.clone(), false)));
            self.0.views.lock().unwrap().push(first);
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for Owner {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}
struct Contributor(Fixture, bool);
impl TypedPlugin for Contributor {
    type Deps = (Arc<Methods>,);
    fn name(&self) -> &str {
        if self.1 {
            "FirstManagement"
        } else {
            "OtherManagement"
        }
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (management,): Self::Deps,
    ) -> rutis::BoxFuture<'a, Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            let fixture = self.0.clone();
            let first = self.1;
            let handler: Handler = Arc::new(move |method, params, cancel| {
                let fixture = fixture.clone();
                Box::pin(async move {
                    if method == "shell.wait" {
                        fixture.entered.notify_one();
                        cancel.cancelled().await;
                        fixture.cancelled.fetch_add(1, Ordering::SeqCst);
                    }
                    if first && params["hold"] == true {
                        fixture.entered.notify_one();
                        fixture.release.notified().await;
                        fixture.committed.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(json!({"owner": if first { "first" } else { "other" }}))
                })
            });
            let registration = management
                .register(
                    if first { "shell" } else { "fixture" },
                    if first {
                        vec![
                            MethodDecl::management("shell.run", true, Interruption::Drain),
                            MethodDecl::management("shell.wait", false, Interruption::Report),
                        ]
                    } else {
                        vec![MethodDecl::management(
                            "fixture.other",
                            false,
                            Interruption::Drain,
                        )]
                    },
                    vec![],
                    handler,
                )
                .map_err(|error| rutis::CordisError::PluginFailed(error.message.into()))?;
            ctx.effect(move || {
                rutis::Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        registration.unregister_and_drain().await;
                        Ok(())
                    })
                }))
            })?;
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for Contributor {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn management_contributions_reload_without_disrupting_sessions_and_drain_disconnected_mutations()
 {
    let fixture = Fixture::default();
    let install = fixture.clone();
    let (server, _) = launch_with(
        Arc::new(MockTools),
        ServerConfig::default(),
        move |builder| {
            builder.session_plugin(move |_| Owner(install.clone()));
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    assert!(
        !client
            .features
            .as_array()
            .unwrap()
            .contains(&json!("shell.run"))
    );
    let created = client.rpc("session.create", create(json!([]))).await;
    assert_ok(&created);
    let session = created["result"]["sessionId"].clone();
    let catalog = client.rpc("management.methods", json!({})).await;
    assert!(
        catalog["result"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .any(|method| method["name"] == "shell.run" && method["mutating"] == true)
    );
    let mut later = Client::new_version(&server, false, 3).await;
    assert!(
        later
            .features
            .as_array()
            .unwrap()
            .contains(&json!("shell.run"))
    );
    let management = fixture.management.lock().unwrap().clone().unwrap();
    let duplicate: Handler = Arc::new(|_, _, _| Box::pin(async { Ok(json!({})) }));
    let revision = catalog["result"]["revision"].as_u64().unwrap();
    // Namespace ownership is atomic and independent of mount order. A child
    // namespace must not silently capture another contributor's existing name.
    for namespace in [
        "hello.nested",
        "agent.extra",
        "session",
        "session.mcp.extra",
        "session.create.extra",
        "shell.run",
    ] {
        assert!(
            management
                .register(
                    namespace,
                    vec![MethodDecl::management(
                        &format!("{namespace}.extra"),
                        false,
                        Interruption::Drain
                    )],
                    vec![],
                    duplicate.clone()
                )
                .is_err(),
            "{namespace}"
        );
    }
    assert!(
        management
            .register(
                "fixture",
                vec![MethodDecl::management(
                    "fixture.partial",
                    false,
                    Interruption::Drain
                )],
                vec![],
                duplicate.clone()
            )
            .is_err()
    );
    assert_eq!(
        client.rpc("fixture.partial", json!({})).await["error"]["code"],
        -32601
    );
    let nested = management
        .register(
            "fixture.nested",
            vec![MethodDecl::management(
                "fixture.nested.read",
                false,
                Interruption::Drain,
            )],
            vec![],
            duplicate.clone(),
        )
        .unwrap();
    assert_ok(&client.rpc("fixture.nested.read", json!({})).await);
    assert_eq!(
        client.rpc("fixture.nested.unknown", json!({})).await["error"]["code"],
        -32601
    );
    nested.unregister_and_drain().await;
    assert_eq!(
        management.catalog(MethodLevel::Management)["revision"],
        revision + 2
    );
    assert_eq!(
        client.rpc("fixture.nested.read", json!({})).await["error"]["code"],
        -32601
    );
    assert_eq!(
        client.rpc("shell.run", json!({})).await["result"]["owner"],
        "first"
    );
    let _ = client.send("shell.run", json!({"hold":true})).await;
    fixture.entered.notified().await;
    drop(client);
    let interrupted = later.send("shell.wait", json!({})).await;
    fixture.entered.notified().await;
    let view = fixture.views.lock().unwrap().last().unwrap().clone();
    let reload = tokio::spawn(async move {
        view.restart().await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let catalog = later.rpc("management.methods", json!({})).await;
            if !catalog["result"]["methods"]
                .as_array()
                .unwrap()
                .iter()
                .any(|method| method["name"] == "shell.run")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        later.response(interrupted).await["error"]["data"]["appError"],
        "operation_interrupted"
    );
    assert!(
        !reload.is_finished(),
        "unload must drain the accepted mutation"
    );
    assert_eq!(
        later.rpc("shell.run", json!({})).await["error"]["code"],
        -32601
    );
    assert_eq!(
        later.rpc("fixture.other", json!({})).await["result"]["owner"],
        "other"
    );
    let listed = later.rpc("session.list", json!({})).await;
    assert_eq!(listed["result"][0]["sessionId"], session);
    fixture.release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), reload)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fixture.committed.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.cancelled.load(Ordering::SeqCst), 1);
    assert_eq!(
        later.rpc("shell.run", json!({})).await["result"]["owner"],
        "first"
    );
    // Entry reload interrupts Report calls but cannot finish ahead of Drain calls.
    let drain = later.send("shell.run", json!({"hold":true})).await;
    fixture.entered.notified().await;
    let report = later.send("shell.wait", json!({})).await;
    fixture.entered.notified().await;
    {
        let restart = server.restart("Rpc");
        tokio::pin!(restart);
        tokio::select! {
            result = &mut restart => panic!("RPC unloaded before draining its call: {result:?}"),
            response = later.response(report) => assert_eq!(response["error"]["data"]["appError"], "operation_interrupted"),
        }
        fixture.release.notify_one();
        restart.await.unwrap();
    }
    assert_ok(&later.response(drain).await);
    assert_eq!(fixture.committed.load(Ordering::SeqCst), 2);
    assert_ok(&later.rpc("fixture.other", json!({})).await);
    let mut remote = Client::new_version(&server, true, 3).await;
    assert!(
        !remote
            .features
            .as_array()
            .unwrap()
            .contains(&json!("shell.run"))
    );
    assert_eq!(
        remote.rpc("shell.run", json!({})).await["error"]["data"]["appError"],
        "unknown_method"
    );
    server.shutdown();
    server.join().await.unwrap();
}
