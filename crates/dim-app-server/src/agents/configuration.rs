use super::{AgentIndex, SESSION_NAMESPACE};
use crate::connection::{configuration_outcome, provider_error};
use dim_agent_capabilities::{
    methods::{Interruption, MethodDecl, RpcError, SessionMethods, params, reply},
    prompt::Prompt,
    prompt_sources,
    timeline_tree::TreeCommands,
};
use dim_agent_runtime::{Names, PromptSourceTarget};
use dim_agent_types::{ModelSelection, PromptScope};
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use serde_json::json;
use std::sync::Arc;

/// Agents' Session entry adopts profile configuration on the addressed branch.
pub(crate) struct AgentsSessionPlugin {
    index: Key<AgentIndex>,
    tree: Key<TreeCommands>,
    methods: Key<SessionMethods>,
}
impl AgentsSessionPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            index: names.key()?,
            tree: names.key()?,
            methods: names.key()?,
        })
    }
}
impl DynPlugin for AgentsSessionPlugin {
    fn name(&self) -> &str {
        "AgentsSession"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.index.erase(), self.tree.erase(), self.methods.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let index = ctx.require_at(&self.index)?;
            let tree = ctx.require_at(&self.tree)?;
            let methods = ctx.require_at(&self.methods)?;
            methods.register(
                ctx,
                SESSION_NAMESPACE,
                vec![MethodDecl::new(
                    "session.agent.update",
                    true,
                    Interruption::Report,
                )],
                Arc::new(move |_, value, cancel, _| {
                    let index = index.clone();
                    let tree = tree.clone();
                    Box::pin(async move {
                        #[derive(serde::Deserialize)]
                        #[serde(rename_all = "camelCase", deny_unknown_fields)]
                        struct Params {
                            #[serde(rename = "sessionId")]
                            _session_id: String,
                            branch_id: String,
                            agent_id: String,
                        }
                        let Params {
                            branch_id,
                            agent_id,
                            ..
                        } = params(value)?;
                        if branch_id != "main" {
                            return Err(RpcError::invalid(
                                "Agent configuration updates only support main",
                            ));
                        }
                        let profile = index
                            .host(Some(&agent_id))
                            .ok_or_else(crate::protocol::agent_unknown)?
                            .profile
                            .clone();
                        let model = match profile.model {
                            Some(target) if index.selects_models() => Some(
                                index
                                    .model_selection()
                                    .ok_or_else(RpcError::reloading)?
                                    .0
                                    .resolve(Some(target), None, cancel.clone())
                                    .await
                                    .map_err(provider_error)?,
                            ),
                            Some(target) => Some(ModelSelection {
                                provider_id: target.provider_id,
                                model_id: target.model_id,
                                capability_snapshot: None,
                                config: None,
                                metadata: None,
                            }),
                            None => None,
                        };
                        let scope = PromptScope::Branch {
                            timeline: branch_id.clone(),
                        };
                        let target = PromptSourceTarget {
                            source: prompt_sources::SESSION.into(),
                            scope: scope.clone(),
                        };
                        let entry = prompt_sources::session_entry(profile.system_prompt, scope);
                        let adoption = tree
                            .get(&branch_id)?
                            .service::<Prompt>()?
                            .configure_loaded(model, vec![(target, entry)]);
                        tokio::select! {
                            result = adoption => reply(json!({"outcome": configuration_outcome(result?)?})),
                            _ = cancel.cancelled() => Err(RpcError::interrupted()),
                        }
                    })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}
