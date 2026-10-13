pub use dim_agent_types::ModelId;
use dim_agent_types::ModelSelection;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    pub name: String,
    pub system_prompt: String,
    pub model: ModelId,
}
impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: "dim-agent".into(),
            system_prompt: String::new(),
            model: ModelId {
                provider_id: "mock".into(),
                model_id: "deterministic".into(),
            },
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    #[serde(default)]
    pub permission_policy: crate::PermissionPolicy,
    pub cwd: String,
    pub model: ModelSelection,
    pub system_prompt: String,
    pub tools: Vec<String>,
    /// Fixed at creation; only the collaboration host creates members.
    #[serde(default)]
    pub collaboration_role: CollaborationRole,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollaborationRole {
    #[default]
    Leader,
    Member,
}
impl Default for SessionConfig {
    fn default() -> Self {
        let model = AgentConfig::default().model;
        Self {
            permission_policy: crate::PermissionPolicy::default(),
            cwd: ".".into(),
            model: ModelSelection {
                provider_id: model.provider_id,
                model_id: model.model_id,
                capability_snapshot: None,
                config: None,
                metadata: None,
            },
            system_prompt: String::new(),
            tools: vec!["echo".into(), "wait".into(), "branch".into()],
            collaboration_role: CollaborationRole::Leader,
        }
    }
}
#[derive(Debug, Clone)]
pub struct TimelineIdentity {
    pub session_id: String,
    pub branch_id: String,
}
#[derive(Debug, Clone)]
pub struct SessionIdentity {
    /// Set only for idle instance release, never for business close.
    pub unloading: tokio_util::sync::CancellationToken,
    pub session_id: String,
    /// True when this Session was loaded from storage rather than newly created.
    pub restored: bool,
}

/// Phase of one branch's lifecycle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BranchPhase {
    /// Its capabilities are loading; it may still fail and never start.
    #[default]
    Loading,
    /// Fully started and published: background work may begin.
    Active,
    Closing,
    Closed,
}

/// Lifecycle of one branch: provided by its scope, advanced only by the
/// branch's [`crate::TimelineContext`]. Capabilities that spawn background
/// work in `apply` wait for [`Self::activated`] before doing it.
#[derive(Clone)]
pub struct BranchLifecycle(std::sync::Arc<tokio::sync::watch::Sender<BranchPhase>>);
impl BranchLifecycle {
    pub(crate) fn new() -> Self {
        Self(std::sync::Arc::new(
            tokio::sync::watch::channel(BranchPhase::Loading).0,
        ))
    }
    pub fn phase(&self) -> BranchPhase {
        *self.0.borrow()
    }
    pub fn watch(&self) -> tokio::sync::watch::Receiver<BranchPhase> {
        self.0.subscribe()
    }
    /// Waits until the branch leaves `Loading`: true when it became Active,
    /// false when it closes without ever starting.
    pub async fn activated(&self) -> bool {
        let mut phase = self.0.subscribe();
        let reached = phase
            .wait_for(|phase| *phase != BranchPhase::Loading)
            .await
            .map(|phase| *phase);
        matches!(reached, Ok(BranchPhase::Active))
    }
    /// Waits until the branch starts closing; background work ends then,
    /// without waiting for its capability to unload.
    pub async fn closing(&self) {
        let mut phase = self.0.subscribe();
        let _ = phase
            .wait_for(|phase| matches!(phase, BranchPhase::Closing | BranchPhase::Closed))
            .await;
    }
    /// Advances only: a later phase is never replaced by an earlier one.
    pub(crate) fn advance(&self, next: BranchPhase) {
        self.0.send_if_modified(|phase| {
            let rank = |phase: BranchPhase| phase as u8;
            let changed = rank(next) > rank(*phase);
            if changed {
                *phase = next;
            }
            changed
        });
    }
}
