//! ToolBinding with a dynamic tool source (`tool-contributions.md` §7.7):
//! adoption, request snapshots, version pinning and release.

mod local_support;

use dim_agent_capabilities::{
    defaults,
    history::HistoryRenderer,
    host::*,
    tool_binding::ToolBinding,
    tool_registry::{
        SessionToolSource, ToolContribution, ToolHandler, ToolProvider, ToolRegistry,
        ToolSourceSet, ToolSourcesView, definition,
    },
};
use dim_agent_runtime::*;
use dim_agent_types::*;
use local_support::*;
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// Calls and handler drops, by source version.
#[derive(Default)]
struct Log {
    calls: Mutex<Vec<String>>,
    dropped: Mutex<Vec<String>>,
}
struct Handler {
    version: String,
    log: Arc<Log>,
    hold: Option<Arc<Gate>>,
}
impl Drop for Handler {
    fn drop(&mut self) {
        self.log.dropped.lock().unwrap().push(self.version.clone());
    }
}
impl ToolHandler for Handler {
    fn execute(
        &self,
        call: ToolCall,
        _: ToolExecutionContext,
        _: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            self.log
                .calls
                .lock()
                .unwrap()
                .push(format!("{}:{}", self.version, call.name));
            if let Some(gate) = &self.hold {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            Ok(ToolResult::text(format!(
                "{} ran {}",
                self.version, call.name
            )))
        })
    }
}

struct TestSource {
    sets: Mutex<Vec<Arc<ToolSourceSet>>>,
    changed: watch::Sender<()>,
}
impl TestSource {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            sets: Mutex::default(),
            changed: watch::Sender::new(()),
        })
    }
    fn offer(&self, sets: Vec<Arc<ToolSourceSet>>) {
        *self.sets.lock().unwrap() = sets;
        self.changed.send_replace(());
    }
}
impl ToolProvider for TestSource {
    fn sets(&self) -> Vec<Arc<ToolSourceSet>> {
        self.sets.lock().unwrap().clone()
    }
    fn changed(&self) -> watch::Receiver<()> {
        self.changed.subscribe()
    }
}

fn set(
    key: &str,
    version: &str,
    names: &[&str],
    log: &Arc<Log>,
    hold: Option<Arc<Gate>>,
) -> Arc<ToolSourceSet> {
    Arc::new(ToolSourceSet {
        key: key.into(),
        version: version.into(),
        tools: names
            .iter()
            .map(|name| {
                let mut definition =
                    definition(name, "Source tool", serde_json::json!({"type":"object"}));
                definition.risk = Some("session".into());
                Arc::new(ToolContribution {
                    definition,
                    handler: Arc::new(Handler {
                        version: version.into(),
                        log: log.clone(),
                        hold: hold.clone(),
                    }),
                })
            })
            .collect(),
    })
}

struct SourcePlugin {
    scope: SessionScope,
    source: Arc<TestSource>,
}
impl TypedPlugin for SourcePlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "TestSource"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as(
                self.scope.named_key::<SessionToolSource>("mcp"),
                Arc::new(SessionToolSource(self.source.clone())),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for SourcePlugin {
    fn keys(&self) -> KeysOf<Self> {}
}

struct Env {
    agent: Agent,
    model: Arc<ScriptedModel>,
}

fn selection(model_id: &str) -> ModelSelection {
    ModelSelection {
        provider_id: "mock".into(),
        model_id: model_id.into(),
        capability_snapshot: Some(ModelCapabilitySnapshot {
            target: ModelId {
                provider_id: "mock".into(),
                model_id: model_id.into(),
            },
            capabilities: ModelCapabilities {
                context_window: if model_id == "second" { 98765 } else { 135791 },
                ..Default::default()
            },
            reasoning: None,
            service_tier: None,
        }),
        config: None,
        metadata: None,
    }
}

fn env(source: Arc<TestSource>, store: Option<Arc<MemoryTimelineStore>>) -> Env {
    let model = Arc::new(ScriptedModel::default());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    if let Some(store) = store {
        builder.timeline_store(store);
    }
    builder
        .session_value(Arc::new(HistoryRenderer::default()))
        .session_value(Arc::new(ModelHost(model.clone())))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_plugin(dim_agent_capabilities::adapters::DefaultPrepareModelPlugin::new);
    defaults::tool_host(&mut builder, Arc::new(GateTools(Arc::new(Gate::default()))));
    defaults::install_with_prompt(
        &mut builder,
        Arc::new(EmptyPromptHost),
        Some(ConfigureUpdate {
            sources: Vec::new(),
            model: Some(selection("scripted")),
        }),
    );
    builder
        .session_tool_source("mcp")
        .session_plugin(move |scope| SourcePlugin {
            scope: *scope,
            source: source.clone(),
        });
    Env {
        agent: builder.build(),
        model,
    }
}

impl Env {
    async fn session(&self) -> Session {
        self.agent
            .create_session(SessionConfig {
                tools: vec!["gate".into()],
                ..SessionConfig::default()
            })
            .await
            .unwrap()
    }
    /// Runs one turn on `branch` whose model makes `calls`, then replies.
    async fn turn(&self, session: &Session, branch: &str, calls: &[&'static str]) {
        for name in calls {
            self.model.push(Step::Tool(name, serde_json::json!({})));
        }
        self.model.push(Step::Text("done"));
        session
            .submit("go".into(), InputMode::Queue, branch)
            .await
            .unwrap();
        session.idle(branch).await.unwrap();
    }
}

/// Waits until `branch` adopted exactly these source versions.
async fn adopted(session: &Session, branch: &str, sources: &[(&str, &str)]) {
    let expected: Vec<ToolSourceVersion> = sources
        .iter()
        .map(|(key, version)| ToolSourceVersion {
            key: (*key).into(),
            version: (*version).into(),
        })
        .collect();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = session
                .history(branch, None)
                .await
                .unwrap()
                .iter()
                .rev()
                .find_map(|event| match &event.kind {
                    EventKind::ContextConfigured(data) => Some(data.settings.tool_sources.clone()),
                    _ => None,
                });
            if current.as_ref() == Some(&expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("tool set adopted");
}

fn declared(request: &ModelRequest) -> Vec<String> {
    request
        .tools
        .iter()
        .flat_map(|tools| tools.iter())
        .map(|tool| tool.name.clone())
        .collect()
}

async fn last_result(session: &Session, branch: &str) -> String {
    session
        .history(branch, None)
        .await
        .unwrap()
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ToolResult(data) => match &data.results[0].result.content {
                ToolContent::Text(text) => Some(text.clone()),
                _ => None,
            },
            _ => None,
        })
        .unwrap()
}

struct ContributorEnabled;
struct EnableContributor;
impl TypedPlugin for EnableContributor {
    type Deps = ();
    fn name(&self) -> &str {
        "EnableContributor"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): (),
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide(ContributorEnabled)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for EnableContributor {
    fn keys(&self) -> KeysOf<Self> {}
}
struct OptionalContributor {
    scope: SessionScope,
    generations: Arc<AtomicUsize>,
    hold: Arc<Gate>,
}
struct OptionalHandler {
    generation: usize,
    stopping: CancellationToken,
    hold: Arc<Gate>,
}
impl ToolHandler for OptionalHandler {
    fn execute(
        &self,
        call: ToolCall,
        _: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            if self.stopping.is_cancelled() {
                return Err(HostError::Cancelled("provider unloaded".into()));
            }
            if call.input.get("hold") == Some(&serde_json::json!(true)) {
                self.hold.entered.notify_one();
                tokio::select! {
                    biased;
                    _ = self.stopping.cancelled() => return Err(HostError::Cancelled("provider unloaded".into())),
                    _ = cancel.cancelled() => return Err(HostError::Cancelled("call cancelled".into())),
                    _ = self.hold.release.notified() => {}
                }
            }
            Ok(ToolResult::text(format!("generation {}", self.generation)))
        })
    }
}
impl TypedPlugin for OptionalContributor {
    type Deps = Arc<ContributorEnabled>;
    fn name(&self) -> &str {
        "OptionalContributor"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        _: Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let generation = self.generations.fetch_add(1, Ordering::SeqCst) + 1;
            ctx.provide_as(
                self.scope.named_key::<ToolContribution>("optional"),
                Arc::new(ToolContribution {
                    definition: definition(
                        "optional",
                        "Report provider generation",
                        serde_json::json!({"type":"object"}),
                    ),
                    handler: Arc::new(OptionalHandler {
                        generation,
                        stopping: ctx.cancellation_token(),
                        hold: self.hold.clone(),
                    }),
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for OptionalContributor {
    fn keys(&self) -> KeysOf<Self> {}
}

async fn plugin_state(view: &rutis::FiberView, state: rutis::FiberState) {
    tokio::time::timeout(Duration::from_secs(5), async {
        view.watch()
            .wait_for(|snapshot| snapshot.state == state)
            .await
            .unwrap();
    })
    .await
    .unwrap();
}

async fn fixed_tools_adopted(session: &Session, expected: &[&str]) {
    let mut progress = session.dispatch_watch("main").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let history = session.history("main", None).await.unwrap();
            let tools = history.iter().rev().find_map(|event| match &event.kind {
                EventKind::ContextConfigured(data) => data.settings.tools.as_ref(),
                _ => None,
            });
            if tools.is_some_and(|tools| {
                tools
                    .iter()
                    .map(|tool| tool.name.as_str())
                    .eq(expected.iter().copied())
            }) {
                return;
            }
            progress.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn optional_contributors_do_not_gate_the_loop_and_new_generations_wait_for_reload() {
    let root = Ctx::root().unwrap();
    let enabled = root.plugin(capability_plugin(EnableContributor));
    plugin_state(&enabled, rutis::FiberState::Active).await;
    let generations = Arc::new(AtomicUsize::new(0));
    let hold = Arc::new(Gate::default());
    let held = hold.clone();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::mock_hosts(&mut builder);
    defaults::install_with_prompt(
        &mut builder,
        Arc::new(EmptyPromptHost),
        Some(ConfigureUpdate {
            sources: Vec::new(),
            model: Some(selection("scripted")),
        }),
    );
    builder
        .session_tool("optional")
        .session_tool("absent")
        .session_plugin(move |scope| OptionalContributor {
            scope: *scope,
            generations: generations.clone(),
            hold: held.clone(),
        });
    let agent = builder.build_in(&root);
    let session = agent
        .create_session(SessionConfig {
            tools: vec!["optional".into(), "absent".into(), "echo".into()],
            permission_policy: PermissionPolicy {
                allowed_tools: vec!["echo".into(), "optional".into()],
                ..PermissionPolicy::default()
            },
            ..SessionConfig::default()
        })
        .await
        .unwrap();
    fixed_tools_adopted(&session, &["optional", "echo"]).await;
    session
        .submit("tool:optional {}".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    assert_eq!(last_result(&session, "main").await, "generation 1");
    let snapshot = session.diagnostics().unwrap();
    let registry = snapshot
        .plugins
        .iter()
        .find(|plugin| plugin.name == "ToolRegistry")
        .unwrap();
    let registry_id = registry.id;
    let registry_generation = registry.generation;
    let provider = root
        .view(
            snapshot
                .plugins
                .iter()
                .find(|plugin| plugin.name == "OptionalContributor")
                .unwrap()
                .id,
        )
        .unwrap();
    let before = session.sequence("main").unwrap();
    session
        .submit(
            "tool:optional {\"hold\":true}".into(),
            InputMode::Queue,
            "main",
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), hold.entered.notified())
        .await
        .unwrap();
    enabled.dispose().await.unwrap();
    plugin_state(&provider, rutis::FiberState::Pending).await;
    session.idle("main").await.unwrap();
    assert!(
        last_result(&session, "main")
            .await
            .contains("provider unloaded")
    );
    let history = session.history("main", None).await.unwrap();
    assert_eq!(
        history
            .iter()
            .filter(
                |event| event.sequence > before && matches!(event.kind, EventKind::ToolResult(_))
            )
            .count(),
        1
    );
    session
        .submit(
            "tool:echo {\"text\":\"still running\"}".into(),
            InputMode::Queue,
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    assert!(
        last_result(&session, "main")
            .await
            .contains("still running")
    );
    let current = session.diagnostics().unwrap();
    let registry = current
        .plugins
        .iter()
        .find(|plugin| plugin.id == registry_id)
        .unwrap();
    assert_eq!(registry.state, rutis::FiberState::Active);
    assert_eq!(registry.generation, registry_generation);

    let enabled = root.plugin(capability_plugin(EnableContributor));
    plugin_state(&enabled, rutis::FiberState::Active).await;
    plugin_state(&provider, rutis::FiberState::Active).await;
    session
        .submit("tool:optional {}".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    assert!(
        last_result(&session, "main")
            .await
            .contains("provider unloaded")
    );
    let branches = session.branch_diagnostics("main").unwrap();
    let previous_loop = branches
        .plugins
        .iter()
        .find(|plugin| plugin.name == "Loop")
        .unwrap();
    let loop_view = root.view(previous_loop.id).unwrap();
    root.view(registry_id).unwrap().restart().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop_view
            .watch()
            .wait_for(|snapshot| {
                snapshot.generation > previous_loop.generation
                    && snapshot.state == rutis::FiberState::Active
            })
            .await
            .unwrap();
    })
    .await
    .unwrap();
    fixed_tools_adopted(&session, &["optional", "echo"]).await;
    session
        .submit("tool:optional {}".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    assert_eq!(last_result(&session, "main").await, "generation 2");
    agent.close().await.unwrap();
    root.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_runs_the_version_its_request_declared_and_old_versions_are_released() {
    let (source, log, hold) = (
        TestSource::new(),
        Arc::new(Log::default()),
        Arc::new(Gate::default()),
    );
    source.offer(vec![set(
        "mcp:a",
        "v1",
        &["lookup"],
        &log,
        Some(hold.clone()),
    )]);
    let env = env(source.clone(), None);
    let session = env.session().await;
    adopted(&session, "main", &[("mcp:a", "v1")]).await;

    // The call starts on v1; v2 is adopted while it runs.
    env.model.push(Step::Tool("lookup", serde_json::json!({})));
    env.model.push(Step::Text("done"));
    session
        .submit("go".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    hold.entered.notified().await;
    source.offer(vec![set("mcp:a", "v2", &["lookup"], &log, None)]);
    adopted(&session, "main", &[("mcp:a", "v2")]).await;
    hold.release.notify_one();
    session.idle("main").await.unwrap();
    assert_eq!(last_result(&session, "main").await, "v1 ran lookup");
    assert_eq!(declared(&requests(&session).await[0]), ["gate", "lookup"]);

    // The next request declares and runs v2; binding it releases v1.
    env.turn(&session, "main", &["lookup"]).await;
    assert_eq!(last_result(&session, "main").await, "v2 ran lookup");
    assert!(log.dropped.lock().unwrap().contains(&"v1".to_owned()));
    assert!(!log.dropped.lock().unwrap().contains(&"v2".to_owned()));

    // A withdrawn source is no longer declared or run.
    source.offer(Vec::new());
    adopted(&session, "main", &[]).await;
    let calls = log.calls.lock().unwrap().len();
    env.turn(&session, "main", &[]).await;
    assert_eq!(log.calls.lock().unwrap().len(), calls);
    assert_eq!(declared(requests(&session).await.last().unwrap()), ["gate"]);
    env.agent.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn conflicting_sets_are_rejected_whole_and_recover_once_the_name_is_free() {
    let (source, log) = (TestSource::new(), Arc::new(Log::default()));
    // `gate` is a fixed tool; `b` repeats `a`'s tool.
    source.offer(vec![
        set("mcp:a", "a1", &["find"], &log, None),
        set("mcp:b", "b1", &["find", "other"], &log, None),
        set("mcp:c", "c1", &["gate"], &log, None),
        set("mcp:d", "d1", &["branch"], &log, None),
    ]);
    let env = env(source.clone(), None);
    let session = env.session().await;
    adopted(&session, "main", &[("mcp:a", "a1")]).await;
    let state = session.session_service::<ToolSourcesView>().unwrap().get();
    assert_eq!(state.0["mcp:a"].adopted.as_deref(), Some("a1"));
    assert_eq!(state.0["mcp:b"].adopted, None);
    assert_eq!(state.0["mcp:b"].rejected.as_ref().unwrap().version, "b1");
    assert!(state.0["mcp:c"].rejected.is_some());
    assert!(state.0["mcp:d"].rejected.is_some());
    env.turn(&session, "main", &[]).await;
    assert_eq!(declared(&requests(&session).await[0]), ["gate", "find"]);

    // Withdrawing `a` frees the name: `b` is adopted as a whole.
    source.offer(vec![set("mcp:b", "b1", &["find", "other"], &log, None)]);
    adopted(&session, "main", &[("mcp:b", "b1")]).await;
    env.turn(&session, "main", &["other"]).await;
    assert_eq!(last_result(&session, "main").await, "b1 ran other");
    assert_eq!(
        declared(requests(&session).await.last().unwrap()),
        ["gate", "find", "other"]
    );
    env.agent.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_name_freed_in_the_same_refresh_admits_the_rejected_set() {
    let (source, log) = (TestSource::new(), Arc::new(Log::default()));
    source.offer(vec![set("mcp:z", "z1", &["lookup"], &log, None)]);
    let env = env(source.clone(), None);
    let session = env.session().await;
    adopted(&session, "main", &[("mcp:z", "z1")]).await;
    // `a` wants the name `z` holds.
    source.offer(vec![
        set("mcp:a", "a1", &["lookup"], &log, None),
        set("mcp:z", "z1", &["lookup"], &log, None),
    ]);
    tokio::time::timeout(Duration::from_secs(5), async {
        while session
            .session_service::<ToolSourcesView>()
            .unwrap()
            .get()
            .0
            .get("mcp:a")
            .is_none_or(|status| status.rejected.is_none())
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a rejected");
    // One change frees the name: `a` is checked before `z` is replaced, and
    // still adopted in the same refresh.
    source.offer(vec![
        set("mcp:a", "a1", &["lookup"], &log, None),
        set("mcp:z", "z2", &["other"], &log, None),
    ]);
    adopted(&session, "main", &[("mcp:a", "a1"), ("mcp:z", "z2")]).await;
    env.turn(&session, "main", &["lookup"]).await;
    assert_eq!(last_result(&session, "main").await, "a1 ran lookup");
    env.agent.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_child_branch_binds_the_inherited_tools() {
    let (source, log) = (TestSource::new(), Arc::new(Log::default()));
    source.offer(vec![set("mcp:a", "v1", &["lookup"], &log, None)]);
    let env = env(source.clone(), None);
    let session = env.session().await;
    adopted(&session, "main", &[("mcp:a", "v1")]).await;
    env.model.push(Step::Tool("lookup", serde_json::json!({})));
    env.model.push(Step::Text("done"));
    let child = session
        .branch(
            "main".into(),
            session.sequence("main").unwrap(),
            BranchOptions {
                prompt: "child".into(),
                source: BranchSource::Host {
                    operation_id: "child".into(),
                },
            },
        )
        .await
        .unwrap();
    session.idle(&child).await.unwrap();
    assert_eq!(last_result(&session, &child).await, "v1 ran lookup");
    env.agent.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restored_branch_adopts_the_new_versions_before_its_first_request() {
    let store = Arc::new(MemoryTimelineStore::default());
    let (source, log) = (TestSource::new(), Arc::new(Log::default()));
    source.offer(vec![set("mcp:a", "v1", &["lookup"], &log, None)]);
    let env1 = env(source, Some(store.clone()));
    let session = env1.session().await;
    adopted(&session, "main", &[("mcp:a", "v1")]).await;
    let id = session.id().to_owned();
    env1.agent.close().await.unwrap();

    // A new process: the source offers a new version of the same key.
    let source = TestSource::new();
    source.offer(vec![set("mcp:a", "v2", &["lookup"], &log, None)]);
    let env2 = env(source, Some(store));
    let restored = env2.agent.restore_session(id).await.unwrap();
    adopted(&restored, "main", &[("mcp:a", "v2")]).await;
    env2.turn(&restored, "main", &["lookup"]).await;
    assert_eq!(last_result(&restored, "main").await, "v2 ran lookup");
    env2.agent.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_model_update_and_a_tool_adoption_both_take_effect() {
    let (source, log) = (TestSource::new(), Arc::new(Log::default()));
    let env = env(source.clone(), None);
    let session = env.session().await;
    let (configured, _) = tokio::join!(
        session.configure(
            ConfigureUpdate {
                sources: Vec::new(),
                model: Some(selection("second")),
            },
            "main",
        ),
        async { source.offer(vec![set("mcp:a", "v1", &["lookup"], &log, None)]) },
    );
    assert_eq!(configured.unwrap(), ConfigureOutcome::Adopted);
    adopted(&session, "main", &[("mcp:a", "v1")]).await;
    env.turn(&session, "main", &["lookup"]).await;
    let request = requests(&session).await.pop().unwrap();
    assert_eq!(request.model_id, "second");
    assert_eq!(
        request
            .capability_snapshot
            .as_ref()
            .unwrap()
            .capabilities
            .context_window,
        98765
    );
    assert_eq!(declared(&request), ["gate", "lookup"]);
    assert_eq!(last_result(&session, "main").await, "v1 ran lookup");
    env.agent.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tool_updates_wait_for_branch_activation_and_do_not_run_after_failed_assembly() {
    struct StartupGate {
        scope: BranchScope,
        gate: Arc<Gate>,
        ready: tokio::sync::mpsc::UnboundedSender<(Arc<Timeline>, Arc<ToolRegistry>)>,
        fail: bool,
    }
    impl TypedPlugin for StartupGate {
        type Deps = (
            BranchGate<ToolBinding>,
            InBranch<Timeline>,
            InSession<ToolRegistry>,
        );
        fn name(&self) -> &str {
            "ToolBindingStartupGate"
        }
        fn apply<'a>(
            &'a self,
            _ctx: &'a Ctx,
            (_, InBranch(timeline), InSession(registry)): Self::Deps,
        ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
            Box::pin(async move {
                self.ready.send((timeline, registry)).unwrap();
                self.gate.release.notified().await;
                if self.fail {
                    return Err(CordisError::PluginFailed("assembly rejected".into()));
                }
                Ok(Effect::Done)
            })
        }
    }
    impl Capability for StartupGate {
        fn keys(&self) -> KeysOf<Self> {
            scoped::<Self::Deps, _>(&self.scope)
        }
    }

    for fail in [false, true] {
        let (source, log) = (TestSource::new(), Arc::new(Log::default()));
        source.offer(vec![set("mcp:a", "v1", &["lookup"], &log, None)]);
        let model = Arc::new(ScriptedModel::default());
        let mut builder = AgentBuilder::new(AgentConfig::default());
        builder
            .session_value(Arc::new(HistoryRenderer::default()))
            .session_value(Arc::new(ModelHost(model.clone())))
            .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
            .session_value(Arc::new(ContextModelHost(None)))
            .session_plugin(dim_agent_capabilities::adapters::DefaultPrepareModelPlugin::new);
        defaults::tool_host(&mut builder, Arc::new(GateTools(Arc::new(Gate::default()))));
        defaults::install_with_prompt(
            &mut builder,
            Arc::new(EmptyPromptHost),
            Some(ConfigureUpdate {
                sources: Vec::new(),
                model: Some(selection("scripted")),
            }),
        );
        let provider = source.clone();
        builder
            .session_tool_source("mcp")
            .session_plugin(move |scope| SourcePlugin {
                scope: *scope,
                source: provider.clone(),
            });
        let gate = Arc::new(Gate::default());
        let hold = gate.clone();
        let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
        builder.timeline_plugin(move |scope| StartupGate {
            scope: *scope,
            gate: hold.clone(),
            ready: send.clone(),
            fail,
        });
        let env = Env {
            agent: builder.build(),
            model,
        };
        let agent = env.agent.clone();
        let creating = tokio::spawn(async move {
            agent
                .create_session(SessionConfig {
                    tools: vec!["gate".into()],
                    ..SessionConfig::default()
                })
                .await
        });
        let (timeline, registry) = tokio::time::timeout(Duration::from_secs(5), receive.recv())
            .await
            .unwrap()
            .unwrap();
        let before = timeline.sequence();
        let mut published = timeline.watch();
        let mut sources = registry.sources_state();
        source.offer(vec![set("mcp:a", "v2", &["lookup"], &log, None)]);
        tokio::time::timeout(
            Duration::from_secs(5),
            sources.wait_for(|state| state.0["mcp:a"].adopted.as_deref() == Some("v2")),
        )
        .await
        .unwrap()
        .unwrap();
        // The branch remains in assembly even though ToolBinding is available.
        let premature = tokio::time::timeout(Duration::from_millis(150), published.changed())
            .await
            .is_ok();
        gate.release.notify_one();
        let created = tokio::time::timeout(Duration::from_secs(5), creating)
            .await
            .unwrap()
            .unwrap();
        if fail {
            assert!(created.is_err());
            assert_eq!(
                timeline.sequence(),
                before,
                "failed assembly adopted an update"
            );
        } else {
            let session = created.unwrap();
            adopted(&session, "main", &[("mcp:a", "v2")]).await;
            env.turn(&session, "main", &["lookup"]).await;
            assert_eq!(last_result(&session, "main").await, "v2 ran lookup");
        }
        env.agent.close().await.unwrap();
        assert!(
            !premature,
            "tool update was adopted before branch activation"
        );
    }
}
