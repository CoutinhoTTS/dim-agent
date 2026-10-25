//! The Session's prompt sources: the parts that provide one register it here
//! by name ([rutis usage R6](../../../docs/design/rutis-usage.md#r6-contributions-register-with-their-owner)).
//! It serves them as the `PromptHost` every branch's Prompt loads from, and
//! as the initial configuration that delivers all of them.
use crate::{
    host::{HostError, HostFuture, LoadPrompt, PromptHost},
    prompt::InitialPromptConfiguration,
};
use dim_agent_runtime::{ConfigureUpdate, Names, PromptSourceTarget, Registrations, SessionConfig};
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, DepKey, Effect, Keyed, TypedPlugin};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// The prompt sources registered in one Session.
#[derive(Default)]
pub struct PromptRegistry(Arc<Registrations<Arc<dyn LoadPrompt>>>);
impl PromptRegistry {
    /// Registers the source `name` on behalf of the part that owns `ctx`,
    /// until that part unloads. A name registered already fails.
    pub fn register(
        &self,
        ctx: &Ctx,
        name: impl Into<String>,
        source: Arc<dyn LoadPrompt>,
    ) -> Result<(), CordisError> {
        self.0.register(ctx, name, source).map(|_| ())
    }
    /// The names registered now, in name order.
    pub fn names(&self) -> Vec<String> {
        self.0
            .all()
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    }
}

impl LoadPrompt for PromptRegistry {
    fn load(
        &self,
        target: PromptSourceTarget,
        cancel: CancellationToken,
    ) -> HostFuture<'_, Option<PromptEntry>> {
        let Some(registration) = self.0.get(&target.source) else {
            return Box::pin(async move {
                Err(HostError::failed(format!(
                    "unknown prompt source: {}",
                    target.source
                )))
            });
        };
        Box::pin(async move { registration.value.load(target, cancel).await })
    }
}

/// Every source applies to the root branch and the branches derived from it.
pub fn targets<'a>(sources: impl IntoIterator<Item = &'a str>) -> Vec<PromptSourceTarget> {
    sources
        .into_iter()
        .map(|source| PromptSourceTarget {
            source: source.into(),
            scope: PromptScope::Branch {
                timeline: "main".into(),
            },
        })
        .collect()
}

pub struct PromptRegistryPlugin {
    registry: DepKey<PromptRegistry>,
    host: DepKey<PromptHost>,
    initial: DepKey<InitialPromptConfiguration>,
}
impl PromptRegistryPlugin {
    pub fn new(names: &Names<'_>) -> Result<Self, CordisError> {
        Ok(Self {
            registry: names.key()?,
            host: names.key()?,
            initial: names.key()?,
        })
    }
}
impl TypedPlugin for PromptRegistryPlugin {
    type Deps = (Keyed<SessionConfig>,);
    fn name(&self) -> &str {
        "PromptRegistry"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(config),): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let registry = Arc::new(PromptRegistry::default());
            // A branch adopts every source registered when it opens; a new
            // Session also adopts its selected model.
            let sources = registry.clone();
            let initial = InitialPromptConfiguration::read(move || {
                Some(ConfigureUpdate {
                    sources: targets(sources.names().iter().map(String::as_str)),
                    model: Some(config.model.clone()),
                })
            });
            ctx.provide_as(
                self.host.key().clone(),
                Arc::new(PromptHost(registry.clone())),
            )?;
            ctx.provide_as(self.initial.key().clone(), Arc::new(initial))?;
            ctx.provide_as(self.registry.key().clone(), registry)?;
            Ok(Effect::Done)
        })
    }
}
