use dim_agent_capabilities::host::{HostError, ToolExecutionContext};
use dim_agent_types::ToolResult;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

/// One branch's program executor, as the Session tool sees it.
pub trait CodeModeRunner: Send + Sync + 'static {
    /// Runs one program against the branch's pinned tools. The caller owns
    /// the outer call and its cancellation; every child call uses the same
    /// request identity.
    fn run(
        &self,
        code: String,
        timeout_ms: Option<u64>,
        max_calls: Option<u64>,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, HostError>> + Send + '_>>;
}

/// The branch runners of one process. A branch part inserts its runner when
/// it mounts and removes it when it closes; the Session tool resolves the
/// caller's branch here, never from Session state.
#[derive(Default)]
pub struct CodeModeRuns {
    runners: Mutex<HashMap<String, Arc<dyn CodeModeRunner>>>,
}

impl CodeModeRuns {
    pub fn get(&self, branch_id: &str) -> Option<Arc<dyn CodeModeRunner>> {
        self.runners.lock().unwrap().get(branch_id).cloned()
    }

    pub fn insert(&self, branch_id: String, runner: Arc<dyn CodeModeRunner>) {
        self.runners.lock().unwrap().insert(branch_id, runner);
    }

    pub fn remove(&self, branch_id: &str) {
        self.runners.lock().unwrap().remove(branch_id);
    }
}
