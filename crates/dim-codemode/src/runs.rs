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

/// A branch id is session-local (`main` is every Session's default), so the
/// process map is keyed by both ids.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CodeModeRunKey {
    pub session_id: String,
    pub branch_id: String,
}

impl CodeModeRunKey {
    pub fn new(session_id: impl Into<String>, branch_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            branch_id: branch_id.into(),
        }
    }
}

/// The branch runners of one process. A branch part inserts its runner when
/// it mounts and removes it when it closes; the Session tool resolves the
/// caller's Session and branch here, never from Session state.
#[derive(Default)]
pub struct CodeModeRuns {
    runners: Mutex<HashMap<CodeModeRunKey, Arc<dyn CodeModeRunner>>>,
}

impl CodeModeRuns {
    pub fn get(&self, session_id: &str, branch_id: &str) -> Option<Arc<dyn CodeModeRunner>> {
        self.runners
            .lock()
            .unwrap()
            .get(&CodeModeRunKey::new(session_id, branch_id))
            .cloned()
    }

    pub fn insert(&self, key: CodeModeRunKey, runner: Arc<dyn CodeModeRunner>) {
        self.runners.lock().unwrap().insert(key, runner);
    }

    pub fn remove(&self, key: &CodeModeRunKey) {
        self.runners.lock().unwrap().remove(key);
    }
}
