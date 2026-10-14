//! Independent public regressions for construction ownership and close failures.
use dim_agent_runtime::{AgentBuilder, AgentConfig, AgentError, Capability, KeysOf, SessionConfig};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

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

mod extensions {
    use super::*;
    use dim_agent_runtime::{extension_registry::*, register_capability};
    use rutis::TypeKey;
    use rutis_loader::{Builtins, Declared, Layer, LoaderPlugin, Patch};
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Point;
    impl ExtensionPoint for Point {
        type Service = usize;
        const POINT: &'static str = "test-extensions";
        fn validate(
            name: &str,
            _: &usize,
            _: &RegistrySnapshot<Self>,
        ) -> Result<(), RegistryError> {
            if name == "rejected" {
                Err(RegistryError::Invalid {
                    name: name.into(),
                    reason: "rejected by the consumer contract".into(),
                })
            } else {
                Ok(())
            }
        }
    }
    struct Contributor {
        name: &'static str,
        loads: Arc<AtomicUsize>,
    }
    impl TypedPlugin for Contributor {
        type Deps = ();
        fn name(&self) -> &str {
            self.name
        }
        fn apply<'a>(
            &'a self,
            ctx: &'a Ctx,
            (): Self::Deps,
        ) -> BoxFuture<'a, Result<Effect, CordisError>> {
            Box::pin(async move {
                let value = self.loads.fetch_add(1, Ordering::SeqCst);
                if self.name != "missing" {
                    ctx.provide_as(service_key::<Point>(self.name), Arc::new(value))?;
                }
                Ok(Effect::Done)
            })
        }
    }
    impl Capability for Contributor {
        fn keys(&self) -> KeysOf<Self> {}
    }
    struct Consumer(Arc<AtomicUsize>);
    impl TypedPlugin for Consumer {
        type Deps = (Arc<ExtensionRegistry<Point>>,);
        fn name(&self) -> &str {
            "extension-consumer"
        }
        fn apply<'a>(
            &'a self,
            _: &'a Ctx,
            _: Self::Deps,
        ) -> BoxFuture<'a, Result<Effect, CordisError>> {
            Box::pin(async move {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(Effect::Done)
            })
        }
    }
    impl Capability for Consumer {
        fn keys(&self) -> KeysOf<Self> {
            ((),)
        }
    }
    fn layer(rows: Vec<Value>) -> Vec<Layer> {
        vec![Layer::new(
            "test",
            vec![Patch {
                insert: Some(rows),
                ..Default::default()
            }],
        )]
    }

    async fn until(mut predicate: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !predicate() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("extension lifecycle did not settle");
    }

    #[tokio::test]
    async fn loader_extensions_isolate_reload_and_drain_admitted_generations() {
        let mut builtins = Builtins::new();
        let mut rows = register::<Point>(&mut builtins);
        for name in ["alpha", "beta", "missing", "rejected"] {
            let loads = Arc::new(AtomicUsize::new(0));
            register_capability(&mut builtins, move || Contributor {
                name,
                loads: loads.clone(),
            });
            builtins.declare(
                name,
                Arc::new(move |_| {
                    Ok(Declared {
                        provides: vec![service_name::<Point>(name)],
                        ..Default::default()
                    })
                }),
            );
            rows.push(json!({"id":name,"name":name}));
        }
        let consumer_loads = Arc::new(AtomicUsize::new(0));
        let loads = consumer_loads.clone();
        register_capability(&mut builtins, move || Consumer(loads.clone()));
        // Missing declarations stay visible to fail-closed consumers.
        rows.push(json!({"id":"consumer","name":"extension-consumer"}));
        let root = Ctx::root().unwrap();
        let plugin = LoaderPlugin::new(builtins, Default::default());
        let loader = plugin.handle();
        let view = root.plugin(plugin);
        until(|| root.get::<rutis_loader::Loader>().is_some()).await;
        let report = loader.reconcile(layer(rows.clone()), None).await.unwrap();
        assert!(report.failures.is_empty(), "{report:?}");
        let registry = root.get::<ExtensionRegistry<Point>>().unwrap();
        tokio::time::timeout(Duration::from_secs(5), registry.ready())
            .await
            .unwrap();
        let snapshot = registry.snapshot();
        assert_eq!(
            snapshot
                .active
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
        assert_eq!(
            snapshot.inactive().collect::<Vec<_>>(),
            ["missing", "rejected"]
        );
        assert!(
            snapshot
                .undetermined
                .iter()
                .any(|row| row.row == "consumer")
        );
        let alpha = snapshot.active["alpha"].clone();
        let beta = snapshot.active["beta"].clone();
        let call = alpha.admit().unwrap();
        let reloader = loader.clone();
        let mut reload = tokio::spawn(async move { reloader.restart("alpha").await });
        tokio::time::timeout(Duration::from_secs(5), call.cancel.cancelled())
            .await
            .unwrap();
        assert!(alpha.admit().is_none());
        assert!(beta.admit().is_some());
        assert!(matches!(registry.lookup("alpha"), Lookup::Inactive));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut reload)
                .await
                .is_err()
        );
        drop(call);
        reload.await.unwrap().unwrap();
        until(|| matches!(registry.lookup("alpha"), Lookup::Active(ref active) if active.generation != alpha.generation)).await;
        assert_eq!(consumer_loads.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&beta, &registry.snapshot().active["beta"]));
        rows.retain(|row| row["id"] != "alpha");
        loader.reconcile(layer(rows), None).await.unwrap();
        until(|| matches!(registry.lookup("alpha"), Lookup::Absent)).await;
        assert_eq!(consumer_loads.load(Ordering::SeqCst), 1);
        assert!(
            root.get_as::<usize>(TypeKey::keyed_dynamic::<usize>("alpha".to_owned()))
                .is_none()
        );
        view.shutdown().await.unwrap();
        assert!(beta.admit().is_none());
        root.shutdown().await.unwrap();
    }
}
