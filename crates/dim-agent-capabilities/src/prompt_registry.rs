//! Collects the prompt sources Session capabilities provide by name into the
//! `PromptHost` every branch's Prompt loads from, and the initial
//! configuration that delivers all of them.
use crate::{
    host::{HostError, HostFuture, LoadPrompt, PromptHost},
    prompt::InitialPromptConfiguration,
};
use dim_agent_runtime::{
    Capability, ConfigureUpdate, Contributions, InSession, KeysOf, PromptSourceTarget,
    SessionConfig, SessionScope,
};
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{collections::BTreeMap, sync::Arc};
use tokio_util::sync::CancellationToken;

/// One prompt source a Session capability provides, under the name it is
/// registered with (`AgentBuilder::session_prompt_source`).
pub struct PromptSource(pub Arc<dyn LoadPrompt>);

struct Registry(BTreeMap<String, Arc<dyn LoadPrompt>>);
impl LoadPrompt for Registry {
    fn load(
        &self,
        target: PromptSourceTarget,
        cancel: CancellationToken,
    ) -> HostFuture<'_, Option<PromptEntry>> {
        match self.0.get(&target.source) {
            Some(source) => source.load(target, cancel),
            None => Box::pin(async move {
                Err(HostError::failed(format!(
                    "unknown prompt source: {}",
                    target.source
                )))
            }),
        }
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
    scope: SessionScope,
    names: Arc<[String]>,
}
impl PromptRegistryPlugin {
    pub fn new(scope: &SessionScope, names: &[String]) -> Self {
        Self {
            scope: *scope,
            names: names.into(),
        }
    }
}
impl TypedPlugin for PromptRegistryPlugin {
    type Deps = (InSession<SessionConfig>, Contributions<PromptSource>);
    fn name(&self) -> &str {
        "PromptRegistry"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InSession(config), Contributions(sources)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let sources: BTreeMap<_, _> = sources
                .into_iter()
                .map(|(name, source)| (name, source.0.clone()))
                .collect();
            // A new Session adopts every source and its selected model in
            // one root configuration.
            let initial = ConfigureUpdate {
                sources: targets(self.names.iter().map(String::as_str)),
                model: Some(config.model.clone()),
            };
            ctx.provide_as(
                self.scope.key::<PromptHost>(),
                Arc::new(PromptHost(Arc::new(Registry(sources)))),
            )?;
            ctx.provide_as(
                self.scope.key::<InitialPromptConfiguration>(),
                Arc::new(InitialPromptConfiguration(Some(initial))),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for PromptRegistryPlugin {
    fn keys(&self) -> KeysOf<Self> {
        (self.scope, (self.scope, self.names.clone()))
    }
}
