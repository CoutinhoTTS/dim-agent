use crate::{CODEMODE_TOOL, config::CodeModeConfig, runs::CodeModeRuns};
use dim_agent_capabilities::{
    host::{HostError, HostFuture, ToolExecutionContext},
    tool_registry::{ToolContribution, ToolHandler, ToolRegistry, definition},
};
use dim_agent_runtime::{Names, SessionIdentity};
use dim_agent_types::{ToolResult, ToolSource};
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use rutis::{Ctx, Error};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const DESCRIPTION: &str = "Run a bounded JavaScript program in an isolated QuickJS context. The code is an async function body: use `await tools.call(name, arguments)`, `tools.list()` and `tools.describe(name)`; `return` a JSON-serializable value. Only tools from this request are callable; each call keeps its own Permission and Approval. The program cannot access the filesystem, network, process or host objects. The result is the returned value, serialized as JSON.";

pub struct CodeModeSessionPlugin {
    runs: Key<CodeModeRuns>,
    tools: Key<ToolRegistry>,
    identity: Key<SessionIdentity>,
    config: Arc<CodeModeConfig>,
}

impl CodeModeSessionPlugin {
    pub fn with_config(names: &Names<'_>, config: Arc<CodeModeConfig>) -> Result<Self, Error> {
        Ok(Self {
            runs: names.key()?,
            tools: names.key()?,
            identity: names.key()?,
            config,
        })
    }
}

impl DynPlugin for CodeModeSessionPlugin {
    fn name(&self) -> &str {
        "CodeModeSession"
    }

    fn injects(&self) -> Vec<TypeKey> {
        vec![self.runs.erase(), self.tools.erase(), self.identity.erase()]
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let runs = ctx.require_at(&self.runs)?;
            let tools = ctx.require_at(&self.tools)?;
            let identity = ctx.require_at(&self.identity)?;
            tools.register(
                ctx,
                ToolContribution {
                    definition: codemode_definition(&self.config),
                    handler: Arc::new(CodeModeTool {
                        runs,
                        session_id: identity.session_id.clone(),
                    }),
                },
            )?;
            Ok(Effect::Done)
        })
    }
}

fn codemode_definition(config: &CodeModeConfig) -> dim_agent_types::ToolDefinition {
    let mut definition = definition(
        CODEMODE_TOOL,
        DESCRIPTION,
        json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": config.max_code_bytes,
                    "description": "An async JavaScript function body. Use await tools.call(name, arguments); return a JSON-serializable value."
                },
                "timeoutMs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": config.max_runtime_ms,
                    "description": "Optional wall-clock budget, capped by the host maximum."
                },
                "maxCalls": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": config.max_calls,
                    "description": "Optional nested-call budget, capped by the host maximum."
                }
            },
            "required": ["code"],
            "additionalProperties": false
        }),
    );
    definition.source = ToolSource::Extension;
    definition.risk = Some("session".into());
    definition
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodeModeInput {
    code: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    max_calls: Option<u64>,
}

struct CodeModeTool {
    runs: Arc<CodeModeRuns>,
    session_id: String,
}

impl ToolHandler for CodeModeTool {
    fn execute(
        &self,
        call: dim_agent_types::ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            let input: CodeModeInput =
                serde_json::from_value(serde_json::Value::Object(call.input)).map_err(|error| {
                    HostError::failed(format!("Invalid codemode arguments: {error}"))
                })?;
            let runner = self
                .runs
                .get(&self.session_id, &context.branch_id)
                .ok_or_else(|| {
                    HostError::failed("Code Mode is not available on the calling branch")
                })?;
            runner
                .run(
                    input.code,
                    input.timeout_ms,
                    input.max_calls,
                    context,
                    cancel,
                )
                .await
        })
    }
}
