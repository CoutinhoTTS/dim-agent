//! The execution switches the host applies at process start. The storage is a
//! Settings source owned by the App Server; this port has no settings or file
//! dependency.
use rutis::host::BoxFuture;
use tokio_util::sync::CancellationToken;

pub const SOURCE: &str = "execution";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecutionSettings {
    pub sandbox: bool,
    pub network: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionSettingsError {
    #[error("execution settings are unavailable")]
    Unavailable,
    #[error("execution settings operation cancelled")]
    Cancelled,
    #[error("execution settings: {0}")]
    Failed(String),
}

pub trait ExecutionSettingsSource: Send + Sync + 'static {
    fn read(
        &self,
        cancel: CancellationToken,
    ) -> BoxFuture<'_, Result<ExecutionSettings, ExecutionSettingsError>>;
}

pub struct ExecutionSettingsHost(pub std::sync::Arc<dyn ExecutionSettingsSource>);
