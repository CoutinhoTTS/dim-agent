//! Branch permission: decides a tool call by tool name and risk category.
//! The policy is `SessionConfig.permission_policy` folded with this branch's
//! dispatched `permission/changed` events.
use dim_agent_runtime::{
    AgentError, BranchScope, Capability, Dispatcher, InBranch, InSession, KeysOf, SessionConfig,
    StateCell, Timeline, scoped,
};
use dim_agent_types::{
    AgentEvent, EventKind, PermissionChangedData, PermissionPolicy, PermissionPreset,
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

/// The dispatched policy; `version` is the local sequence of the last
/// `permission/changed` of this branch, 0 when there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Default)]
struct Inner {
    closed: bool,
    /// Local sequence of the last written `permission/changed`.
    written: u64,
    /// Events up to this sequence were folded at load.
    rebuilt: u64,
    /// `update` waiters by event id, released when the event is dispatched.
    waiters: HashMap<String, oneshot::Sender<u64>>,
}

pub struct Permission {
    timeline: Arc<Timeline>,
    state: Arc<StateCell<PermissionState>>,
    inner: Mutex<Inner>,
}
impl Permission {
    /// Decides with the policy dispatched at the time of the call.
    pub fn decide(&self, name: &str, risk: Option<&str>) -> Decision {
        decide(&self.state.get().policy, name, risk)
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
    fn consume(&self, event: &AgentEvent) {
        let EventKind::PermissionChanged(data) = &event.kind else {
            return;
        };
        let mut inner = self.inner.lock().unwrap();
        if event.sequence <= inner.rebuilt {
            return;
        }
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

fn decide(policy: &PermissionPolicy, name: &str, risk: Option<&str>) -> Decision {
    if policy.disallowed_tools.iter().any(|item| item == name) {
        return Decision::Deny {
            reason: format!("Tool `{name}` is disallowed by the permission policy"),
        };
    }
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
    scope: BranchScope,
}
impl PermissionPlugin {
    pub fn new(scope: &BranchScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for PermissionPlugin {
    type Deps = (
        InSession<SessionConfig>,
        InBranch<Timeline>,
        InBranch<Dispatcher>,
    );
    fn name(&self) -> &str {
        "Permission"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InSession(config), InBranch(timeline), InBranch(dispatcher)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Arc::new(Permission {
                timeline: timeline.clone(),
                state: Arc::new(StateCell::new(PermissionState {
                    version: 0,
                    policy: config.permission_policy.clone(),
                })),
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
            // Subscribe, then fold the history while the gate holds handlers;
            // see the reload protocol.
            let gate = Arc::new(tokio::sync::Mutex::new(()));
            let rebuilding = gate.lock().await;
            let owner = service.clone();
            let handler_gate = gate.clone();
            dispatcher.subscribe(ctx, "Permission", &["permission/changed"], move |event| {
                let gate = handler_gate.clone();
                let owner = owner.clone();
                Box::pin(async move {
                    let _ready = gate.lock().await;
                    owner.consume(&event);
                    Ok(())
                })
            })?;
            // The ancestor prefix, then this branch's own events.
            let inherited = timeline.history(Some(0)).await?;
            let through = timeline.committed();
            let local = timeline.local(..=through).await?;
            let mut state = service.state.get();
            for event in &inherited {
                if let EventKind::PermissionChanged(data) = &event.kind {
                    state.policy = data.policy.clone();
                }
            }
            for event in &local {
                if let EventKind::PermissionChanged(data) = &event.kind {
                    state = PermissionState {
                        version: event.sequence,
                        policy: data.policy.clone(),
                    };
                }
            }
            {
                let mut inner = service.inner.lock().unwrap();
                inner.written = state.version;
                inner.rebuilt = local.last().map_or(0, |event| event.sequence);
            }
            service.state.set(state);
            drop(rebuilding);
            ctx.provide_as(
                self.scope.key::<StateCell<PermissionState>>(),
                service.state.clone(),
            )?;
            ctx.provide_as(self.scope.key::<Permission>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for PermissionPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}
