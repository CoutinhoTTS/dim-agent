//! Shared fixture for PromptSources and Skills flows: temporary home and cwd,
//! process services on a host root, a scripted model and a gated tool.
#![allow(dead_code)]

use dim_agent_capabilities::{
    defaults,
    history::HistoryRenderer,
    host::*,
    host_platform::{
        ExecShellRuntime, HostPlatform, Os, OutputEncoding, ShellEdition, ShellSource, ShellStatus,
    },
    model_provider::ModelProviderPlugin,
    permission::{CommandRulesHost, NoCommandRules},
    prompt_sources,
    skill_settings::{SkillSettings, SkillSettingsHost},
    skills::{self, MemorySkillSettings, SkillsCatalog, SkillsCatalogPlugin},
    timeline_tree::TreeCommands,
};
use dim_agent_runtime::*;
use dim_agent_types::*;
use rutis::TypeKey;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// A temporary directory tree removed on drop.
pub struct Dirs {
    pub root: PathBuf,
    pub home: PathBuf,
}
impl Dirs {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "dim-local-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Self { root, home }
    }
    /// Creates `rel` under the root and returns its path.
    pub fn dir(&self, rel: &str) -> PathBuf {
        let path = self.root.join(rel);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    pub fn write(&self, path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}
impl Drop for Dirs {
    fn drop(&mut self) {
        #[cfg(unix)]
        restore_permissions(&self.root);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[cfg(unix)]
fn restore_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                restore_permissions(&path);
            } else {
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
            }
        }
    }
}

pub fn skill_file(name: &str, description: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n{body}\n")
}

pub fn platform(home: &Path) -> HostPlatform {
    let mut platform = HostPlatform::current(ShellStatus::Available(ExecShellRuntime {
        executable_path: "/bin/zsh".into(),
        edition: ShellEdition::Zsh,
        source: ShellSource::Login,
        output_encoding: OutputEncoding::Utf8,
    }))
    .unwrap();
    platform.home = home.to_owned();
    platform.os = Os::Linux;
    platform.arch = "x64".into();
    platform.timezone = Some("Asia/Shanghai".into());
    platform.env.clear();
    platform
}

pub enum Step {
    Tool(&'static str, serde_json::Value),
    Text(&'static str),
}

/// Replies with scripted steps, then with plain text.
#[derive(Default)]
pub struct ScriptedModel(Mutex<VecDeque<Step>>);
impl ScriptedModel {
    pub fn push(&self, step: Step) {
        self.0.lock().unwrap().push_back(step);
    }
}
impl ModelDriver for ScriptedModel {
    fn invoke(
        &self,
        _: Arc<ModelRequest>,
        _: ModelConnection,
        _: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        let step = self.0.lock().unwrap().pop_front();
        Box::pin(async move {
            let id = "message".to_owned();
            let (body, reason) = match step {
                Some(Step::Tool(name, input)) => {
                    let call = ToolCall {
                        id: format!("call-{}", uuid_like()),
                        name: name.into(),
                        input: input.as_object().unwrap().clone(),
                    };
                    let start = ModelChunk::ToolStart {
                        id: call.id.clone(),
                        name: call.name.clone(),
                    };
                    (
                        vec![start, ModelChunk::ToolUse(call)],
                        ModelStopReason::ToolUse,
                    )
                }
                Some(Step::Text(text)) => (
                    vec![ModelChunk::Text(text.into()), ModelChunk::TextEnd],
                    ModelStopReason::EndTurn,
                ),
                None => (
                    vec![ModelChunk::Text("ok".into()), ModelChunk::TextEnd],
                    ModelStopReason::EndTurn,
                ),
            };
            let mut chunks = vec![Ok(ModelChunk::MessageStart {
                message_id: id.clone(),
            })];
            chunks.extend(body.into_iter().map(Ok));
            chunks.push(Ok(ModelChunk::MessageStop {
                message_id: id,
                reason,
            }));
            Ok(Box::pin(futures_util::stream::iter(chunks)) as ModelStream)
        })
    }
}
fn uuid_like() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    NEXT.fetch_add(1, Ordering::SeqCst)
}
/// `gate` blocks until released, giving tests a window inside a turn.
#[derive(Default)]
pub struct Gate {
    pub entered: Notify,
    pub release: Notify,
}
pub struct GateTools(pub Arc<Gate>);
impl ToolDriver for GateTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        let mut gate = dim_agent_capabilities::tool_registry::definition(
            "gate",
            "Wait until the test releases it.",
            serde_json::json!({"type":"object"}),
        );
        // Only blocks inside the turn; allowed under the default read-only policy.
        gate.risk = Some("session".into());
        vec![gate]
    }
    fn execute(
        &self,
        _: ToolCall,
        _: ToolExecutionContext,
        _: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            self.0.entered.notify_one();
            self.0.release.notified().await;
            Ok(ToolResult::text("gated"))
        })
    }
}

/// Holds the next Session's assembly open until released, while its other
/// capabilities load concurrently: a window before the Session is published.
/// A flow that needs another capability applied waits for it separately.
#[derive(Default)]
pub struct AssemblyHold {
    armed: std::sync::atomic::AtomicBool,
    pub entered: Notify,
    pub release: Notify,
}
impl AssemblyHold {
    pub fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
}
struct AssemblyHoldPlugin(Arc<AssemblyHold>);
impl rutis::TypedPlugin for AssemblyHoldPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "AssemblyHold"
    }
    fn apply<'a>(
        &'a self,
        _: &'a rutis::Ctx,
        (): (),
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            if self.0.armed.swap(false, Ordering::SeqCst) {
                self.0.entered.notify_one();
                self.0.release.notified().await;
            }
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for AssemblyHoldPlugin {
    fn keys(&self) -> KeysOf<Self> {}
}

/// Records what each Session's `SessionLifecycle` reports, and can hold one
/// Session's unload to keep a close in progress.
#[derive(Default)]
pub struct LifecycleProbe {
    pub events: Mutex<Vec<(String, &'static str)>>,
    pub recorded: Notify,
    lifecycles: Mutex<std::collections::HashMap<String, Arc<SessionLifecycle>>>,
    trees: Mutex<std::collections::HashMap<String, Arc<TreeCommands>>>,
    hold_close: std::sync::atomic::AtomicBool,
    pub close_held: Notify,
    pub release_close: Notify,
}
impl LifecycleProbe {
    pub fn hold_next_close(&self) {
        self.hold_close.store(true, Ordering::SeqCst);
    }
    /// The `SessionLifecycle` a part of `session` injects, as an owner of
    /// detached work uses it.
    pub fn lifecycle(&self, session: &str) -> Arc<SessionLifecycle> {
        self.lifecycles.lock().unwrap()[session].clone()
    }
    /// The branch tree commands a part of `session` injects, which reach each
    /// published branch's own operations.
    pub fn tree(&self, session: &str) -> Arc<TreeCommands> {
        self.trees.lock().unwrap()[session].clone()
    }
    pub fn count(&self, event: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, recorded)| *recorded == event)
            .count()
    }
    pub async fn wait_for(&self, event: &str, count: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let recorded = self.recorded.notified();
                if self.count(event) >= count {
                    return;
                }
                recorded.await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{count} {event} events: {:?}", self.events.lock().unwrap()));
    }
    fn record(&self, session: &str, event: &'static str) {
        self.events.lock().unwrap().push((session.into(), event));
        self.recorded.notify_waiters();
    }
}
struct LifecycleProbePlugin {
    scope: SessionScope,
    probe: Arc<LifecycleProbe>,
}
impl rutis::TypedPlugin for LifecycleProbePlugin {
    type Deps = (
        InSession<SessionLifecycle>,
        InSession<SessionIdentity>,
        InSession<TreeCommands>,
    );
    fn name(&self) -> &str {
        "LifecycleProbe"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (InSession(lifecycle), InSession(identity), InSession(tree)): Self::Deps,
    ) -> rutis::BoxFuture<'a, std::result::Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            let probe = self.probe.clone();
            let id = identity.session_id.clone();
            probe
                .lifecycles
                .lock()
                .unwrap()
                .insert(id.clone(), lifecycle.clone());
            probe.trees.lock().unwrap().insert(id.clone(), tree);
            // Session parts load concurrently: a flow that closes during
            // assembly waits for this record first.
            probe.record(&id, "applied");
            let task = tokio::spawn(async move {
                let published = lifecycle.activated().await;
                probe.record(&id, if published { "active" } else { "inactive" });
                if published {
                    lifecycle.closing().await;
                    probe.record(&id, "closing");
                }
            });
            let probe = self.probe.clone();
            ctx.effect(move || {
                rutis::Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        if probe.hold_close.swap(false, Ordering::SeqCst) {
                            probe.close_held.notify_one();
                            probe.release_close.notified().await;
                        }
                        // Like a Session part's own work, the task ends once the
                        // Session is closing or never published; aborting it
                        // could drop that record.
                        let _ = task.await;
                        Ok(())
                    })
                }))
            })?;
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for LifecycleProbePlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

pub struct Env {
    pub root: rutis::Ctx,
    pub agent: Agent,
    pub model: Arc<ScriptedModel>,
    pub gate: Arc<Gate>,
    pub today: Arc<Mutex<String>>,
    pub settings: Arc<dyn SkillSettings>,
}
impl Env {
    pub fn catalog(&self) -> Arc<SkillsCatalog> {
        self.root.get::<SkillsCatalog>().unwrap()
    }
    pub async fn session(&self, cwd: &Path, system_prompt: &str) -> Result<Session> {
        self.agent
            .create_session(SessionConfig {
                cwd: cwd.to_string_lossy().into_owned(),
                system_prompt: system_prompt.into(),
                tools: vec!["skill".into(), "gate".into()],
                ..SessionConfig::default()
            })
            .await
    }
}

pub struct Options {
    pub store: Option<Arc<MemoryTimelineStore>>,
    pub settings: Arc<dyn SkillSettings>,
    pub today: Arc<Mutex<String>>,
    pub timezone: &'static str,
    pub hold: Option<Arc<AssemblyHold>>,
    pub probe: Option<Arc<LifecycleProbe>>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            store: None,
            settings: Arc::new(MemorySkillSettings::default()),
            today: Arc::new(Mutex::new("2026-09-30".into())),
            timezone: "Asia/Shanghai",
            hold: None,
            probe: None,
        }
    }
}

/// The product profile: PromptSources and Skills with scripted hosts.
pub async fn env(dirs: &Dirs, options: Options) -> Env {
    let root = rutis::Ctx::root_with(tokio::runtime::Handle::current());
    let mut host = platform(&dirs.home);
    host.timezone = Some(options.timezone.into());
    root.provide_as(TypeKey::of::<HostPlatform>(), Arc::new(host))
        .unwrap();
    root.provide_as(
        TypeKey::of::<SkillSettingsHost>(),
        Arc::new(SkillSettingsHost(options.settings.clone())),
    )
    .unwrap();
    (&root.plugin(
        dim_agent_runtime::names::process_plugin(
            &dim_agent_capabilities::catalog(),
            SkillsCatalogPlugin::new(),
        )
        .unwrap(),
    ))
        .await
        .unwrap();
    let model = Arc::new(ScriptedModel::default());
    let gate = Arc::new(Gate::default());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    if let Some(store) = options.store {
        builder.timeline_store(store);
    }
    builder
        .session_value(Arc::new(HistoryRenderer::default()))
        .session_value(Arc::new(ModelHost(model.clone())))
        .session_value(Arc::new(ToolHost(Arc::new(GateTools(gate.clone())))))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_value(Arc::new(CommandRulesHost(Arc::new(NoCommandRules))))
        .session_value(Arc::new(CompactionStrategyHost(None)))
        .session_plugin(dim_agent_capabilities::adapters::DefaultPrepareModelPlugin::new);
    defaults::install_with_prompt_capability(&mut builder, ModelProviderPlugin::new);
    let today = options.today.clone();
    prompt_sources::install_with_clock(
        &mut builder,
        Arc::new(move || today.lock().unwrap().clone()),
        std::time::Duration::from_millis(10),
    );
    skills::install(&mut builder);
    if let Some(hold) = options.hold {
        builder.session_plugin(move |_| AssemblyHoldPlugin(hold.clone()));
    }
    if let Some(probe) = options.probe {
        builder.session_plugin(move |scope| LifecycleProbePlugin {
            scope: *scope,
            probe: probe.clone(),
        });
    }
    Env {
        agent: builder.build_in(&root),
        root,
        model,
        gate,
        today: options.today,
        settings: options.settings,
    }
}

pub fn system_text(prompt: &SystemPrompt) -> String {
    match prompt {
        SystemPrompt::Text(text) => text.clone(),
        SystemPrompt::Parts(parts) => parts
            .iter()
            .map(|part| match part {
                SystemPromptPart::Text { text, .. } => text.as_str(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Every model request of `branch`, oldest first.
pub async fn requests(session: &Session) -> Vec<Arc<ModelRequest>> {
    session
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(data.request.clone()),
            _ => None,
        })
        .collect()
}

pub fn request_text(request: &ModelRequest) -> String {
    request
        .messages
        .iter()
        .map(|message| match &message.content {
            MessageContent::Text(text) => text.clone(),
            MessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::Text { text, .. } => Some(text.clone()),
                    MessagePart::ToolResult { content, .. } => match content {
                        ToolContent::Text(text) => Some(text.clone()),
                        ToolContent::Blocks(_) => None,
                    },
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The input messages of `main`, oldest first.
pub async fn inputs(session: &Session) -> Vec<InputMessageData> {
    session
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::InputMessage(data) => Some(data.clone()),
            _ => None,
        })
        .collect()
}

/// The `dim-state` value an input delivers for `source`, unescaped;
/// `Some(None)` clears it.
pub fn state_value(input: &InputMessageData, source: &str) -> Option<Option<String>> {
    let open = format!("<dim-state source=\"{source}\"");
    input.blocks.iter().find_map(|block| {
        let InputBlock::Raw { text, retire, .. } = block;
        let rest = text.strip_prefix(&open)?;
        if *retire {
            return Some(None);
        }
        let body = rest.strip_prefix('>')?.strip_suffix("</dim-state>")?;
        Some(Some(
            body.replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&"),
        ))
    })
}

/// Tool results of `main`, oldest first.
pub async fn tool_results(session: &Session) -> Vec<ToolResult> {
    session
        .history("main", None)
        .await
        .unwrap()
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ToolResult(data) => Some(
                data.results
                    .iter()
                    .map(|item| item.result.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect()
}

pub fn result_text(result: &ToolResult) -> String {
    match &result.content {
        ToolContent::Text(text) => text.clone(),
        ToolContent::Blocks(_) => String::new(),
    }
}

pub async fn run(session: &Session, text: &str) {
    session
        .submit(text.into(), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
}
