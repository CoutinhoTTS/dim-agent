use dim_agent_runtime::*;
use dim_agent_types::BranchSource;
use dim_agent_types::InputMode;
use rutis::TypedPlugin;
use std::time::Duration;
use test_extension_a::Provider;
use test_extension_b::{Consumer, Snapshot};
fn builder() -> AgentBuilder {
    let mut b = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::mock_hosts(&mut b);
    dim_agent_capabilities::defaults::install(&mut b);
    b
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_cross_crate_capabilities_are_isolated_and_installed_in_every_branch() {
    let mut b = builder();
    b.timeline_plugin(|scope| Consumer::new(scope, true))
        .timeline_plugin(Provider::new);
    let agent = b.build();
    let one = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let two = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    one.submit("hello".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    one.idle("main").await.unwrap();
    assert_eq!(one.get::<Snapshot>("main").unwrap().unwrap().inputs, 1);
    assert_eq!(two.get::<Snapshot>("main").unwrap().unwrap().inputs, 0);
    let branch = one
        .branch(
            "main".into(),
            one.sequence("main").unwrap(),
            BranchOptions {
                prompt: "branch".into(),
                source: BranchSource::Host {
                    operation_id: "fork".into(),
                },
            },
        )
        .await
        .unwrap();
    one.idle(&branch).await.unwrap();
    let initial = one.get::<Snapshot>(&branch).unwrap().unwrap().inputs;
    one.submit("child".into(), InputMode::Queue, &branch)
        .await
        .unwrap();
    one.idle(&branch).await.unwrap();
    assert_eq!(
        one.get::<Snapshot>(&branch).unwrap().unwrap().inputs,
        initial + 1
    );
    assert_eq!(one.get::<Snapshot>("main").unwrap().unwrap().inputs, 1);
    let mut changes = one.on::<Snapshot>(&branch).unwrap().unwrap();
    one.submit("another".into(), InputMode::Queue, &branch)
        .await
        .unwrap();
    changes.changed().await.unwrap();
    assert_eq!(changes.borrow().inputs, initial + 2);
    let diag = one.branch_diagnostics(&branch).unwrap();
    let consumer = diag
        .plugins
        .iter()
        .find(|p| p.name == "ExternalConsumer")
        .unwrap();
    assert!(consumer.accesses.iter().all(|a| !a.external || a.declared));
    assert!(
        consumer
            .resolved_dependencies
            .iter()
            .any(|d| d.key.describe().contains("Counter"))
    );
    agent.close().await.unwrap();
    assert!(agent.sessions().is_empty());
}
#[tokio::test]
async fn missing_dependency_and_undeclared_read_fail_before_session_publication() {
    let mut missing = builder();
    missing
        .startup_timeout(Duration::from_millis(50))
        .timeline_plugin(|scope| Consumer::new(scope, true));
    let agent = missing.build();
    let error = agent
        .create_session(SessionConfig::default())
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("ExternalConsumer"));
    assert!(agent.sessions().is_empty());
    agent.close().await.unwrap();
    // Scoped service reads are strict in every build.
    let mut undeclared = builder();
    undeclared
        .timeline_plugin(Provider::new)
        .timeline_plugin(|scope| Consumer::new(scope, false));
    let agent = undeclared.build();
    let error = agent
        .create_session(SessionConfig::default())
        .await
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("dependency not declared"),
        "{error}"
    );
    assert!(agent.sessions().is_empty());
    agent.close().await.unwrap();
}

#[tokio::test]
async fn abandoned_first_creation_is_owned_until_agent_close_finishes() {
    let root = rutis::Ctx::root().unwrap();
    let agent = builder().build_in(&root);
    let mut creating = Box::pin(agent.create_session(SessionConfig::default()));
    assert!(futures_util::poll!(creating.as_mut()).is_pending());
    drop(creating);
    let (first, second) = tokio::join!(agent.close(), agent.close());
    first.unwrap();
    second.unwrap();
    assert!(agent.sessions().is_empty());
    assert_eq!(
        root.diagnostics().plugins.len(),
        1,
        "only the caller's root remains"
    );
    root.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_continues_after_waiter_is_dropped_and_concurrent_closers_join() {
    let agent = builder().build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let first = session.close();
    drop(first);
    let (a, b) = tokio::join!(session.close(), session.close());
    a.unwrap();
    b.unwrap();
    assert!(agent.sessions().is_empty());
    agent.close().await.unwrap();
}

#[test]
fn owning_agent_can_close_or_drop_outside_the_runtime_enter_guard() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let agent = builder().build();
    let session = runtime
        .block_on(agent.create_session(SessionConfig::default()))
        .unwrap();
    // Construct the close future before entering block_on.
    let close = agent.close();
    runtime.block_on(close).unwrap();
    runtime.block_on(session.close()).unwrap();
    let agent = builder().build();
    let session = runtime
        .block_on(agent.create_session(SessionConfig::default()))
        .unwrap();
    drop(agent);
    runtime.block_on(session.close()).unwrap();
}

struct FailingRollback {
    fail_apply: bool,
    cleanups: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl TypedPlugin for FailingRollback {
    type Deps = ();
    fn name(&self) -> &str {
        "test.failing-rollback"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (): Self::Deps,
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            for _ in 0..if self.fail_apply { 2 } else { 1 } {
                let cleanups = self.cleanups.clone();
                ctx.effect(|| {
                    rutis::Effect::Disposer(Box::new(move || {
                        cleanups.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Err(rutis::CordisError::ServiceNotFound("cleanup-marker".into()))
                    }))
                })?;
            }
            if self.fail_apply {
                Err(rutis::CordisError::ServiceNotFound("build-marker".into()))
            } else {
                Ok(rutis::Effect::Done)
            }
        })
    }
}
impl Capability for FailingRollback {
    fn keys(&self) -> KeysOf<Self> {}
}

#[tokio::test]
async fn initialization_preserves_each_distinct_cleanup_failure_once() {
    let cleanups = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let timeline = cleanups.clone();
    let session = cleanups.clone();
    let mut builder = builder();
    builder.timeline_plugin(move |_| FailingRollback {
        fail_apply: true,
        cleanups: timeline.clone(),
    });
    builder.session_plugin(move |_| FailingRollback {
        fail_apply: false,
        cleanups: session.clone(),
    });
    let agent = builder.build();
    let error = agent
        .create_session(SessionConfig::default())
        .await
        .err()
        .expect("initialization failed");
    let detail = format!("{error:?}");
    assert_eq!(cleanups.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(detail.matches("build-marker").count(), 1, "{detail}");
    // Equal text from different disposers must survive; repeated observation of
    // the very same failure through tree/root close must not multiply it.
    assert_eq!(detail.matches("cleanup-marker").count(), 3, "{detail}");
    assert!(agent.sessions().is_empty());
    agent.close().await.unwrap();
}

struct Marker;
impl SessionService for Marker {}
/// Provides `Marker` under its Session's key and reads the marker of the
/// first Session that loaded a probe.
struct ScopeProbe {
    provide: Option<rutis::TypeKey>,
    read: std::sync::Arc<std::sync::Mutex<Option<rutis::TypeKey>>>,
    seen: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl TypedPlugin for ScopeProbe {
    type Deps = ();
    fn name(&self) -> &str {
        "test.scope-probe"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (): Self::Deps,
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            if let Some(key) = &self.provide {
                ctx.provide_as(key.clone(), std::sync::Arc::new(Marker))?;
                self.read.lock().unwrap().get_or_insert(key.clone());
            }
            if let Some(key) = self.read.lock().unwrap().clone() {
                self.seen.store(
                    ctx.get_as::<Marker>(key).is_some(),
                    std::sync::atomic::Ordering::SeqCst,
                );
            }
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for ScopeProbe {
    fn keys(&self) -> KeysOf<Self> {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_services_stay_inside_their_session_and_unkeyed_ones_fail_publication() {
    let foreign = std::sync::Arc::new(std::sync::Mutex::new(None));
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut scoped = builder();
    let (read, observed) = (foreign.clone(), seen.clone());
    scoped.session_plugin(move |scope| ScopeProbe {
        provide: Some(scope.key::<Marker>()),
        read: read.clone(),
        seen: observed.clone(),
    });
    let agent = scoped.build();
    let first = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    assert!(seen.load(std::sync::atomic::Ordering::SeqCst));
    // The second Session reads the first Session's key: out of scope, not resolvable.
    let second = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    assert!(!seen.load(std::sync::atomic::Ordering::SeqCst));
    first.close().await.unwrap();
    second.close().await.unwrap();
    agent.close().await.unwrap();

    // Assembly validation runs in debug builds only.
    if !cfg!(debug_assertions) {
        return;
    }
    let mut unkeyed = builder();
    unkeyed.session_plugin(|_| ScopeProbe {
        provide: Some(rutis::TypeKey::of::<Marker>()),
        read: Default::default(),
        seen: Default::default(),
    });
    let agent = unkeyed.build();
    let error = agent
        .create_session(SessionConfig::default())
        .await
        .err()
        .expect("unkeyed scoped service");
    assert!(
        error.to_string().contains("without an instance key"),
        "{error}"
    );
    assert!(agent.sessions().is_empty());
    agent.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closed_session_and_branch_subtrees_leave_no_plugins_or_bindings() {
    let agent = builder().build();
    let mut baseline = None;
    for round in 0..20 {
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        let branch = session
            .branch(
                "main".into(),
                0,
                BranchOptions {
                    prompt: "round".into(),
                    source: BranchSource::Host {
                        operation_id: format!("round-{round}"),
                    },
                },
            )
            .await
            .unwrap();
        session.idle(&branch).await.unwrap();
        session.close().await.unwrap();
        let diagnostics = agent.diagnostics().unwrap();
        let counts = (diagnostics.plugins.len(), diagnostics.bindings.len());
        assert_eq!(*baseline.get_or_insert(counts), counts, "round {round}");
    }
    // What remains is the Agent's own: the root, its loader, the Agent
    // instance and its anchor, and the loader's service.
    assert_eq!(baseline, Some((4, 1)));
    agent.close().await.unwrap();
}

struct NamedCapability {
    name: &'static str,
    bit: usize,
    installed: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pending: bool,
}
impl Capability for NamedCapability {
    fn keys(&self) -> KeysOf<Self> {
        self.pending.then_some(())
    }
}
impl TypedPlugin for NamedCapability {
    type Deps = When<rutis::Gate<NamedCapability>>;
    fn name(&self) -> &str {
        self.name
    }
    fn apply<'a>(
        &'a self,
        _: &'a rutis::Ctx,
        _: Self::Deps,
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            self.installed
                .fetch_or(self.bit, std::sync::atomic::Ordering::SeqCst);
            Ok(rutis::Effect::Done)
        })
    }
}

#[tokio::test]
async fn plugin_names_cannot_replace_other_capabilities_or_loader_groups() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let installed = Arc::new(AtomicUsize::new(0));
    let mut b = builder();
    for (i, name) in [
        "Probe",
        "Probe#2",
        "Probe",
        "branch",
        "session",
        "agent",
        "AgentAnchor",
        "AgentScope",
        "trace",
    ]
    .into_iter()
    .enumerate()
    {
        let installed = installed.clone();
        b.session_plugin(move |_| NamedCapability {
            name,
            bit: 1 << i,
            installed: installed.clone(),
            pending: false,
        });
    }
    b.session_extension("trace", "Probe", |_, installed: &Arc<AtomicUsize>| {
        NamedCapability {
            name: "Probe",
            bit: 1 << 9,
            installed: installed.clone(),
            pending: false,
        }
    });
    let agent = b.build();
    agent
        .create_session_with_extension(SessionConfig::default(), "trace", installed.clone())
        .await
        .unwrap();
    assert_eq!(installed.load(Ordering::SeqCst), (1 << 10) - 1);
    agent.close().await.unwrap();
}

#[tokio::test]
async fn factory_and_extension_failures_reject_the_session_instead_of_omitting_capabilities() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    for panic_at in [0, 1] {
        let mut b = builder();
        let calls = Arc::new(AtomicUsize::new(0));
        b.session_plugin(move |_| {
            assert_ne!(
                calls.fetch_add(1, Ordering::SeqCst),
                panic_at,
                "factory failure"
            );
            NamedCapability {
                name: "factory",
                bit: 1,
                installed: Arc::default(),
                pending: false,
            }
        });
        let agent = b.build();
        let result = agent.create_session(SessionConfig::default()).await;
        assert!(result.is_err());
        assert!(agent.sessions().is_empty());
        agent.close().await.unwrap();
    }
    let mut b = builder();
    b.session_extension("extension", "declared", |_, _: &u32| NamedCapability {
        name: "actual",
        bit: 1,
        installed: Arc::default(),
        pending: false,
    });
    let agent = b.build();
    let missing_value = agent
        .create_session_with_extension(SessionConfig::default(), "extension", "wrong value type")
        .await;
    assert!(missing_value.is_err());
    let wrong_name = agent
        .create_session_with_extension(SessionConfig::default(), "extension", 1_u32)
        .await;
    assert!(wrong_name.is_err());
    assert!(agent.sessions().is_empty());
    agent.close().await.unwrap();

    let agent = builder().build();
    assert!(
        agent
            .create_session_with_extension(SessionConfig::default(), "missing", ())
            .await
            .is_err()
    );
    agent.close().await.unwrap();

    let mut b = builder();
    b.startup_timeout(Duration::from_millis(50));
    b.session_extension("pending", "pending", |_, _: &()| NamedCapability {
        name: "pending",
        bit: 1,
        installed: Arc::default(),
        pending: true,
    });
    let agent = b.build();
    assert!(
        agent
            .create_session_with_extension(SessionConfig::default(), "pending", ())
            .await
            .is_err()
    );
    assert!(agent.sessions().is_empty());
    agent.close().await.unwrap();
}

/// A Session service provided once per name, as `sessionMethods` is once per
/// namespace.
struct Named(&'static str);
impl SessionService for Named {}
impl PublicService for Named {}
struct NamedProvider(SessionScope);
impl TypedPlugin for NamedProvider {
    type Deps = ();
    fn name(&self) -> &str {
        "test.named"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (): Self::Deps,
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            for name in ["one", "two"] {
                ctx.provide_as(
                    self.0.named_key::<Named>(name),
                    std::sync::Arc::new(Named(name)),
                )?;
            }
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for NamedProvider {
    fn keys(&self) -> KeysOf<Self> {}
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_facade_reads_a_named_session_service_by_name() {
    let mut b = builder();
    b.session_plugin(|scope| NamedProvider(*scope));
    let agent = b.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    for name in ["one", "two"] {
        assert_eq!(
            session.session_named_service::<Named>(name).unwrap().0,
            name
        );
    }
    // A name nobody provides reads as reloading; telling an absent name from
    // a reloading provider is the caller's (methods contract §9.1).
    assert!(matches!(
        session.session_named_service::<Named>("three"),
        Err(AgentError::Reloading(_))
    ));
    session.close().await.unwrap();
    assert!(matches!(
        session.session_named_service::<Named>("one"),
        Err(AgentError::Closed(_))
    ));
    agent.close().await.unwrap();
}
