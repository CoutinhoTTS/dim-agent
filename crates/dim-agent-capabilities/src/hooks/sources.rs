//! The Session port of the hook documents (Hooks A.2, appendix H): the
//! agent-side parts never name Settings or App Server.
use super::document::HooksDocument;
use crate::host::{HostError, HostFuture};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

/// One source of hook documents, in catalog order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HookSourceId {
    User,
    Project,
}
impl HookSourceId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

/// Whether a source's document was read, absent or refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookSourceState {
    Ok,
    Missing,
    Invalid(String),
}

/// One source as the port returns it.
#[derive(Clone, Debug)]
pub struct HookSourceSnapshot {
    pub id: HookSourceId,
    pub path: Option<PathBuf>,
    pub enabled: bool,
    pub state: HookSourceState,
    /// Parsed only when the state is `Ok`.
    pub document: Option<HooksDocument>,
}

/// Every effective source of one working directory, in catalog order.
#[derive(Clone, Debug, Default)]
pub struct HookSourcesSnapshot {
    pub sources: Vec<HookSourceSnapshot>,
}

/// The hook documents for a working directory. `cwd` resolves the project
/// source the same way the configuration owner authorizes it.
pub trait ReadHookSources: Send + Sync + 'static {
    /// Errors mean no source could be read; the hook point then runs nothing.
    fn sources(&self, cwd: &Path, cancel: CancellationToken)
    -> HostFuture<'_, HookSourcesSnapshot>;
    /// The product home of the App Server that runs the Session, which a hook
    /// reads as `DIMCODE_HOME`; `None` when the App Server has none.
    fn home(&self) -> Option<PathBuf>;
}

/// Session port of the hook sources; each profile binds one.
pub struct HookSourcesHost(pub Arc<dyn ReadHookSources>);
impl std::fmt::Debug for HookSourcesHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookSourcesHost").finish_non_exhaustive()
    }
}

/// The sources of a host that keeps no hook documents.
pub struct NoHookSources;
impl ReadHookSources for NoHookSources {
    fn sources(&self, _: &Path, _: CancellationToken) -> HostFuture<'_, HookSourcesSnapshot> {
        Box::pin(async { Ok(HookSourcesSnapshot::default()) })
    }
    fn home(&self) -> Option<PathBuf> {
        None
    }
}

impl HookSourcesHost {
    pub async fn read(
        &self,
        cwd: &Path,
        cancel: &CancellationToken,
    ) -> Result<HookSourcesSnapshot, HostError> {
        self.0.sources(cwd, cancel.clone()).await
    }
}
