//! The unified background-task stop port.
//!
//! The branch's current task holder provides `dyn ExecStop`. Local assembly
//! provides it from BackgroundExec; minimal assembly provides it from
//! ShellSession. Neither knows the other; the protocol layer routes
//! `session.exec.stop` through this port.

use dim_agent_types::ExecEndedData;
use rutis::BoxFuture;
use std::sync::Arc;

/// Stop-path errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskError {
    Unknown,
    Closed,
}

/// Stops a background task by id.
pub trait ExecStop: Send + Sync + 'static {
    fn stop(
        self: Arc<Self>,
        task_id: String,
    ) -> BoxFuture<'static, Result<ExecEndedData, TaskError>>;
}
