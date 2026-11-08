use dim_agent_capabilities::{
    host::{HostError, ToolExecutionContext},
    tool_executor::ToolExecutor,
};
use dim_agent_types::{ToolCall, ToolDefinition, ToolResult};
use std::{future::Future, pin::Pin};
use tokio_util::sync::CancellationToken;

/// The child-call surface a program sees. The production implementation is
/// `ToolExecutor`; a test supplies its own.
pub trait NestedToolHost: Send + Sync + 'static {
    /// The declarations the request pinned, without the `codemode` tool.
    fn definitions(&self, request_id: &str) -> Result<Vec<ToolDefinition>, String>;

    /// Runs one child call through the host's pinned binding and admission.
    fn invoke(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, HostError>> + Send + '_>>;
}

impl NestedToolHost for ToolExecutor {
    fn definitions(&self, request_id: &str) -> Result<Vec<ToolDefinition>, String> {
        ToolExecutor::definitions(self, request_id)
    }

    fn invoke(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, HostError>> + Send + '_>> {
        Box::pin(ToolExecutor::nested(self, call, context, cancel))
    }
}
