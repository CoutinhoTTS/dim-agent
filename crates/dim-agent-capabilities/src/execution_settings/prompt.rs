//! Model-facing execution settings: the sandbox and network switches as state.
//!
//! The App Server's `ExecutionSettings` owns the Settings document and
//! provides `ExecutionSettingsHost`. This capability registers
//! `dim/settings/execution` and adopts it when the Settings service reports a
//! committed write; an assembly without the host registers nothing.
use super::{ExecutionSettings, ExecutionSettingsHost};
use crate::{
    host::{HostError, HostFuture, LoadPrompt},
    prompt_registry::{PromptRegistry, targets},
    prompt_sources::sha256_version,
    settings::{SettingsChanged, settings_changes},
};
use dim_agent_runtime::{
    ConfigureControl, ConfigureUpdate, Names, PartRegistry, PartScope, PromptSourceTarget,
};
use dim_agent_types::*;
use rutis::{Ctx, Error};
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// The prompt source of the execution switches.
pub const PROMPT_SOURCE: &str = "dim/settings/execution";
/// The Settings source whose committed writes wake the projection.
const SETTINGS_SOURCE: &str = "execution";

/// The body: effective values and when they apply.
fn body(settings: ExecutionSettings) -> String {
    if settings.sandbox {
        format!(
            "Execution sandbox: on; network: {}. New processes only; a running shell keeps its original boundary.",
            if settings.network { "on" } else { "off" }
        )
    } else {
        "Execution sandbox: off. New processes run with the host's normal access.".into()
    }
}

struct SettingsSource {
    host: Arc<ExecutionSettingsHost>,
}
impl LoadPrompt for SettingsSource {
    fn load(
        &self,
        target: PromptSourceTarget,
        cancel: CancellationToken,
    ) -> HostFuture<'_, Option<PromptEntry>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled(
                    "execution settings load cancelled".into(),
                ));
            }
            let settings = self
                .host
                .0
                .read(cancel.child_token())
                .await
                .map_err(|error| HostError::failed(error.to_string()))?;
            let body = body(settings);
            Ok(Some(PromptEntry {
                source: target.source,
                version: sha256_version(&body),
                body,
                scope: target.scope,
                delivery: PromptDelivery::State,
                order: 240,
            }))
        })
    }
}

/// The Session and branch plugins this projection registers.
pub const SESSION_PART: &str = "ExecutionSettingsPrompt";
pub const BRANCH_PART: &str = "ExecutionSettingsPromptBranch";

/// Registers the projection on behalf of the process plugin that owns
/// `ExecutionSettingsHost`.
pub fn register_parts(ctx: &Ctx, parts: &Arc<PartRegistry>) -> Result<(), Error> {
    parts.register(ctx, PartScope::Session, SESSION_PART, |mount| {
        Ok(Some(Box::new(ProjectionPlugin::new(mount.names)?)))
    })?;
    parts.register(ctx, PartScope::Branch, BRANCH_PART, |mount| {
        Ok(Some(Box::new(ProjectionBranchPlugin::new(mount.names)?)))
    })
}

struct ProjectionPlugin {
    host: Option<Key<ExecutionSettingsHost>>,
    prompts: Key<PromptRegistry>,
}
impl ProjectionPlugin {
    fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            host: names
                .registered::<ExecutionSettingsHost>()
                .then(|| names.key::<ExecutionSettingsHost>())
                .transpose()?,
            prompts: names.key()?,
        })
    }
}
impl DynPlugin for ProjectionPlugin {
    fn name(&self) -> &str {
        "ExecutionSettingsPrompt"
    }
    fn injects(&self) -> Vec<TypeKey> {
        let mut deps = vec![self.prompts.erase()];
        deps.extend(self.host.iter().map(Key::erase));
        deps
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let prompts = ctx.require_at(&self.prompts)?;
            let Some(host) = &self.host else {
                return Ok(Effect::Done);
            };
            let host = ctx.require_at(host)?;
            prompts.register(ctx, PROMPT_SOURCE, Arc::new(SettingsSource { host }))?;
            Ok(Effect::Done)
        })
    }
}

struct ProjectionBranchPlugin {
    host: Option<Key<ExecutionSettingsHost>>,
    configure: Key<dyn ConfigureControl>,
}
impl ProjectionBranchPlugin {
    fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            host: names
                .registered::<ExecutionSettingsHost>()
                .then(|| names.key::<ExecutionSettingsHost>())
                .transpose()?,
            configure: names.key()?,
        })
    }
}
impl DynPlugin for ProjectionBranchPlugin {
    fn name(&self) -> &str {
        "ExecutionSettingsPromptBranch"
    }
    fn injects(&self) -> Vec<TypeKey> {
        let mut deps = vec![self.configure.erase()];
        deps.extend(self.host.iter().map(Key::erase));
        deps
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            if self.host.is_none() {
                return Ok(Effect::Done);
            }
            let configure = ctx.require_at(&self.configure)?;
            let tasks = TaskTracker::new();
            let owned = tasks.clone();
            ctx.listen::<SettingsChanged>()
                .name(settings_changes(SETTINGS_SOURCE))
                .on(move |_, _| {
                    let configure = configure.clone();
                    owned.spawn(async move {
                        let update = ConfigureUpdate {
                            sources: targets([PROMPT_SOURCE]),
                            model: None,
                        };
                        if let Err(error) = configure.configure(update).await {
                            tracing::warn!(%error, "execution settings were not adopted");
                        }
                    });
                    async { Ok(None) }
                })?;
            ctx.effect(move || async move {
                tasks.close();
                tasks.wait().await;
                Ok(())
            })?;
            Ok(Effect::Done)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_off_states_normal_access() {
        assert_eq!(
            body(ExecutionSettings::default()),
            "Execution sandbox: off. New processes run with the host's normal access."
        );
    }

    #[test]
    fn sandbox_on_states_network_and_the_running_shell() {
        assert_eq!(
            body(ExecutionSettings {
                sandbox: true,
                network: false
            }),
            "Execution sandbox: on; network: off. New processes only; a running shell keeps its original boundary."
        );
        assert_eq!(
            body(ExecutionSettings {
                sandbox: true,
                network: true
            }),
            "Execution sandbox: on; network: on. New processes only; a running shell keeps its original boundary."
        );
    }
}
