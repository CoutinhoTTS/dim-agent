//! Rust parts that depend on names only (plugin contract P2): no layer type,
//! no scope, no group in the part. Where the row is placed decides.
use dim_agent_runtime::{
    AgentBuilder, AgentConfig, Feature, SessionConfig, SessionIdentity, Timeline, TimelineIdentity,
    names, register_part,
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, Keyed, TypedPlugin};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Log = Arc<Mutex<Vec<String>>>;

/// Records the Session it runs in.
struct SessionPart(Log);
impl TypedPlugin for SessionPart {
    type Deps = (Keyed<SessionIdentity>,);
    fn name(&self) -> &str {
        "test.session-part"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        (Keyed(identity),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .push(format!("session {}", identity.session_id));
            Ok(Effect::Done)
        })
    }
}

/// Records the branch it runs in, reading a Session and a branch name.
struct BranchPart(Log);
impl TypedPlugin for BranchPart {
    type Deps = (
        Keyed<SessionIdentity>,
        Keyed<TimelineIdentity>,
        Keyed<Timeline>,
    );
    fn name(&self) -> &str {
        "test.branch-part"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        (Keyed(session), Keyed(branch), Keyed(timeline)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            assert_eq!(session.session_id, branch.session_id);
            self.0.lock().unwrap().push(format!(
                "branch {}/{} at {}",
                branch.session_id,
                branch.branch_id,
                timeline.sequence()
            ));
            Ok(Effect::Done)
        })
    }
}

/// A Session service a Session part provides by name and a branch part
/// injects by name: each Session's branches see their own Session's copy.
struct Greeting(String);
struct GreetingPart(rutis::DepKey<Greeting>);
impl TypedPlugin for GreetingPart {
    type Deps = (Keyed<SessionIdentity>,);
    fn name(&self) -> &str {
        "test.greeting"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(identity),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as(
                self.0.key().clone(),
                Arc::new(Greeting(format!("hello {}", identity.session_id))),
            )?;
            Ok(Effect::Done)
        })
    }
}
struct GreetedPart(Log);
impl TypedPlugin for GreetedPart {
    type Deps = (Keyed<Greeting>, Keyed<TimelineIdentity>);
    fn name(&self) -> &str {
        "test.greeted"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        (Keyed(greeting), Keyed(branch)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .push(format!("{} in {}", greeting.0, branch.session_id));
            Ok(Effect::Done)
        })
    }
}

/// A service that crosses languages: provided in each Session at its
/// dispatcher's key, as a row of another language provides it, and read
/// there by a branch part through the catalog.
struct Greeter(String);
impl rutis_bridge::session::HostDispatch for Greeter {
    fn invoke(&self, _: &str, _: rutis_bridge::session::Value) -> rutis_bridge::session::Reply {
        Ok(json!(self.0).into())
    }
    fn provider(&self) -> Option<String> {
        Some(self.0.clone())
    }
}
struct GreeterPart(rutis::InstanceId);
impl TypedPlugin for GreeterPart {
    type Deps = (Keyed<SessionIdentity>,);
    fn name(&self) -> &str {
        "test.greeter"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(identity),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as::<dyn rutis_bridge::session::HostDispatch>(
                rutis_bridge::session::host_key_in("greeter", self.0),
                Arc::new(Greeter(format!("greeter of {}", identity.session_id))),
            )?;
            Ok(Effect::Done)
        })
    }
}
struct GreeterReader(Log);
impl TypedPlugin for GreeterReader {
    type Deps = (
        Keyed<dyn rutis_bridge::session::HostDispatch>,
        Keyed<TimelineIdentity>,
    );
    fn name(&self) -> &str {
        "test.greeter-reader"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        (Keyed(greeter), Keyed(branch)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(format!(
                "{} for {}",
                greeter.provider().unwrap(),
                branch.session_id
            ));
            Ok(Effect::Done)
        })
    }
}

/// A process service and its provider and consumer parts.
struct Clock(u64);
struct ClockPart;
impl TypedPlugin for ClockPart {
    type Deps = ();
    fn name(&self) -> &str {
        "test.clock"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (): ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide(Clock(7))?;
            Ok(Effect::Done)
        })
    }
}
struct ProcessPart(Log);
impl TypedPlugin for ProcessPart {
    type Deps = (Keyed<Clock>,);
    fn name(&self) -> &str {
        "test.process-part"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        (Keyed(clock),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(format!("process {}", clock.0));
            Ok(Effect::Done)
        })
    }
}

fn builder() -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    builder
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_part_without_layer_types_mounts_in_every_group() {
    let log = Log::default();
    let mut builder = builder();
    let (session_log, branch_log) = (log.clone(), log.clone());
    builder.require_feature(
        Feature::new("test.named-parts")
            .session("test.session-part", move |names| {
                names.part(SessionPart(session_log.clone()))
            })
            .branch("test.branch-part", move |names| {
                names.part(BranchPart(branch_log.clone()))
            }),
    );
    let agent = builder.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let id = session.id().to_owned();
    assert!(log.lock().unwrap().contains(&format!("session {id}")));
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .any(|entry| entry.starts_with(&format!("branch {id}/main at "))),
        "{:?}",
        log.lock().unwrap()
    );
    agent.close().await.unwrap();

    // At the process: a loader row with a process name.
    let mut catalog = names::catalog();
    names::register::<Clock>(&mut catalog, None);
    let mut builtins = rutis_loader::Builtins::new();
    let provider = register_part(&mut builtins, &catalog, |names| names.part(ClockPart)).unwrap();
    let process_log = log.clone();
    let consumer = register_part(&mut builtins, &catalog, move |names| {
        names.part(ProcessPart(process_log.clone()))
    })
    .unwrap();
    let root = Ctx::root().unwrap();
    let plugin = rutis_loader::LoaderPlugin::new(
        builtins,
        rutis_loader::LoaderOptions {
            catalog,
            ..Default::default()
        },
    );
    let loader = plugin.handle();
    let view = root.plugin(plugin);
    dim_agent_runtime::wait_active(
        &root,
        std::slice::from_ref(&view),
        Duration::from_secs(5),
        &Default::default(),
    )
    .await
    .unwrap();
    let rows = serde_json::from_value(json!([{ "insert": [
        { "id": provider, "name": provider },
        { "id": consumer, "name": consumer },
    ] }]))
    .unwrap();
    let report = loader
        .reconcile(vec![rutis_loader::Layer::new("test", rows)], None)
        .await
        .unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !log.lock().unwrap().contains(&"process 7".to_owned()) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    root.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_part_provides_by_name_and_each_instance_sees_its_own_copy() {
    let log = Log::default();
    let mut builder = builder();
    names::register::<Greeting>(
        builder.catalog_mut(),
        Some(dim_agent_runtime::SESSION_GROUP),
    );
    let greeted = log.clone();
    builder.require_feature(
        Feature::new("test.greeting")
            .session("test.greeting", |names| {
                names.part(GreetingPart(names.key::<Greeting>()?))
            })
            .branch("test.greeted", move |names| {
                names.part(GreetedPart(greeted.clone()))
            }),
    );
    let agent = builder.build();
    let mut ids = Vec::new();
    for _ in 0..2 {
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        ids.push(session.id().to_owned());
    }
    let log = log.lock().unwrap().clone();
    for id in &ids {
        assert!(log.contains(&format!("hello {id} in {id}")), "{log:?}");
    }
    assert_eq!(log.len(), 2, "{log:?}");
    agent.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rust_part_reads_a_crossing_name_at_its_catalog_key() {
    let log = Log::default();
    let mut builder = builder();
    builder
        .catalog_mut()
        .register_shared_instance("greeter", dim_agent_runtime::SESSION_GROUP);
    let read = log.clone();
    builder.require_feature(
        Feature::new("test.greeter")
            .session("test.greeter", |names| {
                names.part(GreeterPart(
                    names.instance(dim_agent_runtime::SESSION_GROUP)?,
                ))
            })
            .branch("test.greeter-reader", move |names| {
                Ok(names.part_with(
                    GreeterReader(read.clone()),
                    (names.named("greeter")?, names.key::<TimelineIdentity>()?),
                ))
            }),
    );
    let agent = builder.build();
    let mut ids = Vec::new();
    for _ in 0..2 {
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        ids.push(session.id().to_owned());
    }
    let log = log.lock().unwrap().clone();
    for id in &ids {
        assert!(
            log.contains(&format!("greeter of {id} for {id}")),
            "{log:?}"
        );
    }
    assert_eq!(log.len(), 2, "{log:?}");
    agent.close().await.unwrap();
}

/// A registered part's names are checked where an instance mounts it: a
/// Session part reading a branch name fails that Session's creation, and a
/// process row reading a Session name fails its registration.
#[tokio::test]
async fn a_misplaced_part_fails_where_it_is_mounted_naming_the_group() {
    let log = Log::default();
    let mut builder = builder();
    builder.require_feature(
        Feature::new("test.misplaced").session("test.branch-part", move |names| {
            names.part(BranchPart(log.clone()))
        }),
    );
    let agent = builder.build();
    let error = agent
        .create_session(SessionConfig::default())
        .await
        .err()
        .expect("a Session part reading branch names fails");
    assert!(
        error.to_string().contains(
            "timelineIdentity is a branch service: place the part inside the branch group"
        ),
        "{error}"
    );
    assert!(agent.sessions().is_empty());
    agent.close().await.unwrap();

    let mut builtins = rutis_loader::Builtins::new();
    let error = register_part(&mut builtins, &names::catalog(), |names| {
        names.part(SessionPart(Log::default()))
    })
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("place the part inside the session group"),
        "{error}"
    );
}

/// Waits in `apply` for work that never comes, until its generation is
/// cancelled: rutis waits for `apply` to return and never aborts it.
struct Stuck;
impl TypedPlugin for Stuck {
    type Deps = (Keyed<SessionIdentity>,);
    fn name(&self) -> &str {
        "test.stuck"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        _: Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.cancelled().await;
            Err(CordisError::PluginFailed("cancelled while waiting".into()))
        })
    }
}

/// One deadline covers loader startup, reconcile and rows becoming Active
/// (#6): a row that never finishes `apply` fails the creation, and nothing
/// of the Session stays.
#[tokio::test]
async fn a_row_blocking_in_apply_fails_creation_at_the_startup_deadline() {
    let mut builder = builder();
    builder
        .startup_timeout(Duration::from_millis(100))
        .require_feature(
            Feature::new("test.stuck").session("test.stuck", |names| names.part(Stuck)),
        );
    let agent = builder.build();
    let started = std::time::Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(20),
        agent.create_session(SessionConfig::default()),
    )
    .await
    .expect("creation returns at the deadline")
    .err()
    .expect("the stuck row fails the creation");
    assert!(started.elapsed() < Duration::from_secs(5), "{error}");
    assert!(error.to_string().contains("test.stuck"), "{error}");
    assert!(agent.sessions().is_empty());
    tokio::time::timeout(Duration::from_secs(20), agent.close())
        .await
        .expect("close returns")
        .unwrap();
}
