//! Workspace tools, see `docs/architecture/rust-agent/plugins/WorkspaceTools.md`.
//!
//! `WorkspaceToolsPlugin` (Session) provides `S:ToolHost` executing `read`
//! `write` `edit` `glob` `grep` `exec`. Branch parts are looked up per call on
//! the calling branch: `FileReads` for file tools and `BackgroundExec` for a
//! command that moves to the background. Admission happened before a call
//! reaches this host; nothing here knows permissions, approvals or sandboxes.
pub mod background;
pub mod exec;
pub mod file_reads;
pub mod files;

use crate::{
    host::{HostError, HostFuture, ToolDriver, ToolExecutionContext, ToolHost},
    host_platform::HostPlatform,
    methods::{Interruption, MethodDecl, RpcError, SessionMethods, params},
    timeline_tree::TreeCommands,
};
use background::BackgroundExecPlugin;
use dim_agent_runtime::{
    AgentBuilder, AgentError, CAPABILITY_RELOADING, CollaborationRole, Names, SessionConfig,
    SessionIdentity,
};
use dim_agent_types::*;
use file_reads::{FileReads, FileReadsPlugin};
use files::{FileTools, FileWriteCoordinator};
use rutis::{BoxFuture, CordisError, Ctx, Effect, Keyed, TypeKey, TypedPlugin};
use std::{path::PathBuf, sync::Arc};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// Host tools this capability provides.
pub const TOOL_NAMES: [&str; 6] = ["read", "write", "edit", "glob", "grep", "exec"];
const OUTPUT_METHOD: &str = "session.exec.output";

/// Process-owned coordinator. Clone the plugin across loader generations so
/// in-flight mutations and newly loaded Sessions retain the same lock domain.
#[derive(Clone, Default)]
pub struct FileWriteCoordinatorPlugin(Arc<FileWriteCoordinator>);
impl TypedPlugin for FileWriteCoordinatorPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "FileWriteCoordinator"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as(TypeKey::of::<FileWriteCoordinator>(), self.0.clone())?;
            Ok(Effect::Done)
        })
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceToolsConfig {
    /// Absolute root of command output files, `<output_root>/<session_id>/`.
    pub output_root: PathBuf,
}

/// Registers the host tool names, WorkspaceTools and its branch parts.
pub fn install(builder: &mut AgentBuilder, config: WorkspaceToolsConfig) {
    builder
        .host_tools(TOOL_NAMES.map(String::from))
        .session_methods(OUTPUT_METHOD)
        .session_part(move |names| names.part(WorkspaceToolsPlugin::new(names, config.clone())?))
        .branch_part(|names| names.part(FileReadsPlugin::new(names)?))
        .branch_part(|names| names.part(BackgroundExecPlugin::new(names)?));
}

pub(crate) struct WorkspaceTools {
    platform: Arc<HostPlatform>,
    cwd: PathBuf,
    output_dir: PathBuf,
    /// Collaboration members keep commands in the foreground: the leader
    /// treats member idle as the end of a request, so a backgrounded command
    /// would finish after its result. Member asynchronous execution is a
    /// separate design.
    member: bool,
    tree: Arc<TreeCommands>,
    files: FileTools,
    /// Cancelled on unload: new calls settle as reloading, running ones stop.
    lifecycle: CancellationToken,
    /// Calls in flight, including a command being handed to BackgroundExec.
    calls: TaskTracker,
}

fn host_error(error: AgentError) -> HostError {
    match error {
        AgentError::Reloading(_) => HostError::Cancelled(CAPABILITY_RELOADING.into()),
        error => HostError::failed(error.to_string()),
    }
}

impl WorkspaceTools {
    async fn run(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> Result<ToolResult, HostError> {
        let _call = self.calls.token();
        if self.lifecycle.is_cancelled() {
            return Err(HostError::Cancelled(CAPABILITY_RELOADING.into()));
        }
        let branch = self.tree.get(&context.branch_id).map_err(host_error)?;
        // One token for the call and the unload of this instance.
        let combined = self.lifecycle.child_token();
        let link = async {
            cancel.cancelled().await;
            combined.cancel();
            std::future::pending::<()>().await
        };
        let work = async {
            if call.name == "exec" {
                exec::execute(self, &branch, &call, &context, combined.clone()).await
            } else if FileTools::handles(&call.name) {
                let reads = branch.service::<FileReads>().map_err(host_error)?;
                self.files.execute(&call, &reads, combined.clone()).await
            } else {
                Ok(ToolResult::error(format!("Unknown tool: {}", call.name)))
            }
        };
        let result = tokio::select! {
            result = work => result,
            _ = link => unreachable!("the link never completes"),
        };
        match result {
            Err(HostError::Cancelled(_)) if self.lifecycle.is_cancelled() => {
                Err(HostError::Cancelled(CAPABILITY_RELOADING.into()))
            }
            result => result,
        }
    }
}

impl ToolDriver for WorkspaceTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = FileTools::definitions();
        definitions.push(exec::definition_for(&self.platform));
        definitions
    }
    fn execute(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(self.run(call, context, cancel))
    }
}

struct WorkspaceToolsPlugin {
    host: rutis::DepKey<ToolHost>,
    methods: rutis::DepKey<SessionMethods>,
    config: WorkspaceToolsConfig,
}
impl WorkspaceToolsPlugin {
    fn new(names: &Names<'_>, config: WorkspaceToolsConfig) -> Result<Self, CordisError> {
        Ok(Self {
            host: names.key()?,
            methods: SessionMethods::key(names, OUTPUT_METHOD)?,
            config,
        })
    }
}
impl TypedPlugin for WorkspaceToolsPlugin {
    type Deps = (
        Keyed<HostPlatform>,
        Keyed<FileWriteCoordinator>,
        Keyed<SessionConfig>,
        Keyed<SessionIdentity>,
        Keyed<TreeCommands>,
    );
    fn name(&self) -> &str {
        "WorkspaceTools"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(platform), Keyed(coordinator), Keyed(config), Keyed(identity), Keyed(tree)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            // `session.create` validated and canonicalized the cwd.
            let cwd = PathBuf::from(&config.cwd);
            let output_dir = self.config.output_root.join(&identity.session_id);
            let tools = Arc::new(WorkspaceTools {
                files: FileTools::with_coordinator(
                    cwd.clone(),
                    platform.home.clone(),
                    output_dir.clone(),
                    coordinator,
                ),
                output_dir,
                member: config.collaboration_role == CollaborationRole::Member,
                cwd,
                platform,
                tree,
                lifecycle: CancellationToken::new(),
                calls: TaskTracker::new(),
            });
            let owner = tools.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        owner.lifecycle.cancel();
                        owner.calls.close();
                        owner.calls.wait().await;
                        Ok(())
                    })
                }))
            })?;
            let tree = tools.tree.clone();
            SessionMethods::provide_at(
                ctx,
                &self.methods,
                OUTPUT_METHOD,
                vec![MethodDecl::new(OUTPUT_METHOD, false, Interruption::Report)],
                Arc::new(move |_, value, _, _| {
                    let tree = tree.clone();
                    Box::pin(async move {
                        #[derive(serde::Deserialize)]
                        #[serde(rename_all = "camelCase", deny_unknown_fields)]
                        struct Params {
                            session_id: String,
                            branch_id: String,
                            task_id: String,
                        }
                        let Params {
                            session_id,
                            branch_id,
                            task_id,
                        } = params(value)?;
                        if session_id.is_empty() || branch_id.is_empty() || task_id.is_empty() {
                            return Err(RpcError::invalid(
                                "sessionId, branchId and taskId must not be empty",
                            ));
                        }
                        let text = tree
                            .get(&branch_id)?
                            .service::<background::BackgroundExec>()?
                            .output(&task_id)
                            .await
                            .map_err(|error| match error {
                                background::TaskError::Unknown => RpcError::app(
                                    -32000,
                                    "exec_task_unknown",
                                    "No live output is available for this task",
                                ),
                                background::TaskError::Closed => RpcError::reloading(),
                            })?;
                        Ok(serde_json::json!({"text":text}))
                    })
                }),
            )?;
            ctx.provide_as(self.host.key().clone(), Arc::new(ToolHost(tools)))?;
            Ok(Effect::Done)
        })
    }
}
