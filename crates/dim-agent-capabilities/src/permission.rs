//! Branch permission: decides a tool call by risk, workspace target and policy.
//! The policy is `SessionConfig.permission_policy` folded with this branch's
//! dispatched `permission/changed` events. The user's command rules, read
//! through the Session's `CommandRulesHost` on each decision, deny or approve
//! commands of process-starting calls.
use crate::{
    host::{HostError, HostFuture},
    session_grants::SessionGrantsHost,
    workspace_tools::files::canonical_prefix,
};
use dim_agent_runtime::{
    AgentError, Delivery, Dispatcher, Names, SessionConfig, StateCell, Timeline,
};
use dim_agent_types::{
    AgentEvent, ApprovalCallData, COMMAND_SEPARATORS, CommandMatch, CommandRule,
    CommandRuleDecision, EventKind, PermissionChangedData, PermissionPolicy, PermissionPreset,
    grant::{GrantKind, GrantMode, SessionGrant},
};
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use rutis::{Ctx, Error};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

/// The dispatched policy; `version` is the local sequence of the last
/// `permission/changed` of this branch, 0 when there is none.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PermissionState {
    pub version: u64,
    pub policy: PermissionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PermissionUpdateError {
    #[error("permission version conflict; current version is {current}")]
    Conflict { current: u64 },
    #[error("branch is closed")]
    Closed,
    #[error("{0}")]
    Failed(String),
}

/// The user's command rules, read when a command call is decided. Errors are
/// `Failed` (unreadable) or `Cancelled`.
pub trait ReadCommandRules: Send + Sync + 'static {
    fn rules(&self, cancel: CancellationToken) -> HostFuture<'_, Vec<CommandRule>>;
}
/// Session port of the command rules; each profile binds exactly one.
pub struct CommandRulesHost(pub Arc<dyn ReadCommandRules>);

/// The rules of a host without a rules source.
pub struct NoCommandRules;
impl ReadCommandRules for NoCommandRules {
    fn rules(&self, _: CancellationToken) -> HostFuture<'_, Vec<CommandRule>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[derive(Default)]
struct Inner {
    closed: bool,
    /// Local sequence of the last written `permission/changed`.
    written: u64,
    /// `update` waiters by event id, released when the event is dispatched.
    waiters: HashMap<String, oneshot::Sender<u64>>,
}

pub struct Permission {
    timeline: Arc<Timeline>,
    state: Arc<StateCell<PermissionState>>,
    rules: Arc<CommandRulesHost>,
    grants: Arc<SessionGrantsHost>,
    config: Arc<SessionConfig>,
    inner: Mutex<Inner>,
}
impl Permission {
    /// Decides with the policy dispatched when the decision is made and, for a
    /// command call, the rules read now. Workspace writes resolve their target
    /// before sampling that policy. Cancelled reads stop admission.
    pub async fn decide(
        &self,
        call: &ApprovalCallData,
        risk: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Decision, HostError> {
        let write_target = if matches!(Risk::of(risk), Risk::Write)
            && self.state.get().policy.preset == PermissionPreset::WorkspaceWrite
            && let Some(path) = call.input.get("path").and_then(|path| path.as_str())
            && std::path::Path::new(path).is_absolute()
        {
            let cwd = PathBuf::from(&self.config.cwd);
            let path = PathBuf::from(path);
            let target = tokio::task::spawn_blocking(move || {
                let workspace = std::fs::canonicalize(&cwd).map_err(|error| {
                    format!("Failed to resolve workspace {}: {error}", cwd.display())
                })?;
                canonical_prefix(path).map(|target| (workspace, target))
            });
            Some(tokio::select! {
                result = target => result.map_err(|error| HostError::failed(error.to_string()))?,
                _ = cancel.cancelled() => return Err(HostError::Cancelled("Tool execution cancelled.".into())),
            })
        } else {
            None
        };
        let command = match (Risk::of(risk), call.input.get("command")) {
            (Risk::Execute, Some(serde_json::Value::String(command))) => Some(command.as_str()),
            _ => None,
        };
        let rules = match command {
            Some(_) => match self.rules.0.rules(cancel.clone()).await {
                Err(HostError::Cancelled(reason)) => return Err(HostError::Cancelled(reason)),
                read => Some(read),
            },
            None => None,
        };
        let grant_target = matches!(Risk::of(risk), Risk::Read | Risk::Write)
            .then(|| call.input.get("path").and_then(|path| path.as_str()))
            .flatten()
            .map(|path| {
                let path = std::path::Path::new(path);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    PathBuf::from(&self.config.cwd).join(path)
                }
            });
        let policy = self.state.get().policy;
        let name = &call.name;
        if policy.disallowed_tools.contains(name) {
            return Ok(Decision::Deny {
                reason: format!("Tool `{name}` is disallowed by the permission policy"),
            });
        }
        if policy.preset == PermissionPreset::WorkspaceWrite
            && matches!(Risk::of(risk), Risk::Write)
        {
            let decision = match write_target {
                Some(Ok((workspace, target))) if target.starts_with(&workspace) => {
                    decide(&policy, name, risk)
                }
                Some(Ok((_, target))) => Decision::Ask {
                    reason: format!(
                        "Writing outside the session workspace requires approval: {}",
                        target.display()
                    ),
                },
                Some(Err(reason)) => Decision::Deny { reason },
                None => Decision::Ask {
                    reason: "A file write without a verified workspace target requires approval"
                        .into(),
                },
            };
            return Ok(self
                .apply_grant(decision, Risk::Write, grant_target.as_deref(), cancel)
                .await);
        }
        let (Some(command), Some(rules)) = (command, rules) else {
            return Ok(decide(&policy, name, risk));
        };
        let rules = match rules {
            Ok(rules) => rules,
            Err(error) => {
                return Ok(Decision::Deny {
                    reason: format!("Command rules could not be read: {error}"),
                });
            }
        };
        if let Some(rule) = rules.iter().find(|rule| {
            rule.decision == CommandRuleDecision::Deny
                && command
                    .split(COMMAND_SEPARATORS)
                    .any(|segment| matches(rule, segment))
        }) {
            let matching = match rule.matching {
                CommandMatch::Exact => "exact",
                CommandMatch::Prefix => "prefix",
            };
            return Ok(Decision::Deny {
                reason: format!(
                    "Command denied by command rule `{}` ({matching})",
                    rule.command
                ),
            });
        }
        let decision = match decide(&policy, name, risk) {
            Decision::Ask { .. }
                if !command.contains(COMMAND_SEPARATORS)
                    && rules.iter().any(|rule| {
                        rule.decision == CommandRuleDecision::Allow && matches(rule, command)
                    }) =>
            {
                Decision::Allow
            }
            decision => decision,
        };
        Ok(self
            .apply_grant(decision, Risk::of(risk), grant_target.as_deref(), cancel)
            .await)
    }

    async fn apply_grant(
        &self,
        decision: Decision,
        risk: Risk,
        target: Option<&std::path::Path>,
        cancel: &CancellationToken,
    ) -> Decision {
        let (Decision::Ask { .. }, Some(target)) = (&decision, target) else {
            return decision;
        };
        let Ok(grants) = self.grants.0.grants(cancel.clone()).await else {
            return decision;
        };
        if grants
            .iter()
            .any(|grant| grant_matches(grant, risk, target))
        {
            Decision::Allow
        } else {
            decision
        }
    }
    /// Writes `permission/changed` when `expected` is the last written
    /// version; returns the new version once it is dispatched.
    pub async fn update(
        &self,
        expected: u64,
        policy: PermissionPolicy,
    ) -> Result<u64, PermissionUpdateError> {
        let (waiter, dispatched) = oneshot::channel();
        let (mut conflict, mut registered) = (None, None);
        let committed = self.timeline.append_registered(
            EventKind::PermissionChanged(PermissionChangedData { policy }).into(),
            |event| {
                let mut inner = self.inner.lock().unwrap();
                if inner.closed {
                    return Err(AgentError::Closed("Permission".into()));
                }
                if inner.written != expected {
                    conflict = Some(inner.written);
                    return Err(AgentError::InvalidInput(
                        "permission version conflict".into(),
                    ));
                }
                inner.written = event.sequence;
                inner.waiters.insert(event.id.clone(), waiter);
                registered = Some(event.id.clone());
                Ok(())
            },
        );
        if let Err(error) = committed {
            if let Some(id) = registered {
                let mut inner = self.inner.lock().unwrap();
                inner.written = expected;
                inner.waiters.remove(&id);
            }
            return Err(match (conflict, error) {
                (Some(current), _) => PermissionUpdateError::Conflict { current },
                (None, AgentError::Closed(_) | AgentError::Cancelled(_)) => {
                    PermissionUpdateError::Closed
                }
                (None, error) => PermissionUpdateError::Failed(error.to_string()),
            });
        }
        dispatched.await.map_err(|_| PermissionUpdateError::Closed)
    }
    /// Folds the ancestor prefix, then this branch's own events.
    async fn rebuild(&self, local: &[Arc<AgentEvent>]) -> dim_agent_runtime::Result<()> {
        let inherited = self.timeline.history(Some(0)).await?;
        let mut state = self.state.get();
        for event in &inherited {
            if let EventKind::PermissionChanged(data) = &event.kind {
                state.policy = data.policy.clone();
            }
        }
        for event in local {
            if let EventKind::PermissionChanged(data) = &event.kind {
                state = PermissionState {
                    version: event.sequence,
                    policy: data.policy.clone(),
                };
            }
        }
        self.inner.lock().unwrap().written = state.version;
        self.state.set(state);
        Ok(())
    }
    fn consume(&self, event: &AgentEvent) {
        let EventKind::PermissionChanged(data) = &event.kind else {
            return;
        };
        let mut inner = self.inner.lock().unwrap();
        self.state.set(PermissionState {
            version: event.sequence,
            policy: data.policy.clone(),
        });
        if let Some(waiter) = inner.waiters.remove(&event.id) {
            let _ = waiter.send(event.sequence);
        }
    }
}

/// Risk categories declared by tool owners in `ToolDefinition.risk`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Risk {
    Read,
    Write,
    Execute,
    Network,
    Session,
    Unclassified,
}
impl Risk {
    fn of(risk: Option<&str>) -> Self {
        match risk {
            Some("read_filesystem") => Self::Read,
            Some("write_filesystem") => Self::Write,
            Some("execute_process") => Self::Execute,
            Some("network") => Self::Network,
            Some("session") => Self::Session,
            _ => Self::Unclassified,
        }
    }
    fn describe(self) -> &'static str {
        match self {
            Self::Read => "reading files",
            Self::Write => "writing files",
            Self::Execute => "starting processes",
            Self::Network => "network access",
            Self::Session => "session operations",
            Self::Unclassified => "unclassified tools",
        }
    }
}

/// Compares words; `prefix` matches whole leading words.
fn matches(rule: &CommandRule, command: &str) -> bool {
    let expected: Vec<_> = rule.command.split_whitespace().collect();
    let words: Vec<_> = command.split_whitespace().collect();
    match rule.matching {
        CommandMatch::Exact => words == expected,
        CommandMatch::Prefix => words.starts_with(&expected),
    }
}

/// `allowedTools`, then the preset by risk; `disallowedTools` comes first.
fn grant_matches(grant: &SessionGrant, risk: Risk, target: &std::path::Path) -> bool {
    if risk == Risk::Write && grant.mode != GrantMode::Write {
        return false;
    }
    match grant.kind {
        GrantKind::File => target == grant.path,
        GrantKind::Dir => target.starts_with(&grant.path),
    }
}

fn decide(policy: &PermissionPolicy, name: &str, risk: Option<&str>) -> Decision {
    if policy.allowed_tools.iter().any(|item| item == name) {
        return Decision::Allow;
    }
    let risk = Risk::of(risk);
    let (preset, verdict) = match policy.preset {
        PermissionPreset::FullAccess => return Decision::Allow,
        PermissionPreset::ReadOnly => (
            "read-only",
            match risk {
                Risk::Read | Risk::Session => return Decision::Allow,
                _ => false,
            },
        ),
        PermissionPreset::WorkspaceWrite => (
            "workspace-write",
            match risk {
                Risk::Read | Risk::Write | Risk::Session => return Decision::Allow,
                _ => true,
            },
        ),
    };
    let what = risk.describe();
    if verdict {
        Decision::Ask {
            reason: format!("Tool `{name}` requires approval for {what} under the {preset} preset"),
        }
    } else {
        Decision::Deny {
            reason: format!("Tool `{name}` is not allowed by the {preset} preset ({what})"),
        }
    }
}

pub struct PermissionPlugin {
    state: Key<StateCell<PermissionState>>,
    permission: Key<Permission>,
    config: Key<SessionConfig>,
    rules: Key<CommandRulesHost>,
    grants: Key<SessionGrantsHost>,
    timeline: Key<Timeline>,
    dispatcher: Key<Dispatcher>,
}
impl PermissionPlugin {
    pub fn new(names: &Names<'_>) -> Result<Self, Error> {
        Ok(Self {
            state: names.state_cell()?,
            permission: names.key()?,
            config: names.key()?,
            rules: names.key()?,
            grants: names.key()?,
            timeline: names.key()?,
            dispatcher: names.key()?,
        })
    }
}
impl DynPlugin for PermissionPlugin {
    fn name(&self) -> &str {
        "Permission"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.config.erase(),
            self.rules.erase(),
            self.grants.erase(),
            self.timeline.erase(),
            self.dispatcher.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let config = ctx.require_at(&self.config)?;
            let rules = ctx.require_at(&self.rules)?;
            let grants = ctx.require_at(&self.grants)?;
            let timeline = ctx.require_at(&self.timeline)?;
            let dispatcher = ctx.require_at(&self.dispatcher)?;
            let service = Arc::new(Permission {
                timeline,
                state: Arc::new(StateCell::new(PermissionState {
                    version: 0,
                    policy: config.permission_policy.clone(),
                })),
                rules,
                grants,
                config,
                inner: Mutex::new(Inner::default()),
            });
            let closing = service.clone();
            ctx.effect(move || async move {
                let mut inner = closing.inner.lock().unwrap();
                inner.closed = true;
                inner.waiters.clear();
                Ok(())
            })?;
            let owner = service.clone();
            dispatcher
                .follow(
                    ctx,
                    "Permission",
                    &["permission/changed"],
                    move |delivery| {
                        let owner = owner.clone();
                        Box::pin(async move {
                            match delivery {
                                Delivery::Replay(local) => owner.rebuild(&local).await?,
                                Delivery::Live(event) => owner.consume(&event),
                            }
                            Ok(())
                        })
                    },
                )
                .await?;
            ctx.provide_at(self.state.clone(), service.state.clone())?;
            ctx.provide_at(self.permission.clone(), service)?;
            Ok(Effect::Done)
        })
    }
}
