//! Per-call lookup, provider-owned calls and namespace ownership contracts.
use super::*;
use dim_agent_capabilities::methods::Interruption;
use dim_agent_runtime::{
    names::{NamedDeps, register},
    register_part, wait_active,
};
use rutis::{BoxFuture, Effect, KeyedGate, TypedPlugin};
use rutis_loader::{Builtins, Layer, LoaderPlugin, Patch};
use serde_json::json;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

/// Provides one namespace with a fixed handler.
#[derive(Clone)]
struct Owner {
    name: &'static str,
    namespace: &'static str,
    declarations: Vec<MethodDecl>,
    handler: Handler,
}
impl Owner {
    fn new(name: &'static str, namespace: &'static str, declarations: &[&str]) -> Self {
        Self {
            name,
            namespace,
            declarations: declarations
                .iter()
                .map(|method| decl(method, Interruption::Drain))
                .collect(),
            handler: Arc::new(|_, _, _, _| Box::pin(async { Ok(json!({})) })),
        }
    }
}
impl TypedPlugin for Owner {
    type Deps = ();
    fn name(&self) -> &str {
        self.name
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ManagementMethods::provide(
                ctx,
                self.namespace,
                self.declarations.clone(),
                self.handler.clone(),
            )?;
            Ok(Effect::Done)
        })
    }
}

/// A provider that stays Pending on a service nobody provides.
struct Missing;
struct Waiting;
impl TypedPlugin for Waiting {
    type Deps = KeyedGate<Missing>;
    fn name(&self) -> &str {
        "Waiting"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        _: Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ManagementMethods::provide(
                ctx,
                "pending",
                vec![decl("pending.read", Interruption::Drain)],
                Arc::new(|_, _, _, _| Box::pin(async { Ok(json!({})) })),
            )?;
            Ok(Effect::Done)
        })
    }
}

/// A provider whose apply fails before it provides `broken`.
struct Broken;
impl TypedPlugin for Broken {
    type Deps = ();
    fn name(&self) -> &str {
        "Broken"
    }
    fn apply<'a>(&'a self, _: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async { Err(CordisError::PluginFailed("broken owner".into())) })
    }
}

/// Registers `make`'s part as a process row and returns its name.
fn part<P>(builtins: &mut Builtins, make: impl Fn() -> P + Send + Sync + 'static) -> String
where
    P: TypedPlugin,
    P::Deps: NamedDeps,
{
    let mut catalog = dim_agent_runtime::names::catalog();
    register::<Missing>(&mut catalog, None);
    register_part(builtins, &catalog, move |names| names.part(make())).unwrap()
}
fn decl(name: &str, interruption: Interruption) -> MethodDecl {
    MethodDecl::new(name, false, interruption)
}
fn rejected(result: Result<Accepted, RpcError>) -> String {
    result.err().unwrap().data.unwrap().app_error
}

/// Owner rows in a real loader, as the process assembles them, and the
/// namespace table Rpc receives.
struct Assembly {
    root: Ctx,
    loader: Loader,
    owners: Owners,
}
impl Assembly {
    /// `rows` registers each row and returns its loader rows.
    async fn new(
        rows: impl FnOnce(&mut Builtins) -> Vec<Value>,
        namespaces: &[(&str, &str)],
    ) -> Self {
        let root = Ctx::root().unwrap();
        let mut builtins = Builtins::new();
        let rows = rows(&mut builtins);
        let plugin = LoaderPlugin::new(builtins, Default::default());
        let loader = plugin.handle();
        let view = root.plugin(plugin);
        wait_active(
            &root,
            &[view],
            Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let assembly = Layer::new(
            "assembly",
            vec![Patch {
                insert: Some(rows),
                ..Default::default()
            }],
        );
        loader.reconcile(vec![assembly], None).await.unwrap();
        let namespaces: Vec<_> = namespaces
            .iter()
            .map(|(namespace, row)| (namespace.to_string(), row.to_string()))
            .collect();
        let owners = Owners::new(loader.clone(), &namespaces);
        Self {
            root,
            loader,
            owners,
        }
    }
    fn dispatch(&self, method: &str, entry: &CancellationToken) -> Result<Accepted, RpcError> {
        self.owners
            .dispatch(&self.root, method.into(), json!({}), entry.clone(), |_| {})
    }
    async fn state(&self, row: &str, state: FiberState) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !matches!(
                self.loader.get(row).map(|entry| entry.status),
                Some(EntryStatus::Running(snapshot)) if snapshot.state == state
            ) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{row} did not reach {state:?}"));
    }
}
fn row(name: String) -> Value {
    json!({"id": name, "name": name})
}
fn disabled(name: String) -> Value {
    json!({"id": name, "name": name, "disabled": true})
}

#[derive(Default)]
struct State {
    generation: AtomicUsize,
    committed: AtomicUsize,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    cancelled: tokio::sync::Notify,
    cancelled_count: AtomicUsize,
}
/// `fixture.*` with a blocking Drain write, a Report wait and a Drain read
/// that returns the provider generation.
#[derive(Clone)]
struct Fixture(Arc<State>);
impl TypedPlugin for Fixture {
    type Deps = ();
    fn name(&self) -> &str {
        "Fixture"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let state = self.0.clone();
            let generation = state.generation.fetch_add(1, Ordering::SeqCst);
            ManagementMethods::provide(
                ctx,
                "fixture",
                vec![
                    decl("fixture.write", Interruption::Drain),
                    decl("fixture.wait", Interruption::Report),
                    decl("fixture.read", Interruption::Drain),
                ],
                Arc::new(move |method, _, cancel, _| {
                    let state = state.clone();
                    Box::pin(async move {
                        if method == "fixture.write" {
                            state.entered.notify_one();
                            state.release.notified().await;
                            // The unload signal reaches it, but the unload waits.
                            assert!(cancel.is_cancelled());
                            state.committed.fetch_add(1, Ordering::SeqCst);
                        } else if method == "fixture.wait" {
                            state.entered.notify_one();
                            cancel.cancelled().await;
                            state.cancelled_count.fetch_add(1, Ordering::SeqCst);
                            state.cancelled.notify_one();
                        }
                        Ok(json!(generation))
                    })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_reload_interrupts_report_calls_and_waits_for_drain_calls() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let state = Arc::new(State::default());
        let fixture = Fixture(state.clone());
        let assembly = Assembly::new(
            |builtins| vec![row(part(builtins, move || fixture.clone()))],
            &[("fixture", "Fixture")],
        )
        .await;
        // The calling load never unloads here: only the provider reloads.
        let entry = CancellationToken::new();
        let write = assembly.dispatch("fixture.write", &entry).unwrap();
        state.entered.notified().await;
        let wait = assembly.dispatch("fixture.wait", &entry).unwrap();
        state.entered.notified().await;
        let loader = assembly.loader.clone();
        let restart = tokio::spawn(async move { loader.restart("Fixture").await.unwrap() });
        let interrupted = wait.reply.await.unwrap().unwrap_err();
        assert_eq!(interrupted.data.unwrap().app_error, "operation_interrupted");
        assert!(interrupted.message.contains("may already have committed"));
        state.cancelled.notified().await;
        assert!(
            !restart.is_finished(),
            "an accepted drain call holds its provider's unload"
        );
        // Between generations the namespace is refused as reloading, never
        // queued: the client retries once the next generation is active.
        assert_eq!(
            rejected(assembly.dispatch("fixture.read", &entry)),
            "service_reloading"
        );
        state.release.notify_one();
        assert_eq!(write.reply.await.unwrap().unwrap(), json!(0));
        restart.await.unwrap();
        assembly.state("Fixture", FiberState::Active).await;
        assert_eq!(state.committed.load(Ordering::SeqCst), 1);
        assert_eq!(
            assembly
                .dispatch("fixture.read", &entry)
                .unwrap()
                .reply
                .await
                .unwrap()
                .unwrap(),
            json!(1)
        );
        assembly.root.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_caller_unload_interrupts_report_calls_and_an_inactive_provider_is_isolated() {
    let state = Arc::new(State::default());
    let fixture = Fixture(state.clone());
    let off = Owner::new("Off", "off", &["off.read"]);
    let unlisted = Owner::new("Unlisted", "unlisted", &["unlisted.read"]);
    let assembly = Assembly::new(
        |builtins| {
            vec![
                row(part(builtins, move || fixture.clone())),
                row(part(builtins, || Waiting)),
                row(part(builtins, || Broken)),
                disabled(part(builtins, move || off.clone())),
                row(part(builtins, move || unlisted.clone())),
                // Names a module the loader cannot resolve.
                json!({"id": "Unresolvable", "name": "unregistered"}),
            ]
        },
        &[
            ("fixture", "Fixture"),
            ("pending", "Waiting"),
            ("broken", "Broken"),
            ("unresolvable", "Unresolvable"),
            ("off", "Off"),
            ("absent", "Absent"),
        ],
    )
    .await;
    assembly.state("Waiting", FiberState::Pending).await;
    assembly.state("Broken", FiberState::Failed).await;
    assembly.state("Unlisted", FiberState::Active).await;
    assert!(matches!(
        assembly
            .loader
            .get("Unresolvable")
            .map(|entry| entry.status),
        Some(EntryStatus::Unresolved(_))
    ));
    let entry = CancellationToken::new();
    // An enabled owner that is not active affects only its own namespace. A
    // Pending one is retryable; a Failed or Unresolved one fails the same way
    // until it is fixed or restarted, so clients must not retry it.
    assert_eq!(
        rejected(assembly.dispatch("pending.read", &entry)),
        "service_reloading"
    );
    for method in ["broken.read", "unresolvable.read"] {
        let error = assembly.dispatch(method, &entry).err().unwrap();
        assert_eq!(error.code, -32000, "{method}");
        assert_eq!(
            error.data.unwrap().app_error,
            "service_unavailable",
            "{method}"
        );
    }
    // A disabled owner, a row assembly does not have, and a provider whose
    // namespace assembly does not configure serve nothing.
    for method in ["off.read", "absent.read", "unlisted.read", "other.read"] {
        assert_eq!(
            rejected(assembly.dispatch(method, &entry)),
            "unknown_method",
            "{method}"
        );
    }
    assert_eq!(
        assembly.owners.listed(&assembly.root),
        ["fixture.read", "fixture.wait", "fixture.write"]
    );
    // The calling load's unload interrupts a Report call; the provider stays.
    let entry = CancellationToken::new();
    let wait = assembly.dispatch("fixture.wait", &entry).unwrap();
    state.entered.notified().await;
    entry.cancel();
    assert_eq!(
        wait.reply
            .await
            .unwrap()
            .unwrap_err()
            .data
            .unwrap()
            .app_error,
        "operation_interrupted"
    );
    // The interrupted operation settles after its reply; a caller that waits
    // for the settle signal never observes it still running.
    let _ = wait.settled.await;
    assert_eq!(state.cancelled_count.load(Ordering::SeqCst), 1);
    assembly.state("Fixture", FiberState::Active).await;
    assembly.root.shutdown().await.unwrap();
}

#[tokio::test]
async fn namespace_validation_and_nested_ownership_follow_the_longest_provided_namespace() {
    let root = Ctx::root().unwrap();
    for namespace in [
        "hello.nested",
        "daemon.extra",
        "agent.extra",
        "approval.extra",
        "session",
        "session.event",
        "session.goal",
        "session.mcp.extra",
        "session.create.extra",
    ] {
        let read = format!("{namespace}.read");
        let invalid = root.plugin(rutis::Typed::new(Owner::new(
            "Owner",
            namespace,
            &[read.as_str()],
        )));
        let cancellation = CancellationToken::new();
        assert!(
            wait_active(&root, &[invalid], Duration::from_secs(5), &cancellation)
                .await
                .is_err(),
            "{namespace}"
        );
    }
    root.shutdown().await.unwrap();

    let parent = Owner::new(
        "Parent",
        "fixture",
        &["fixture.read", "fixture.nested.read"],
    );
    let child = Owner::new("Child", "fixture.nested", &["fixture.nested.other"]);
    let entry = CancellationToken::new();
    for child_enabled in [true, false] {
        let (parent, child) = (parent.clone(), child.clone());
        let assembly = Assembly::new(
            |builtins| {
                let child = part(builtins, move || child.clone());
                vec![
                    row(part(builtins, move || parent.clone())),
                    if child_enabled {
                        row(child)
                    } else {
                        disabled(child)
                    },
                ]
            },
            &[("fixture", "Parent"), ("fixture.nested", "Child")],
        )
        .await;
        assembly.state("Parent", FiberState::Active).await;
        // A child namespace is the child's whether or not it is enabled: the
        // parent's method inside it is neither dispatched nor listed, and the
        // parent keeps its other methods.
        assert_eq!(
            rejected(assembly.dispatch("fixture.nested.read", &entry)),
            "unknown_method"
        );
        assembly
            .dispatch("fixture.read", &entry)
            .unwrap()
            .reply
            .await
            .unwrap()
            .unwrap();
        if child_enabled {
            assembly.state("Child", FiberState::Active).await;
            assembly
                .dispatch("fixture.nested.other", &entry)
                .unwrap()
                .reply
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                assembly.owners.listed(&assembly.root),
                ["fixture.nested.other", "fixture.read"]
            );
        } else {
            assert_eq!(
                rejected(assembly.dispatch("fixture.nested.other", &entry)),
                "unknown_method"
            );
            assert_eq!(assembly.owners.listed(&assembly.root), ["fixture.read"]);
        }
        // A name outside the declaration grammar is unknown without a lookup.
        let long = format!("fixture.{}read", "a.".repeat(1 << 17));
        let started = std::time::Instant::now();
        for method in [
            "fixture.Read",
            "fixture..read",
            "fixture.read.",
            long.as_str(),
        ] {
            assert_eq!(
                rejected(assembly.dispatch(method, &entry)),
                "unknown_method"
            );
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        assembly.root.shutdown().await.unwrap();
    }
}
