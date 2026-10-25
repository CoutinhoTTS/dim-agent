//! The part registry and the instances' mounters (ADR 006 D3): a plugin
//! registers parts while it runs, each Session and branch mounts them, and
//! the plugin's unload takes them out of every instance.
use dim_agent_runtime::{
    Agent, AgentBuilder, AgentConfig, BranchOptions, PartRegistry, PartScope, Session,
    SessionConfig, SessionIdentity, TimelineIdentity,
};
use dim_agent_types::BranchSource;
use rutis::{BoxFuture, CordisError, Ctx, Effect, FiberView, Keyed, TypedPlugin};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

type Log = Arc<Mutex<Vec<String>>>;

fn builder() -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    builder
}

/// A process plugin that registers one part while it runs.
struct Producer {
    registry: Arc<PartRegistry>,
    scope: PartScope,
    name: &'static str,
    generator: dim_agent_runtime::Generator,
}
impl TypedPlugin for Producer {
    type Deps = ();
    fn name(&self) -> &str {
        "test.producer"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let generator = self.generator.clone();
            self.registry
                .register(ctx, self.scope, self.name, move |mount| generator(mount))?;
            Ok(Effect::Done)
        })
    }
}
async fn produce(
    root: &Ctx,
    registry: Arc<PartRegistry>,
    scope: PartScope,
    name: &'static str,
    generator: dim_agent_runtime::Generator,
) -> FiberView {
    let view = root.plugin(rutis::Typed::new(Producer {
        registry,
        scope,
        name,
        generator,
    }));
    dim_agent_runtime::wait_active(
        root,
        std::slice::from_ref(&view),
        Duration::from_secs(5),
        &Default::default(),
    )
    .await
    .unwrap();
    view
}

/// Records where it is mounted and when it unloads.
struct Recorder(Log);
impl TypedPlugin for Recorder {
    type Deps = (Keyed<SessionIdentity>,);
    fn name(&self) -> &str {
        "test.recorder"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(identity),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let (log, id) = (self.0.clone(), identity.session_id.clone());
            log.lock().unwrap().push(format!("mounted {id}"));
            ctx.effect(move || {
                Effect::Disposer(Box::new(move || {
                    log.lock().unwrap().push(format!("unmounted {id}"));
                    Ok(())
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}
fn recorder(log: &Log) -> dim_agent_runtime::Generator {
    let log = log.clone();
    Arc::new(move |mount| mount.names.part(Recorder(log.clone())).map(Some))
}

/// A branch part that records its branch.
struct BranchRecorder(Log);
impl TypedPlugin for BranchRecorder {
    type Deps = (Keyed<TimelineIdentity>,);
    fn name(&self) -> &str {
        "test.branch-recorder"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        (Keyed(identity),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(format!(
                "branch {}/{}",
                identity.session_id, identity.branch_id
            ));
            Ok(Effect::Done)
        })
    }
}

/// A part whose `apply` waits for a gate (or its unload: rutis does not
/// abort an `apply`), or fails.
struct Gated(Option<Arc<Notify>>);
impl TypedPlugin for Gated {
    type Deps = ();
    fn name(&self) -> &str {
        "test.gated"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            match &self.0 {
                Some(gate) => {
                    let cancel = ctx.cancellation_token();
                    tokio::select! {
                        _ = gate.notified() => {}
                        _ = cancel.cancelled() => {}
                    }
                    Ok(Effect::Done)
                }
                None => Err(CordisError::PluginFailed("gated part refused".into())),
            }
        })
    }
}

fn mounted(session: &Session, name: &str) -> bool {
    session
        .diagnostics()
        .unwrap()
        .plugins
        .iter()
        .any(|plugin| plugin.name == name)
}
async fn eventually(mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached");
}
async fn close(agent: Agent, root: Ctx) {
    agent.close().await.unwrap();
    root.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_registered_part_reaches_open_and_later_sessions_and_leaves_with_its_plugin() {
    let root = Ctx::root().unwrap();
    let builder = builder();
    let agent = builder.build_in(&root);
    let before = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let log = Log::default();
    let producer = produce(
        &root,
        builder.parts(),
        PartScope::Session,
        "test.recorder",
        recorder(&log),
    )
    .await;
    // An open Session mounts it as soon as it is registered.
    let first = format!("mounted {}", before.id());
    eventually(|| log.lock().unwrap().contains(&first)).await;
    // A later Session mounts it when it opens.
    let after = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let second = format!("mounted {}", after.id());
    eventually(|| log.lock().unwrap().contains(&second)).await;
    assert!(mounted(&before, "test.recorder") && mounted(&after, "test.recorder"));

    // The plugin's unload finishes after every copy has unloaded.
    producer.dispose().await.unwrap();
    let entries = log.lock().unwrap().clone();
    for session in [&before, &after] {
        assert!(
            entries.contains(&format!("unmounted {}", session.id())),
            "{entries:?}"
        );
        assert!(!mounted(session, "test.recorder"));
    }
    let later = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!mounted(&later, "test.recorder"));
    assert!(
        !log.lock()
            .unwrap()
            .contains(&format!("mounted {}", later.id()))
    );
    close(agent, root).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_generator_may_mount_nothing_for_an_instance() {
    let root = Ctx::root().unwrap();
    let builder = builder();
    let agent = builder.build_in(&root);
    let log = Log::default();
    let inner = recorder(&log);
    let generator: dim_agent_runtime::Generator = Arc::new(move |mount| {
        // Only Sessions whose configuration enables the feature get the part.
        if !mount.session.system_prompt.contains("recorded") {
            return Ok(None);
        }
        inner(mount)
    });
    let _producer = produce(
        &root,
        builder.parts(),
        PartScope::Session,
        "test.recorder",
        generator,
    )
    .await;
    let off = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let on = agent
        .create_session(SessionConfig {
            system_prompt: "recorded".into(),
            ..SessionConfig::default()
        })
        .await
        .unwrap();
    let mounted_on = format!("mounted {}", on.id());
    eventually(|| log.lock().unwrap().contains(&mounted_on)).await;
    assert!(mounted(&on, "test.recorder"));
    assert!(!mounted(&off, "test.recorder"));
    assert!(
        !log.lock()
            .unwrap()
            .contains(&format!("mounted {}", off.id()))
    );
    close(agent, root).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_required_part_gates_publication_and_its_failure_fails_the_session() {
    let root = Ctx::root().unwrap();
    let mut builder = builder();
    builder.require_part("test.gated");
    let agent = builder.build_in(&root);
    let gate = Arc::new(Notify::new());
    let held = gate.clone();
    let producer = produce(
        &root,
        builder.parts(),
        PartScope::Session,
        "test.gated",
        Arc::new(move |mount| mount.names.part(Gated(Some(held.clone()))).map(Some)),
    )
    .await;
    let creating = {
        let agent = agent.clone();
        tokio::spawn(async move { agent.create_session(SessionConfig::default()).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !creating.is_finished(),
        "published before its required part"
    );
    gate.notify_waiters();
    let session = creating.await.unwrap().unwrap();
    assert!(mounted(&session, "test.gated"));
    session.close().await.unwrap();
    producer.dispose().await.unwrap();

    let _failing = produce(
        &root,
        builder.parts(),
        PartScope::Session,
        "test.gated",
        Arc::new(|mount| mount.names.part(Gated(None)).map(Some)),
    )
    .await;
    let error = agent
        .create_session(SessionConfig::default())
        .await
        .err()
        .expect("a failed required part fails its Session");
    assert!(error.to_string().contains("gated part refused"), "{error}");
    close(agent, root).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_optional_part_neither_gates_nor_fails_its_session() {
    let root = Ctx::root().unwrap();
    let builder = builder();
    let agent = builder.build_in(&root);
    let never = Arc::new(Notify::new());
    let _waiting = produce(
        &root,
        builder.parts(),
        PartScope::Session,
        "test.gated",
        Arc::new(move |mount| mount.names.part(Gated(Some(never.clone()))).map(Some)),
    )
    .await;
    let _failing = produce(
        &root,
        builder.parts(),
        PartScope::Session,
        "test.failing",
        Arc::new(|mount| mount.names.part(Gated(None)).map(Some)),
    )
    .await;
    let session = tokio::time::timeout(
        Duration::from_secs(5),
        agent.create_session(SessionConfig::default()),
    )
    .await
    .expect("an optional part does not gate publication")
    .unwrap();
    eventually(|| {
        session
            .diagnostics()
            .unwrap()
            .plugins
            .iter()
            .any(|plugin| plugin.name == "test.gated" && plugin.state == rutis::FiberState::Failed)
    })
    .await;
    assert!(mounted(&session, "test.gated"));
    close(agent, root).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_branch_part_mounts_in_each_branch() {
    let root = Ctx::root().unwrap();
    let builder = builder();
    let agent = builder.build_in(&root);
    let log = Log::default();
    let branch_log = log.clone();
    let _producer = produce(
        &root,
        builder.parts(),
        PartScope::Branch,
        "test.branch-recorder",
        Arc::new(move |mount| {
            assert!(mount.branch_id.is_some());
            mount
                .names
                .part(BranchRecorder(branch_log.clone()))
                .map(Some)
        }),
    )
    .await;
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let main = format!("branch {}/main", session.id());
    eventually(|| log.lock().unwrap().contains(&main)).await;
    let child = session
        .branch(
            "main".into(),
            0,
            BranchOptions {
                prompt: "child".into(),
                source: BranchSource::Host {
                    operation_id: "test-branch".into(),
                },
            },
        )
        .await
        .unwrap();
    let child = format!("branch {}/{child}", session.id());
    eventually(|| log.lock().unwrap().contains(&child)).await;
    close(agent, root).await;
}
