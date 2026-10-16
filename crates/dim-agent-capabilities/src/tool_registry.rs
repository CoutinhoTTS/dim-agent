//! The Session's tools: the fixed table built from `SessionConfig.tools` and
//! the dynamic sets of registered `SessionToolSource`s, see
//! `docs/architecture/rust-agent/plugins/ToolRegistry.md`.
use crate::host::*;
use dim_agent_runtime::{
    AgentError, Capability, Contributions, Diagnostics, InSession, KeysOf, SessionConfig,
    SessionScope, SessionService, ToolNames,
};
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, Effect, Event, EventKey, Listener, TypedPlugin};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, LazyLock, Mutex, Weak},
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// A provider updated its sets; emitted on its Session instance's channel.
pub struct ToolProviderChanged;
impl Event for ToolProviderChanged {
    const NAME: &'static str = "tools.provider_changed";
    type Value = ();
}

/// The registry replaced or withdrew sets; emitted after its state is updated.
pub struct ToolsChanged;
impl Event for ToolsChanged {
    const NAME: &'static str = "tools.changed";
    type Value = ();
}

pub trait ToolHandler: Send + Sync + 'static {
    fn execute(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult>;
}
/// A tool a Session capability provides under
/// `SessionScope::named_key::<ToolContribution>(name)` for a name registered
/// with `AgentBuilder::session_tool`.
pub struct ToolContribution {
    pub definition: ToolDefinition,
    pub handler: Arc<dyn ToolHandler>,
}
impl SessionService for ToolContribution {}

/// A dynamic tool source a Session capability provides under
/// `SessionScope::named_key::<SessionToolSource>(name)` for a name
/// registered with `AgentBuilder::session_tool_source`.
pub struct SessionToolSource(pub Arc<dyn ToolProvider>);
impl SessionService for SessionToolSource {}

/// After replacing its sets, a provider emits `ToolProviderChanged` with
/// `EventKey::of().instance(session_scope.instance)` from its own context.
pub trait ToolProvider: Send + Sync + 'static {
    /// Every complete set this provider currently offers, one per key.
    fn sets(&self) -> Vec<Arc<ToolSourceSet>>;
}

/// One complete version of a source's tools. The handlers are bound to this
/// version's resources, which live as long as the set.
pub struct ToolSourceSet {
    /// `<registered name>` or `<registered name>:<instance>`.
    pub key: String,
    /// Never reused for this key.
    pub version: String,
    pub tools: Vec<Arc<ToolContribution>>,
}

/// Read-only view of the source statuses for hosts; unlike `ToolRegistry`
/// it exposes no handler, so a Session handle may obtain it.
pub struct ToolSourcesView(watch::Receiver<ToolSourcesState>);
impl ToolSourcesView {
    pub fn get(&self) -> ToolSourcesState {
        self.0.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<ToolSourcesState> {
        self.0.clone()
    }
}

/// What the registry did with each source key, for management views.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolSourcesState(pub BTreeMap<String, ToolSourceStatus>);
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolSourceStatus {
    pub adopted: Option<String>,
    pub rejected: Option<ToolSourceRejection>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSourceRejection {
    pub version: String,
    pub reason: String,
}

#[derive(Default)]
struct Sources {
    current: BTreeMap<String, Arc<ToolSourceSet>>,
    /// Replaced versions, reachable while someone still holds them.
    replaced: HashMap<(String, String), Weak<ToolSourceSet>>,
    /// Every version ever held, so none is adopted twice.
    held: HashSet<(String, String)>,
}

type FixedTools = Vec<Arc<ToolContribution>>;

pub struct ToolRegistry {
    ctx: Ctx,
    scope: SessionScope,
    /// Collected once on first use, after the Session services are Active.
    fixed: LazyLock<FixedTools, Box<dyn FnOnce() -> FixedTools + Send>>,
    /// Configured fixed tools and every registered contributor reserve names.
    reserved: HashSet<String>,
    /// Registered name and provider of every dynamic source.
    providers: Vec<(String, Arc<dyn ToolProvider>)>,
    /// Serializes refreshes from read to commit, so an earlier read never
    /// overwrites a later one.
    refreshing: Mutex<()>,
    sources: Mutex<Sources>,
    state: watch::Sender<ToolSourcesState>,
}
impl ToolRegistry {
    /// Every declaration: fixed tools first, then source tools by key, in
    /// set order.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let sources = self.sources.lock().unwrap();
        self.fixed
            .iter()
            .chain(sources.current.values().flat_map(|set| set.tools.iter()))
            .map(|tool| tool.definition.clone())
            .collect()
    }
    /// Declarations and the source sets they were built from, read together.
    pub fn snapshot(&self) -> (Vec<ToolDefinition>, Vec<Arc<ToolSourceSet>>) {
        let sources = self.sources.lock().unwrap();
        let sets: Vec<_> = sources.current.values().cloned().collect();
        let definitions = self
            .fixed
            .iter()
            .chain(sets.iter().flat_map(|set| set.tools.iter()))
            .map(|tool| tool.definition.clone())
            .collect();
        (definitions, sets)
    }
    /// The fixed tool of this name.
    pub fn fixed(&self, name: &str) -> Option<Arc<ToolContribution>> {
        self.fixed
            .iter()
            .find(|tool| tool.definition.name == name)
            .cloned()
    }
    /// The current sets, by key.
    pub fn sources(&self) -> Vec<Arc<ToolSourceSet>> {
        self.sources
            .lock()
            .unwrap()
            .current
            .values()
            .cloned()
            .collect()
    }
    /// The current or a still-held replaced version of a key.
    pub fn source(&self, key: &str, version: &str) -> Option<Arc<ToolSourceSet>> {
        let sources = self.sources.lock().unwrap();
        match sources.current.get(key) {
            Some(set) if set.version == version => Some(set.clone()),
            _ => sources
                .replaced
                .get(&(key.to_owned(), version.to_owned()))
                .and_then(Weak::upgrade),
        }
    }
    pub fn sources_state(&self) -> watch::Receiver<ToolSourcesState> {
        self.state.subscribe()
    }

    /// Applies what every provider offers now: each changed key is
    /// validated as a whole and replaced atomically; a missing key is
    /// withdrawn. All providers are read on any change, so a set rejected for
    /// a name another source held is retried once that name is free.
    fn refresh(&self) {
        let _serial = self.refreshing.lock().unwrap();
        let offered: Vec<(&str, BTreeMap<String, Arc<ToolSourceSet>>)> = self
            .providers
            .iter()
            .map(|(name, provider)| {
                (
                    name.as_str(),
                    provider
                        .sets()
                        .into_iter()
                        .map(|set| (set.key.clone(), set))
                        .collect(),
                )
            })
            .collect();
        let mut sources = self.sources.lock().unwrap();
        let mut statuses = self.state.borrow().clone();
        let mut changed = false;
        let withdrawn: Vec<String> = sources
            .current
            .keys()
            .filter(|key| {
                !offered
                    .iter()
                    .any(|(name, sets)| owns(name, key) && sets.contains_key(*key))
            })
            .cloned()
            .collect();
        for key in withdrawn {
            let set = sources.current.remove(&key).expect("current key");
            sources
                .replaced
                .insert((key, set.version.clone()), Arc::downgrade(&set));
            changed = true;
        }
        statuses
            .0
            .retain(|key, _| offered.iter().any(|(_, sets)| sets.contains_key(key)));
        // Repeat until nothing more is adopted: a set rejected for a name that
        // a later replacement in the same pass frees is adopted now.
        let mut progressed = true;
        while progressed {
            progressed = false;
            for (name, set) in offered
                .iter()
                .flat_map(|(name, sets)| sets.values().map(move |set| (*name, set)))
            {
                let key = set.key.clone();
                if sources
                    .current
                    .get(&key)
                    .is_some_and(|current| current.version == set.version)
                {
                    continue;
                }
                match self.validate(name, &sources, set) {
                    Ok(()) => {
                        progressed = true;
                        if let Some(old) = sources.current.insert(key.clone(), set.clone()) {
                            sources
                                .replaced
                                .insert((key.clone(), old.version.clone()), Arc::downgrade(&old));
                        }
                        sources.held.insert((key.clone(), set.version.clone()));
                        statuses.0.insert(
                            key,
                            ToolSourceStatus {
                                adopted: Some(set.version.clone()),
                                rejected: None,
                            },
                        );
                        changed = true;
                    }
                    Err(reason) => {
                        let rejection = ToolSourceRejection {
                            version: set.version.clone(),
                            reason,
                        };
                        let status = statuses.0.entry(key.clone()).or_default();
                        if status.rejected.as_ref() != Some(&rejection) {
                            tracing::warn!(
                                key,
                                version = set.version,
                                reason = rejection.reason,
                                "tool source rejected"
                            );
                            status.rejected = Some(rejection);
                        }
                    }
                }
            }
        }
        sources.replaced.retain(|_, set| set.strong_count() > 0);
        drop(sources);
        self.state.send_if_modified(|state| {
            let modified = *state != statuses;
            *state = statuses;
            modified
        });
        if changed
            && let Err(error) = self.ctx.events().emit(
                &self.ctx,
                &EventKey::<ToolsChanged>::of().instance(self.scope.instance),
                Arc::new(ToolsChanged),
            )
        {
            tracing::warn!(%error, "failed to publish tools.changed");
        }
    }

    fn validate(&self, name: &str, sources: &Sources, set: &ToolSourceSet) -> Result<(), String> {
        if !owns(name, &set.key) {
            return Err(format!("key {} does not belong to source {name}", set.key));
        }
        if sources
            .held
            .contains(&(set.key.clone(), set.version.clone()))
        {
            return Err(format!("version {} was already used", set.version));
        }
        let mut names = HashSet::new();
        for tool in &set.tools {
            let tool_name = &tool.definition.name;
            if !valid_tool_name(tool_name) {
                return Err(format!("invalid tool name {tool_name}"));
            }
            if !names.insert(tool_name.as_str()) {
                return Err(format!("duplicate tool {tool_name}"));
            }
            if self.reserved.contains(tool_name) {
                return Err(format!("tool {tool_name} is a fixed tool"));
            }
            if let Some(other) = sources.current.values().find(|other| {
                other.key != set.key
                    && other
                        .tools
                        .iter()
                        .any(|tool| tool.definition.name == *tool_name)
            }) {
                return Err(format!("tool {tool_name} is provided by {}", other.key));
            }
        }
        Ok(())
    }
}

impl Listener<ToolProviderChanged> for Arc<ToolRegistry> {
    fn call<'a>(
        &'a self,
        _ctx: &'a Ctx,
        _: &'a ToolProviderChanged,
    ) -> BoxFuture<'a, Result<Option<()>, CordisError>> {
        Box::pin(async move {
            self.refresh();
            Ok(None)
        })
    }
}

fn owns(name: &str, key: &str) -> bool {
    key == name
        || key
            .strip_prefix(name)
            .is_some_and(|rest| rest.starts_with(':'))
}

fn valid_tool_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

struct HostTool(Arc<ToolHost>);
impl ToolHandler for HostTool {
    fn execute(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        self.0.0.execute(call, context, cancel)
    }
}
pub fn definition(name: &str, description: &str, schema: serde_json::Value) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema: serde_json::from_value(schema).expect("built-in schema is valid"),
        source: ToolSource::Builtin,
        risk: None,
        metadata: None,
        file_changes: None,
    }
}

pub struct ToolRegistryPlugin {
    scope: SessionScope,
    contributed: Arc<[String]>,
    sources: Arc<[String]>,
}
impl ToolRegistryPlugin {
    pub fn new(scope: &SessionScope, names: ToolNames<'_>) -> Self {
        Self {
            scope: *scope,
            contributed: names.contributed.into(),
            sources: names.sources.into(),
        }
    }
}
impl TypedPlugin for ToolRegistryPlugin {
    type Deps = (
        InSession<ToolHost>,
        InSession<SessionConfig>,
        Contributions<SessionToolSource>,
        InSession<Diagnostics>,
    );
    fn name(&self) -> &str {
        "ToolRegistry"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (
            InSession(host),
            InSession(config),
            Contributions(sources),
            InSession(diagnostics),
        ): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let host_tools: BTreeMap<_, _> = host
                .0
                .definitions()
                .into_iter()
                .map(|definition| {
                    (
                        definition.name.clone(),
                        Arc::new(ToolContribution {
                            definition,
                            handler: Arc::new(HostTool(host.clone())),
                        }),
                    )
                })
                .collect();
            let mut selected = HashSet::new();
            for name in &config.tools {
                if !selected.insert(name)
                    || (self.contributed.contains(name) && host_tools.contains_key(name))
                {
                    return Err(AgentError::InvalidInput(format!("duplicate tool {name}")).into());
                }
                if !host_tools.contains_key(name) && !self.contributed.contains(name) {
                    return Err(AgentError::InvalidInput(format!(
                        "tool {name} is not provided by the host"
                    ))
                    .into());
                }
            }
            let reserved = selected
                .into_iter()
                .cloned()
                .chain(self.contributed.iter().cloned())
                .collect();
            let providers: Vec<_> = sources
                .into_iter()
                .map(|(name, provider)| (name, provider.0.clone()))
                .collect();
            let context = ctx.clone();
            let scope = self.scope;
            let contributed = self.contributed.clone();
            let registry = Arc::new(ToolRegistry {
                ctx: ctx.clone(),
                scope,
                fixed: LazyLock::new(Box::new(move || {
                    let mut fixed = Vec::new();
                    for name in &config.tools {
                        let tool = if contributed.contains(name) {
                            context.get_as::<ToolContribution>(
                                scope.named_key::<ToolContribution>(name),
                            )
                        } else {
                            host_tools.get(name).cloned()
                        };
                        match tool {
                            Some(tool) => fixed.push(tool),
                            None => {
                                let span = diagnostics.span("tool.attach", None, None, None);
                                span.record("outcome", format!("unavailable:{name}"));
                                tracing::warn!(
                                    tool = name,
                                    "optional tool provider is unavailable at registry initialization"
                                );
                            }
                        }
                    }
                    fixed
                })),
                reserved,
                providers,
                refreshing: Mutex::default(),
                sources: Mutex::default(),
                state: watch::Sender::new(ToolSourcesState::default()),
            });
            // Subscribe before the first read; notifications carry no state.
            ctx.events().on(
                ctx,
                &EventKey::<ToolProviderChanged>::of().instance(scope.instance),
                registry.clone(),
            )?;
            registry.refresh();
            ctx.provide_as(
                self.scope.key::<ToolSourcesView>(),
                Arc::new(ToolSourcesView(registry.sources_state())),
            )?;
            ctx.provide_as(self.scope.key::<ToolRegistry>(), registry)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for ToolRegistryPlugin {
    fn keys(&self) -> KeysOf<Self> {
        let scope = self.scope;
        (scope, scope, (scope, self.sources.clone()), scope)
    }
}
