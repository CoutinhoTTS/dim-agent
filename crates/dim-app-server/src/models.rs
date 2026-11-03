//! The `models` tool: the models this Session can use, with their provider
//! IDs, as Provider reports them (provider management §3.6).

use dim_agent_capabilities::{
    host::{HostError, HostFuture, SelectableModelsHost, ToolExecutionContext},
    tool_registry::{ToolContribution, ToolHandler, ToolRegistry, definition},
};
use dim_agent_runtime::{Names, PartRegistry, PartScope};
use dim_agent_types::{ModelCapabilitySnapshot, ToolCall, ToolDefinition, ToolResult, ToolSource};
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
use tokio_util::sync::CancellationToken;

/// Process row: it registers the Session part and owns nothing itself.
pub struct ModelsPlugin {
    parts: Key<PartRegistry>,
}

impl ModelsPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            parts: names.key()?,
        })
    }
}

impl DynPlugin for ModelsPlugin {
    fn name(&self) -> &str {
        "models"
    }

    fn injects(&self) -> Vec<TypeKey> {
        vec![self.parts.erase()]
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let parts = ctx.require_at(&self.parts)?;
            PartRegistry::register(&parts, ctx, PartScope::Session, "Models", |mount| {
                Ok(Some(Box::new(ModelsSessionPlugin::new(mount.names)?)))
            })?;
            Ok(Effect::Done)
        })
    }
}

/// Session part: the `models` tool.
pub struct ModelsSessionPlugin {
    tools: Key<ToolRegistry>,
    selectable: Key<SelectableModelsHost>,
}

impl ModelsSessionPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            tools: names.key()?,
            selectable: names.key()?,
        })
    }
}

impl DynPlugin for ModelsSessionPlugin {
    fn name(&self) -> &str {
        "Models"
    }

    fn injects(&self) -> Vec<TypeKey> {
        vec![self.tools.erase(), self.selectable.erase()]
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let tools = ctx.require_at(&self.tools)?;
            let selectable = ctx.require_at(&self.selectable)?;
            tools.register(
                ctx,
                ToolContribution {
                    definition: models_definition(),
                    handler: Arc::new(ModelsTool { selectable }),
                },
            )?;
            Ok(Effect::Done)
        })
    }
}

fn models_definition() -> ToolDefinition {
    let mut definition = definition(
        "models",
        "List the models this Session can use now, grouped by provider ID. Use it when the user asks which models are available or which provider to name.",
        json!({"type":"object","properties":{},"additionalProperties":false}),
    );
    definition.source = ToolSource::Extension;
    definition.risk = Some("session".into());
    definition
}

struct ModelsTool {
    selectable: Arc<SelectableModelsHost>,
}

impl ToolHandler for ModelsTool {
    fn execute(
        &self,
        _: ToolCall,
        _: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            let models = match self.selectable.0.selectable(cancel).await {
                Ok(models) => models,
                Err(error @ HostError::Cancelled(_)) => return Err(error),
                Err(error) => {
                    return Ok(ToolResult::error(format!(
                        "Models are unavailable: {error}"
                    )));
                }
            };
            Ok(ToolResult::text(list(&models)))
        })
    }
}

/// One heading per provider, in provider ID order, and one line per model in
/// the order the read reported them.
fn list(models: &[ModelCapabilitySnapshot]) -> String {
    if models.is_empty() {
        return "No models are available to this Session; sign in to a provider or add one first."
            .to_owned();
    }
    let mut by_provider: BTreeMap<&str, Vec<&ModelCapabilitySnapshot>> = BTreeMap::new();
    for model in models {
        by_provider
            .entry(model.target.provider_id.as_str())
            .or_default()
            .push(model);
    }
    let mut lines = vec![format!("Available models: {}", models.len())];
    for (provider, models) in by_provider {
        lines.push(String::new());
        lines.push(format!("provider {provider}"));
        for model in models {
            let capabilities = &model.capabilities;
            let mut line = format!(
                "  {}  context {}  output {}",
                model.target.model_id, capabilities.context_window, capabilities.max_output_tokens
            );
            if capabilities.vision {
                line.push_str("  vision");
            }
            if capabilities.reasoning {
                line.push_str("  reasoning");
            }
            lines.push(line);
        }
    }
    lines.join("\n")
}
