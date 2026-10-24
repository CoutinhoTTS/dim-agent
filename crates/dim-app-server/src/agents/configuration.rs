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
use rutis::{BoxFuture, CordisError, Ctx, DepKey, Effect, Keyed, TypedPlugin};
use serde_json::json;
use std::sync::Arc;

/// Agents' Session entry adopts profile configuration on the addressed branch.
pub(crate) struct AgentsSessionPlugin {
    methods: DepKey<SessionMethods>,
}
impl AgentsSessionPlugin {
    pub(crate) fn new(names: &Names<'_>) -> Result<Self, CordisError> {
        Ok(Self {
            methods: SessionMethods::key(names, SESSION_NAMESPACE)?,
        })
    }
}
impl TypedPlugin for AgentsSessionPlugin {
    type Deps = (Keyed<AgentIndex>, Keyed<TreeCommands>);
    fn name(&self) -> &str {
        "AgentsSession"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(index), Keyed(tree)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            SessionMethods::provide_at(
                ctx,
                &self.methods,
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
