//! The Session part: the `schedule` tool, which every Session declares. The
//! handler refuses non-interactive Sessions and Collaboration members before
//! it calls the process service ([design §1.1, A.2](../../../docs/design/plugins/Scheduler.md)).
use crate::methods::{Caller, ScheduleJobs, ToolRequest};
use dim_agent_capabilities::{
    host::{HostFuture, ToolExecutionContext},
    tool_registry::{ToolContribution, ToolHandler, ToolRegistry, definition},
};
use dim_agent_runtime::{CollaborationRole, Names, SessionConfig, SessionIdentity};
use dim_agent_types::{ToolCall, ToolDefinition, ToolResult, ToolSource};
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const DESCRIPTION: &str = "Manage scheduled jobs across this host, including other workspaces. Every create/update, even a rename, replaces the saving branch and authority cap; future reports go to this branch. An omitted policy keeps its stored value, whose default can change with the saving origin. Inspect the old-to-new authority transition in the result.";

const UNAVAILABLE: &str = "schedule is available only in interactive leader Sessions.";

fn declaration() -> ToolDefinition {
    let mut tool = definition(
        "schedule",
        DESCRIPTION,
        json!({
            "type": "object",
            "required": ["action"],
            "additionalProperties": false,
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "get", "create", "update", "delete", "set_enabled", "run_now", "history"]
                },
                "job_id": {"type": "string"},
                "enabled": {"type": "boolean"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50},
                "job": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "name": {"type": "string"},
                        "schedule": {"type": "object"},
                        "prompt": {"type": "string"},
                        "cwd": {"type": "string"},
                        "model": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "provider_id": {"type": "string"},
                                "model_id": {"type": "string"}
                            }
                        },
                        "mode": {"type": "string", "enum": ["create", "reuse"]},
                        "policy": {"type": "string", "enum": ["read-only", "workspace-write", "full-access"]},
                        "max_run_minutes": {"type": "integer", "minimum": 1}
                    }
                }
            }
        }),
    );
    tool.source = ToolSource::Extension;
    tool.risk = Some("persistent_configuration".into());
    tool
}

pub struct SchedulerSession {
    jobs: Key<ScheduleJobs>,
    tools: Key<ToolRegistry>,
    config: Key<SessionConfig>,
    identity: Key<SessionIdentity>,
}

impl SchedulerSession {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            jobs: names.key()?,
            tools: names.key()?,
            config: names.key()?,
            identity: names.key()?,
        })
    }
}

impl DynPlugin for SchedulerSession {
    fn name(&self) -> &str {
        "SchedulerSession"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.jobs.erase(),
            self.tools.erase(),
            self.config.erase(),
            self.identity.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let jobs = ctx.require_at(&self.jobs)?;
            let tools = ctx.require_at(&self.tools)?;
            let config = ctx.require_at(&self.config)?;
            let identity = ctx.require_at(&self.identity)?;
            tools.register(
                ctx,
                ToolContribution {
                    definition: declaration(),
                    handler: Arc::new(ScheduleTool {
                        jobs,
                        config,
                        session_id: identity.session_id.clone(),
                    }),
                },
            )?;
            Ok(Effect::Done)
        })
    }
}

struct ScheduleTool {
    jobs: Arc<ScheduleJobs>,
    config: Arc<SessionConfig>,
    session_id: String,
}

impl ToolHandler for ScheduleTool {
    fn execute(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        _cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            if !self.config.interactive
                || self.config.collaboration_role == CollaborationRole::Member
            {
                return Ok(ToolResult::error(UNAVAILABLE));
            }
            let request: ToolRequest = match serde_json::from_value(Value::Object(call.input)) {
                Ok(request) => request,
                Err(error) => {
                    return Ok(ToolResult::error(format!(
                        "schedule: schedule_invalid\n{error}"
                    )));
                }
            };
            let caller = Caller {
                session_id: self.session_id.clone(),
                branch_id: context.branch_id,
                cwd: self.config.cwd.clone(),
            };
            Ok(match self.jobs.call(request, caller).await {
                Ok(reply) => {
                    let mut result = ToolResult::text(reply.content);
                    result.structured_content = reply.records.as_object().cloned();
                    result
                }
                Err(error) => {
                    ToolResult::error(format!("schedule: {}\n{}", error.code, error.message))
                }
            })
        })
    }
}
