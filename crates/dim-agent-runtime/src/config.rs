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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum BranchPhase {
    /// Its capabilities are loading; it may still fail and never start.
    #[default]
    Loading,
    /// Fully started and published: background work may begin.
    Active,
    Closing,
    Closed,
}

/// Phase of one Session's lifecycle, ordered like [`BranchPhase`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum SessionPhase {
    /// Its capabilities are loading; it may still fail and never be published.
    #[default]
    Loading,
    /// Built and published by its Agent: background work may begin.
    Active,
    Closing,
    Closed,
}

/// Monotonic phase behind both lifecycles: Loading < Active < Closing <
/// Closed, and a later phase is never replaced by an earlier one.
#[derive(Clone)]
struct Phases<P>(std::sync::Arc<tokio::sync::watch::Sender<P>>);
impl<P: Copy + Ord + Default + Send + Sync + 'static> Phases<P> {
    fn new() -> Self {
        Self(std::sync::Arc::new(
            tokio::sync::watch::channel(P::default()).0,
        ))
    }
    fn get(&self) -> P {
        *self.0.borrow()
    }
    fn watch(&self) -> tokio::sync::watch::Receiver<P> {
        self.0.subscribe()
    }
    async fn reach(&self, done: impl FnMut(&P) -> bool) -> Option<P> {
        let mut phase = self.0.subscribe();
        phase.wait_for(done).await.map(|phase| *phase).ok()
    }
    fn advance(&self, next: P) {
        self.0.send_if_modified(|phase| {
            let changed = next > *phase;
            if changed {
                *phase = next;
            }
            changed
        });
    }
}

/// Lifecycle of one branch: provided by its scope, advanced only by the
/// branch's [`crate::TimelineContext`]. Capabilities that spawn background
/// work in `apply` wait for [`Self::activated`] before doing it.
#[derive(Clone)]
pub struct BranchLifecycle(Phases<BranchPhase>);
impl BranchLifecycle {
    pub(crate) fn new() -> Self {
        Self(Phases::new())
    }
    pub fn phase(&self) -> BranchPhase {
        self.0.get()
    }
    pub fn watch(&self) -> tokio::sync::watch::Receiver<BranchPhase> {
        self.0.watch()
    }
    /// Waits until the branch leaves `Loading`: true when it became Active,
    /// false when it closes without ever starting.
    pub async fn activated(&self) -> bool {
        self.0.reach(|phase| *phase != BranchPhase::Loading).await == Some(BranchPhase::Active)
    }
    /// Waits until the branch starts closing; background work ends then,
    /// without waiting for its capability to unload.
    pub async fn closing(&self) {
        self.0.reach(|phase| *phase >= BranchPhase::Closing).await;
    }
    pub(crate) fn advance(&self, next: BranchPhase) {
        self.0.advance(next);
    }
}

/// Lifecycle of one Session: provided by its scope and advanced by its Agent.
/// Session capabilities that spawn background work in `apply`, such as
/// creating Session-owned instances or contacting external services, wait for
/// [`Self::activated`]. A Session whose assembly fails never activates, and
/// readiness of its assembly never depends on that work.
#[derive(Clone)]
pub struct SessionLifecycle(Phases<SessionPhase>);
impl SessionLifecycle {
    pub(crate) fn new() -> Self {
        Self(Phases::new())
    }
    pub fn phase(&self) -> SessionPhase {
        self.0.get()
    }
    /// Waits until the Session leaves `Loading`: true when it was published,
    /// false when its assembly ended without publication.
    pub async fn activated(&self) -> bool {
        self.0.reach(|phase| *phase != SessionPhase::Loading).await == Some(SessionPhase::Active)
    }
    /// Waits until the Session starts closing.
    pub async fn closing(&self) {
        self.0.reach(|phase| *phase >= SessionPhase::Closing).await;
    }
    pub(crate) fn advance(&self, next: SessionPhase) {
        self.0.advance(next);
    }
}
