use crate::{
    config::CodeModeConfig,
    host::NestedToolHost,
    interpreter::run_program,
    runs::{CodeModeRunner, CodeModeRuns},
};
use dim_agent_capabilities::{
    host::{HostError, ToolExecutionContext},
    tool_executor::ToolExecutor,
};
use dim_agent_runtime::{Names, TimelineIdentity};
use dim_agent_types::ToolResult;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use rutis::{Ctx, Error};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio_util::sync::CancellationToken;

pub struct CodeModeBranchPlugin {
    runs: Key<CodeModeRuns>,
    executor: Key<ToolExecutor>,
    identity: Key<TimelineIdentity>,
    config: Arc<CodeModeConfig>,
}

impl CodeModeBranchPlugin {
    pub fn with_config(names: &Names<'_>, config: Arc<CodeModeConfig>) -> Result<Self, Error> {
        Ok(Self {
            runs: names.key()?,
            executor: names.key()?,
            identity: names.key()?,
            config,
        })
    }
}

impl DynPlugin for CodeModeBranchPlugin {
    fn name(&self) -> &str {
        "CodeModeBranch"
    }

    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.runs.erase(),
            self.executor.erase(),
            self.identity.erase(),
        ]
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let runs = ctx.require_at(&self.runs)?;
            let executor: Arc<dyn NestedToolHost> = ctx.require_at(&self.executor)?;
            let identity = ctx.require_at(&self.identity)?;
            let branch = Arc::new(CodeModeBranch {
                executor,
                config: self.config.clone(),
            });
            let branch_id = identity.branch_id.clone();
            runs.insert(branch_id.clone(), branch);
            let cleanup = runs.clone();
            ctx.effect(move || async move {
                cleanup.remove(&branch_id);
                Ok(())
            })?;
            Ok(Effect::Done)
        })
    }
}

struct CodeModeBranch {
    executor: Arc<dyn NestedToolHost>,
    config: Arc<CodeModeConfig>,
}

impl CodeModeRunner for CodeModeBranch {
    fn run(
        &self,
        code: String,
        timeout_ms: Option<u64>,
        max_calls: Option<u64>,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, HostError>> + Send + '_>> {
        Box::pin(run_program(
            self.executor.clone(),
            self.config.clone(),
            code,
            timeout_ms,
            max_calls,
            context,
            cancel,
        ))
    }
}
