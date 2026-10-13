use dim_agent_capabilities::defaults;
use dim_agent_runtime::*;
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

async fn soon<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .expect("public flow completed")
}

struct PanicFlowProbe {
    scope: BranchScope,
    delivered: Arc<AtomicUsize>,
    dispatcher: Arc<OnceLock<Arc<Dispatcher>>>,
}
impl TypedPlugin for PanicFlowProbe {
    type Deps = (InBranch<Dispatcher>,);
    fn name(&self) -> &str {
        "test.panic-flow-probe"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InBranch(dispatcher),): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        let counter = self.delivered.clone();
        Box::pin(async move {
            // One apply registers both listeners in order; independent plugins
            // have no initialization ordering without an injects dependency.
            dispatcher.subscribe(
                ctx,
                "test.counting-subscriber",
                &["input/message"],
                move |_| {
                    let counter = counter.clone();
                    Box::pin(async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                },
            )?;
            dispatcher.subscribe(ctx, "test.panic-subscriber", &["input/message"], |_| {
                Box::pin(async { panic!("subscription handler exploded") })
            })?;
            assert!(self.dispatcher.set(dispatcher).is_ok());
            Ok(Effect::Done)
        })
    }
}
impl Capability for PanicFlowProbe {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handler_panic_is_attributed_and_freezes_the_dispatch_cursor() {
    let delivered = Arc::new(AtomicUsize::new(0));
    let dispatcher = Arc::new(OnceLock::new());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts(&mut builder);
    defaults::install(&mut builder);
    let counted = delivered.clone();
    let captured = dispatcher.clone();
    builder.timeline_plugin(move |scope| PanicFlowProbe {
        scope: *scope,
        delivered: counted.clone(),
        dispatcher: captured.clone(),
    });
    let agent = builder.build();
    let session = soon(agent.create_session(SessionConfig::default()))
        .await
        .unwrap();
    let input = soon(session.submit("hello".into(), InputMode::Queue, "main"))
        .await
        .unwrap();
    let error = soon(session.idle("main")).await.unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("test.panic-subscriber"),
        "failure attributed to the subscriber: {message}"
    );
    assert!(
        message.contains("subscription handler exploded"),
        "panic payload preserved: {message}"
    );
    assert_eq!(delivered.load(Ordering::SeqCst), 1);
    assert_eq!(dispatcher.get().unwrap().sequence(), input.sequence - 1);
    // The dispatch loop exited on the defect: the facade now rejects input
    // with the attributed failure, and no further event is ever delivered.
    let rejected = session
        .submit("second".into(), InputMode::Queue, "main")
        .await
        .unwrap_err();
    assert!(rejected.to_string().contains("test.panic-subscriber"));
    assert_eq!(delivered.load(Ordering::SeqCst), 1);
    // Close aggregates the same failure from the dispatcher state and the
    // root error sink; either way the attribution must survive.
    match soon(agent.close()).await {
        Ok(()) => {}
        Err(error) => assert!(error.to_string().contains("test.panic-subscriber")),
    }
    assert_eq!(dispatcher.get().unwrap().sequence(), input.sequence - 1);
}

struct DomainProbe {
    scope: BranchScope,
    seen: Arc<std::sync::Mutex<Vec<String>>>,
}
impl TypedPlugin for DomainProbe {
    type Deps = (InBranch<Dispatcher>,);
    fn name(&self) -> &str {
        "test.domain-probe"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InBranch(dispatcher),): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        let seen = self.seen.clone();
        Box::pin(async move {
            let rejected = dispatcher.subscribe(ctx, "test.bad-wildcard", &["input*"], |_| {
                Box::pin(async { Ok(()) })
            });
            assert!(matches!(rejected, Err(AgentError::InvalidInput(_))));
            dispatcher.subscribe(ctx, "test.input-domain", &["input/*"], move |event| {
                seen.lock().unwrap().push(event.kind.key().to_owned());
                Box::pin(async { Ok(()) })
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for DomainProbe {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn domain_wildcard_subscribes_to_every_key_in_that_domain() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts(&mut builder);
    defaults::install(&mut builder);
    let recorded = seen.clone();
    builder.timeline_plugin(move |scope| DomainProbe {
        scope: *scope,
        seen: recorded.clone(),
    });
    let agent = builder.build();
    let session = soon(agent.create_session(SessionConfig::default()))
        .await
        .unwrap();
    soon(session.submit("hello".into(), InputMode::Queue, "main"))
        .await
        .unwrap();
    soon(session.idle("main")).await.unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        ["input/message", "input/applied", "input/consumed"]
    );
    soon(agent.close()).await.unwrap();
}

/// Reads Loop's published state while handling each event after Loop did.
struct PhaseProbe {
    scope: BranchScope,
    seen: Arc<std::sync::Mutex<Vec<(String, LoopPhase)>>>,
}
impl TypedPlugin for PhaseProbe {
    // Loop provides the state cell: this probe handles each event after it.
    type Deps = (InBranch<Dispatcher>, InBranch<StateCell<LoopSnapshot>>);
    fn name(&self) -> &str {
        "test.phase-probe"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InBranch(dispatcher), InBranch(state)): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        let seen = self.seen.clone();
        Box::pin(async move {
            dispatcher.subscribe(ctx, "test.phase-probe", &["*"], move |event| {
                seen.lock()
                    .unwrap()
                    .push((event.kind.key().to_owned(), state.get().phase));
                Box::pin(async { Ok(()) })
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for PhaseProbe {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loop_publishes_running_only_once_turn_start_is_dispatched() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts(&mut builder);
    defaults::install(&mut builder);
    let recorded = seen.clone();
    builder.timeline_plugin(move |scope| PhaseProbe {
        scope: *scope,
        seen: recorded.clone(),
    });
    let agent = builder.build();
    let session = soon(agent.create_session(SessionConfig::default()))
        .await
        .unwrap();
    soon(session.submit("hello".into(), InputMode::Queue, "main"))
        .await
        .unwrap();
    soon(session.idle("main")).await.unwrap();
    let seen = seen.lock().unwrap().clone();
    let start = seen
        .iter()
        .position(|(key, _)| key == "turn/start")
        .unwrap();
    // Before turn/start is dispatched, the prefix shows no open turn.
    assert!(
        seen[..start]
            .iter()
            .all(|(_, phase)| *phase == LoopPhase::Idle)
    );
    assert_eq!(seen[start].1, LoopPhase::Running);
    let end = seen.iter().position(|(key, _)| key == "turn/end").unwrap();
    assert_eq!(seen[end].1, LoopPhase::Idle);
    soon(agent.close()).await.unwrap();
}
