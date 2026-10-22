//! Independent public regressions for construction ownership and close failures.
use dim_agent_runtime::{
    AgentBuilder, AgentConfig, AgentError, Capability, KeysOf, SessionConfig, capability_plugin,
    wait_active,
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

struct SlowInitialization {
    entered: Arc<Notify>,
    cleaning: Arc<Notify>,
    release: Arc<Notify>,
}
impl TypedPlugin for SlowInitialization {
    type Deps = ();
    fn name(&self) -> &str {
        "review.slow-initialization"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let cleaning = self.cleaning.clone();
            let release = self.release.clone();
            ctx.effect(|| {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        cleaning.notify_one();
                        release.notified().await;
                        Err(CordisError::ServiceNotFound(
                            "rollback resource failure".into(),
                        ))
                    })
                }))
            })?;
            self.entered.notify_one();
            ctx.cancelled().await;
            Err(CordisError::ServiceNotFound(
                "initialization cancelled".into(),
            ))
        })
    }
}
impl Capability for SlowInitialization {
    fn keys(&self) -> KeysOf<Self> {}
}
fn framework_contains(error: &CordisError, message: &str) -> bool {
    match error {
        CordisError::ServiceNotFound(value) => value == message,
        CordisError::Aggregate { errors } => errors.iter().any(|e| framework_contains(e, message)),
        _ => false,
    }
}
fn contains(error: &AgentError, message: &str) -> bool {
    match error {
        AgentError::Framework(error) => framework_contains(error, message),
        AgentError::Multiple(errors) => errors.iter().any(|e| contains(e, message)),
        _ => false,
    }
}
#[tokio::test]
async fn agent_close_joins_cancelled_construction_and_retains_async_rollback_failure() {
    let entered = Arc::new(Notify::new());
    let cleaning = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let setup = (entered.clone(), cleaning.clone(), release.clone());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.session_plugin(move |_| SlowInitialization {
        entered: setup.0.clone(),
        cleaning: setup.1.clone(),
        release: setup.2.clone(),
    });
    let agent = builder.build();
    let owner = agent.clone();
    let create = tokio::spawn(async move { owner.create_session(SessionConfig::default()).await });
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    let mut close = tokio::spawn(agent.close());
    tokio::time::timeout(Duration::from_secs(2), cleaning.notified())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut close)
            .await
            .is_err()
    );
    // Abandoning the create waiter must not abandon the admitted construction.
    create.abort();
    release.notify_one();
    let error = tokio::time::timeout(Duration::from_secs(2), close)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(contains(&error, "rollback resource failure"), "{error:?}");
    assert!(agent.sessions().is_empty());
    let repeated = agent.close().await.unwrap_err();
    assert!(contains(&repeated, "rollback resource failure"));
}

#[test]
fn agent_close_freezes_build_ownership_before_the_close_task_can_be_polled() {
    // The owner runtime is deliberately not driven until construction finishes on
    // a second runtime. This makes the admission-to-first-poll gap observable.
    let owner_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let construction_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let entered = Arc::new(Notify::new());
    let cleaning = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let setup = (entered.clone(), cleaning.clone(), release.clone());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder.session_plugin(move |_| SlowInitialization {
        entered: setup.0.clone(),
        cleaning: setup.1.clone(),
        release: setup.2.clone(),
    });
    let agent = {
        let _enter = owner_runtime.enter();
        builder.build()
    };
    let creator = agent.clone();
    let create = construction_runtime
        .spawn(async move { creator.create_session(SessionConfig::default()).await });
    construction_runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .unwrap();
    });
    let close = agent.close();
    construction_runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), cleaning.notified())
            .await
            .unwrap();
        release.notify_one();
        let error = tokio::time::timeout(Duration::from_secs(2), create)
            .await
            .unwrap()
            .unwrap()
            .err()
            .expect("cancelled initialization must fail");
        assert!(contains(&error, "rollback resource failure"), "{error:?}");
    });
    assert!(agent.sessions().is_empty());
    let error = owner_runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(2), close)
            .await
            .unwrap()
            .unwrap_err()
    });
    assert!(contains(&error, "rollback resource failure"), "{error:?}");
}

/// A service no row provides.
struct Unprovided;
/// Stays Pending: it injects `Unprovided`.
struct Dependent;
impl TypedPlugin for Dependent {
    type Deps = Arc<Unprovided>;
    fn name(&self) -> &str {
        "review.dependent"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        _: Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async { Ok(Effect::Done) })
    }
}
impl Capability for Dependent {
    fn keys(&self) -> KeysOf<Self> {}
}
struct Failing;
impl TypedPlugin for Failing {
    type Deps = ();
    fn name(&self) -> &str {
        "review.failing"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async { Err(CordisError::ServiceNotFound("required row failed".into())) })
    }
}
impl Capability for Failing {
    fn keys(&self) -> KeysOf<Self> {}
}

#[tokio::test]
async fn a_failed_row_ends_the_wait_while_an_earlier_row_stays_pending() {
    let root = Ctx::root().unwrap();
    let views = vec![
        root.plugin(capability_plugin(Dependent)),
        root.plugin(capability_plugin(Failing)),
    ];
    // The wait sits on the Pending row first; the timeout would only be
    // reached if the Failed row were not watched.
    let started = std::time::Instant::now();
    let error = wait_active(
        &root,
        &views,
        Duration::from_secs(30),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(contains(&error, "required row failed"), "{error}");
    // Well under the 30 s deadline, with room for a loaded scheduler.
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    root.shutdown().await.unwrap();
}
