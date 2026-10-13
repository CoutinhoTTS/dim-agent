use crate::config::{ModelConfigFile, ProviderKind};
use dim_agent_capabilities::host::HostError;
use dim_agent_runtime::ModelId;
use dim_agent_types::{ModelConfig, ModelRequest};
use std::collections::HashMap;

pub(crate) struct Model {
    pub base_url: String,
    pub headers: HashMap<String, String>,
    pub api_key_env: Option<String>,
    pub defaults: ModelConfig,
    pub kind: ProviderKind,
    pub preset: Option<String>,
}

/// Validated development catalog; credentials are resolved for each request.
pub struct AimuxModels {
    default_model: ModelId,
    providers: HashMap<String, HashMap<String, Model>>,
}
impl std::fmt::Debug for AimuxModels {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AimuxModels")
            .field("default_model", &self.default_model)
            .field("provider_count", &self.providers.len())
            .finish_non_exhaustive()
    }
}
impl AimuxModels {
    pub fn from_config(config: ModelConfigFile) -> Result<Self, HostError> {
        let mut providers = HashMap::new();
        for provider in config.providers {
            if provider.kind == ProviderKind::Bedrock {
                return Err(HostError::failed(
                    "Bedrock is not supported by the development model configuration",
                ));
            }
            if provider.id.is_empty() || providers.contains_key(&provider.id) {
                return Err(HostError::failed(
                    "Provider IDs must be nonempty and unique",
                ));
            }
            let url = url::Url::parse(&provider.base_url)
                .map_err(|_| HostError::failed(format!("Invalid baseUrl for {}", provider.id)))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(HostError::failed(format!(
                    "Invalid baseUrl for {}",
                    provider.id
                )));
            }
            for (name, value) in &provider.headers {
                if http::header::HeaderName::from_bytes(name.as_bytes()).is_err()
                    || http::header::HeaderValue::from_str(value).is_err()
                {
                    return Err(HostError::failed(format!(
                        "Invalid header for {}",
                        provider.id
                    )));
                }
            }
            let preset = provider.compatible.and_then(|value| value.preset);
            if let Some(preset) = &preset {
                if !matches!(provider.kind, ProviderKind::OpenaiCompatible) {
                    return Err(HostError::failed(
                        "compatible.preset requires openai-compatible",
                    ));
                }
                aimux_providers::provider_registry_entry(preset).ok_or_else(|| {
                    HostError::failed(format!("Unknown compatible preset: {preset}"))
                })?;
            }
            let mut models = HashMap::new();
            for entry in provider.models {
                if entry.id.is_empty() || models.contains_key(&entry.id) {
                    return Err(HostError::failed(format!(
                        "Model IDs for {} must be nonempty and unique",
                        provider.id
                    )));
                }
                if entry.config.max_tokens.is_none() {
                    return Err(HostError::failed(format!(
                        "{}.{}.config.maxTokens is required",
                        provider.id, entry.id
                    )));
                }
                if entry
                    .config
                    .temperature
                    .is_some_and(|value| !value.is_finite() || value < 0.)
                    || entry
                        .config
                        .top_p
                        .is_some_and(|value| !(0. ..=1.).contains(&value))
                    || entry.config.top_k.is_some_and(|value| {
                        !value.is_finite()
                            || value < 1.
                            || value.fract() != 0.
                            || value > 9_007_199_254_740_991.
                    })
                {
                    return Err(HostError::failed(format!(
                        "Invalid generation settings for {}/{}",
                        provider.id, entry.id
                    )));
                }
                let mut request = ModelRequest::new(&provider.id, &entry.id, Vec::new());
                request.config = Some(entry.config.clone());
                crate::request::convert(&request, &provider.kind, preset.as_deref())?;
                models.insert(
                    entry.id,
                    Model {
                        base_url: provider.base_url.clone(),
                        headers: provider.headers.clone(),
                        api_key_env: provider.api_key_env.clone(),
                        defaults: entry.config,
                        kind: provider.kind,
                        preset: preset.clone(),
                    },
                );
            }
            providers.insert(provider.id, models);
        }
        let models = Self {
            default_model: config.default_model,
            providers,
        };
        models.get(
            &models.default_model.provider_id,
            &models.default_model.model_id,
        )?;
        Ok(models)
    }

    pub fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    pub(crate) fn get(&self, provider: &str, model: &str) -> Result<&Model, HostError> {
        self.providers
            .get(provider)
            .and_then(|models| models.get(model))
            .ok_or_else(|| {
                HostError::failed(format!("Model is not configured: {provider}/{model}"))
            })
    }
}
