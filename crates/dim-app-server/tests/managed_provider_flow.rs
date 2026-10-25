use dim_agent_runtime::Names;
use dim_app_server::managed_provider::{
    ManagedProviderBinding, ManagedProviders, ManagedProvidersPlugin,
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, FiberState, Keyed, Typed, TypedPlugin};
use rutis_bridge::session::{HostDispatch, Reply, Value};
use serde_json::json;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Fixture {
    issued: Mutex<Vec<String>>,
    released: Mutex<Vec<String>>,
    response_error: Mutex<Option<&'static str>>,
    entered: Notify,
    resume: Notify,
    preflight_cancelled: Notify,
}
impl HostDispatch for Fixture {
    fn invoke(&self, method: &str, args: Value) -> Reply {
        let args = args.json().unwrap();
        let mut value = match method {
            "providers" => json!([]),
            "quiesce" => {
                self.preflight_cancelled.notify_one();
                json!({})
            }
            "release" => {
                self.released
                    .lock()
                    .unwrap()
                    .push(args[0]["leaseId"].as_str().unwrap().into());
                json!(null)
            }
            "credential" | "recoverRejected" => {
                let mut issued = self.issued.lock().unwrap();
                let id = format!("lease-{}", issued.len());
                issued.push(id.clone());
                json!({"leaseId":id,"account":{"credentialEpoch":"epoch","generation":0,
                    "account":{"issuer":"fixture","subject":"subject","grantedScopes":[]}},
                    "protocol":"openai-compatible","baseUrl":"http://localhost/v1",
                    "headers":{},"bearer":"private-fixture-token"})
            }
            _ => panic!("unexpected operation {method}"),
        };
        if matches!(method, "credential" | "recoverRejected") {
            match *self.response_error.lock().unwrap() {
                Some("decode") => {
                    value.as_object_mut().unwrap().remove("bearer");
                }
                Some("url") => value["baseUrl"] = json!("file:///private"),
                Some("account") => value["account"]["generation"] = json!(1),
                _ => {}
            }
        }
        Ok(json!({"ok":true,"value":value}).into())
    }
}
struct AsyncFixture(Arc<Fixture>);
impl HostDispatch for AsyncFixture {
    fn invoke(&self, method: &str, args: Value) -> Reply {
        if method == "status" {
            let owner = self.0.clone();
            Ok(Value::future(async move {
                owner.entered.notify_one();
                owner.preflight_cancelled.notified().await;
                Ok(
                    json!({"ok":false,"error":{"code":"service_reloading","message":"stopping"}})
                        .into(),
                )
            }))
        } else if matches!(method, "credential" | "recoverRejected") {
            let owner = self.0.clone();
            let method = method.to_owned();
            Ok(Value::future(async move {
                owner.entered.notify_one();
                owner.resume.notified().await;
                owner.invoke(&method, args)
            }))
        } else {
            self.0.invoke(method, args)
        }
    }
}
struct FixturePlugin(Arc<Fixture>);
impl TypedPlugin for FixturePlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "managed-fixture"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as::<dyn HostDispatch>(
                rutis_bridge::session::host_key("managedProvider"),
                Arc::new(AsyncFixture(self.0.clone())),
            )?;
            Ok(Effect::Done)
        })
    }
}
struct Capture(Arc<OnceLock<Arc<ManagedProviders>>>);
impl TypedPlugin for Capture {
    type Deps = Keyed<ManagedProviders>;
    fn name(&self) -> &str {
        "capture-managed"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        Keyed(owner): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let _ = self.0.set(owner);
            Ok(Effect::Done)
        })
    }
}
async fn active(view: &rutis::FiberView) {
    let mut watch = view.watch();
    while watch.borrow().state != FiberState::Active {
        assert_ne!(
            watch.borrow().state,
            FiberState::Failed,
            "{:?}",
            *watch.borrow()
        );
        watch.changed().await.unwrap();
    }
}

async fn setup() -> (Ctx, Arc<Fixture>, Arc<ManagedProviders>, rutis::FiberView) {
    let root = Ctx::root().unwrap();
    // The management table the process providers register with.
    root.plugin(
        dim_app_server::methods::ManagementMethodsPlugin::typed(&dim_app_server::catalog())
            .unwrap(),
    );
    let fixture = Arc::new(Fixture::default());
    let captured = Arc::new(OnceLock::new());
    let catalog = dim_app_server::catalog();
    let owner_plugin = root.plugin(Typed::new(ManagedProvidersPlugin));
    let fixture_plugin = root.plugin(Typed::new(FixturePlugin(fixture.clone())));
    let binding = root
        .plugin(ManagedProviderBinding::part(&Names::copy(&catalog, &Default::default())).unwrap());
    let capture = root.plugin(Typed::new(Capture(captured.clone())));
    for view in [&owner_plugin, &fixture_plugin, &binding, &capture] {
        active(view).await;
    }
    (root, fixture, captured.get().unwrap().clone(), binding)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abandoned_credential_and_recovery_waiters_release_every_issued_lease() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        for recovery in [false, true] {
            let (root, fixture, owner, _) = setup().await;
            // Normal control proves the fixture's release observation works.
            fixture.resume.notify_one();
            let initial = owner
                .credential("fixture", "model", &CancellationToken::new())
                .await
                .unwrap();
            fixture.entered.notified().await;
            let waiter = tokio::spawn(async move {
                let cancel = CancellationToken::new();
                if recovery {
                    owner.recover(&initial, &cancel).await
                } else {
                    drop(initial);
                    owner.credential("fixture", "model", &cancel).await
                }
            });
            fixture.entered.notified().await;
            waiter.abort();
            assert!(matches!(waiter.await, Err(error) if error.is_cancelled()));
            fixture.resume.notify_one();
            root.shutdown().await.unwrap();
            let mut issued = fixture.issued.lock().unwrap().clone();
            let mut released = fixture.released.lock().unwrap().clone();
            issued.sort();
            released.sort();
            assert_eq!(issued.len(), 2);
            assert_eq!(released, issued, "recovery={recovery}");
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_lease_and_changed_recovery_responses_are_released() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        for malformed in ["decode", "url", "account"] {
            let (root, fixture, owner, _) = setup().await;
            fixture.resume.notify_one();
            let initial = owner.credential("fixture", "model", &CancellationToken::new()).await.unwrap();
            *fixture.response_error.lock().unwrap() = Some(malformed);
            fixture.resume.notify_one();
            let result = owner.recover(&initial, &CancellationToken::new()).await;
            assert!(matches!(result, Err(error) if error.code == if malformed == "account" { "account_changed" } else { "account_invalid_response" }));
            drop(initial);
            root.shutdown().await.unwrap();
            let mut released = fixture.released.lock().unwrap().clone();
            released.sort();
            assert_eq!(released, ["lease-0", "lease-1"]);
        }
    }).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reload_drains_accepted_lease_and_cooperative_cancel_does_not_abort_it() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let (root, fixture, owner, binding) = setup().await;
        let cancel = CancellationToken::new();
        let cancelled = cancel.clone();
        let caller = owner.clone();
        let waiter =
            tokio::spawn(async move { caller.credential("fixture", "model", &cancel).await });
        fixture.entered.notified().await;
        cancelled.cancel();
        fixture.resume.notify_one();
        let initial = waiter.await.unwrap().unwrap();
        // Cancellation after admission drains rather than aborts secret work.
        drop(initial);
        let caller = owner.clone();
        let waiter = tokio::spawn(async move {
            caller
                .credential("fixture", "model", &CancellationToken::new())
                .await
        });
        fixture.entered.notified().await;
        // Consume the notification for the second operation (the first one was
        // consumed before cancellation), then revoke its generation.
        let restarting = tokio::spawn(async move {
            binding.restart().await.unwrap();
            active(&binding).await;
        });
        loop {
            if owner
                .call::<serde_json::Value>("providers", json!(null), &CancellationToken::new())
                .await
                .is_err()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        fixture.resume.notify_one();
        assert!(waiter.await.unwrap().is_err());
        restarting.await.unwrap();
        fixture.resume.notify_one();
        drop(
            owner
                .credential("fixture", "model", &CancellationToken::new())
                .await
                .unwrap(),
        );
        root.shutdown().await.unwrap();
        let mut released = fixture.released.lock().unwrap().clone();
        released.sort();
        assert_eq!(released, ["lease-0", "lease-1", "lease-2"]);
        assert_eq!(fixture.issued.lock().unwrap().len(), 3);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn generation_cannot_finish_closing_before_a_delivered_lease_is_released() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let (root, fixture, owner, _) = setup().await;
        fixture.resume.notify_one();
        let lease = owner
            .credential("fixture", "model", &CancellationToken::new())
            .await
            .unwrap();
        let mut closing = tokio::spawn(async move { root.shutdown().await });
        while owner
            .call::<serde_json::Value>("providers", json!(null), &CancellationToken::new())
            .await
            .is_ok()
        {
            tokio::task::yield_now().await;
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut closing)
                .await
                .is_err(),
            "generation closed while a caller still held its credential"
        );
        drop(lease);
        closing.await.unwrap().unwrap();
        assert_eq!(*fixture.issued.lock().unwrap(), ["lease-0"]);
        assert_eq!(*fixture.released.lock().unwrap(), ["lease-0"]);
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unload_quiesces_preflight_before_draining_calls() {
    let (root, fixture, owner, _) = setup().await;
    let waiter = tokio::spawn(async move {
        owner
            .call::<serde_json::Value>(
                "status",
                json!({"providerId":"fixture"}),
                &CancellationToken::new(),
            )
            .await
    });
    fixture.entered.notified().await;
    tokio::time::timeout(std::time::Duration::from_secs(1), root.shutdown())
        .await
        .expect("shutdown waited for work that only quiesce can cancel")
        .unwrap();
    assert!(waiter.await.unwrap().is_err());
}
