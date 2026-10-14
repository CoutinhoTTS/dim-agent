//! Product-owned Skill enablement storage. Discovery and effective Skill views
//! belong to Skills; this port has no filesystem or app-server dependency.
use rutis::BoxFuture;
use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// Opaque current-intent revision, not a notification counter. A saved revision
/// survives restart; an applied-but-unsaved revision does not imply durability.
pub type SettingsVersion = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSettingsSnapshot {
    pub version: SettingsVersion,
    pub disabled: BTreeSet<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SkillSettingsError {
    #[error("Skill settings changed; read the current version before updating")]
    Conflict,
    #[error("invalid Skill settings: {0}")]
    Invalid(String),
    #[error("Skill settings operation cancelled")]
    Cancelled,
    #[error("Skill settings service is closed")]
    Closed,
    #[error("Skill settings: {0}")]
    Failed(String),
}

pub trait SkillSettings: Send + Sync + 'static {
    fn skills(
        &self,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<SkillSettingsSnapshot, SkillSettingsError>>;

    /// Change one ID while preserving other IDs and extension fields. A stale
    /// expected version conflicts even if the requested state already matches.
    fn update_skills(
        self: Arc<Self>,
        expected: SettingsVersion,
        id: String,
        enabled: bool,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<SettingsVersion, SkillSettingsError>>;

    /// Subscribe before reading `skills()`, then read again after notifications.
    /// Starts at `unread`, then carries current Skill intent revisions; use
    /// skills() for the authoritative version. Notifications may coalesce and
    /// do not certify persistence. Does not watch writes by other processes.
    fn changes(&self) -> watch::Receiver<SettingsVersion>;
}

/// Process service supplied by the product Settings plugin. Consumers declare
/// this key in `injects()`; they never depend on the concrete storage owner.
pub struct SkillSettingsHost(pub Arc<dyn SkillSettings>);
