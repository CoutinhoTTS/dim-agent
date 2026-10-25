//! Per-call lookup, provider-owned calls and namespace ownership contracts.
use super::*;
use dim_agent_capabilities::methods::{Handler, Interruption};
use dim_agent_runtime::{
    names::{NamedDeps, register},
    register_part, wait_active,
};
use rutis::{Keyed, KeyedGate, TypeKey};
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
    type Deps = (Keyed<ManagementMethods>,);
    fn name(&self) -> &str {
        self.name
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(methods),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            methods.register(
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
    type Deps = (KeyedGate<Missing>, Keyed<ManagementMethods>);
    fn name(&self) -> &str {
        "Waiting"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (_, Keyed(methods)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            methods.register(
                ctx,
                "pending",
                vec![decl("pending.read", Interruption::Drain)],
                Arc::new(|_, _, _, _| Box::pin(async { Ok(json!({})) })),
            )?;
            Ok(Effect::Done)
        })
    }
}

/// A provider whose apply fails before it registers `broken`.
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
    register_part(builtins, &catalog(), move |names| names.part(make())).unwrap()
}
fn catalog() -> dim_agent_runtime::names::ServiceCatalog {
    let mut catalog = dim_agent_runtime::names::catalog();
    register::<Missing>(&mut catalog, None);
    register::<ManagementMethods>(&mut catalog, None);
    catalog
}
fn decl(name: &str, interruption: Interruption) -> MethodDecl {
    MethodDecl::new(name, false, interruption)
}
fn rejected(result: Result<Accepted, RpcError>) -> String {
    result.err().unwrap().data.unwrap().app_error
}

/// Owner rows in a real loader, as the process assembles them, and the
/// management table they register with.
struct Assembly {
    root: Ctx,
    loader: Loader,
    table: Arc<ManagementMethods>,
}
impl Assembly {
    /// `rows` registers each row and returns its loader rows; the table's
    /// row comes first.
    async fn new(rows: impl FnOnce(&mut Builtins) -> Vec<Value>) -> Self {
        let root = Ctx::root().unwrap();
        let mut builtins = Builtins::new();
        let table = register_part(&mut builtins, &catalog(), |names| {
            names.part(ManagementMethodsPlugin::new(names)?)
        })
        .unwrap();
        let rows = std::iter::once(row(table))
            .chain(rows(&mut builtins))
            .collect();
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
        let table = root
            .get_as::<ManagementMethods>(TypeKey::of::<ManagementMethods>())
            .expect("the management table is active");
        Self {
            root,
            loader,
            table,
        }
    }
    fn dispatch(&self, method: &str, entry: &CancellationToken) -> Result<Accepted, RpcError> {
        dispatch(
            &self.table,
            &self.loader,
            method.into(),
            json!({}),
            entry.clone(),
            |_| {},
        )
    }
    fn listed(&self) -> Vec<String> {
        self.table
            .listed()
            .into_iter()
            .map(|decl| decl.name)
            .collect()
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
    /// Its next generations fail before registering.
    fail_again: std::sync::atomic::AtomicBool,
}
/// `fixture.*` with a blocking Drain write, a Report wait and a Drain read
/// that returns the provider generation.
#[derive(Clone)]
struct Fixture(Arc<State>);
impl TypedPlugin for Fixture {
    type Deps = (Keyed<ManagementMethods>,);
    fn name(&self) -> &str {
        "Fixture"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(methods),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let state = self.0.clone();
            let generation = state.generation.fetch_add(1, Ordering::SeqCst);
            if generation > 0 && state.fail_again.load(Ordering::SeqCst) {
                return Err(CordisError::PluginFailed("fixture fails again".into()));
            }
            methods.register(
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
        let assembly =
            Assembly::new(|builtins| vec![row(part(builtins, move || fixture.clone()))]).await;
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
    let other = Owner::new("Other", "other", &["other.read"]);
    let assembly = Assembly::new(|builtins| {
        vec![
            row(part(builtins, move || fixture.clone())),
            row(part(builtins, || Waiting)),
            row(part(builtins, || Broken)),
            disabled(part(builtins, move || off.clone())),
            row(part(builtins, move || other.clone())),
            // Names a module the loader cannot resolve.
            json!({"id": "Unresolvable", "name": "unregistered"}),
        ]
    })
    .await;
    assembly.state("Waiting", FiberState::Pending).await;
    assembly.state("Broken", FiberState::Failed).await;
    assembly.state("Other", FiberState::Active).await;
    assert!(matches!(
        assembly
            .loader
            .get("Unresolvable")
            .map(|entry| entry.status),
        Some(EntryStatus::Unresolved(_))
    ));
    let entry = CancellationToken::new();
    // A namespace nothing has registered is unknown: an owner that has not
    // applied yet, one that failed before registering, one that cannot be
    // resolved or is disabled, and a name no plugin serves.
    for method in [
        "pending.read",
        "broken.read",
        "unresolvable.read",
        "off.read",
        "absent.read",
    ] {
        assert_eq!(
            rejected(assembly.dispatch(method, &entry)),
            "unknown_method",
            "{method}"
        );
    }
    // A plugin's registration is its opt-in.
    assembly
        .dispatch("other.read", &entry)
        .unwrap()
        .reply
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        assembly.listed(),
        [
            "fixture.read",
            "fixture.wait",
            "fixture.write",
            "other.read"
        ]
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
    // A registered owner whose next generation fails answers from its row:
    // it cannot run again until it is fixed or restarted, so clients must
    // not retry.
    state.fail_again.store(true, Ordering::SeqCst);
    let _ = assembly.loader.restart("Fixture").await;
    assembly.state("Fixture", FiberState::Failed).await;
    let error = assembly.dispatch("fixture.read", &entry).err().unwrap();
    assert_eq!(error.code, -32000);
    assert_eq!(error.data.unwrap().app_error, "service_unavailable");
    assembly.root.shutdown().await.unwrap();
}

#[tokio::test]
async fn namespace_validation_and_nested_ownership_follow_the_longest_provided_namespace() {
    let root = Ctx::root().unwrap();
    let table = root.plugin(rutis::Typed::new(ManagementMethodsPlugin {
        table: DepKey::of(),
    }));
    let cancellation = CancellationToken::new();
    wait_active(&root, &[table], Duration::from_secs(5), &cancellation)
        .await
        .unwrap();
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
        let invalid = root.plugin(rutis::Typed::with_keys(
            Owner::new("Owner", namespace, &[read.as_str()]),
            (DepKey::of(),),
        ));
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
        let assembly = Assembly::new(|builtins| {
            let child = part(builtins, move || child.clone());
            vec![
                row(part(builtins, move || parent.clone())),
                if child_enabled {
                    row(child)
                } else {
                    disabled(child)
                },
            ]
        })
        .await;
        assembly.state("Parent", FiberState::Active).await;
        assembly
            .dispatch("fixture.read", &entry)
            .unwrap()
            .reply
            .await
            .unwrap()
            .unwrap();
        if child_enabled {
            // A registered child namespace is the child's: the parent's
            // method inside it is neither dispatched nor listed, and the
            // parent keeps its other methods.
            assembly.state("Child", FiberState::Active).await;
            assert_eq!(
                rejected(assembly.dispatch("fixture.nested.read", &entry)),
                "unknown_method"
            );
            assembly
                .dispatch("fixture.nested.other", &entry)
                .unwrap()
                .reply
                .await
                .unwrap()
                .unwrap();
            assert_eq!(assembly.listed(), ["fixture.nested.other", "fixture.read"]);
        } else {
            // A child that never registered holds nothing: the parent serves
            // and lists the method.
            assembly
                .dispatch("fixture.nested.read", &entry)
                .unwrap()
                .reply
                .await
                .unwrap()
                .unwrap();
            assert_eq!(assembly.listed(), ["fixture.nested.read", "fixture.read"]);
            assert_eq!(
                rejected(assembly.dispatch("fixture.nested.other", &entry)),
                "unknown_method"
            );
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
