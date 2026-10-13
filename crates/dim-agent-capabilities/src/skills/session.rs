use super::{SkillSummary, SkillView, SkillsCatalog, SkillsError};
use crate::{
    host::{HostError, HostFuture, LoadPrompt, ToolExecutionContext},
    prompt_registry::{PromptSource, targets},
    prompt_sources::sha256_version,
    tool_registry::{ToolContribution, ToolHandler},
};
use dim_agent_runtime::{
    BranchLifecycle, BranchScope, Capability, ConfigureControl, ConfigureUpdate, InBranch,
    InSession, KeysOf, PromptSourceTarget, SessionConfig, SessionScope, scoped,
};
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// The prompt source of the Skill list, delivered as state.
pub const LIST: &str = "dim/skills/list";
/// The prompt source of how to use Skills, part of the system prompt.
pub const INSTRUCTIONS_SOURCE: &str = "dim/skills/instructions";

type View = Result<Arc<SkillView>, SkillsError>;

/// Identifies a view for change detection, including an unavailable one.
fn view_version(view: &View) -> String {
    match view {
        Ok(view) => view.version.clone(),
        Err(error) => format!("unavailable:{}", sha256_version(&error.to_string())),
    }
}

/// The Session's current Skill view, or why it is unavailable.
pub struct SessionSkills {
    current: Mutex<View>,
    /// Notified when the current view changes.
    changed: watch::Sender<()>,
}
impl SessionSkills {
    fn new(view: View) -> Self {
        Self {
            current: Mutex::new(view),
            changed: watch::Sender::new(()),
        }
    }
    pub fn current(&self) -> View {
        self.current.lock().unwrap().clone()
    }
    /// Enabled Skills of the current view, for input suggestions.
    pub fn list(&self) -> Result<Vec<SkillSummary>, SkillsError> {
        Ok(self
            .current()?
            .skills
            .values()
            .map(|skill| SkillSummary {
                id: skill.id.clone(),
                name: skill.name.clone(),
                description: skill.description.clone(),
            })
            .collect())
    }
    fn replace(&self, view: View) {
        let mut current = self.current.lock().unwrap();
        if view_version(&current) != view_version(&view) {
            *current = view;
            drop(current);
            self.changed.send_replace(());
        }
    }
}

/// Two prompt sources: how to use Skills, in the system prompt, and the
/// Skill list, state after the rules and the environment. The list escapes
/// its own SKILL.md values.
impl LoadPrompt for SessionSkills {
    fn load(
        &self,
        target: PromptSourceTarget,
        cancel: CancellationToken,
    ) -> HostFuture<'_, Option<PromptEntry>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled("prompt source load cancelled".into()));
            }
            if target.source == INSTRUCTIONS_SOURCE {
                return Ok(Some(PromptEntry {
                    source: target.source,
                    version: sha256_version(super::INSTRUCTIONS),
                    body: super::INSTRUCTIONS.into(),
                    scope: target.scope,
                    delivery: PromptDelivery::System,
                    order: 30,
                }));
            }
            let list = match self.current() {
                Ok(view) => view.list.clone(),
                Err(error) => Some(crate::context_block::text(&format!(
                    "Skills unavailable: {error}"
                ))),
            };
            Ok(list.map(|body| PromptEntry {
                source: target.source,
                version: sha256_version(&body),
                body,
                scope: target.scope,
                delivery: PromptDelivery::State,
                order: 300,
            }))
        })
    }
}

/// Session part: the current view, its list as a prompt source, and the
/// fixed `skill` tool.
pub struct SkillsPlugin {
    scope: SessionScope,
}
impl SkillsPlugin {
    pub fn new(scope: &SessionScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for SkillsPlugin {
    type Deps = (Arc<SkillsCatalog>, InSession<SessionConfig>);
    fn name(&self) -> &str {
        "Skills"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (catalog, InSession(config)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let cwd = PathBuf::from(&config.cwd);
            let lifecycle = ctx.cancellation_token();
            let failed = |error: super::SkillsError| CordisError::PluginFailed(Box::new(error));
            let mut changes = catalog.subscribe().map_err(failed)?;
            let initial = catalog
                .query(Some(&cwd))
                .await
                .map(|view| Arc::new(SkillView::from_catalog(&view)));
            let skills = Arc::new(SessionSkills::new(initial));
            let tasks = TaskTracker::new();
            let owner = skills.clone();
            let stopped = lifecycle.clone();
            tasks.spawn(async move {
                loop {
                    tokio::select! {
                        _ = stopped.cancelled() => return,
                        changed = changes.changed() => if changed.is_err() { return },
                    }
                    owner.replace(
                        catalog
                            .query(Some(&cwd))
                            .await
                            .map(|view| Arc::new(SkillView::from_catalog(&view))),
                    );
                }
            });
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        tasks.close();
                        tasks.wait().await;
                        Ok(())
                    })
                }))
            })?;
            ctx.provide_as(
                self.scope.named_key::<ToolContribution>("skill"),
                Arc::new(ToolContribution {
                    definition: skill_definition(),
                    handler: Arc::new(SkillTool {
                        skills: skills.clone(),
                        stopped: lifecycle,
                    }),
                }),
            )?;
            for source in [INSTRUCTIONS_SOURCE, LIST] {
                ctx.provide_as(
                    self.scope.named_key::<PromptSource>(source),
                    Arc::new(PromptSource(skills.clone())),
                )?;
            }
            ctx.provide_as(self.scope.key::<SessionSkills>(), skills)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for SkillsPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

/// TS `skillToolDefinition`.
fn skill_definition() -> ToolDefinition {
    ToolDefinition {
        risk: Some("session".into()),
        ..crate::tool_registry::definition(
            "skill",
            "Load and apply a named skill's full SKILL.md instructions. When the user's request matches an available skill, calling this tool is a BLOCKING REQUIREMENT. A request matches a skill when the user names the skill or the request clearly aligns with the skill name or description. Invoke this tool BEFORE generating any response about the task.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "skill_name": {
                        "type": "string",
                        "description": "Skill name to load from the listed skills catalog"
                    }
                },
                "required": ["skill_name"]
            }),
        )
    }
}

struct SkillTool {
    skills: Arc<SessionSkills>,
    stopped: CancellationToken,
}
impl ToolHandler for SkillTool {
    fn execute(
        &self,
        call: ToolCall,
        _: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled("skill load cancelled".into()));
            }
            let Some(name) = call.input.get("skill_name").and_then(|name| name.as_str()) else {
                return Ok(ToolResult::error(
                    "Invalid input: skill_name must be a string",
                ));
            };
            if self.stopped.is_cancelled() {
                return Err(HostError::Cancelled("skills reloading".into()));
            }
            let view = match self.skills.current() {
                Ok(view) => view,
                Err(error) => return Ok(failure(format!("Skills unavailable: {error}"))),
            };
            let skill = match view.skills.get(name) {
                Some(skill) => skill.clone(),
                None if view.disabled.contains(name) => {
                    return Ok(failure(format!("Skill is disabled: {name}")));
                }
                None => return Ok(failure(format!("Unknown skill: {name}"))),
            };
            // TS `loadSkill`.
            let mut parts = vec![
                format!("Skill: {}", skill.name),
                format!("Description: {}", skill.description),
                format!("File: {}", skill.file_path.display()),
            ];
            if !skill.allowed_tools.is_empty() {
                parts.push(format!("Allowed tools: {}", skill.allowed_tools.join(", ")));
            }
            parts.extend(["".into(), "Instructions:".into(), skill.body.clone()]);
            let content = parts.join("\n");
            let structured = serde_json::json!({
                "success": true,
                "name": skill.name,
                "description": skill.description,
                "allowedTools": skill.allowed_tools,
                "filePath": skill.file_path.to_string_lossy(),
            });
            Ok(ToolResult {
                structured_content: structured.as_object().cloned(),
                ..ToolResult::text(content)
            })
        })
    }
}

fn failure(message: String) -> ToolResult {
    let structured = serde_json::json!({ "success": false, "error": message });
    ToolResult {
        structured_content: structured.as_object().cloned(),
        ..ToolResult::error(message)
    }
}

/// Branch part: while the branch is loaded, has Prompt adopt the Skill list
/// when the Session's view changes. Prompt delivers the list at
/// `branch/start` and `branch/resume`.
pub struct SkillsBranch {
    skills: Arc<SessionSkills>,
}
impl SkillsBranch {
    /// The Session's view, for entry points that reach Skills through a branch.
    pub fn session(&self) -> Arc<SessionSkills> {
        self.skills.clone()
    }
}

pub struct SkillsBranchPlugin {
    scope: BranchScope,
}
impl SkillsBranchPlugin {
    pub fn new(scope: &BranchScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for SkillsBranchPlugin {
    type Deps = (
        InSession<SessionSkills>,
        InBranch<BranchLifecycle>,
        InBranch<dyn ConfigureControl>,
    );
    fn name(&self) -> &str {
        "SkillsBranch"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InSession(skills), InBranch(phase), InBranch(configure)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let lifecycle = ctx.cancellation_token();
            let mut changed = skills.changed.subscribe();
            changed.mark_unchanged();
            let tasks = TaskTracker::new();
            tasks.spawn(async move {
                // Background work begins once the branch is Active; a view
                // change before that stays marked in `changed`.
                tokio::select! {
                    _ = lifecycle.cancelled() => return,
                    active = phase.activated() => if !active { return },
                }
                loop {
                    tokio::select! {
                        _ = lifecycle.cancelled() => return,
                        _ = phase.closing() => return,
                        result = changed.changed() => if result.is_err() { return },
                    }
                    let update = ConfigureUpdate {
                        sources: targets([LIST]),
                        model: None,
                    };
                    tokio::select! {
                        _ = lifecycle.cancelled() => return,
                        result = configure.clone().configure(update) => if let Err(error) = result {
                            tracing::warn!(%error, "Skill list was not adopted");
                        },
                    }
                }
            });
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        tasks.close();
                        tasks.wait().await;
                        Ok(())
                    })
                }))
            })?;
            ctx.provide_as(
                self.scope.key::<SkillsBranch>(),
                Arc::new(SkillsBranch { skills }),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for SkillsBranchPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}
