//! Per-call lookup, provider-owned calls and namespace ownership contracts.
use super::*;
use dim_agent_runtime::{Capability, KeysOf, capability_plugin, wait_active};
use rutis::{FiberState, Gate, TypedPlugin};
use serde_json::json;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

/// Provides one namespace with a fixed handler.
struct Owner {
    namespace: &'static str,
    declarations: Vec<MethodDecl>,
    handler: Handler,
}
impl TypedPlugin for Owner {
    type Deps = ();
    fn name(&self) -> &str {
        "Owner"
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
impl Capability for Owner {
    fn keys(&self) -> KeysOf<Self> {}
}

/// A provider that stays Pending on a service nobody provides.
struct Missing;
struct Waiting;
impl TypedPlugin for Waiting {
    type Deps = Gate<Missing>;
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
                Arc::new(|_, _, _| Box::pin(async { Ok(json!({})) })),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for Waiting {
    fn keys(&self) -> KeysOf<Self> {}
}

fn decl(name: &str, interruption: Interruption) -> MethodDecl {
    MethodDecl::management(name, false, interruption)
}
fn rejected(result: Result<Accepted, RpcError>) -> &'static str {
    result.err().unwrap().data.unwrap().app_error
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
                Arc::new(move |method, _, cancel| {
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
impl Capability for Fixture {
    fn keys(&self) -> KeysOf<Self> {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_reload_interrupts_report_calls_and_waits_for_drain_calls() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let root = Ctx::root().unwrap();
        let state = Arc::new(State::default());
        let owner = root.plugin(capability_plugin(Fixture(state.clone())));
        let cancellation = CancellationToken::new();
        wait_active(
            &root,
            std::slice::from_ref(&owner),
            Duration::from_secs(5),
            &cancellation,
        )
        .await
        .unwrap();
        // The calling load never unloads here: only the provider reloads.
        let entry = CancellationToken::new();
        let write = dispatch(&root, "fixture.write".into(), json!({}), entry.clone()).unwrap();
        state.entered.notified().await;
        let wait = dispatch(&root, "fixture.wait".into(), json!({}), entry.clone()).unwrap();
        state.entered.notified().await;
        let restarting = owner.clone();
        let restart = tokio::spawn(async move { restarting.restart().await.unwrap() });
        let interrupted = wait.reply.await.unwrap().unwrap_err();
        assert_eq!(interrupted.data.unwrap().app_error, "operation_interrupted");
        assert!(interrupted.message.contains("may already have committed"));
        state.cancelled.notified().await;
        assert!(
            !restart.is_finished(),
            "an accepted drain call holds its provider's unload"
        );
        // Between generations the namespace is unknown, never a hidden queue.
        assert_eq!(
            rejected(dispatch(
                &root,
                "fixture.read".into(),
                json!({}),
                entry.clone()
            )),
            "unknown_method"
        );
        state.release.notify_one();
        assert_eq!(write.reply.await.unwrap().unwrap(), json!(0));
        restart.await.unwrap();
        wait_active(&root, &[owner], Duration::from_secs(5), &cancellation)
            .await
            .unwrap();
        assert_eq!(state.committed.load(Ordering::SeqCst), 1);
        assert_eq!(
            dispatch(&root, "fixture.read".into(), json!({}), entry)
                .unwrap()
                .reply
                .await
                .unwrap()
                .unwrap(),
            json!(1)
        );
        root.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_caller_unload_interrupts_report_calls_and_an_inactive_provider_is_isolated() {
    let root = Ctx::root().unwrap();
    let state = Arc::new(State::default());
    let owner = root.plugin(capability_plugin(Fixture(state.clone())));
    let pending = root.plugin(capability_plugin(Waiting));
    let cancellation = CancellationToken::new();
    wait_active(
        &root,
        std::slice::from_ref(&owner),
        Duration::from_secs(5),
        &cancellation,
    )
    .await
    .unwrap();
    assert_eq!(pending.state().state, FiberState::Pending);
    // A provider that never becomes active affects only its own namespace.
    assert_eq!(
        rejected(dispatch(
            &root,
            "pending.read".into(),
            json!({}),
            CancellationToken::new()
        )),
        "unknown_method"
    );
    assert_eq!(
        listed(&root, &["fixture".into(), "pending".into()]),
        ["fixture.read", "fixture.wait", "fixture.write"]
    );
    // The calling load's unload interrupts a Report call; the provider stays.
    let entry = CancellationToken::new();
    let wait = dispatch(&root, "fixture.wait".into(), json!({}), entry.clone()).unwrap();
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
    assert_eq!(owner.state().state, FiberState::Active);
    root.shutdown().await.unwrap();
}

#[tokio::test]
async fn namespace_validation_and_nested_ownership_follow_the_longest_active_namespace() {
    let root = Ctx::root().unwrap();
    let handler: Handler = Arc::new(|_, _, _| Box::pin(async { Ok(json!({})) }));
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
        let invalid = root.plugin(capability_plugin(Owner {
            namespace,
            declarations: vec![decl(&format!("{namespace}.read"), Interruption::Drain)],
            handler: handler.clone(),
        }));
        let cancellation = CancellationToken::new();
        assert!(
            wait_active(&root, &[invalid], Duration::from_secs(5), &cancellation)
                .await
                .is_err(),
            "{namespace}"
        );
    }
    let views = [
        root.plugin(capability_plugin(Owner {
            namespace: "fixture",
            declarations: vec![
                decl("fixture.read", Interruption::Drain),
                decl("fixture.nested.read", Interruption::Drain),
            ],
            handler: handler.clone(),
        })),
        root.plugin(capability_plugin(Owner {
            namespace: "fixture.nested",
            declarations: vec![decl("fixture.nested.other", Interruption::Drain)],
            handler,
        })),
    ];
    wait_active(
        &root,
        &views,
        Duration::from_secs(5),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    // An active child owns its namespace: the parent's method inside it is
    // neither dispatched nor listed, and the parent keeps its other methods.
    assert_eq!(
        rejected(dispatch(
            &root,
            "fixture.nested.read".into(),
            json!({}),
            CancellationToken::new()
        )),
        "unknown_method"
    );
    for method in ["fixture.read", "fixture.nested.other"] {
        dispatch(&root, method.into(), json!({}), CancellationToken::new())
            .unwrap()
            .reply
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(
        listed(&root, &["fixture".into(), "fixture.nested".into()]),
        ["fixture.nested.other", "fixture.read"]
    );
    // A name outside the declaration grammar is unknown without a lookup per
    // prefix: a long dotted name would otherwise cost quadratic time.
    let long = format!("fixture.{}read", "a.".repeat(1 << 17));
    let started = std::time::Instant::now();
    for method in [
        "fixture.Read",
        "fixture..read",
        "fixture.read.",
        long.as_str(),
    ] {
        assert_eq!(
            rejected(dispatch(
                &root,
                method.into(),
                json!({}),
                CancellationToken::new()
            )),
            "unknown_method"
        );
    }
    assert!(started.elapsed() < Duration::from_secs(1));
    root.shutdown().await.unwrap();
}
