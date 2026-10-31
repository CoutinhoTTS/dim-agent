//! Hook points through the public Session interfaces and real commands
//! (`docs/design/plugins/Hooks.md`, appendix D).
#![cfg(unix)]
use dim_agent_capabilities::{
    defaults,
    execution_settings::ExecutionSettingsHost,
    history::HistoryRenderer,
    hooks::{
        HookSourceId, HookSourceSnapshot, HookSourceState, HookSourcesHost, HookSourcesSnapshot,
        HooksDocument, ReadHookSources,
    },
    host::*,
    host_platform::{
        ExecShellRuntime, HostPlatform, OutputEncoding, ShellEdition, ShellSource, ShellStatus,
    },
    model_provider::ModelProviderPlugin,
    permission::{CommandRulesHost, NoCommandRules},
    skill_settings::SkillSettingsHost,
    skills::{MemorySkillSettings, SkillsCatalogPlugin},
    workspace_tools::WorkspaceToolsConfig,
};
use dim_agent_runtime::testing::MinimalHost;
use dim_agent_runtime::*;
use dim_agent_types::*;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// The test compaction strategy: the legal cut the retention rule allows and
/// a one-message summary.
#[derive(Clone, Copy, Default)]
struct Small;
impl ResolveContextModel for Small {
    fn resolve(
        &self,
        _: Arc<ModelRequest>,
        _: CancellationToken,
    ) -> HostFuture<'_, Arc<dyn ContextModel>> {
        Box::pin(async move { Ok(Arc::new(Small) as Arc<dyn ContextModel>) })
    }
}
impl ContextModel for Small {
    fn context_window(&self) -> f64 {
        4096.
    }
    fn output_tokens(&self) -> f64 {
        64.
    }
    fn threshold_tokens(&self) -> f64 {
        10_000.
    }
    fn count_tokens(&self, request: &ModelRequest) -> f64 {
        serde_json::to_vec(&request.messages).unwrap().len() as f64
    }
}
fn diagnostics() -> StrategyDiagnostics {
    StrategyDiagnostics {
        model_usage: None,
        model_call_count: 1,
        record: CompactionStrategyRecord {
            kind: "test".into(),
            details: JsonObject::new(),
        },
    }
}
impl CompactionStrategy for Small {
    fn name(&self) -> &str {
        "test"
    }
    fn replacement_size(&self, model: &dyn ContextModel) -> f64 {
        model.count_tokens(&ModelRequest::new(
            "",
            "",
            vec![dim_agent_capabilities::history::summary_message("short")],
        ))
    }
    fn compact(
        &self,
        input: CompactionInput,
        _: CancellationToken,
    ) -> HostFuture<'_, StrategyOutcome> {
        Box::pin(async move {
            Ok(match input.cut() {
                Some(cut) => StrategyOutcome::Completed {
                    retain_from: cut.retain_from.clone(),
                    replacement: vec![dim_agent_capabilities::history::summary_message("short")],
                    diagnostics: diagnostics(),
                },
                None => StrategyOutcome::Skipped {
                    reason: "nothing_to_compact".into(),
                    diagnostics: diagnostics(),
                },
            })
        })
    }
}

/// The hook documents the test serves.
#[derive(Default)]
struct Sources {
    user: Mutex<Option<HooksDocument>>,
    /// Whether the workspace's hooks are enabled, and its document.
    project: Mutex<Option<(bool, HooksDocument)>>,
    /// The documents plugins registered, with their directories.
    plugins: Mutex<Vec<(String, PathBuf, HooksDocument)>>,
    home: Mutex<Option<PathBuf>>,
}
impl Sources {
    fn set(&self, document: HooksDocument) {
        *self.user.lock().unwrap() = Some(document);
    }
    fn set_project(&self, document: HooksDocument, enabled: bool) {
        *self.project.lock().unwrap() = Some((enabled, document));
    }
    fn set_plugin(&self, id: &str, dir: &Path, document: HooksDocument) {
        self.plugins
            .lock()
            .unwrap()
            .push((id.to_owned(), dir.to_owned(), document));
    }
}
impl ReadHookSources for Sources {
    fn sources(&self, _: &Path, _: CancellationToken) -> HostFuture<'_, HookSourcesSnapshot> {
        let user = self.user.lock().unwrap().clone();
        let project = self.project.lock().unwrap().clone();
        let plugins = self.plugins.lock().unwrap().clone();
        Box::pin(async move {
            let mut sources = vec![HookSourceSnapshot {
                id: HookSourceId::User,
                path: None,
                enabled: true,
                state: if user.is_some() {
                    HookSourceState::Ok
                } else {
                    HookSourceState::Missing
                },
                document: user,
            }];
            if let Some((enabled, document)) = project {
                sources.push(HookSourceSnapshot {
                    id: HookSourceId::Project,
                    path: None,
                    enabled,
                    state: HookSourceState::Ok,
                    document: Some(document),
                });
            }
            for (id, dir, document) in plugins {
                sources.push(HookSourceSnapshot {
                    id: HookSourceId::Plugin(id),
                    path: Some(dir),
                    enabled: true,
                    state: HookSourceState::Ok,
                    document: Some(document),
                });
            }
            Ok(HookSourcesSnapshot { sources })
        })
    }
    fn home(&self) -> Option<PathBuf> {
        self.home.lock().unwrap().clone()
    }
}

/// The scripted model records each request, then answers like the echo model,
/// or fails on the provider budget so the turn compacts and retries.
#[derive(Default)]
struct Model {
    requests: Mutex<Vec<Arc<ModelRequest>>>,
    exhaust: bool,
}
impl ModelDriver for Model {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        self.requests.lock().unwrap().push(request.clone());
        if self.exhaust {
            return Box::pin(async {
                Err(HostError::Failed {
                    kind: HostErrorKind::Other,
                    message: "Provider output budget exhausted".into(),
                    model: None,
                    presentation: None,
                    budget: Some(ProviderOutputBudgetFacts {
                        input_tokens: 800.,
                        context_window: 1000.,
                        requested_max_tokens: 400.,
                        allowed_max_tokens: 200.,
                        ratio: 1.,
                    }),
                })
            });
        }
        dim_agent_capabilities::adapters::EchoModel.invoke(request, connection, cancel)
    }
}
impl Model {
    fn requests(&self) -> Vec<Arc<ModelRequest>> {
        self.requests.lock().unwrap().clone()
    }
}

struct Env {
    agent: MinimalHost,
    sources: Arc<Sources>,
    model: Arc<Model>,
    dir: PathBuf,
}
impl Env {
    async fn new() -> Self {
        Self::with(Arc::new(Model::default()), None).await
    }

    async fn with(model: Arc<Model>, strategy: Option<Arc<Small>>) -> Self {
        let context_model: Option<Arc<dyn ResolveContextModel>> = match &strategy {
            Some(strategy) => {
                let model: Arc<dyn ResolveContextModel> = strategy.clone();
                Some(model)
            }
            None => None,
        };
        let compaction: Option<Arc<dyn CompactionStrategy>> = match strategy {
            Some(strategy) => {
                let compaction: Arc<dyn CompactionStrategy> = strategy;
                Some(compaction)
            }
            None => None,
        };
        let dir = std::env::temp_dir().join(format!("dim-hooks-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("work")).unwrap();
        std::fs::create_dir_all(dir.join("home")).unwrap();
        let dir = dir.canonicalize().unwrap();
        let mut platform = HostPlatform::current(ShellStatus::Available(ExecShellRuntime {
            executable_path: "/bin/sh".into(),
            edition: ShellEdition::Sh,
            source: ShellSource::Fallback,
            output_encoding: OutputEncoding::Utf8,
        }))
        .unwrap()
        .with_execution_settings(ExecutionSettingsHost::fixed(Default::default()));
        platform.home = dir.join("home");
        let sources = Arc::new(Sources::default());
        *sources.home.lock().unwrap() = Some(dir.join("home"));
        let mut builder = AgentBuilder::new(AgentConfig::default());
        builder.timeline_store(Arc::new(MemoryTimelineStore::default()));
        builder
            .session_value(Arc::new(HistoryRenderer::default()))
            .process_value(Arc::new(ModelHost(model.clone())))
            .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
            .session_value(Arc::new(ContextModelHost(context_model)))
            .session_value(Arc::new(CommandRulesHost(Arc::new(NoCommandRules))))
            .session_value(Arc::new(CompactionStrategyHost(compaction)))
            .session_value(Arc::new(HookSourcesHost(sources.clone())))
            .require_feature(Feature::new("PrepareModelHost").session(
                "PrepareModelHost",
                |names| {
                    Ok(Box::new(
                        dim_agent_capabilities::adapters::DefaultPrepareModelPlugin::new(names)?,
                    ))
                },
            ));
        defaults::install_local_execution(
            &mut builder,
            ModelProviderPlugin::new,
            WorkspaceToolsConfig {
                output_root: dir.join("outputs"),
            },
            dim_agent_capabilities::timeline_tree::BranchToolConfig::default(),
        );
        builder.plugin(|names| Ok(Box::new(SkillsCatalogPlugin::new(names)?)));
        dim_agent_capabilities::hooks::install(&mut builder);
        let agent = MinimalHost::start_with(&builder, |root, _| {
            root.provide_as::<HostPlatform>(Arc::new(platform))?;
            root.provide_as::<SkillSettingsHost>(Arc::new(SkillSettingsHost(Arc::new(
                MemorySkillSettings::default(),
            ))))?;
            Ok(())
        })
        .await
        .unwrap();
        Self {
            agent,
            sources,
            model,
            dir,
        }
    }

    async fn session(&self) -> Session {
        self.session_with(PermissionPreset::FullAccess).await
    }

    async fn session_with(&self, preset: PermissionPreset) -> Session {
        self.agent
            .create_session(SessionConfig {
                cwd: self.dir.join("work").to_string_lossy().into_owned(),
                tools: Some(vec!["read".into(), "exec".into()]),
                permission_policy: PermissionPolicy {
                    preset,
                    ..Default::default()
                },
                ..Default::default()
            })
            .await
            .unwrap()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    async fn close(self) {
        self.agent.close().await.unwrap();
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}

fn document(hooks: serde_json::Value) -> HooksDocument {
    serde_json::from_value(serde_json::json!({ "hooks": hooks })).unwrap()
}
fn command(group: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "hooks": [group] })
}

async fn turn(session: &Session, text: &str) {
    session
        .submit(text.into(), InputMode::Queue, "main")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), session.idle("main"))
        .await
        .unwrap()
        .unwrap();
}

async fn events(session: &Session) -> Vec<Arc<AgentEvent>> {
    session.local("main", ..).await.unwrap()
}

/// The raw blocks of every input, in order.
async fn blocks(session: &Session) -> Vec<InputBlock> {
    events(session)
        .await
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::InputMessage(data) => Some(data.blocks.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn block_text(block: &InputBlock) -> &str {
    match block {
        InputBlock::Raw { text, .. } => text,
    }
}

/// The text of every hook block of `kind`, in order.
async fn hook_blocks(session: &Session, kind: &str) -> Vec<String> {
    blocks(session)
        .await
        .iter()
        .filter(|block| block_text(block).contains(&format!("<dim-hooks kind=\"{kind}\"")))
        .map(|block| block_text(block).to_owned())
        .collect()
}

/// Waits for a hook block of `kind` to be written.
async fn wait_blocks(session: &Session, kind: &str, count: usize) -> Vec<String> {
    for _ in 0..500 {
        let found = hook_blocks(session, kind).await;
        if found.len() >= count {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "{count} {kind} blocks were not written: {:?}",
        hook_blocks(session, kind).await
    );
}

/// The text of the last model request.
fn last_request(model: &Model) -> String {
    let requests = model.requests();
    let request = requests.last().expect("a model request");
    serde_json::to_string(&request.messages).unwrap()
}

fn sleep(millis: u64) -> String {
    format!("sleep {}", millis as f64 / 1000.0)
}

fn marker_command(path: &Path, text: &str) -> String {
    format!("printf '{text}' >> {}", path.display())
}

/// Waits for an event matching `find` on `main`.
async fn wait_event<T>(session: &Session, find: impl Fn(&EventKind) -> Option<T>) -> T {
    for _ in 0..500 {
        if let Some(found) = session
            .local("main", ..)
            .await
            .unwrap()
            .iter()
            .find_map(|event| find(&event.kind))
        {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the event was not committed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_start_writes_one_keyed_block_per_hook_and_the_model_sees_it() {
    let env = Env::new().await;
    env.sources.set(document(serde_json::json!({
        "SessionStart": [command(serde_json::json!({
            "type": "command", "command": "printf 'first context'"
        }))],
        "Stop": [command(serde_json::json!({
            "type": "command", "command": "printf '{\"hookSpecificOutput\":{\"additionalContext\":\"stop context\"}}'"
        }))],
    })));
    let session = env.session().await;
    let blocks = wait_blocks(&session, "session-start", 1).await;
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    assert!(blocks[0].contains("first context"), "{blocks:?}");
    let keyed = blocks_of_key(&session, "hooks:session-start/1").await;
    assert_eq!(keyed, 1, "the block is keyed once");

    turn(&session, "hello").await;
    let request = last_request(&env.model);
    assert!(
        request.contains("first context"),
        "the first request carries it"
    );
    env.close().await;
}

async fn blocks_of_key(session: &Session, key: &str) -> usize {
    blocks(session)
        .await
        .iter()
        .filter(|block| matches!(block, InputBlock::Raw { key: Some(found), .. } if found == key))
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_tool_use_follows_the_protocol_for_success_block_and_failure() {
    let env = Env::new().await;
    let ok = env.path("ok.sh");
    std::fs::write(
        &ok,
        "#!/bin/sh\nprintf '{\"hookSpecificOutput\":{\"additionalContext\":\"tool context\"}}'\n",
    )
    .unwrap();
    let blocked = env.path("blocked.sh");
    std::fs::write(&blocked, "#!/bin/sh\necho 'blocked reason' >&2\nexit 2\n").unwrap();
    let broken = env.path("broken.sh");
    std::fs::write(&broken, "#!/bin/sh\nexit 1\n").unwrap();
    for path in [&ok, &blocked, &broken] {
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
    }
    env.sources.set(document(serde_json::json!({
        "PostToolUse": [
            command(serde_json::json!({"type": "command", "command": ok.to_str().unwrap()})),
        ],
    })));
    let session = env.session().await;
    let note = env.path("work/note.txt");
    std::fs::write(&note, "note\n").unwrap();
    turn(
        &session,
        &format!("tool:read {{\"path\":{:?}}}", note.to_str().unwrap()),
    )
    .await;
    let blocks = wait_blocks(&session, "post-tool", 1).await;
    assert!(blocks[0].contains("tool context"), "{blocks:?}");
    assert!(blocks[0].contains("call_id="), "{blocks:?}");
    assert!(blocks[0].contains("tool=\"read\""), "{blocks:?}");
    turn(&session, "again").await;
    let request = last_request(&env.model);
    assert!(
        request.contains("tool context"),
        "the context reaches the model"
    );

    // Exit 2 contributes its stderr as context; a failing exit contributes
    // nothing.
    env.sources.set(document(serde_json::json!({
        "PostToolUse": [
            command(serde_json::json!({"type": "command", "command": blocked.to_str().unwrap()})),
            command(serde_json::json!({"type": "command", "command": broken.to_str().unwrap()})),
        ],
    })));
    turn(
        &session,
        &format!("tool:read {{\"path\":{:?}}}", note.to_str().unwrap()),
    )
    .await;
    let blocks = wait_blocks(&session, "post-tool", 2).await;
    assert!(blocks[1].contains("blocked reason"), "{blocks:?}");
    assert!(!blocks[1].contains("exited with"), "{blocks:?}");
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_tool_use_failure_runs_only_for_error_results() {
    let env = Env::new().await;
    let marker = env.path("failure-marker");
    env.sources.set(document(serde_json::json!({
        "PostToolUseFailure": [command(serde_json::json!({
            "type": "command",
            "command": format!("printf 'failed' > {}", marker.display()),
        }))],
    })));
    let session = env.session().await;
    let note = env.path("work/note.txt");
    std::fs::write(&note, "note\n").unwrap();
    turn(
        &session,
        &format!("tool:read {{\"path\":{:?}}}", note.to_str().unwrap()),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!marker.exists(), "a success runs only PostToolUse");
    turn(
        &session,
        &format!(
            "tool:read {{\"path\":{:?}}}",
            env.path("work/missing.txt").to_str().unwrap()
        ),
    )
    .await;
    for _ in 0..500 {
        if marker.exists() {
            assert_eq!(std::fs::read_to_string(&marker).unwrap(), "failed");
            env.close().await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("PostToolUseFailure did not run");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_timeout_is_a_failure_and_a_blocking_failure_still_answers() {
    let env = Env::new().await;
    let slow = env.path("slow.sh");
    std::fs::write(&slow, "#!/bin/sh\nsleep 5\n").unwrap();
    std::fs::set_permissions(&slow, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    env.sources.set(document(serde_json::json!({
        "PostToolUse": [command(serde_json::json!({
            "type": "command", "command": slow.to_str().unwrap(), "timeout": 1
        }))],
    })));
    let session = env.session().await;
    let started = std::time::Instant::now();
    turn(&session, "tool:read {}").await;
    assert!(started.elapsed() < Duration::from_secs(5));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(hook_blocks(&session, "post-tool").await.is_empty());
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_follow_ups_are_bounded_at_three_and_flag_the_driven_turns() {
    let env = Env::new().await;
    let seen = env.path("stop-payloads");
    env.sources.set(document(serde_json::json!({
        "Stop": [command(serde_json::json!({
            "type": "command",
            "command": format!("{{ cat; echo; }} >> {}; echo again >&2; exit 2", seen.display()),
        }))],
    })));
    let session = env.session().await;
    turn(&session, "go").await;
    let blocks = wait_blocks(&session, "follow-up", 3).await;
    assert_eq!(blocks.len(), 3, "{blocks:?}");
    assert!(blocks[0].contains("again"), "{blocks:?}");
    session.idle("main").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let payloads: Vec<serde_json::Value> = std::fs::read_to_string(&seen)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        hook_blocks(&session, "follow-up").await.len(),
        3,
        "{:?}",
        payloads
            .iter()
            .map(|payload| payload["stop_hook_active"].clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(payloads.len(), 3, "{payloads:?}");
    assert_eq!(
        payloads
            .iter()
            .map(|payload| payload["stop_hook_active"].clone())
            .collect::<Vec<_>>(),
        vec![
            serde_json::json!(false),
            serde_json::json!(true),
            serde_json::json!(true)
        ]
    );
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_async_handler_does_not_delay_the_turn_or_the_next_point() {
    let env = Env::new().await;
    let late = env.path("late");
    let now = env.path("now");
    env.sources.set(document(serde_json::json!({
        "PostToolUse": [
            command(serde_json::json!({
                "type": "command", "async": true,
                "command": format!("{}; printf 'late' > {}", sleep(1500), late.display()),
            })),
            command(serde_json::json!({
                "type": "command",
                "command": format!(
                    "printf 'now' > {}; printf '{{\"hookSpecificOutput\":{{\"additionalContext\":\"context\"}}}}'",
                    now.display()
                ),
            })),
        ],
    })));
    let session = env.session().await;
    let note = env.path("work/note.txt");
    std::fs::write(&note, "note\n").unwrap();
    let started = std::time::Instant::now();
    turn(
        &session,
        &format!("tool:read {{\"path\":{:?}}}", note.to_str().unwrap()),
    )
    .await;
    assert!(started.elapsed() < Duration::from_millis(1200));
    for _ in 0..100 {
        if now.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(std::fs::read_to_string(&now).unwrap(), "now");
    assert!(!late.exists(), "the async handler is still running");
    let blocks = wait_blocks(&session, "post-tool", 1).await;
    assert_eq!(blocks.len(), 1, "the async handler writes no context");
    tokio::time::sleep(Duration::from_millis(2000)).await;
    assert_eq!(std::fs::read_to_string(&late).unwrap(), "late");
    assert_eq!(hook_blocks(&session, "post-tool").await.len(), 1);
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_request_answers_through_approval() {
    let env = Env::new().await;
    env.sources.set(document(serde_json::json!({
        "PermissionRequest": [command(serde_json::json!({
            "type": "command",
            "command": "printf '{\"hookSpecificOutput\":{\"decision\":{\"behavior\":\"allow\"}}}'",
        }))],
    })));
    let session = env.session_with(PermissionPreset::WorkspaceWrite).await;
    turn(&session, "tool:exec {\"command\":\"true\"}").await;
    assert!(
        wait_event(&session, |kind| match kind {
            EventKind::ApprovalRespond(data) => Some(data.decision),
            _ => None,
        })
        .await
            == ApprovalDecision::Allow
    );
    assert_eq!(last_result(&session).await.is_error, None);

    env.sources.set(document(serde_json::json!({
        "PermissionRequest": [command(serde_json::json!({
            "type": "command",
            "command": "printf '{\"hookSpecificOutput\":{\"decision\":{\"behavior\":\"deny\"}}}'",
        }))],
    })));
    turn(&session, "tool:exec {\"command\":\"true\"}").await;
    assert_eq!(last_result(&session).await.is_error, Some(true));
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_user_answer_cancels_the_hook_and_does_not_delay_the_next_point() {
    let env = Env::new().await;
    let late = env.path("late-hook");
    let failure = env.path("failure-marker");
    env.sources.set(document(serde_json::json!({
        "PermissionRequest": [command(serde_json::json!({
            "type": "command",
            "command": format!("{}; printf 'late' > {}", sleep(3000), late.display()),
        }))],
        "PostToolUseFailure": [command(serde_json::json!({
            "type": "command",
            "command": format!("printf 'failed' > {}", failure.display()),
        }))],
    })));
    let session = env.session_with(PermissionPreset::WorkspaceWrite).await;
    session
        .submit(
            "tool:exec {\"command\":\"true\"}".into(),
            InputMode::Queue,
            "main",
        )
        .await
        .unwrap();
    let approval_id = wait_event(&session, |kind| match kind {
        EventKind::ApprovalRequest(data) => Some(data.approval_id.clone()),
        _ => None,
    })
    .await;
    session
        .service::<dim_agent_capabilities::approval::Approval>("main")
        .unwrap()
        .respond(&approval_id, ApprovalDecision::Deny)
        .await
        .unwrap();
    // A cancelled PermissionRequest must not hold the branch's next point:
    // the hook sleeps three seconds and this asks for the failure marker
    // within two.
    for _ in 0..200 {
        if failure.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        failure.exists(),
        "the next hook point waited for the PermissionRequest hook"
    );
    session.idle("main").await.unwrap();
    tokio::time::sleep(Duration::from_millis(3500)).await;
    assert!(
        !late.exists(),
        "the user's answer did not terminate the running hook"
    );
    let approved = session
        .local("main", ..)
        .await
        .unwrap()
        .iter()
        .filter(|event| matches!(event.kind, EventKind::ApprovalRespond(_)))
        .count();
    assert_eq!(approved, 1, "only the user's answer is committed");
    env.close().await;
}

/// F11, the reverse of the test above: an approval's run is a task spawned
/// when the request is folded, so its `PermissionRequest` command starts
/// while an earlier `PostToolUse` hook on the same branch still runs. The
/// slow hook writes its end marker only after the test releases it, so the
/// order needs no timing race.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_permission_request_does_not_wait_for_a_running_post_tool_hook() {
    let env = Env::new().await;
    let order = env.path("order");
    let started = env.path("post-tool-started");
    let release = env.path("post-tool-release");
    env.sources.set(document(serde_json::json!({
        "PostToolUse": [{
            "matcher": "read|Read",
            "hooks": [{
                "type": "command",
                "command": format!(
                    "printf 'started' > {}; while [ ! -f {} ]; do sleep 0.05; done; printf 'post-tool\\n' >> {}",
                    started.display(),
                    release.display(),
                    order.display(),
                ),
            }],
        }],
        "PermissionRequest": [command(serde_json::json!({
            "type": "command",
            "command": format!("printf 'permission\\n' >> {}", order.display()),
        }))],
    })));
    let session = env.session_with(PermissionPreset::WorkspaceWrite).await;
    let note = env.path("work/note.txt");
    std::fs::write(&note, "note\n").unwrap();
    turn(
        &session,
        &format!("tool:read {{\"path\":{:?}}}", note.to_str().unwrap()),
    )
    .await;
    for _ in 0..500 {
        if started.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(started.exists(), "the PostToolUse hook did not start");
    session
        .submit(
            "tool:exec {\"command\":\"true\"}".into(),
            InputMode::Queue,
            "main",
        )
        .await
        .unwrap();
    let approval_id = wait_event(&session, |kind| match kind {
        EventKind::ApprovalRequest(data) => Some(data.approval_id.clone()),
        _ => None,
    })
    .await;
    // The release file does not exist yet, so the slow hook cannot have
    // ended: a permission marker now proves its point did not wait for it.
    for _ in 0..500 {
        if std::fs::read_to_string(&order).is_ok_and(|text| text.contains("permission")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let seen = std::fs::read_to_string(&order).unwrap_or_default();
    assert!(
        seen.contains("permission"),
        "the PermissionRequest hook waited for the running PostToolUse hook: {seen:?}"
    );
    assert!(
        !seen.contains("post-tool"),
        "the PostToolUse hook ended before the permission marker: {seen:?}"
    );
    session
        .service::<dim_agent_capabilities::approval::Approval>("main")
        .unwrap()
        .respond(&approval_id, ApprovalDecision::Deny)
        .await
        .unwrap();
    std::fs::write(&release, "").unwrap();
    for _ in 0..500 {
        if std::fs::read_to_string(&order).is_ok_and(|text| text.contains("post-tool")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        std::fs::read_to_string(&order).unwrap_or_default(),
        "permission\npost-tool\n"
    );
    session.idle("main").await.unwrap();
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notification_runs_on_an_approval_and_writes_nothing() {
    let env = Env::new().await;
    let seen = env.path("notified");
    env.sources.set(document(serde_json::json!({
        "Notification": [command(serde_json::json!({
            "type": "command",
            "command": format!("cat >> {}", seen.display()),
        }))],
    })));
    let session = env.session_with(PermissionPreset::WorkspaceWrite).await;
    session
        .submit(
            "tool:exec {\"command\":\"true\"}".into(),
            InputMode::Queue,
            "main",
        )
        .await
        .unwrap();
    let approval_id = wait_event(&session, |kind| match kind {
        EventKind::ApprovalRequest(data) => Some(data.approval_id.clone()),
        _ => None,
    })
    .await;
    session
        .service::<dim_agent_capabilities::approval::Approval>("main")
        .unwrap()
        .respond(&approval_id, ApprovalDecision::Deny)
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    for _ in 0..500 {
        let bytes = std::fs::read_to_string(&seen).unwrap_or_default();
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(bytes.trim()) {
            assert_eq!(payload["notification_type"], "permission_prompt");
            assert_eq!(payload["hook_event_name"], "Notification");
            assert!(hook_blocks(&session, "post-tool").await.is_empty());
            assert!(hook_blocks(&session, "follow-up").await.is_empty());
            env.close().await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("Notification did not run");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hook_sees_its_environment_and_no_plugin_root() {
    let env = Env::new().await;
    let seen = env.path("environment");
    let stdin = env.path("payload");
    env.sources.set(document(serde_json::json!({
        "SessionStart": [command(serde_json::json!({
            "type": "command",
            "command": format!("env > {}; cat > {}; printf 'context'", seen.display(), stdin.display()),
        }))],
    })));
    let session = env.session().await;
    wait_blocks(&session, "session-start", 1).await;
    for _ in 0..500 {
        if stdin.exists() && !std::fs::read_to_string(&stdin).unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let environment = std::fs::read_to_string(&seen).unwrap();
    let line = |name: &str| {
        environment
            .lines()
            .find(|line| line.starts_with(&format!("{name}=")))
            .map(str::to_owned)
    };
    assert_eq!(
        line("DIM_SESSION_ID"),
        Some(format!("DIM_SESSION_ID={}", session.id()))
    );
    assert_eq!(
        line("DIM_PROJECT_DIR"),
        Some(format!(
            "DIM_PROJECT_DIR={}",
            env.dir.join("work").display()
        ))
    );
    assert_eq!(
        line("DIM_HOOK_EVENT"),
        Some("DIM_HOOK_EVENT=SessionStart".into())
    );
    assert_eq!(
        line("DIMCODE_HOME"),
        Some(format!("DIMCODE_HOME={}", env.dir.join("home").display()))
    );
    assert_eq!(line("CLAUDE_PLUGIN_ROOT"), None);
    let payload: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&stdin).unwrap().trim()).unwrap();
    assert_eq!(payload["hook_event_name"], "SessionStart");
    assert_eq!(payload["source"], "startup");
    assert_eq!(payload["dim_source"], "user");
    assert_eq!(payload["permission_mode"], "full-access");
    assert_eq!(payload["branch_id"], "main");
    assert_eq!(payload["session_id"], session.id());
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identical_commands_at_one_point_run_once() {
    let env = Env::new().await;
    let runs = env.path("runs");
    let repeat = format!(
        "{}; printf '{{\"hookSpecificOutput\":{{\"additionalContext\":\"context\"}}}}'",
        marker_command(&runs, "run\n")
    );
    env.sources.set(document(serde_json::json!({
        "PostToolUse": [
            command(serde_json::json!({"type": "command", "command": repeat})),
            command(serde_json::json!({"type": "command", "command": repeat})),
        ],
    })));
    let session = env.session().await;
    let note = env.path("work/note.txt");
    std::fs::write(&note, "note\n").unwrap();
    turn(
        &session,
        &format!("tool:read {{\"path\":{:?}}}", note.to_str().unwrap()),
    )
    .await;
    wait_blocks(&session, "post-tool", 1).await;
    assert_eq!(std::fs::read_to_string(&runs).unwrap(), "run\n");
    assert_eq!(hook_blocks(&session, "post-tool").await.len(), 1);
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_output_is_dropped_after_an_interrupt() {
    let env = Env::new().await;
    env.sources.set(document(serde_json::json!({
        "PostToolUse": [command(serde_json::json!({
            "type": "command",
            "command": format!("{}; printf 'late context'", sleep(400)),
        }))],
    })));
    let session = env.session().await;
    let note = env.path("work/note.txt");
    std::fs::write(&note, "note\n").unwrap();
    session
        .submit(
            format!("tool:read {{\"path\":{:?}}}", note.to_str().unwrap()),
            InputMode::Queue,
            "main",
        )
        .await
        .unwrap();
    wait_event(&session, |kind| match kind {
        EventKind::ToolResult(_) => Some(()),
        _ => None,
    })
    .await;
    session.interrupt("main", "stop".into()).unwrap();
    let _ = session.idle("main").await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        hook_blocks(&session, "post-tool").await.is_empty(),
        "an interrupt drops the output"
    );
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_session_start_does_not_run_the_hooks_again() {
    let env = Env::new().await;
    let runs = env.path("starts");
    env.sources.set(document(serde_json::json!({
        "SessionStart": [command(serde_json::json!({
            "type": "command",
            "command": format!("{}; printf 'context'", marker_command(&runs, "start\n")),
        }))],
    })));
    let session = env.session().await;
    wait_blocks(&session, "session-start", 1).await;
    let id = session.id().to_owned();
    session.close().await.unwrap();
    let restored = env.agent.restore_session(id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(std::fs::read_to_string(&runs).unwrap(), "start\n");
    assert_eq!(hook_blocks(&restored, "session-start").await.len(), 1);
    env.close().await;
}

async fn last_result(session: &Session) -> ToolResult {
    session
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ToolResult(data) => Some(data.results[0].result.clone()),
            _ => None,
        })
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_session_start_block_is_still_in_context_after_compaction() {
    let env = Env::with(
        Arc::new(Model {
            exhaust: true,
            ..Model::default()
        }),
        Some(Arc::new(Small)),
    )
    .await;
    env.sources.set(document(serde_json::json!({
        "SessionStart": [command(serde_json::json!({
            "type": "command", "command": "printf 'start context'",
        }))],
    })));
    let session = env.session().await;
    let blocks = wait_blocks(&session, "session-start", 1).await;
    assert!(blocks[0].contains("start context"), "{blocks:?}");
    session
        .submit("x".repeat(4_000), InputMode::Queue, "main")
        .await
        .unwrap();
    let _ = session.idle("main").await;
    assert!(
        session
            .history("main", None)
            .await
            .unwrap()
            .iter()
            .any(|event| matches!(event.kind, EventKind::ContextCompaction(_))),
        "the turn compacted"
    );
    // The retry after the recovery compaction is the request the model saw.
    let requests = env.model.requests();
    assert_eq!(requests.len(), 1);
    let text = serde_json::to_string(&requests[0].messages).unwrap();
    assert!(text.contains("start context"), "{text}");
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_project_source_runs_nothing_until_the_user_enables_it() {
    let env = Env::new().await;
    let runs = env.path("project-runs");
    let project = document(serde_json::json!({
        "SessionStart": [command(serde_json::json!({
            "type": "command",
            "command": format!(
                "{}; printf 'project context'",
                marker_command(&runs, "run\n")
            ),
        }))],
    }));
    env.sources.set_project(project.clone(), false);
    let session = env.session().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!runs.exists(), "a disabled project source runs nothing");
    assert!(hook_blocks(&session, "session-start").await.is_empty());

    env.sources.set_project(project, true);
    let session = env.session().await;
    let blocks = wait_blocks(&session, "session-start", 1).await;
    assert!(blocks[0].contains("project context"), "{blocks:?}");
    assert_eq!(std::fs::read_to_string(&runs).unwrap(), "run\n");
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disable_all_hooks_turns_every_source_off() {
    let env = Env::new().await;
    let runs = env.path("all-off");
    let handler = serde_json::json!({
        "type": "command",
        "command": format!("{}; printf 'context'", marker_command(&runs, "run\n")),
    });
    env.sources.set(
        serde_json::from_value(serde_json::json!({
            "disableAllHooks": true,
            "hooks": {"SessionStart": [{"hooks": [handler]}]},
        }))
        .unwrap(),
    );
    let session = env.session().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!runs.exists());
    assert!(hook_blocks(&session, "session-start").await.is_empty());
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_source_runs_like_any_other_source() {
    let env = Env::new().await;
    let plugin = env.path("plugin");
    std::fs::create_dir_all(&plugin).unwrap();
    let seen = env.path("plugin-environment");
    let payload_file = env.path("plugin-payload");
    let user_seen = env.path("user-environment");
    env.sources.set(document(serde_json::json!({
        "SessionStart": [
            command(serde_json::json!({
                "type": "command",
                "command": format!("printf 'user context'; env > {}", user_seen.display()),
            })),
            command(serde_json::json!({"type": "command", "command": "printf 'shared'"})),
        ],
    })));
    env.sources.set_plugin(
        "kit",
        &plugin,
        document(serde_json::json!({
            "SessionStart": [
                command(serde_json::json!({
                    "type": "command",
                    "command": format!(
                        "printf 'plugin context'; env > {}; cat > {}",
                        seen.display(),
                        payload_file.display()
                    ),
                })),
                command(serde_json::json!({"type": "command", "command": "printf 'shared'"})),
            ],
        })),
    );
    let session = env.session().await;
    let blocks = wait_blocks(&session, "session-start", 3).await;
    assert_eq!(blocks.len(), 3, "{blocks:?}");
    assert!(blocks[0].contains("user context"), "{blocks:?}");
    assert!(blocks[1].contains("shared"), "{blocks:?}");
    assert!(blocks[2].contains("plugin context"), "{blocks:?}");

    // The plugin's command reads its directory; the user's reads none.
    let environment = std::fs::read_to_string(&seen).unwrap();
    assert!(
        environment
            .lines()
            .any(|line| line == format!("DIM_PLUGIN_ROOT={}", plugin.display())),
        "{environment}"
    );
    assert!(
        !environment
            .lines()
            .any(|line| line.starts_with("CLAUDE_PLUGIN_ROOT=")),
        "{environment}"
    );
    let user_environment = std::fs::read_to_string(&user_seen).unwrap();
    assert!(
        !user_environment
            .lines()
            .any(|line| line.starts_with("DIM_PLUGIN_ROOT=")),
        "{user_environment}"
    );
    let payload: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&payload_file).unwrap().trim()).unwrap();
    assert_eq!(payload["dim_source"], "plugin:kit");

    // `disableAllHooks` in the user document turns the plugin source off too.
    let mut off = document(serde_json::json!({
        "SessionStart": [command(serde_json::json!({
            "type": "command", "command": "printf 'user context'"
        }))],
    }));
    off.disable_all_hooks = true;
    env.sources.set(off);
    let second = env.session().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(hook_blocks(&second, "session-start").await.is_empty());
    env.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_follow_ups_turn_reports_stop_hook_active() {
    let env = Env::new().await;
    let seen = env.path("payloads");
    // Blocks once, then lets the driven turn end: the case reads the compact
    // payload the hook receives on stdin.
    let script = format!(
        "sleep 0.3; payload=$(cat); printf '%s\\n' \"$payload\" >> {seen}; \
         case \"$payload\" in *'\"stop_hook_active\":false'*) printf 'again' >&2; exit 2;; esac",
        seen = seen.display()
    );
    env.sources.set(document(serde_json::json!({
        "Stop": [command(serde_json::json!({"type": "command", "command": script}))],
    })));
    let session = env.session().await;
    session
        .submit("go".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    // A second producer writes its own input right after the first turn ends,
    // before the Stop hook has finished. It is not a follow-up.
    wait_event(&session, |kind| match kind {
        EventKind::TurnEnd(_) => Some(()),
        _ => None,
    })
    .await;
    session
        .input(
            InputMessageData {
                blocks: vec![dim_agent_capabilities::context_block::raw(
                    "<other/>".into(),
                    None,
                )],
                mode: Some(InputMode::Queue),
                ..InputMessageData::default()
            },
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    // Three turns run; each Stop hook sleeps before it records.
    let payloads = || -> Vec<serde_json::Value> {
        std::fs::read_to_string(&seen)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    for _ in 0..500 {
        if payloads().len() >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let payloads = payloads();
    let active: Vec<bool> = payloads
        .iter()
        .map(|payload| payload["stop_hook_active"].as_bool().unwrap())
        .collect();
    // The operator's turn and the other producer's turn are not follow-ups;
    // every turn a Hooks follow-up drives is.
    assert!(active.len() >= 3, "{payloads:?}");
    assert_eq!(active[..2], [false, false], "{payloads:?}");
    assert!(active[2..].iter().all(|active| *active), "{payloads:?}");
    env.close().await;
}
