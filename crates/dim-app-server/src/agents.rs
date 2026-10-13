use crate::{
    ServerConfig,
    agent_records::AgentRecords,
    protocol::AgentInfo,
    settings::SettingsService,
    storage::{SessionStore, SqliteStorage},
    trace::Traces,
};
use dim_agent_runtime::{
    AGENT_GROUP, Agent, AgentBuilder, AgentError, Capability, KeysOf, NoConfig, Session,
    SessionConfig, TimelineKey, When, capability_plugin,
};
pub use dim_agent_types::AgentProfile;
use dim_agent_types::{ContentBlock, EventKind};
use rutis::{
    BoxFuture, CordisError, Ctx, Effect, Event, EventKey, FiberState, FiberView, TypeKey,
    TypedFactory, TypedPlugin, TypedPluginFactory,
};
use rutis_loader::{Builtins, InstanceResult, Loader};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// The process default profile carries the process Session defaults.
fn process_default(defaults: &SessionConfig) -> AgentProfile {
    AgentProfile {
        name: "default".into(),
        description: String::new(),
        system_prompt: defaults.system_prompt.clone(),
        tools: defaults.tools.clone(),
        model: None,
    }
}
/// Session configuration of an Agent before explicit request values.
pub(crate) fn session_config(profile: &AgentProfile, defaults: &SessionConfig) -> SessionConfig {
    let mut config = defaults.clone();
    config.system_prompt = profile.system_prompt.clone();
    config.tools = profile.tools.clone();
    if let Some(model) = &profile.model {
        config.model = dim_agent_types::ModelSelection {
            provider_id: model.provider_id.clone(),
            model_id: model.model_id.clone(),
            capability_snapshot: None,
            config: None,
            metadata: None,
        };
    }
    config
}

/// One protocol instance: the Agent, its Sessions, their trace archives.
/// Agents are peers: hosting knows no relationship between them.
pub struct AgentHost {
    pub agent: Agent,
    pub agent_id: String,
    pub default: bool,
    pub profile: AgentProfile,
    /// Host of the Agent's Session listeners. It unloads after the Agent scope
    /// has closed every Session and drained archives.
    pub(crate) ctx: Ctx,
    pub(crate) traces: Traces,
}

/// A new loaded incarnation of a stored Session, before its caller receives it.
pub(crate) struct SessionRestored {
    pub agent_id: String,
    pub session: Session,
}
impl Event for SessionRestored {
    const NAME: &'static str = "agent/session-restored";
    type Value = ();
}

/// Agent-scoped operations fail before any Session is involved.
#[derive(Debug, thiserror::Error)]
pub enum AgentOpError {
    #[error("unknown agent: {0}")]
    Unknown(String),
    #[error("agent initialization failed: {0}")]
    Build(#[from] AgentError),
    #[error("agent settlement failed: {0}")]
    Settle(AgentError),
    #[error("agent unload failed: {0}")]
    Unload(Arc<CordisError>),
}

#[derive(Default)]
struct SessionSlot {
    gate: tokio::sync::Mutex<()>,
    uses: AtomicUsize,
    closed: AtomicBool,
}
/// An accepted request or subscription pins one loaded instance.
pub(crate) struct SessionUse {
    pub host: Arc<AgentHost>,
    pub session: Session,
    slot: Arc<SessionSlot>,
}
impl Clone for SessionUse {
    fn clone(&self) -> Self {
        self.slot.uses.fetch_add(1, Ordering::AcqRel);
        Self {
            host: self.host.clone(),
            session: self.session.clone(),
            slot: self.slot.clone(),
        }
    }
}
impl Drop for SessionUse {
    fn drop(&mut self) {
        self.slot.uses.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Entry {
    host: Arc<AgentHost>,
    view: FiberView,
}
#[derive(Default)]
struct Inner {
    agents: BTreeMap<String, Entry>,
    sessions: HashMap<String, Arc<SessionSlot>>,
    unavailable_agents: HashMap<String, String>,
    unavailable_sessions: HashMap<String, String>,
    /// Registered by an unloading scope apply; completed by its loader.
    pending: HashMap<String, Arc<AgentHost>>,
    /// Agents removed from the live index whose settlement still runs.
    closing: HashMap<String, Arc<AgentHost>>,
    /// Session id -> owning agent id and negotiated protocol version.
    versions: HashMap<String, (String, u32)>,
    /// Loader instances of the Agents this index created, until they close.
    /// They outlive the index's entries: an unloading scope forgets its
    /// entry, and the index closes the instance when it unloads.
    instances: HashMap<String, FiberView>,
}

/// Process service routing every Agent: creation, addressing and close.
/// Relationships between Agents are not hosting concerns; whoever needs them
/// (a subagent capability, a workflow) tracks its own edges and policies.
/// The process's Agents and the state kept for their Sessions. It has no
/// dependencies and is not restartable: Agents register here, so they and
/// this state outlive a reload of the `Agents` directory logic.
pub(crate) struct AgentDirectory {
    inner: Mutex<Inner>,
    default_agent_id: OnceLock<String>,
    /// The protocol instance: it stays while the Agents and their Sessions
    /// do, so clients of an `Agents` reload keep their Sessions.
    server_instance_id: String,
}
impl AgentDirectory {
    /// Registration of a scope that finished apply. A first load completes
    /// the entry with the instance it created; a scope that applies again
    /// in an existing instance enters directly.
    fn register(&self, host: Arc<AgentHost>) {
        let mut inner = self.inner.lock().expect("agent index");
        match inner.instances.get(&host.agent_id).cloned() {
            Some(view) if !inner.pending.contains_key(&host.agent_id) => {
                inner
                    .agents
                    .insert(host.agent_id.clone(), Entry { host, view });
            }
            _ => {
                inner.pending.insert(host.agent_id.clone(), host);
            }
        }
    }
    /// Drops an Agent and its Session protocol versions; idempotent, so
    /// explicit close and scope unload agree.
    fn forget(&self, agent_id: &str) {
        let mut inner = self.inner.lock().expect("agent index");
        inner.agents.remove(agent_id);
        inner.pending.remove(agent_id);
        inner.versions.retain(|_, (owner, _)| owner != agent_id);
    }
}
/// Provides the [`AgentDirectory`].
pub(crate) struct AgentDirectoryPlugin;
impl TypedPlugin for AgentDirectoryPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "AgentDirectory"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as(
                TypeKey::of::<AgentDirectory>(),
                Arc::new(AgentDirectory {
                    inner: Mutex::new(Inner::default()),
                    default_agent_id: OnceLock::new(),
                    server_instance_id: uuid::Uuid::new_v4().to_string(),
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for AgentDirectoryPlugin {
    fn keys(&self) -> KeysOf<Self> {}
}

/// Directory logic and Agent management over the [`AgentDirectory`]:
/// lookups, Session admission and creating and closing Agents through the
/// loader. A reload of it is a reload of the directory logic only.
pub struct AgentIndex {
    directory: Arc<AgentDirectory>,
    pub(crate) session_restored: EventKey<SessionRestored>,
    builder: AgentBuilder,
    session_defaults: SessionConfig,
    /// Tools the process assembly provides: host tools and contributions.
    tools: BTreeSet<String>,
    /// Loading context of `AgentsPlugin`.
    ctx: OnceLock<Ctx>,
    /// The process loader Agents are instances of, and the root they are
    /// created from.
    loader: OnceLock<(Loader, Ctx)>,
    startup_timeout: Duration,
    records: Option<Arc<AgentRecords>>,
    session_store: Option<Arc<SessionStore>>,
    settings: Option<Arc<SettingsService>>,
    stopping: CancellationToken,
}
impl AgentIndex {
    const SUMMARY_SCAN_LIMIT: usize = 256;
    const SUMMARY_PREVIEW_CHARS: usize = 120;
    fn new(
        directory: Arc<AgentDirectory>,
        builder: AgentBuilder,
        config: &ServerConfig,
        records: Option<Arc<AgentRecords>>,
        session_store: Option<Arc<SessionStore>>,
        settings: Option<Arc<SettingsService>>,
        process: rutis::InstanceId,
    ) -> Self {
        Self {
            directory,
            session_restored: EventKey::of().instance(process),
            tools: builder.provided_tools().map(str::to_owned).collect(),
            builder,
            session_defaults: config.session_defaults.clone(),
            ctx: OnceLock::new(),
            loader: OnceLock::new(),
            startup_timeout: config.startup_timeout,
            records,
            session_store,
            settings,
            stopping: CancellationToken::new(),
        }
    }
    /// Process Session defaults that Agent profiles apply on top of.
    pub fn session_defaults(&self) -> &SessionConfig {
        &self.session_defaults
    }
    pub fn server_instance_id(&self) -> &str {
        &self.directory.server_instance_id
    }
    pub fn default_agent_id(&self) -> &str {
        self.directory
            .default_agent_id
            .get()
            .expect("AgentsPlugin apply fixed the default id")
    }
    pub fn default_host(&self) -> Option<Arc<AgentHost>> {
        self.host(Some(self.default_agent_id()))
    }
    pub fn hosts(&self) -> Vec<Arc<AgentHost>> {
        self.directory
            .inner
            .lock()
            .expect("agent index")
            .agents
            .values()
            .map(|entry| entry.host.clone())
            .collect()
    }
    pub fn host(&self, agent_id: Option<&str>) -> Option<Arc<AgentHost>> {
        let id =
            agent_id.or_else(|| self.directory.default_agent_id.get().map(|id| id.as_str()))?;
        self.directory
            .inner
            .lock()
            .expect("agent index")
            .agents
            .get(id)
            .map(|entry| entry.host.clone())
    }
    pub(crate) fn unavailable_agent(&self, id: &str) -> Option<String> {
        self.directory
            .inner
            .lock()
            .expect("agent index")
            .unavailable_agents
            .get(id)
            .cloned()
    }
    pub(crate) fn unavailable_session(&self, id: &str) -> Option<String> {
        self.directory
            .inner
            .lock()
            .expect("agent index")
            .unavailable_sessions
            .get(id)
            .cloned()
    }
    /// Session ids are UUIDs, unique across every Agent of the process.
    pub fn session(&self, session_id: &str) -> Option<(Arc<AgentHost>, Session)> {
        let inner = self.directory.inner.lock().expect("agent index");
        for entry in inner.agents.values() {
            if let Some(session) = entry.host.agent.get_session(session_id) {
                return Some((entry.host.clone(), session));
            }
        }
        None
    }
    /// The Agent owning a Session, also after the Session itself closed and
    /// while its Agent is closing; its archives may still be finishing.
    pub fn host_by_session(&self, session_id: &str) -> Option<Arc<AgentHost>> {
        let inner = self.directory.inner.lock().expect("agent index");
        if let Some((agent, _)) = inner.versions.get(session_id) {
            if let Some(entry) = inner.agents.get(agent) {
                return Some(entry.host.clone());
            }
            if let Some(host) = inner.closing.get(agent) {
                return Some(host.clone());
            }
        }
        inner
            .agents
            .values()
            .find(|entry| entry.host.agent.get_session(session_id).is_some())
            .map(|entry| entry.host.clone())
    }
    pub fn version_of(&self, session_id: &str) -> Option<u32> {
        self.directory
            .inner
            .lock()
            .expect("agent index")
            .versions
            .get(session_id)
            .map(|(_, version)| *version)
    }
    pub fn insert_version(&self, session_id: &str, agent_id: &str, version: u32) {
        self.directory
            .inner
            .lock()
            .expect("agent index")
            .versions
            .insert(session_id.to_owned(), (agent_id.to_owned(), version));
    }
    pub fn remove_version(&self, session_id: &str) {
        self.slot(session_id).closed.store(true, Ordering::Release);
        if self.session_store.is_none() {
            let mut inner = self.directory.inner.lock().unwrap();
            inner.versions.remove(session_id);
            inner.sessions.remove(session_id);
        }
    }
    /// Sessions can also close through the runtime fault supervisor.
    /// Versions of a closing Agent stay until its settlement forgets them.
    pub fn retain_versions(&self) {
        if self.session_store.is_some() {
            return;
        }
        let hosts = self.hosts();
        let mut inner = self.directory.inner.lock().expect("agent index");
        let closing = inner.closing.clone();
        let Inner {
            versions,
            unavailable_sessions,
            sessions,
            ..
        } = &mut *inner;
        versions.retain(|session, (agent, _)| {
            unavailable_sessions.contains_key(session)
                || closing.contains_key(agent)
                || hosts.iter().any(|host| {
                    host.agent_id == *agent && host.agent.get_session(session).is_some()
                })
        });
        sessions.retain(|session, _| versions.contains_key(session));
    }
    pub fn session_count(&self) -> usize {
        self.hosts()
            .iter()
            .map(|host| host.agent.sessions().len())
            .sum()
    }
    pub fn list(&self) -> Vec<AgentInfo> {
        self.hosts()
            .into_iter()
            .map(|host| AgentInfo {
                agent_id: host.agent_id.clone(),
                default: host.default,
                name: host.profile.name.clone(),
                description: host.profile.description.clone(),
                sessions: host.agent.sessions().len(),
            })
            .collect()
    }

    /// Loads the default Agent. Called from the startup sequence and on
    /// `Agents` reload, once the provider is Active. Agents an earlier
    /// generation loaded are still running and stay as they are.
    pub async fn install_default(self: &Arc<Self>) -> Result<(), AgentOpError> {
        let agent_id = self.default_agent_id().to_owned();
        let stored = match &self.records {
            Some(records) => records.agents().await?,
            None => vec![],
        };
        let owners = match &self.records {
            Some(records) => records.owners().await?,
            None => vec![],
        };
        if stored.is_empty() {
            if !self.running(&agent_id) {
                self.load(agent_id, true, process_default(&self.session_defaults))
                    .await?;
            }
        } else {
            for record in stored
                .into_iter()
                .filter(|record| !self.running(&record.id))
            {
                let profile = record
                    .profile
                    .and_then(|profile| self.check_profile(&profile).map(|_| profile));
                let loaded = match profile {
                    Ok(profile) => self
                        .load(record.id.clone(), record.default, profile)
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string()),
                    Err(error) => Err(error),
                };
                if let Err(error) = loaded {
                    tracing::error!(agent_id=%record.id, %error, "Stored Agent is unavailable");
                    self.directory
                        .inner
                        .lock()
                        .expect("agent index")
                        .unavailable_agents
                        .insert(record.id, error);
                }
            }
        }
        for owner in owners {
            if let Some((agent_id, version)) = &owner.owner {
                self.insert_version(&owner.session_id, agent_id, *version);
            }
            if let Some(error) = owner.error {
                tracing::error!(session_id=%owner.session_id, %error, "Invalid Session ownership");
                self.directory
                    .inner
                    .lock()
                    .unwrap()
                    .unavailable_sessions
                    .insert(owner.session_id, error);
            }
        }
        Ok(())
    }

    /// An instance of the Agent exists, created by this or an earlier
    /// generation.
    fn running(&self, agent_id: &str) -> bool {
        self.directory
            .inner
            .lock()
            .expect("agent index")
            .instances
            .contains_key(agent_id)
    }
    pub(crate) fn session_available(&self, id: &str) -> bool {
        self.host_by_session(id).is_some() && !self.slot(id).closed.load(Ordering::Acquire)
    }
    pub(crate) async fn read_page(
        &self,
        id: &str,
        after: u64,
        limit: usize,
        through: Option<u64>,
    ) -> dim_agent_runtime::Result<(u64, Vec<Arc<dim_agent_types::AgentEvent>>)> {
        if let Some(store) = &self.session_store {
            let head = match self.session(id) {
                Some((_, session)) => session.sequence("main")?,
                None => {
                    store
                        .timeline_store()
                        .head(&dim_agent_runtime::TimelineKey {
                            session_id: id.into(),
                            branch_id: "main".into(),
                        })
                        .await?
                }
            };
            let through = through.unwrap_or(head);
            if through > head {
                return Err(AgentError::InvalidInput("future Timeline sequence".into()));
            }
            return Ok((through, store.read_page(id, after, limit, through).await?));
        }
        let (_, session) = self
            .session(id)
            .ok_or_else(|| AgentError::Closed(id.into()))?;
        let through = through.unwrap_or(session.sequence("main")?);
        Ok((
            through,
            session.local_page("main", after, limit, through).await?,
        ))
    }
    pub(crate) async fn read_event(
        &self,
        id: &str,
        event_id: &str,
    ) -> dim_agent_runtime::Result<Option<Arc<dim_agent_types::AgentEvent>>> {
        if let Some(store) = &self.session_store {
            return store.read_event(id, event_id).await;
        }
        let (_, session) = self
            .session(id)
            .ok_or_else(|| AgentError::Closed(id.into()))?;
        Ok(session
            .history("main", None)
            .await?
            .into_iter()
            .find(|event| event.id == event_id))
    }
    /// The main-branch `step/tool-use-start` of call `call_id` in request
    /// `request_id`.
    pub(crate) async fn read_tool_use_start(
        &self,
        id: &str,
        request_id: &str,
        call_id: &str,
    ) -> dim_agent_runtime::Result<Option<Arc<dim_agent_types::AgentEvent>>> {
        if let Some(store) = &self.session_store {
            return store.read_tool_use_start(id, request_id, call_id).await;
        }
        let (_, session) = self
            .session(id)
            .ok_or_else(|| AgentError::Closed(id.into()))?;
        Ok(session
            .history("main", None)
            .await?
            .into_iter()
            .find(|event| {
                matches!(&event.kind, dim_agent_types::EventKind::ToolUseStart(start)
                    if start.request_id == request_id && start.call_id == call_id)
            }))
    }
    fn slot(&self, id: &str) -> Arc<SessionSlot> {
        self.directory
            .inner
            .lock()
            .unwrap()
            .sessions
            .entry(id.to_owned())
            .or_default()
            .clone()
    }
    pub(crate) async fn acquire(&self, id: &str) -> dim_agent_runtime::Result<SessionUse> {
        if let Some(reason) = self.unavailable_session(id) {
            return Err(AgentError::Storage(reason));
        }
        let host = self.host_by_session(id);
        let Some(host) = host else {
            if let Some(store) = &self.session_store
                && store.has_records(id).await?
            {
                return Err(AgentError::Storage(format!(
                    "Session {id} has no available owner"
                )));
            }
            return Err(AgentError::Closed(id.into()));
        };
        let slot = self.slot(id);
        let _gate = slot.gate.lock().await;
        if slot.closed.load(Ordering::Acquire) || self.stopping.is_cancelled() {
            return Err(AgentError::Closed(id.into()));
        }
        let session = match host.agent.get_session(id) {
            Some(session) => session,
            None if self.session_store.is_some() => {
                if let Some(settings) = &self.settings {
                    let config = self
                        .session_store
                        .as_ref()
                        .unwrap()
                        .timeline_store()
                        .session_config(id)
                        .await?
                        .ok_or_else(|| AgentError::Storage(format!("unknown session {id}")))?;
                    settings
                        .authorize_project(config.cwd.into(), &self.stopping)
                        .await
                        .map_err(|error| {
                            AgentError::fault("project authorization", error.to_string())
                        })?;
                }
                let session = host.agent.restore_session(id.into()).await?;
                let ctx = self.ctx.get().expect("AgentsPlugin apply sets its context");
                ctx.events()
                    .parallel(
                        ctx,
                        &self.session_restored,
                        Arc::new(SessionRestored {
                            agent_id: host.agent_id.clone(),
                            session: session.clone(),
                        }),
                    )
                    .await?;
                session
            }
            None => return Err(AgentError::Closed(id.into())),
        };
        slot.uses.fetch_add(1, Ordering::AcqRel);
        Ok(SessionUse {
            host,
            session,
            slot: slot.clone(),
        })
    }
    pub(crate) async fn session_directory(
        &self,
        agent: Option<String>,
        version: u32,
    ) -> dim_agent_runtime::Result<Option<Vec<crate::protocol::SessionInfo>>> {
        let Some(store) = &self.session_store else {
            return Ok(None);
        };
        let mut records = store.directory(agent, version).await?;
        records.retain(|record| {
            !self.slot(&record.session_id).closed.load(Ordering::Acquire)
                && record
                    .agent_id
                    .as_deref()
                    .is_some_and(|id| self.host(Some(id)).is_some())
        });
        for record in &mut records {
            if let Some((_, session)) = self.session(&record.session_id) {
                record.running = session.state("main")?.running;
                record.sequence = session.sequence("main")?;
            }
            if version < 3 {
                record.agent_id = None;
            }
        }
        Ok(Some(records))
    }

    pub(crate) async fn session_summary(
        &self,
        id: &str,
        through: u64,
        created_at: u64,
    ) -> dim_agent_runtime::Result<(Option<String>, u64)> {
        // Capture one source and the directory's main head for both reads.
        // A concurrent turn cannot move updatedAt beyond the returned sequence.
        let (events, last) = if let Some((_, session)) = self.session(id) {
            let events = session
                .local_page("main", 0, Self::SUMMARY_SCAN_LIMIT, through)
                .await?;
            // A range may start inside a batch; a page cursor may not.
            let last = session.local("main", through..=through).await?.pop();
            (events, last)
        } else {
            let store = self
                .session_store
                .as_ref()
                .ok_or_else(|| AgentError::Closed(id.into()))?;
            let events = store
                .read_page(id, 0, Self::SUMMARY_SCAN_LIMIT, through)
                .await?;
            let last = store
                .timeline_store()
                .read(
                    &TimelineKey {
                        session_id: id.into(),
                        branch_id: "main".into(),
                    },
                    through..=through,
                )
                .await?
                .pop()
                .map(|record| record.event);
            (events, last)
        };
        let preview = events
            .iter()
            .find_map(|event| {
                let EventKind::InputMessage(data) = &event.kind else {
                    return None;
                };
                data.user.as_ref()
            })
            .and_then(|user| {
                let text = user
                    .content
                    .iter()
                    .filter_map(|block| {
                        if let ContentBlock::Text { text } = block {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (!text.is_empty()).then(|| truncate_preview(&text, Self::SUMMARY_PREVIEW_CHARS))
            });
        let updated_at = last.map_or(created_at, |event| event.create_at);
        Ok((preview, updated_at))
    }

    pub(crate) async fn prepare_session(
        &self,
        host: &AgentHost,
        config: &SessionConfig,
        version: u32,
    ) -> Option<(String, BoxFuture<'static, dim_agent_runtime::Result<()>>)> {
        let store = self.session_store.as_ref()?;
        let id = uuid::Uuid::new_v4().to_string();
        let store = store.clone();
        let prepared_id = id.clone();
        let config = config.clone();
        let agent_id = host.agent_id.clone();
        let settings = self.settings.clone();
        let stopping = self.stopping.clone();
        Some((
            id,
            Box::pin(async move {
                if let Some(settings) = settings {
                    settings
                        .authorize_project(config.cwd.clone().into(), &stopping)
                        .await
                        .map_err(|error| {
                            AgentError::fault("project authorization", error.to_string())
                        })?;
                }
                store
                    .create_owned_session(&prepared_id, &config, &agent_id, version)
                    .await
            }),
        ))
    }

    pub(crate) async fn create_session(
        &self,
        host: &AgentHost,
        config: SessionConfig,
        version: u32,
    ) -> dim_agent_runtime::Result<Session> {
        match self.prepare_session(host, &config, version).await {
            Some((id, prepare)) => {
                host.agent
                    .create_prepared_session(id, config, prepare)
                    .await
            }
            None => host.agent.create_session(config).await,
        }
    }
    /// `agent.create`: one more Agent, a peer of every other. A requested
    /// profile comes from outside the process and may only select provided
    /// tools, each once; without one the process default profile applies.
    /// Callers allocate a fresh id before loading so their own
    /// access policy can precede publication in the shared directory.
    pub(crate) async fn create(
        &self,
        id: uuid::Uuid,
        profile: Option<AgentProfile>,
    ) -> Result<Arc<AgentHost>, AgentOpError> {
        let profile = match profile {
            Some(profile) => {
                self.check_profile(&profile)
                    .map_err(|error| AgentOpError::Build(AgentError::Build(error)))?;
                profile
            }
            None => process_default(&self.session_defaults),
        };
        self.load(id.to_string(), false, profile).await
    }
    /// A profile from outside the process may only select provided tools,
    /// each once.
    pub fn check_profile(&self, profile: &AgentProfile) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for tool in &profile.tools {
            if !self.tools.contains(tool) {
                return Err(format!("tool {tool} is not provided by the process"));
            }
            if !seen.insert(tool) {
                return Err(format!("tool {tool} is listed twice"));
            }
        }
        Ok(())
    }
    /// Closes exactly one Agent; other Agents are unaffected. New business
    /// requests fail with agent_unknown from the removal on; closing requests
    /// still locate the Agent until its settlement finishes.
    pub async fn close(&self, agent_id: &str) -> Result<(), AgentOpError> {
        let entry = {
            let mut inner = self.directory.inner.lock().expect("agent index");
            let Some(entry) = inner.agents.remove(agent_id) else {
                return Err(AgentOpError::Unknown(agent_id.to_owned()));
            };
            inner
                .closing
                .insert(agent_id.to_owned(), entry.host.clone());
            entry
        };
        // Business settlement first, from a plain task context: Session close
        // awaits its own subtree, which a finalizer must not. The scope effect
        // repeats both steps idempotently when rutis releases the subtree.
        let closed = entry.host.agent.close().await;
        entry.host.traces.finish().await;
        let unloaded = entry.view.shutdown().await;
        self.directory
            .inner
            .lock()
            .expect("agent index")
            .instances
            .remove(agent_id);
        self.directory
            .inner
            .lock()
            .expect("agent index")
            .closing
            .remove(agent_id);
        unloaded.map_err(AgentOpError::Unload)?;
        closed.map_err(AgentOpError::Settle)
    }
    /// Process shutdown phase one: business settlement of every Agent.
    pub async fn settle_all(&self) -> Vec<AgentError> {
        self.stopping.cancel();
        let mut errors = vec![];
        let mut tasks = tokio::task::JoinSet::new();
        for host in self.hosts() {
            let agent = host.agent.clone();
            tasks.spawn(async move { agent.close().await });
        }
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Err(error)) => errors.push(error),
                Err(error) => errors.push(AgentError::fault(
                    "close",
                    format!("agent close task failed: {error}"),
                )),
                Ok(Ok(())) => {}
            }
        }
        errors
    }
    /// Process shutdown phase two, after Rpc commands drained: archives
    /// finish after their Sessions close so they keep the final diagnostics.
    pub async fn finish_traces(&self) {
        for host in self.hosts() {
            host.traces.finish().await;
        }
    }

    async fn load(
        &self,
        agent_id: String,
        default: bool,
        profile: AgentProfile,
    ) -> Result<Arc<AgentHost>, AgentOpError> {
        let (loader, root) = self
            .loader
            .get()
            .expect("AgentsPlugin apply sets the loader")
            .clone();
        let spec = AgentSpec {
            builder: self.builder.clone(),
            agent_id: agent_id.clone(),
            default,
            profile,
        };
        let mut creation =
            tokio::spawn(
                async move { loader.create_instance(&root, AGENT_GROUP).with(spec).await },
            );
        let instance = match tokio::time::timeout(self.startup_timeout, &mut creation).await {
            Ok(Ok(Ok(instance))) => instance,
            Ok(Ok(Err(error))) => {
                self.forget(&agent_id);
                return Err(AgentOpError::Build(AgentError::Build(error.to_string())));
            }
            Ok(Err(error)) => {
                self.forget(&agent_id);
                return Err(AgentOpError::Build(AgentError::fault(
                    "agent creation",
                    error.to_string(),
                )));
            }
            Err(_) => {
                self.forget(&agent_id);
                // Dispose the instance once its creation ends.
                tokio::spawn(async move {
                    if let Ok(Ok(instance)) = creation.await {
                        let _ = instance.view.shutdown().await;
                    }
                });
                return Err(AgentOpError::Build(AgentError::Build(format!(
                    "agent {agent_id} initialization timed out"
                ))));
            }
        };
        let view = instance.view;
        self.directory
            .inner
            .lock()
            .expect("agent index")
            .instances
            .insert(agent_id.clone(), view.clone());
        let failed = instance
            .report
            .into_iter()
            .find_map(|(row, result)| match result {
                InstanceResult::Active => None,
                InstanceResult::Failed(error) => Some(match error {
                    rutis_loader::LoaderError::Rejected { error, .. } => AgentError::from(error),
                    error => AgentError::Build(error.to_string()),
                }),
                result => Some(AgentError::Build(format!("{row}: {result:?}"))),
            });
        if let Some(error) = failed {
            // Forget any partial registration before reporting the failure.
            self.forget(&agent_id);
            let _ = view.shutdown().await;
            self.directory
                .inner
                .lock()
                .expect("agent index")
                .instances
                .remove(&agent_id);
            return Err(error.into());
        }
        let host = {
            let mut inner = self.directory.inner.lock().expect("agent index");
            inner.pending.remove(&agent_id)
        }
        .ok_or_else(|| {
            AgentOpError::Build(AgentError::Build(format!(
                "agent {agent_id} did not register"
            )))
        })?;
        if let Some(records) = &self.records
            && let Err(error) = records.save(&agent_id, &host.profile, default).await
        {
            self.forget(&agent_id);
            let settled = host.agent.close().await;
            host.traces.finish().await;
            let unloaded = view.shutdown().await;
            self.directory
                .inner
                .lock()
                .expect("agent index")
                .instances
                .remove(&agent_id);
            return Err(AgentOpError::Build(
                AgentError::aggregate(
                    std::iter::once(error)
                        .chain(settled.err())
                        .chain(unloaded.err().map(AgentError::from))
                        .collect(),
                )
                .expect_err("Agent record write failed"),
            ));
        }
        self.directory
            .inner
            .lock()
            .expect("agent index")
            .agents
            .insert(
                agent_id,
                Entry {
                    host: host.clone(),
                    view,
                },
            );
        Ok(host)
    }
    /// Drops an Agent from the directory; see [`AgentDirectory::forget`].
    pub(crate) fn forget(&self, agent_id: &str) {
        self.directory.forget(agent_id);
    }
}

fn truncate_preview(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let mut result: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        result.pop();
        result.push('…');
    }
    result
}

/// Owns every Agent subtree: the process assembly entry.
pub(crate) struct AgentsPlugin {
    builder: AgentBuilder,
    config: Arc<ServerConfig>,
    /// The process root: top-level loader instances are created from it.
    root: Ctx,
}
impl AgentsPlugin {
    pub fn new(builder: AgentBuilder, config: Arc<ServerConfig>, root: Ctx) -> Self {
        Self {
            builder,
            config,
            root,
        }
    }
}
impl TypedPlugin for AgentsPlugin {
    type Deps = (
        When<(Arc<SessionStore>, Arc<SqliteStorage>)>,
        When<Arc<SettingsService>>,
        Arc<Loader>,
        Arc<AgentDirectory>,
    );
    fn name(&self) -> &str {
        "Agents"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (When(stored), When(settings), loader, directory): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let mut builder = self.builder.clone();
            let (records, session_store) = match stored {
                Some((store, storage)) => {
                    builder.timeline_store(store.timeline_store());
                    (Some(Arc::new(AgentRecords(storage))), Some(store))
                }
                None => (None, None),
            };
            let default_id = match &records {
                Some(records) => records
                    .agents()
                    .await?
                    .into_iter()
                    .find(|record| record.default)
                    .map(|record| record.id),
                None => None,
            }
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            // Fixed by the first generation so every lookup of the default
            // Agent succeeds from the moment the service is visible, and an
            // Agent loaded by an earlier generation keeps its identity.
            let _ = directory.default_agent_id.set(default_id);
            let index = AgentIndex::new(
                directory.clone(),
                builder,
                &self.config,
                records,
                session_store,
                settings,
                self.root.instance(),
            );
            if index.ctx.set(ctx.clone()).is_err() {
                return Err(CordisError::PluginFailed(
                    "AgentsPlugin applied twice".into(),
                ));
            }
            let _ = index.loader.set(((*loader).clone(), self.root.clone()));
            let index = Arc::new(index);
            let cleanup = index.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        // Agents do not depend on the index: they keep
                        // running, and the next generation finds them in
                        // the directory.
                        cleanup.stopping.cancel();
                        Ok(())
                    })
                }))
            })?;
            ctx.provide_as(TypeKey::of::<AgentIndex>(), index)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for AgentsPlugin {
    fn keys(&self) -> KeysOf<Self> {
        (
            self.config.database_path.is_some().then_some(((), ())),
            (self.config.product_home.is_some() && self.config.session_execution).then_some(()),
            (),
            (),
        )
    }
}

/// The loader row of the Agent group: each Agent is its plugin and its
/// Sessions (`session_row`).
pub(crate) fn group_row(session_row: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": AGENT_GROUP,
        "group": true,
        "instanced": true,
        "config": [{ "id": "AgentScope", "name": "AgentScope" }, session_row],
    })
}

/// What an Agent instance is created with.
struct AgentSpec {
    builder: AgentBuilder,
    agent_id: String,
    default: bool,
    profile: AgentProfile,
}

/// Registers the module of the Agent instance's plugin.
pub(crate) fn register(builtins: &mut Builtins) {
    builtins.register_with::<NoConfig, _, _>("AgentScope", |build| {
        let spec = build.value::<AgentSpec>().ok_or_else(|| {
            CordisError::PluginFailed("Agent created without its specification".into())
        })?;
        Ok(TypedFactory::new(AgentScopeFactory(spec)))
    });
}
struct AgentScopeFactory(Arc<AgentSpec>);
impl TypedPluginFactory<NoConfig> for AgentScopeFactory {
    type Plugin = AgentScopePlugin;
    fn name(&self) -> &str {
        "AgentScope"
    }
    fn build(&self, _: &NoConfig) -> Result<AgentScopePlugin, CordisError> {
        Ok(AgentScopePlugin(self.0.clone()))
    }
}

/// One Agent: the plugin of its loader instance. Its Sessions are instances
/// created beside it; it owns their protocol identity and archives. It
/// depends only on the directory and the loader, which do not reload, so it
/// unloads only when its Agent is closed or the process stops; both settle
/// the Sessions first. Its own cleanup repeats that settlement idempotently,
/// then drains the archives; effects unload in reverse order.
struct AgentScopePlugin(Arc<AgentSpec>);
impl TypedPlugin for AgentScopePlugin {
    type Deps = (Arc<AgentDirectory>, Arc<Loader>);
    fn name(&self) -> &str {
        "AgentScope"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (directory, loader): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let spec = &self.0;
            let slot = Arc::new(Mutex::new(None));
            let sessions = ctx.plugin(capability_plugin(Sessions(slot.clone())));
            (&sessions).await.map_err(|error| {
                CordisError::PluginFailed(format!("Session host failed: {error}").into())
            })?;
            let sessions = match (sessions.state().state, slot.lock().unwrap().take()) {
                (FiberState::Active, Some(sessions)) => sessions,
                (state, _) => {
                    return Err(CordisError::PluginFailed(
                        format!("Session host is {state:?}").into(),
                    ));
                }
            };
            let host = Arc::new(AgentHost {
                agent: spec.builder.build_at((*loader).clone(), ctx),
                agent_id: spec.agent_id.clone(),
                default: spec.default,
                profile: spec.profile.clone(),
                ctx: sessions.clone(),
                traces: Traces::new(sessions.clone()),
            });
            let owner = host.clone();
            let unloading = directory.clone();
            let agent_id = spec.agent_id.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        unloading.forget(&agent_id);
                        // Cleanup failures are exactly what archives must keep.
                        let closed = owner.agent.close().await;
                        owner.traces.finish().await;
                        closed.map_err(Into::into)
                    })
                }))
            })?;
            directory.register(host);
            Ok(Effect::Done)
        })
    }
}

/// Host of the listeners on one Agent's Sessions. A child of the Agent
/// plugin registered before its settlement, it unloads after the Sessions
/// closed.
struct Sessions(Arc<Mutex<Option<Ctx>>>);
impl TypedPlugin for Sessions {
    type Deps = ();
    fn name(&self) -> &str {
        "Sessions"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            *self.0.lock().unwrap() = Some(ctx.clone());
            Ok(Effect::Done)
        })
    }
}
impl Capability for Sessions {
    fn keys(&self) -> KeysOf<Self> {}
}
