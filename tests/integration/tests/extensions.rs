use dim_agent_capabilities::host::{HostFuture, ToolExecutionContext};
use dim_agent_capabilities::tool_registry::{
    ToolContribution, ToolHandler, ToolProvider, ToolRegistry, ToolSourceSet, definition,
};
use dim_agent_runtime::*;
use dim_agent_types::BranchSource;
use dim_agent_types::EventKind;
use dim_agent_types::InputMode;
use dim_agent_types::{ToolCall, ToolResult};
use rutis::Plugin;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, FiberState, Key, TypeKey};
use std::sync::Arc;
use std::time::Duration;
use test_extension_a::{Counter, Provider};
use test_extension_b::{Consumer, Snapshot};
use tokio_util::sync::CancellationToken;
fn builder() -> AgentBuilder {
    let mut b = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::mock_hosts(&mut b);
    dim_agent_capabilities::defaults::install(&mut b);
    b
}
/// The external consumer as a feature of one branch part.
fn consumer(declare: bool) -> Feature {
    Feature::new("ExternalConsumer").branch("ExternalConsumer", move |names| {
        Consumer::part(names, declare)
    })
}
/// The external provider as a feature of one branch part.
fn provider() -> Feature {
    Feature::new("ExternalProvider").branch("ExternalProvider", |names| {
        Ok(Box::new(Provider::new(names)?))
    })
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_cross_crate_capabilities_are_isolated_and_installed_in_every_branch() {
    let mut b = builder();
    test_extension_a::register_names(b.catalog_mut());
    b.require_feature(consumer(true))
        .require_feature(provider());
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
/// Tags each input with the branch whose `before` chain it passed.
struct TagOwnBranch {
    timeline: Key<Timeline>,
    identity: Key<TimelineIdentity>,
}
impl TagOwnBranch {
    fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            timeline: names.key()?,
            identity: names.key()?,
        })
    }
}
impl DynPlugin for TagOwnBranch {
    fn name(&self) -> &str {
        "test.tag-own-branch"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.timeline.erase(), self.identity.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let timeline = ctx.require_at(&self.timeline)?;
            let identity = ctx.require_at(&self.identity)?;
            let tag = format!("{}/{}", identity.session_id, identity.branch_id);
            timeline.before(ctx, "TagOwnBranch", &["input/message"], move |mut draft| {
                draft.tags.get_or_insert_with(Vec::new).push(tag.clone());
                Ok(draft)
            })?;
            Ok(Effect::Done)
        })
    }
}

/// On the one bus, a branch's `before` chain runs only for its own commits,
/// in two branches of one Session and in two Sessions.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn before_runs_only_on_its_own_branch() {
    let mut b = builder();
    b.require_feature(
        Feature::new("test.tag-own-branch").branch("test.tag-own-branch", |names| {
            Ok(Box::new(TagOwnBranch::new(names)?))
        }),
    );
    let agent = b.build();
    let one = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let two = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
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
    for (session, branch_id) in [(&one, "main"), (&one, branch.as_str()), (&two, "main")] {
        session
            .submit("tagged".into(), InputMode::Queue, branch_id)
            .await
            .unwrap();
        session.idle(branch_id).await.unwrap();
    }
    for (session, branch_id) in [(&one, "main"), (&one, branch.as_str()), (&two, "main")] {
        let own = format!("{}/{branch_id}", session.id());
        let inputs: Vec<_> = session
            .local(branch_id, ..)
            .await
            .unwrap()
            .into_iter()
            .filter(|event| event.kind.key() == "input/message")
            .collect();
        assert!(!inputs.is_empty());
        for event in inputs {
            assert_eq!(event.tags, Some(vec![own.clone()]), "{branch_id}");
        }
    }
    agent.close().await.unwrap();
}

#[tokio::test]
async fn missing_dependency_and_undeclared_read_fail_before_session_publication() {
    let mut missing = builder();
    test_extension_a::register_names(missing.catalog_mut());
    missing
        .startup_timeout(Duration::from_millis(50))
        .require_feature(consumer(true));
    let agent = missing.build();
    let error = agent
        .create_session(SessionConfig::default())
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("ExternalConsumer"));
    assert!(agent.sessions().is_empty());
    agent.close().await.unwrap();

    let mut undeclared = builder();
    test_extension_a::register_names(undeclared.catalog_mut());
    undeclared
        .require_feature(provider())
        .require_feature(consumer(false));
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

/// A check that inspected nothing does not look like a pass: an Agent that
/// never placed its Sessions has no diagnostics snapshot to inspect.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_assembly_check_with_nothing_to_inspect_is_an_error() {
    let agent = builder().build();
    let error = agent.check_assembly().unwrap_err();
    assert!(
        error.to_string().contains("no diagnostics snapshot"),
        "{error}"
    );
    agent.close().await.unwrap();
}

/// The findings with their scope, as `(session, branch, plugin, rule)` tuples
/// sorted by that prefix, so a test asserts the whole list rather than the
/// presence of one entry.
fn pairs(findings: &[AssemblyFinding]) -> Vec<(String, Option<String>, String, AssemblyRule)> {
    let mut pairs: Vec<_> = findings
        .iter()
        .map(|finding| {
            (
                finding.session.clone(),
                finding.branch.clone(),
                finding.plugin.clone(),
                finding.rule,
            )
        })
        .collect();
    pairs.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
    pairs
}

/// A required part that returns from `apply` without awaiting the child it
/// registered leaves a plugin that is not `Active`. Creation does not wait for
/// it; the assembly check reports it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unawaited_children_are_reported() {
    for scope in [PartScope::Session, PartScope::Branch] {
        let mut b = builder();
        b.require_feature(Feature::new("test.required-owner").when(
            scope,
            "test.required-owner",
            |_| Ok(Some(Box::new(UnawaitedChild))),
        ));
        let agent = b.build();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        let findings = agent.check_assembly().unwrap();
        assert_eq!(
            pairs(&findings),
            vec![(
                session.id().to_string(),
                (scope == PartScope::Branch).then(|| "main".to_string()),
                "unawaited-child".to_string(),
                AssemblyRule::Ready,
            )],
            "{findings:?}"
        );
        session.close().await.unwrap();
        agent.close().await.unwrap();
    }
}

/// A tool source a mounted child registers from its own `apply`.
struct SlowSource;
impl ToolProvider for SlowSource {
    fn sets(&self) -> Vec<Arc<ToolSourceSet>> {
        vec![Arc::new(ToolSourceSet {
            key: "slow".into(),
            version: "1".into(),
            tools: vec![Arc::new(ToolContribution {
                definition: definition(
                    "slow",
                    "A tool of a slow child",
                    serde_json::json!({"type": "object"}),
                ),
                handler: Arc::new(SlowCall),
            })],
        })]
    }
}
struct SlowCall;
impl ToolHandler for SlowCall {
    fn execute(
        &self,
        _: ToolCall,
        _: ToolExecutionContext,
        _: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async { Ok(ToolResult::text("slow")) })
    }
}

/// The service the slow child waits for; the row below provides it late.
struct SlowService;

/// A required row that provides `SlowService` only after it takes its time,
/// so the child mounted by another row cannot load before it.
struct SlowProvider {
    key: Key<SlowService>,
}
impl Plugin for SlowProvider {
    type Deps = ();
    async fn apply(&self, ctx: &rutis::Ctx, _: ()) -> rutis::Result {
        tokio::time::sleep(Duration::from_millis(300)).await;
        ctx.provide_at(self.key.clone(), Arc::new(SlowService))?;
        Ok(())
    }
}

/// A child whose own `apply` takes a while before it registers its tool.
struct SlowChild {
    service: Key<SlowService>,
    tools: Key<ToolRegistry>,
}
impl DynPlugin for SlowChild {
    fn name(&self) -> &str {
        "test.slow-child"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.service.erase(), self.tools.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let tools = ctx.require_at(&self.tools)?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            tools.register_source(ctx, "slow", Arc::new(SlowSource))?;
            Ok(Effect::Done)
        })
    }
}

/// The required row that mounts the slow child.
struct SlowOwner {
    service: Key<SlowService>,
    tools: Key<ToolRegistry>,
    timeout: Option<std::time::Duration>,
}
impl Plugin for SlowOwner {
    type Deps = ();
    async fn apply(&self, ctx: &rutis::Ctx, _: ()) -> rutis::Result {
        mount_active(
            ctx,
            SlowChild {
                service: self.service.clone(),
                tools: self.tools.clone(),
            },
            self.timeout,
        )
        .await?;
        Ok(())
    }
}

/// The tools of the last model call on `branch`, in the order it offered them.
async fn last_request_tools(session: &Session, branch: &str) -> Vec<String> {
    session
        .history(branch, None)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(
                data.request
                    .tools
                    .as_ref()
                    .map(|tools| tools.iter().map(|tool| tool.name.clone()).collect())
                    .unwrap_or_default(),
            ),
            _ => None,
        })
        .expect("a model call")
}

/// #680: a required row waits for the child it mounts, so creation hands back
/// a Session whose child is `Active`, the check reports nothing, and the first
/// request offers the tool the child registers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_required_row_waits_for_the_child_it_mounts() {
    let mut b = builder();
    names::register::<SlowService>(b.catalog_mut(), Some(SESSION_GROUP));
    b.require_feature(
        Feature::new("test.slow-owner").session("test.slow-owner", |names| {
            Ok(Box::new(SlowOwner {
                service: names.key()?,
                tools: names.key()?,
                timeout: names.startup_timeout(),
            }))
        }),
    )
    .require_feature(
        Feature::new("test.slow-provider").session("test.slow-provider", |names| {
            Ok(Box::new(SlowProvider { key: names.key()? }))
        }),
    );
    let agent = b.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let findings = agent.check_assembly().unwrap();
    assert!(findings.is_empty(), "{findings:?}");
    session
        .submit("hello".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let tools = last_request_tools(&session, "main").await;
    assert!(tools.iter().any(|tool| tool == "slow"), "{tools:?}");
    agent.close().await.unwrap();
}

/// A dependency nothing provides: the child below stays `Pending`.
struct NeverProvided;
struct MissingChild;
impl DynPlugin for MissingChild {
    fn name(&self) -> &str {
        "test.missing-child"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![Key::<NeverProvided>::of().erase()]
    }
    fn apply<'a>(&'a self, _: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async { Ok(Effect::Done) })
    }
}
struct MissingOwner {
    timeout: Option<std::time::Duration>,
}
impl Plugin for MissingOwner {
    type Deps = ();
    async fn apply(&self, ctx: &rutis::Ctx, _: ()) -> rutis::Result {
        mount_active(ctx, MissingChild, self.timeout).await?;
        Ok(())
    }
}

/// #680: a child whose dependency never arrives fails the row at the window's
/// bound, so creation fails and names the child.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_that_never_loads_fails_the_row_at_the_deadline() {
    let mut b = builder();
    b.startup_timeout(Duration::from_millis(200));
    b.require_feature(
        Feature::new("test.missing-owner").session("test.missing-owner", |names| {
            Ok(Box::new(MissingOwner {
                timeout: names.startup_timeout(),
            }))
        }),
    );
    let agent = b.build();
    let error = agent
        .create_session(SessionConfig::default())
        .await
        .err()
        .expect("creation fails");
    assert!(error.to_string().contains("test.missing-child"), "{error}");
    agent.close().await.unwrap();
}

/// A service a row below mounts its child under, replaced after creation.
struct Republished;

/// A child that takes a moment to load, so a wait already out of time cannot
/// see it `Active`.
struct RepublishedChild;
impl DynPlugin for RepublishedChild {
    fn name(&self) -> &str {
        "test.republished-child"
    }
    fn apply<'a>(&'a self, _: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(Effect::Done)
        })
    }
}

/// A row that mounts its child and, when the service it depends on is
/// replaced after creation, re-applies and mounts a new one.
struct ReappliedOwner {
    service: Key<Republished>,
    timeout: Option<std::time::Duration>,
}
impl DynPlugin for ReappliedOwner {
    fn name(&self) -> &str {
        "test.reapplied-owner"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.service.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            mount_active(ctx, RepublishedChild, self.timeout).await?;
            Ok(Effect::Done)
        })
    }
}

/// Provides `Republished` until `withdraw` is set, then withdraws the binding
/// and stays loaded.
struct FirstProvider {
    service: Key<Republished>,
    withdraw: Arc<std::sync::atomic::AtomicBool>,
}
impl DynPlugin for FirstProvider {
    fn name(&self) -> &str {
        "test.first-provider"
    }
    fn apply<'a>(&'a self, ctx: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        let (service, withdraw) = (self.service.clone(), self.withdraw.clone());
        Box::pin(async move {
            let disposer = ctx.provide_at(service, Arc::new(Republished))?;
            tokio::spawn(async move {
                while !withdraw.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let _ = disposer.dispose().await;
            });
            Ok(Effect::Done)
        })
    }
}

/// Provides `Republished` again once `republish` is set.
struct SecondProvider {
    service: Key<Republished>,
    republish: Arc<std::sync::atomic::AtomicBool>,
}
impl DynPlugin for SecondProvider {
    fn name(&self) -> &str {
        "test.second-provider"
    }
    fn apply<'a>(&'a self, ctx: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        let (ctx, service, republish) = (ctx.clone(), self.service.clone(), self.republish.clone());
        Box::pin(async move {
            tokio::spawn(async move {
                while !republish.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let _ = ctx.provide_at(service, Arc::new(Republished));
            });
            Ok(Effect::Done)
        })
    }
}

/// The two rows that provide and replace `Republished`, armed by the test.
struct RepublishedProviders {
    withdraw: Arc<std::sync::atomic::AtomicBool>,
    republish: Arc<std::sync::atomic::AtomicBool>,
}
impl RepublishedProviders {
    fn new() -> Self {
        Self {
            withdraw: Arc::default(),
            republish: Arc::default(),
        }
    }
    fn install(&self, b: &mut AgentBuilder) {
        names::register::<Republished>(b.catalog_mut(), Some(SESSION_GROUP));
        let (withdraw, republish) = (self.withdraw.clone(), self.republish.clone());
        b.require_feature(Feature::new("test.first-provider").session(
            "test.first-provider",
            move |names| {
                Ok(Box::new(FirstProvider {
                    service: names.key()?,
                    withdraw: withdraw.clone(),
                }))
            },
        ))
        .require_feature(Feature::new("test.second-provider").session(
            "test.second-provider",
            move |names| {
                Ok(Box::new(SecondProvider {
                    service: names.key()?,
                    republish: republish.clone(),
                }))
            },
        ));
    }
}

/// Waits until one live plugin named `name` is in `state`; fails with the
/// states it saw.
async fn wait_for_state(agent: &Agent, name: &str, state: FiberState) {
    let mut seen: Vec<FiberState> = vec![];
    for _ in 0..400 {
        seen = agent
            .diagnostics()
            .unwrap()
            .plugins
            .into_iter()
            .filter(|plugin| plugin.name == name)
            .map(|plugin| plugin.state)
            .collect();
        if seen.contains(&state) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no plugin {name} in {state:?}; saw {seen:?}");
}

/// #680: a row that re-applies after its scope's creation takes the wait's
/// bound when the wait starts, so the child it mounts becomes `Active`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reapplied_row_waits_for_its_child_after_creation() {
    let mut b = builder();
    let providers = RepublishedProviders::new();
    providers.install(&mut b);
    b.startup_timeout(Duration::from_millis(200));
    b.require_feature(Feature::new("test.reapplied-owner").session(
        "test.reapplied-owner",
        |names| {
            Ok(Box::new(ReappliedOwner {
                service: names.key()?,
                timeout: names.startup_timeout(),
            }))
        },
    ));
    let agent = b.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    wait_for_state(&agent, "test.republished-child", FiberState::Active).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    providers
        .withdraw
        .store(true, std::sync::atomic::Ordering::SeqCst);
    wait_for_state(&agent, "test.reapplied-owner", FiberState::Pending).await;
    providers
        .republish
        .store(true, std::sync::atomic::Ordering::SeqCst);
    wait_for_state(&agent, "test.republished-child", FiberState::Active).await;
    let findings = agent.check_assembly().unwrap();
    assert!(findings.is_empty(), "{findings:?}");
    session.close().await.unwrap();
    agent.close().await.unwrap();
}

/// #680: a Session extension row loaded after its session's creation takes
/// the wait's bound the same way, so the child it mounts becomes `Active`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_extension_row_waits_for_its_child_after_its_session() {
    let mut b = builder();
    let providers = RepublishedProviders::new();
    providers.install(&mut b);
    b.startup_timeout(Duration::from_millis(200));
    b.session_extension_part(
        "test.reapplied-ext",
        "test.reapplied-owner",
        |names, _: &()| {
            Ok(Box::new(ReappliedOwner {
                service: names.key()?,
                timeout: names.startup_timeout(),
            }))
        },
    );
    let agent = b.build();
    let session = agent
        .create_session_with_extension(SessionConfig::default(), "test.reapplied-ext", ())
        .await
        .unwrap();
    wait_for_state(&agent, "test.reapplied-owner", FiberState::Active).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    providers
        .withdraw
        .store(true, std::sync::atomic::Ordering::SeqCst);
    wait_for_state(&agent, "test.reapplied-owner", FiberState::Pending).await;
    providers
        .republish
        .store(true, std::sync::atomic::Ordering::SeqCst);
    wait_for_state(&agent, "test.republished-child", FiberState::Active).await;
    let findings = agent.check_assembly().unwrap();
    assert!(findings.is_empty(), "{findings:?}");
    session.close().await.unwrap();
    agent.close().await.unwrap();
}

/// Two branches of one Session carry the same faulty plugin: the findings are
/// told apart by the branch they name.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn findings_name_their_branch() {
    let mut b = builder();
    b.require_feature(
        Feature::new("test.required-owner")
            .branch("test.required-owner", |_| Ok(Box::new(UnawaitedChild))),
    );
    let agent = b.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let fork = session
        .branch(
            "main".into(),
            session.sequence("main").unwrap(),
            BranchOptions {
                prompt: "fork".into(),
                source: BranchSource::Host {
                    operation_id: "fork".into(),
                },
            },
        )
        .await
        .unwrap();
    let findings = agent.check_assembly().unwrap();
    let expected_branch = |branch: &str| {
        (
            session.id().to_string(),
            Some(branch.to_string()),
            "unawaited-child".to_string(),
            AssemblyRule::Ready,
        )
    };
    let mut expected = vec![expected_branch("main"), expected_branch(fork.as_str())];
    expected.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
    assert_eq!(pairs(&findings), expected, "{findings:?}");
    let fork_scope = format!("session {} branch {}: ", session.id(), fork.as_str());
    assert!(
        findings
            .iter()
            .any(|finding| finding.to_string().starts_with(&fork_scope)),
        "{findings:?}"
    );
    session.close().await.unwrap();
    agent.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undeclared_reads_are_reported() {
    let mut b = builder();
    test_extension_a::register_names(b.catalog_mut());
    names::register::<Marker>(b.catalog_mut(), Some(BRANCH_GROUP));
    b.require_feature(
        Feature::new("test.pair")
            .branch("test.pair", |names| Ok(Box::new(PairProvider::new(names)?))),
    )
    .require_feature(
        Feature::new("test.undeclared-reader").branch("test.undeclared-reader", |names| {
            Ok(Box::new(UndeclaredReader::new(names)?))
        }),
    );
    let agent = b.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let findings = agent.check_assembly().unwrap();
    // Each plugin is checked once, by the innermost scope that contains it, so
    // the reader's branch reports this read once.
    assert_eq!(
        pairs(&findings),
        vec![(
            session.id().to_string(),
            Some("main".to_string()),
            "test.undeclared-reader".to_string(),
            AssemblyRule::UndeclaredRead
        )],
        "{findings:?}"
    );
    session.close().await.unwrap();
    agent.close().await.unwrap();
}

/// A reader whose provider withdraws after the read keeps the access but loses
/// the resolved dependency. Each plugin is checked once, by its innermost
/// scope: the reader's branch reports its `Pending` state and the lost
/// generation, and the required provider that left the tree is reported too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reads_of_a_withdrawn_generation_are_reported() {
    let armed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut b = builder();
    names::register::<Marker>(b.catalog_mut(), Some(BRANCH_GROUP));
    let provider_armed = armed.clone();
    b.require_feature(Feature::new("test.withdrawn-provider").branch(
        "test.withdrawn-provider",
        move |names| {
            Ok(Box::new(WithdrawnProvider {
                marker: names.key()?,
                armed: provider_armed.clone(),
            }))
        },
    ))
    .require_feature(Feature::new("test.declared-reader").branch(
        "test.declared-reader",
        |names| {
            Ok(Box::new(DeclaredReader {
                marker: names.key()?,
            }))
        },
    ));
    let agent = b.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    armed.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut pending = false;
    for _ in 0..400 {
        let state = agent
            .diagnostics()
            .unwrap()
            .plugins
            .into_iter()
            .find(|plugin| plugin.name == "test.declared-reader")
            .map(|plugin| plugin.state);
        if state == Some(FiberState::Pending) {
            pending = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(pending, "the reader loses its provider");
    let findings = agent.check_assembly().unwrap();
    assert_eq!(
        pairs(&findings),
        vec![
            (
                session.id().to_string(),
                Some("main".to_string()),
                "test.declared-reader".to_string(),
                AssemblyRule::Ready
            ),
            (
                session.id().to_string(),
                Some("main".to_string()),
                "test.declared-reader".to_string(),
                AssemblyRule::Generation
            ),
            (
                session.id().to_string(),
                Some("main".to_string()),
                "test.withdrawn-provider".to_string(),
                AssemblyRule::Ready
            ),
        ],
        "{findings:?}"
    );
    session.close().await.unwrap();
    agent.close().await.unwrap();
}

/// Provides the `Marker` a reader gates on and an external `Counter` in one
/// part, so the gate also guarantees the counter is bound.
struct PairProvider {
    marker: Key<Marker>,
    counter: Key<Counter>,
}
impl PairProvider {
    fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            marker: names.key()?,
            counter: names.key()?,
        })
    }
}
impl DynPlugin for PairProvider {
    fn name(&self) -> &str {
        "test.pair-provider"
    }
    fn apply<'a>(&'a self, ctx: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            ctx.provide_at(self.marker.clone(), std::sync::Arc::new(Marker))?;
            ctx.provide_at(
                self.counter.clone(),
                std::sync::Arc::new(Counter::default()),
            )?;
            Ok(Effect::Done)
        })
    }
}

/// Declares the marker and reads the counter through the non-strict read,
/// without declaring it.
struct UndeclaredReader {
    marker: Key<Marker>,
    counter: Key<Counter>,
}
impl UndeclaredReader {
    fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            marker: names.key()?,
            counter: names.key()?,
        })
    }
}
impl DynPlugin for UndeclaredReader {
    fn name(&self) -> &str {
        "test.undeclared-reader"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.marker.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            ctx.require_at(&self.marker)?;
            let _ = ctx.get_at(&self.counter);
            Ok(Effect::Done)
        })
    }
}

/// Provides the marker until the test arms it, after creation, and then
/// closes itself.
struct WithdrawnProvider {
    marker: Key<Marker>,
    armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl DynPlugin for WithdrawnProvider {
    fn name(&self) -> &str {
        "test.withdrawn-provider"
    }
    fn apply<'a>(&'a self, ctx: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        let ctx = ctx.clone();
        let marker = self.marker.clone();
        let armed = self.armed.clone();
        Box::pin(async move {
            ctx.provide_at(marker, std::sync::Arc::new(Marker))?;
            tokio::spawn(async move {
                while !armed.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let _ = ctx.dispose_self();
            });
            Ok(Effect::Done)
        })
    }
}

/// Declares the marker and reads it once.
struct DeclaredReader {
    marker: Key<Marker>,
}
impl DynPlugin for DeclaredReader {
    fn name(&self) -> &str {
        "test.declared-reader"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.marker.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            ctx.require_at(&self.marker)?;
            Ok(Effect::Done)
        })
    }
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
impl Plugin for FailingRollback {
    type Deps = ();
    fn name(&self) -> &str {
        "test.failing-rollback"
    }
    async fn apply(&self, ctx: &rutis::Ctx, (): Self::Deps) -> rutis::Result {
        for _ in 0..if self.fail_apply { 2 } else { 1 } {
            let cleanups = self.cleanups.clone();
            ctx.effect(|| async move {
                cleanups.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(rutis::Error::from(rutis::host::ErrorKind::ServiceNotFound(
                    "cleanup-marker".into(),
                )))
            })?;
        }
        if self.fail_apply {
            Err(rutis::Error::from(rutis::host::ErrorKind::ServiceNotFound(
                "build-marker".into(),
            )))
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn initialization_preserves_each_distinct_cleanup_failure_once() {
    let cleanups = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let timeline = cleanups.clone();
    let session = cleanups.clone();
    let mut builder = builder();
    builder.require_feature(
        Feature::new("test.failing-rollback")
            .branch("test.failing-rollback.branch", move |_| {
                Ok(Box::new(FailingRollback {
                    fail_apply: true,
                    cleanups: timeline.clone(),
                }))
            })
            .session("test.failing-rollback.session", move |_| {
                Ok(Box::new(FailingRollback {
                    fail_apply: false,
                    cleanups: session.clone(),
                }))
            }),
    );
    let agent = builder.build();
    let error = agent
        .create_session(SessionConfig::default())
        .await
        .err()
        .expect("initialization failed");
    let detail = format!("{error:?}");
    assert_eq!(cleanups.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(detail.matches("build-marker").count(), 1, "{detail}");
    assert_eq!(detail.matches("cleanup-marker").count(), 3, "{detail}");
    assert!(agent.sessions().is_empty());
    agent.close().await.unwrap();
}

struct Marker;
/// Provides `Marker` under its Session's key and reads the marker of the
/// first Session that loaded a probe.
struct ScopeProbe {
    provide: Option<Key<Marker>>,
    read: std::sync::Arc<std::sync::Mutex<Option<Key<Marker>>>>,
    seen: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Plugin for ScopeProbe {
    type Deps = ();
    fn name(&self) -> &str {
        "test.scope-probe"
    }
    async fn apply(&self, ctx: &rutis::Ctx, (): Self::Deps) -> rutis::Result {
        if let Some(key) = &self.provide {
            ctx.provide_at(key.clone(), std::sync::Arc::new(Marker))?;
            self.read.lock().unwrap().get_or_insert(key.clone());
        }
        if let Some(key) = self.read.lock().unwrap().clone() {
            self.seen.store(
                ctx.get_at(&key).is_some(),
                std::sync::atomic::Ordering::SeqCst,
            );
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_services_stay_inside_their_session_and_unkeyed_ones_are_reported() {
    let foreign = std::sync::Arc::new(std::sync::Mutex::new(None));
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut scoped = builder();
    names::register::<Marker>(scoped.catalog_mut(), Some(SESSION_GROUP));
    let (read, observed) = (foreign.clone(), seen.clone());
    scoped.require_feature(Feature::new("test.scope-probe").session(
        "test.scope-probe",
        move |names| {
            Ok(Box::new(ScopeProbe {
                provide: Some(names.key::<Marker>()?),
                read: read.clone(),
                seen: observed.clone(),
            }))
        },
    ));
    let agent = scoped.build();
    let first = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    assert!(seen.load(std::sync::atomic::Ordering::SeqCst));
    let second = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    assert!(!seen.load(std::sync::atomic::Ordering::SeqCst));
    first.close().await.unwrap();
    second.close().await.unwrap();
    agent.close().await.unwrap();

    let mut unkeyed = builder();
    unkeyed.require_feature(
        Feature::new("test.scope-probe").session("test.scope-probe", |_| {
            Ok(Box::new(ScopeProbe {
                provide: Some(Key::of()),
                read: Default::default(),
                seen: Default::default(),
            }))
        }),
    );
    let agent = unkeyed.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let findings = agent.check_assembly().unwrap();
    assert_eq!(
        pairs(&findings),
        vec![(
            session.id().to_string(),
            None,
            "test.scope-probe".to_string(),
            AssemblyRule::InstanceKey
        )],
        "{findings:?}"
    );
    session.close().await.unwrap();
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
    assert_eq!(baseline, Some((19, 5)));
    agent.close().await.unwrap();
}

struct NamedCapability {
    name: &'static str,
    bit: usize,
    installed: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pending: bool,
}

struct UnawaitedChild;
impl Plugin for UnawaitedChild {
    type Deps = ();
    fn name(&self) -> &str {
        "test.required-owner"
    }
    async fn apply(&self, ctx: &rutis::Ctx, (): Self::Deps) -> rutis::Result {
        ctx.plugin(NamedCapability {
            name: "unawaited-child",
            bit: 0,
            installed: Default::default(),
            pending: true,
        });
        Ok(())
    }
}
/// Pending until a never-provided gate opens when `pending` is set.
impl DynPlugin for NamedCapability {
    fn name(&self) -> &str {
        self.name
    }
    fn injects(&self) -> Vec<TypeKey> {
        if self.pending {
            vec![TypeKey::of::<NamedCapability>()]
        } else {
            Vec::new()
        }
    }
    fn apply<'a>(&'a self, _: &'a rutis::Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            self.installed
                .fetch_or(self.bit, std::sync::atomic::Ordering::SeqCst);
            Ok(Effect::Done)
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
        let feature = format!("test.named-{i}");
        b.require_feature(Feature::new(feature.clone()).session(feature, move |_| {
            Ok(Box::new(NamedCapability {
                name,
                bit: 1 << i,
                installed: installed.clone(),
                pending: false,
            }))
        }));
    }
    b.session_extension_part("trace", "Probe", |_, installed: &Arc<AtomicUsize>| {
        Ok(Box::new(NamedCapability {
            name: "Probe",
            bit: 1 << 9,
            installed: installed.clone(),
            pending: false,
        }))
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
        b.require_feature(
            Feature::new("test.factory").session("test.factory", move |_| {
                assert_ne!(
                    calls.fetch_add(1, Ordering::SeqCst),
                    panic_at,
                    "factory failure"
                );
                Ok(Box::new(NamedCapability {
                    name: "factory",
                    bit: 1,
                    installed: Arc::default(),
                    pending: false,
                }))
            }),
        );
        let agent = b.build();
        for _ in 0..panic_at {
            agent
                .create_session(SessionConfig::default())
                .await
                .unwrap();
        }
        let error = agent
            .create_session(SessionConfig::default())
            .await
            .err()
            .expect("a required factory failure rejects creation");
        assert!(error.to_string().contains("factory failure"), "{error}");
        assert_eq!(agent.sessions().len(), panic_at);
        agent.close().await.unwrap();
    }
    let mut b = builder();
    b.session_extension_part("extension", "declared", |_, _: &u32| {
        Ok(Box::new(NamedCapability {
            name: "actual",
            bit: 1,
            installed: Arc::default(),
            pending: false,
        }))
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
    b.session_extension_part("pending", "pending", |_, _: &()| {
        Ok(Box::new(NamedCapability {
            name: "pending",
            bit: 1,
            installed: Arc::default(),
            pending: true,
        }))
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

struct AssemblyAnchor(tokio::sync::mpsc::UnboundedSender<rutis::Ctx>);
impl Plugin for AssemblyAnchor {
    type Deps = ();
    fn name(&self) -> &str {
        "test.assembly-anchor"
    }
    async fn apply(&self, ctx: &rutis::Ctx, (): ()) -> rutis::Result {
        self.0.send(ctx.clone()).unwrap();
        Ok(())
    }
}

struct OptionalResource(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl Plugin for OptionalResource {
    type Deps = ();
    fn name(&self) -> &str {
        "test.optional-resource"
    }
    async fn apply(&self, ctx: &rutis::Ctx, (): ()) -> rutis::Result {
        let closed = self.0.clone();
        ctx.effect(move || async move {
            closed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })?;
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn optional_loader_rows_do_not_block_create_restore_or_initial_input_and_close_with_the_scope()
 {
    use rutis_loader::runtime::RuntimeResolver;
    use rutis_loader::{Chain, Layer, LoaderOptions, LoaderPlugin};
    use serde_json::json;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let root = rutis::Ctx::root_with(tokio::runtime::Handle::current());
    let store = Arc::new(MemoryTimelineStore::default());
    let closed = Arc::new(AtomicUsize::new(0));
    let rolled_back = Arc::new(AtomicUsize::new(0));
    let mut builtins = Modules::new();
    let mut b = builder();
    b.timeline_store(store);
    let catalog = b.catalog();
    let pending = register_part(&mut builtins, catalog, |_| {
        Ok(Box::new(NamedCapability {
            name: "SessionScope",
            bit: 0,
            installed: Arc::default(),
            pending: true,
        }))
    })
    .unwrap();
    let failed = register_part(&mut builtins, catalog, {
        let rolled_back = rolled_back.clone();
        move |_| {
            Ok(Box::new(FailingRollback {
                fail_apply: true,
                cleanups: rolled_back.clone(),
            }))
        }
    })
    .unwrap();
    let resource = register_part(&mut builtins, catalog, {
        let closed = closed.clone();
        move |_| Ok(Box::new(OptionalResource(closed.clone())))
    })
    .unwrap();
    let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
    let anchor = register_part(&mut builtins, catalog, move |_| {
        Ok(Box::new(AssemblyAnchor(send.clone())))
    })
    .unwrap();
    let plugins = b.plugin_rows(&mut builtins).await.unwrap();
    let mut session_row = b.install(&mut builtins, []).await.unwrap();
    let workspace = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let runtime = rutis_bridge::runtime::LocalRuntime::node(
        workspace.join("runtime/node_modules/@arcships/rutis-runtime"),
        workspace.join("runtime/package.json"),
    );
    let external = |prefix: &str| {
        vec![
            json!({"id":format!("{prefix}-pending"),"name":pending}),
            json!({"id":format!("{prefix}-failed"),"name":failed}),
            json!({"id":format!("{prefix}-resource"),"name":resource}),
            json!({"id":format!("{prefix}-unresolved"),"name":"missing-optional-module"}),
            json!({"id":format!("{prefix}-runtime"),"name":workspace.join("runtime/spikes/session-part.ts")}),
        ]
    };
    let parts = session_row["config"].as_array_mut().unwrap();
    let branch = parts
        .iter_mut()
        .find(|row| row["id"] == BRANCH_GROUP)
        .unwrap();
    branch["config"]
        .as_array_mut()
        .unwrap()
        .extend(external("optional-branch"));
    parts.extend(external("optional-session"));
    let resolver = RuntimeResolver::node(runtime.handle());
    let plugin = LoaderPlugin::new(
        Chain::new().with(builtins).with(resolver),
        LoaderOptions {
            catalog: b.catalog().clone(),
            ..Default::default()
        },
    );
    let loader = plugin.handle();
    (&root.plugin(plugin)).await.unwrap();
    let mut rows = plugins;
    rows.push(b.agent_row([json!({"id":anchor,"name":anchor}), session_row]));
    loader
        .reconcile(
            vec![Layer::new(
                "fixture",
                serde_json::from_value(json!([{"insert":rows}])).unwrap(),
            )],
            None,
        )
        .await
        .unwrap();
    let instance = loader.create_instance(&root, AGENT_GROUP).await.unwrap();
    let anchor_ctx = receive.recv().await.unwrap();
    let mut saved: Option<String> = None;
    for round in 0..2 {
        let agent = b.build_at(loader.clone(), &anchor_ctx);
        let session = match &saved {
            None => agent
                .create_session(SessionConfig::default())
                .await
                .unwrap(),
            Some(id) => agent.restore_session(id.clone()).await.unwrap(),
        };
        saved = Some(session.id().to_owned());
        let entries: Vec<_> = loader
            .entries()
            .into_iter()
            .filter(|row| row.id.starts_with("optional-") && row.instance.is_some())
            .collect();
        assert_eq!(entries.len(), 10);
        for suffix in ["pending", "runtime"] {
            assert!(
                entries
                    .iter()
                    .filter(|row| row.id.ends_with(suffix))
                    .all(|row| row.view.as_ref().unwrap().state().state
                        == rutis::host::FiberState::Pending)
            );
        }
        assert!(
            entries
                .iter()
                .filter(|row| row.id.ends_with("failed"))
                .all(|row| row.view.as_ref().unwrap().state().state
                    == rutis::host::FiberState::Failed)
        );
        assert!(
            entries
                .iter()
                .filter(|row| row.id.ends_with("unresolved"))
                .all(|row| row.view.is_none())
        );
        session
            .submit("first input".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
        assert!(
            session
                .history("main", None)
                .await
                .unwrap()
                .iter()
                .any(|event| matches!(event.kind, dim_agent_types::EventKind::TurnEnd(_)))
        );
        agent.close().await.unwrap();
        assert!(
            entries
                .iter()
                .filter_map(|row| row.view.as_ref())
                .all(|view| view.state().state == rutis::host::FiberState::Disposed)
        );
        assert_eq!(closed.load(Ordering::SeqCst), (round + 1) * 2);
        assert_eq!(rolled_back.load(Ordering::SeqCst), (round + 1) * 4);
    }
    instance.view.shutdown().await.unwrap();
    root.shutdown().await.unwrap();
}
