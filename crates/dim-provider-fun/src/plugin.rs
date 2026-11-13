use crate::catalog::FunCatalogState;
use dim_agent_capabilities::auxiliary_model::{self, AuxiliaryTarget};
use dim_agent_capabilities::context_model::window_facts;
use dim_agent_capabilities::host::{
    AuxiliaryError, AuxiliaryErrorCode, AuxiliaryReply, AuxiliaryRequest, CompleteAuxiliary,
    ContextModel, HostError, HostErrorKind, HostFuture, HttpModelConnection, ModalityConnection,
    ModalityConnectionHost, ModalityTarget, ModelConnection, ModelHost, ModelSelectionHost,
    PrepareModel, PrepareModelHost, ProviderConnectionResolver, ResolveContextModel,
    ResolveModalityConnection, ResolveModelConnection, ResolveModelSelection,
    ResolvedModelConnection, SelectableModels, SelectableModelsHost, undeclared_modality_operation,
};
use dim_agent_runtime::{AgentBuilder, Feature, Names, SessionConfig, SessionIdentity};
use dim_agent_types::{
    Message, ModelCapabilitySnapshot, ModelId, ModelRequest, ModelSelection, SystemPrompt,
    ToolDefinition,
};
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, ErrorKind, Key, TypeKey};
use std::{future::Future, pin::Pin, sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;

pub struct FunConnection {
    state: Arc<FunCatalogState>,
    generation: CancellationToken,
}

impl FunConnection {
    fn catalog(&self) -> Result<Arc<crate::catalog::FunCatalog>, HostError> {
        if self.generation.is_cancelled() {
            return Err(HostError::classified(
                HostErrorKind::OwnerClosed,
                "Provider catalog owner is closed",
            ));
        }
        Ok(self.state.current())
    }
}

impl ResolveModelSelection for FunConnection {
    fn resolve(
        &self,
        explicit: Option<ModelId>,
        _: Option<std::path::PathBuf>,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelSelection> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled("Model selection cancelled".into()));
            }
            let catalog = self.catalog()?;
            let id = explicit.unwrap_or_else(|| catalog.default_model().clone());
            let model = catalog.get(&id.provider_id, &id.model_id)?;
            Ok(ModelSelection {
                provider_id: id.provider_id,
                model_id: id.model_id,
                capability_snapshot: None,
                config: Some(model.defaults.clone()),
                metadata: Some(dim_agent_types::RequestMetadata {
                    extensions: serde_json::Map::from_iter([(
                        "providerProtocol".into(),
                        serde_json::json!(model.protocol),
                    )]),
                    ..Default::default()
                }),
            })
        })
    }
}

impl ResolveModelConnection for FunConnection {
    fn resolve(
        &self,
        request: Arc<ModelRequest>,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ResolvedModelConnection> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled(
                    "Connection resolution cancelled".into(),
                ));
            }
            let catalog = self.catalog()?;
            let model = catalog.get(&request.provider_id, &request.model_id)?;
            let credential = credential(model)?;
            Ok(ResolvedModelConnection {
                failure_policy: None,
                recovery: None,
                connection: ModelConnection::Http(HttpModelConnection {
                    protocol: model.protocol,
                    base_url: model.base_url.clone(),
                    network: Default::default(),
                    headers: model.headers.clone(),
                    bearer: if matches!(
                        model.protocol,
                        dim_agent_types::ModelProtocol::Anthropic
                            | dim_agent_types::ModelProtocol::Gemini
                    ) {
                        None
                    } else {
                        credential.clone()
                    },
                    api_key: if matches!(
                        model.protocol,
                        dim_agent_types::ModelProtocol::Anthropic
                            | dim_agent_types::ModelProtocol::Gemini
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

fn credential(model: &crate::catalog::FunModel) -> Result<Option<String>, HostError> {
    let file = model
        .key_file
        .as_deref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let environment = model
        .api_key_env
        .as_deref()
        .and_then(|name| std::env::var(name).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let credential = file.or(environment);
    if credential.is_none() && (model.key_file.is_some() || model.api_key_env.is_some()) {
        return Err(HostError::classified(
            HostErrorKind::InvalidRequest,
            format!(
                "Missing credential for {}/{}; set the API key in Fun Settings",
                model.provider_id, model.model_id
            ),
        ));
    }
    Ok(credential)
}

impl ResolveModalityConnection for FunConnection {
    fn resolve_for_modality(
        &self,
        target: ModalityTarget,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModalityConnection> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled("Modality connection cancelled".into()));
            }
            let catalog = self.catalog()?;
            catalog.get(&target.target.provider_id, &target.target.model_id)?;
            Err(undeclared_modality_operation(&target.operation))
        })
    }
}

impl SelectableModels for FunConnection {
    fn selectable(
        &self,
        cancel: CancellationToken,
    ) -> HostFuture<'_, Vec<ModelCapabilitySnapshot>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled("selectable models cancelled".into()));
            }
            Ok(self.catalog()?.selectable())
        })
    }

    fn selectable_for_modality(
        &self,
        cancel: CancellationToken,
    ) -> HostFuture<'_, Vec<dim_agent_types::ModalityModelSnapshot>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(HostError::Cancelled("selectable models cancelled".into()));
            }
            Ok(Vec::new())
        })
    }
}

pub struct FunContextModel {
    state: Arc<FunCatalogState>,
}

impl FunContextModel {
    pub fn new(state: Arc<FunCatalogState>) -> Self {
        Self { state }
    }
}

impl ResolveContextModel for FunContextModel {
    fn resolve(
        &self,
        request: Arc<ModelRequest>,
        _: CancellationToken,
    ) -> HostFuture<'_, Arc<dyn ContextModel>> {
        Box::pin(async move {
            let catalog = self.state.current();
            let model = catalog.get(&request.provider_id, &request.model_id)?;
            Ok(window_facts(
                model.context_window,
                model.output_allowance as f64,
                &request,
            ))
        })
    }
}

struct FunAuxiliary {
    state: Arc<FunCatalogState>,
    connection: Arc<FunConnection>,
    host: Arc<ModelHost>,
}

impl CompleteAuxiliary for FunAuxiliary {
    fn complete(
        &self,
        model: ModelId,
        request: AuxiliaryRequest,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<AuxiliaryReply, AuxiliaryError>> + Send + '_>> {
        Box::pin(async move {
            auxiliary_model::check(&request)?;
            let started = Instant::now();
            let result = match self
                .state
                .current()
                .get(&model.provider_id, &model.model_id)
            {
                Ok(entry) => {
                    let target = AuxiliaryTarget {
                        model: model.clone(),
                        config: entry.defaults.clone(),
                        capability_snapshot: None,
                    };
                    auxiliary_model::complete(
                        &self.host,
                        self.connection.as_ref(),
                        target,
                        request,
                        &cancel,
                    )
                    .await
                }
                Err(_) => Err(AuxiliaryError {
                    code: AuxiliaryErrorCode::ModelUnavailable,
                    message: "Model is unavailable".into(),
                }),
            };
            auxiliary_model::record(&model, started, result.as_ref());
            result
        })
    }
}

pub struct FunProviderPlugin {
    state: Arc<FunCatalogState>,
    host: Key<ModelHost>,
}

impl FunProviderPlugin {
    pub fn new(names: &Names<'_>, state: Arc<FunCatalogState>) -> rutis::Result<Self> {
        Ok(Self {
            state,
            host: names.key()?,
        })
    }
}

impl DynPlugin for FunProviderPlugin {
    fn name(&self) -> &str {
        "FunProvider"
    }

    fn injects(&self) -> Vec<TypeKey> {
        vec![self.host.erase()]
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let host = ctx.require_at(&self.host)?;
            ctx.provide_as::<FunCatalogState>(self.state.clone())?;
            let connection = Arc::new(FunConnection {
                state: self.state.clone(),
                generation: ctx.cancellation_token(),
            });
            ctx.provide(ProviderConnectionResolver(connection.clone()))?;
            ctx.provide(ModelSelectionHost(connection.clone()))?;
            ctx.provide(SelectableModelsHost(connection.clone()))?;
            ctx.provide(ModalityConnectionHost(connection.clone()))?;
            dim_agent_capabilities::auxiliary_model::provide(
                ctx,
                Arc::new(FunAuxiliary {
                    state: self.state.clone(),
                    connection,
                    host,
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}

pub struct FunModelPlugin {
    prepare: Key<PrepareModelHost>,
    catalog: Key<FunCatalogState>,
    config: Key<SessionConfig>,
    identity: Key<SessionIdentity>,
}

impl FunModelPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            prepare: names.key()?,
            catalog: names.key()?,
            config: names.key()?,
            identity: names.key()?,
        })
    }
}

impl DynPlugin for FunModelPlugin {
    fn name(&self) -> &str {
        "FunModel"
    }

    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.catalog.erase(),
            self.config.erase(),
            self.identity.erase(),
        ]
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let catalog = ctx.require_at(&self.catalog)?;
            let config = ctx.require_at(&self.config)?;
            let identity = ctx.require_at(&self.identity)?;
            let defaults = catalog
                .current()
                .get(&config.model.provider_id, &config.model.model_id)
                .map_err(|error| {
                    rutis::Error::from(ErrorKind::Validation {
                        issues: vec![error.to_string()],
                    })
                })?
                .defaults
                .clone();
            ctx.provide_at(
                self.prepare.clone(),
                Arc::new(PrepareModelHost(Arc::new(Prepare {
                    config,
                    identity,
                    defaults,
                }))),
            )?;
            Ok(Effect::Done)
        })
    }
}

struct Prepare {
    config: Arc<SessionConfig>,
    identity: Arc<SessionIdentity>,
    defaults: dim_agent_types::ModelConfig,
}

impl PrepareModel for Prepare {
    fn prepare(
        &self,
        turn_id: &str,
        request_id: &str,
        _: &[Arc<Message>],
        available_tools: &[ToolDefinition],
    ) -> Result<dim_agent_capabilities::host::ModelSettings, HostError> {
        let mut settings = dim_agent_capabilities::host::ModelSettings::new(
            &self.config.model.provider_id,
            &self.config.model.model_id,
        );
        settings.capability_snapshot = self.config.model.capability_snapshot.clone();
        settings.config = dim_agent_capabilities::context_model::adopted_config(
            settings.capability_snapshot.as_ref(),
            Some(self.config.model.config.as_ref().unwrap_or(&self.defaults)),
        );
        settings.system = (!self.config.system_prompt.is_empty())
            .then(|| SystemPrompt::Text(self.config.system_prompt.clone()));
        settings.tools = Some(available_tools.to_vec());
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

pub fn register_names(builder: &mut AgentBuilder) {
    dim_agent_runtime::names::register::<FunCatalogState>(builder.catalog_mut(), None);
}

pub fn install(builder: &mut AgentBuilder) {
    register_names(builder);
    builder.require_feature(Feature::new("FunModel").session("FunModel", |names| {
        Ok(Box::new(FunModelPlugin::new(names)?))
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Write as _;
    use std::path::PathBuf;

    fn model(key_file: Option<PathBuf>) -> crate::catalog::FunModel {
        crate::catalog::FunModel {
            provider_id: "local".into(),
            model_id: "one".into(),
            label: "one".into(),
            protocol: dim_agent_types::ModelProtocol::OpenaiCompatible,
            base_url: "https://example.com/v1".into(),
            headers: HashMap::new(),
            preset: None,
            key_file,
            api_key_env: None,
            defaults: dim_agent_types::ModelConfig::default(),
            context_window: 128_000,
            max_output_tokens: 8_192,
            output_allowance: 8_192,
            capabilities: dim_agent_types::ModelCapabilities::with_limits(128_000, 8_192),
        }
    }

    #[test]
    fn reads_the_key_file_on_every_resolve() {
        let path = std::env::temp_dir().join(format!(
            "dim-provider-fun-key-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"first-key\n")
            .unwrap();
        assert_eq!(
            credential(&model(Some(path.clone()))).unwrap().as_deref(),
            Some("first-key")
        );
        std::fs::write(&path, b"second-key\n").unwrap();
        assert_eq!(
            credential(&model(Some(path.clone()))).unwrap().as_deref(),
            Some("second-key")
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_missing_configured_key_is_an_error() {
        let path = std::env::temp_dir().join("dim-provider-fun-key-does-not-exist");
        let error = credential(&model(Some(path))).unwrap_err();
        assert!(error.to_string().contains("Missing credential"));
    }
}
