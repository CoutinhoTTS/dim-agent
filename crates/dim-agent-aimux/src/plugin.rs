use crate::{AimuxModels, driver::AimuxDriver};
use dim_agent_capabilities::{host::*, tool_registry::ToolRegistry};
use dim_agent_runtime::{
    Capability, InSession, KeysOf, SessionConfig, SessionIdentity, SessionScope, scoped,
};
use dim_agent_types::{Message, SystemPrompt};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypeKey, TypedPlugin};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Process-owned request execution; no catalog or Session dependencies.
pub struct AimuxPlugin;
impl TypedPlugin for AimuxPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "Aimux"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let driver = AimuxDriver::new(ctx.cancellation_token())
                .map_err(|error| CordisError::PluginFailed(error.into()))?;
            let owned = driver.clone();
            if let Err(error) = ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        owned.close().await;
                        Ok(())
                    })
                }))
            }) {
                driver.close().await;
                return Err(error);
            }
            ctx.provide(ModelHost(driver))?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for AimuxPlugin {
    fn keys(&self) -> KeysOf<Self> {}
}

/// The API-key profile supplies model selection and request-local connections;
/// execution capabilities are independent of this provider choice.
pub struct ConfiguredModelsPlugin(pub Arc<AimuxModels>);
impl TypedPlugin for ConfiguredModelsPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "ConfiguredModels"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as(TypeKey::of::<AimuxModels>(), self.0.clone())?;
            let resolver = Arc::new(ConfiguredConnection {
                models: self.0.clone(),
                generation: ctx.cancellation_token(),
            });
            ctx.provide(ProviderConnectionResolver(resolver.clone()))?;
            ctx.provide(ModelSelectionHost(resolver))?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for ConfiguredModelsPlugin {
    fn keys(&self) -> KeysOf<Self> {}
}
struct ConfiguredConnection {
    models: Arc<AimuxModels>,
    generation: CancellationToken,
}
impl ResolveModelSelection for ConfiguredConnection {
    fn resolve(
        &self,
        explicit: Option<dim_agent_types::ModelId>,
        _: Option<std::path::PathBuf>,
        cancel: CancellationToken,
    ) -> HostFuture<'_, dim_agent_types::ModelSelection> {
        Box::pin(async move {
            if self.generation.is_cancelled() {
                return Err(HostError::classified(
                    HostErrorKind::OwnerClosed,
                    "Model catalog owner is closed",
                ));
            }
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled("Model selection cancelled".into()));
            }
            let id = explicit.unwrap_or_else(|| self.models.default_model().clone());
            let model = self.models.get(&id.provider_id, &id.model_id)?;
            Ok(dim_agent_types::ModelSelection {
                provider_id: id.provider_id,
                model_id: id.model_id,
                capability_snapshot: None,
                config: Some(model.defaults.clone()),
                metadata: None,
            })
        })
    }
}
impl ResolveModelConnection for ConfiguredConnection {
    fn resolve(
        &self,
        request: Arc<dim_agent_types::ModelRequest>,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ResolvedModelConnection> {
        Box::pin(async move {
            if self.generation.is_cancelled() {
                return Err(HostError::classified(
                    HostErrorKind::OwnerClosed,
                    "Model catalog owner is closed",
                ));
            }
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled(
                    "Connection resolution cancelled".into(),
                ));
            }
            let model = self.models.get(&request.provider_id, &request.model_id)?;
            let credential = model
                .api_key_env
                .as_deref()
                .map(|name| {
                    let key = std::env::var(name)
                        .ok()
                        .filter(|key| !key.is_empty())
                        .ok_or_else(|| {
                            HostError::classified(
                                HostErrorKind::InvalidRequest,
                                format!("Missing credential environment variable: {name}"),
                            )
                        })?;
                    http::HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| {
                        HostError::classified(
                            HostErrorKind::InvalidRequest,
                            "Invalid credential header",
                        )
                    })?;
                    Ok(key)
                })
                .transpose()?;
            Ok(ResolvedModelConnection {
                failure_policy: None,
                recovery: None,
                connection: ModelConnection::Http(HttpModelConnection {
                    protocol: model.kind,
                    base_url: model.base_url.clone(),
                    network: Default::default(),
                    headers: model.headers.clone(),
                    bearer: if matches!(
                        model.kind,
                        ModelProtocol::Anthropic | ModelProtocol::Gemini
                    ) {
                        None
                    } else {
                        credential.clone()
                    },
                    api_key: if matches!(
                        model.kind,
                        ModelProtocol::Anthropic | ModelProtocol::Gemini
                    ) {
                        credential
                    } else {
                        None
                    },
                    preset: model.preset.clone(),
                }),
            })
        })
    }
}

/// Request preparation stays on the calling side of the process adapter.
pub struct ConfiguredModelPlugin {
    scope: SessionScope,
}
impl ConfiguredModelPlugin {
    pub fn new(scope: &SessionScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for ConfiguredModelPlugin {
    type Deps = (
        Arc<AimuxModels>,
        InSession<SessionConfig>,
        InSession<SessionIdentity>,
        InSession<ToolRegistry>,
    );
    fn name(&self) -> &str {
        "ConfiguredModel"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (models, InSession(config), InSession(identity), InSession(tools)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let defaults = models
                .get(&config.model.provider_id, &config.model.model_id)
                .map_err(|error| CordisError::Validation {
                    issues: vec![error.to_string()],
                })?
                .defaults
                .clone();
            ctx.provide_as(
                self.scope.key::<PrepareModelHost>(),
                Arc::new(PrepareModelHost(Arc::new(Prepare {
                    config,
                    identity,
                    tools,
                    defaults,
                }))),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for ConfiguredModelPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}
struct Prepare {
    config: Arc<SessionConfig>,
    identity: Arc<SessionIdentity>,
    tools: Arc<ToolRegistry>,
    defaults: dim_agent_types::ModelConfig,
}
impl PrepareModel for Prepare {
    fn prepare(
        &self,
        turn_id: &str,
        request_id: &str,
        _: &[Message],
    ) -> Result<ModelSettings, HostError> {
        let mut settings =
            ModelSettings::new(&self.config.model.provider_id, &self.config.model.model_id);
        settings.capability_snapshot = self.config.model.capability_snapshot.clone();
        settings.config = Some(
            self.config
                .model
                .config
                .as_ref()
                .unwrap_or(&self.defaults)
                .clone(),
        );
        settings.system = (!self.config.system_prompt.is_empty())
            .then(|| SystemPrompt::Text(self.config.system_prompt.clone()));
        settings.tools = Some(self.tools.definitions());
        let mut metadata = self.config.model.metadata.clone().unwrap_or_default();
        metadata.session_id = Some(self.identity.session_id.clone());
        metadata.turn_id = Some(turn_id.into());
        metadata
            .extensions
            .insert("requestId".into(), request_id.into());
        settings.metadata = Some(metadata);
        Ok(settings)
    }
}
