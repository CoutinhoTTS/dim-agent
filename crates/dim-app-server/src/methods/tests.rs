//! Cross-plugin contribution, reload and accepted-operation contracts.
use super::*;
use dim_agent_runtime::{Capability, KeysOf, capability_plugin, wait_active};
use rutis::{Effect, TypedPlugin};
use serde_json::json;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct State {
    generation: AtomicUsize,
    committed: AtomicUsize,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    cancelled: tokio::sync::Notify,
    entry: Mutex<Option<(Arc<MethodTable>, CancellationToken, TaskTracker)>>,
}
struct Owner(Arc<State>);
impl TypedPlugin for Owner {
    type Deps = ();
    fn name(&self) -> &str {
        "Owner"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let state = self.0.clone();
            let generation = state.generation.fetch_add(1, Ordering::SeqCst);
            ManagementMethods::provide(
                ctx,
                "fixture",
                vec![
                    MethodDecl::management("fixture.write", true, Interruption::Drain),
                    MethodDecl::management("fixture.wait", false, Interruption::Report),
                    MethodDecl::management("fixture.read", false, Interruption::Drain),
                ],
                vec![],
                Arc::new(move |method, _, cancel| {
                    let state = state.clone();
                    Box::pin(async move {
                        if method == "fixture.write" {
                            state.entered.notify_one();
                            state.release.notified().await;
                            assert!(!cancel.is_cancelled());
                            state.committed.fetch_add(1, Ordering::SeqCst);
                        } else if method == "fixture.wait" {
                            state.entered.notify_one();
                            cancel.cancelled().await;
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
impl Capability for Owner {
    fn keys(&self) -> KeysOf<Self> {}
}

struct Collector(Arc<State>);
impl TypedPlugin for Collector {
    type Deps = Contributions<ManagementMethods>;
    fn name(&self) -> &str {
        "Collector"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        methods: Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let table = Arc::new(MethodTable::new(methods)?);
            let stop = CancellationToken::new();
            let commands = TaskTracker::new();
            *self.0.entry.lock().unwrap() = Some((table, stop.clone(), commands.clone()));
            let state = self.0.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        *state.entry.lock().unwrap() = None;
                        stop.cancel();
                        commands.close();
                        commands.wait().await;
                        Ok(())
                    })
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for Collector {
    fn keys(&self) -> KeysOf<Self> {
        (None, Arc::from(["fixture".into()]))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn contributor_reload_drains_writes_and_interrupts_waits_before_replacement() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let root = Ctx::root().unwrap();
        let state = Arc::new(State::default());
        let owner = root.plugin(capability_plugin(Owner(state.clone())));
        let collector = root.plugin(capability_plugin(Collector(state.clone())));
        let cancellation = CancellationToken::new();
        wait_active(
            &root,
            &[owner.clone(), collector.clone()],
            Duration::from_secs(5),
            &cancellation,
        )
        .await
        .unwrap();
        let (table, stop, commands) = state.entry.lock().unwrap().clone().unwrap();
        let disconnected = CancellationToken::new();
        let write = table
            .dispatch(
                "fixture.write".into(),
                json!({}),
                disconnected.clone(),
                stop.clone(),
                &commands,
            )
            .unwrap();
        state.entered.notified().await;
        disconnected.cancel();
        let wait = table
            .dispatch(
                "fixture.wait".into(),
                json!({}),
                CancellationToken::new(),
                stop,
                &commands,
            )
            .unwrap();
        state.entered.notified().await;
        let restarting = owner.clone();
        let restart = tokio::spawn(async move {
            restarting.restart().await.unwrap();
        });
        assert_eq!(
            wait.await.unwrap().unwrap_err().data.unwrap().app_error,
            "operation_interrupted"
        );
        state.cancelled.notified().await;
        assert!(
            !restart.is_finished(),
            "accepted write must finish before owner replacement"
        );
        assert!(state.entry.lock().unwrap().is_none());
        state.release.notify_one();
        assert_eq!(write.await.unwrap().unwrap(), json!(0));
        restart.await.unwrap();
        wait_active(
            &root,
            &[owner, collector],
            Duration::from_secs(5),
            &cancellation,
        )
        .await
        .unwrap();
        assert_eq!(state.committed.load(Ordering::SeqCst), 1);
        let (table, stop, commands) = state.entry.lock().unwrap().clone().unwrap();
        assert_eq!(
            table
                .dispatch(
                    "fixture.read".into(),
                    json!({}),
                    CancellationToken::new(),
                    stop,
                    &commands,
                )
                .unwrap()
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

#[tokio::test]
async fn namespace_validation_and_nested_ownership_are_order_independent() {
    let root = Ctx::root().unwrap();
    let handler: Handler = Arc::new(|_, _, _| Box::pin(async { Ok(json!({})) }));
    for namespace in [
        "hello.nested",
        "daemon.extra",
        "agent.extra",
        "session",
        "session.mcp.extra",
        "session.create.extra",
    ] {
        assert!(
            ManagementMethods::provide(
                &root,
                namespace,
                vec![MethodDecl::management(
                    &format!("{namespace}.read"),
                    false,
                    Interruption::Drain
                )],
                vec![],
                handler.clone()
            )
            .is_err()
        );
    }
    let parent = ManagementMethods::provide(
        &root,
        "fixture",
        vec![MethodDecl::management(
            "fixture.nested.read",
            false,
            Interruption::Drain,
        )],
        vec![],
        handler.clone(),
    )
    .unwrap();
    let child = ManagementMethods::provide(
        &root,
        "fixture.nested",
        vec![MethodDecl::management(
            "fixture.nested.other",
            false,
            Interruption::Drain,
        )],
        vec![],
        handler,
    )
    .unwrap();
    for owners in [
        vec![
            ("fixture".into(), parent.clone()),
            ("fixture.nested".into(), child.clone()),
        ],
        vec![("fixture.nested".into(), child), ("fixture".into(), parent)],
    ] {
        assert!(MethodTable::new(Contributions(owners)).is_err());
    }
    root.shutdown().await.unwrap();
}
