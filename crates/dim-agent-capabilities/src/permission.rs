//! Branch permission: decides a tool call by tool name and risk category.
//! The policy is `SessionConfig.permission_policy` folded with this branch's
//! dispatched `permission/changed` events. The user's command rules, read
//! through the Session's `CommandRulesHost` on each decision, deny or approve
//! commands of process-starting calls.
use crate::host::{HostError, HostFuture};
use dim_agent_runtime::{
    AgentError, Delivery, Dispatcher, Names, SessionConfig, StateCell, Timeline,
};
use dim_agent_types::{
    AgentEvent, ApprovalCallData, COMMAND_SEPARATORS, CommandMatch, CommandRule,
    CommandRuleDecision, EventKind, PermissionChangedData, PermissionPolicy, PermissionPreset,
};
use rutis::{BoxFuture, CordisError, Ctx, DepKey, Effect, Keyed, TypedPlugin};
use std::{
    collections::HashMap,
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
    inner: Mutex<Inner>,
}
impl Permission {
    /// Decides with the policy dispatched when the decision is made and, for a
    /// command call, the rules read now. Only a cancelled read is an error.
    pub async fn decide(
        &self,
        call: &ApprovalCallData,
        risk: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<Decision, HostError> {
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
        // Taken after the read: a policy dispatched while it waited applies.
        let policy = self.state.get().policy;
        let name = &call.name;
        if policy.disallowed_tools.contains(name) {
            return Ok(Decision::Deny {
                reason: format!("Tool `{name}` is disallowed by the permission policy"),
            });
        }
        let (Some(command), Some(rules)) = (command, rules) else {
            return Ok(decide(&policy, name, risk));
        };
        let rules = match rules {
            Ok(rules) => rules,
            // A deny rule may exist.
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
        // An allow rule only answers the question; it never lifts a denial.
        Ok(match decide(&policy, name, risk) {
            Decision::Ask { .. }
                if !command.contains(COMMAND_SEPARATORS)
                    && rules.iter().any(|rule| {
                        rule.decision == CommandRuleDecision::Allow && matches(rule, command)
                    }) =>
            {
                Decision::Allow
            }
            decision => decision,
        })
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
            // The event was registered but not committed: nothing was written.
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
#[derive(Clone, Copy)]
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
    state: DepKey<StateCell<PermissionState>>,
    permission: DepKey<Permission>,
}
impl PermissionPlugin {
    pub fn new(names: &Names<'_>) -> Result<Self, CordisError> {
        Ok(Self {
            state: names.state_cell()?,
            permission: names.key()?,
        })
    }
}
impl TypedPlugin for PermissionPlugin {
    type Deps = (
        Keyed<SessionConfig>,
        Keyed<CommandRulesHost>,
        Keyed<Timeline>,
        Keyed<Dispatcher>,
    );
    fn name(&self) -> &str {
        "Permission"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(config), Keyed(rules), Keyed(timeline), Keyed(dispatcher)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Arc::new(Permission {
                timeline,
                state: Arc::new(StateCell::new(PermissionState {
                    version: 0,
                    policy: config.permission_policy.clone(),
                })),
                rules,
                inner: Mutex::new(Inner::default()),
            });
            let closing = service.clone();
            ctx.effect(move || {
                Effect::Disposer(Box::new(move || {
                    let mut inner = closing.inner.lock().unwrap();
                    inner.closed = true;
                    // Dropped waiters report `Closed`.
                    inner.waiters.clear();
                    Ok(())
                }))
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
            ctx.provide_as(self.state.key().clone(), service.state.clone())?;
            ctx.provide_as(self.permission.key().clone(), service)?;
            Ok(Effect::Done)
        })
    }
}
