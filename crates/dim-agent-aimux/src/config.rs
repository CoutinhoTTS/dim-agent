use dim_agent_capabilities::host::HostError;
use dim_agent_runtime::ModelId;
use dim_agent_types::ModelConfig;
use serde::Deserialize;
use std::{collections::HashMap, path::Path};

/// Development catalog settings; credentials remain environment references.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConfigFile {
    pub default_model: ModelId,
    pub providers: Vec<ProviderConfig>,
}
impl ModelConfigFile {
    /// Loads a model configuration. A deserialization error reports only its
    /// line and column, never field values, so credentials in
    /// `providers[].headers` stay out of diagnostics.
    pub fn load(path: &Path) -> Result<Self, HostError> {
        let bytes = std::fs::read(path).map_err(|error| {
            HostError::failed(format!(
                "Cannot read model configuration {}: {error}",
                path.display()
            ))
        })?;
        serde_json::from_slice(&bytes).map_err(|error| {
            HostError::failed(format!(
                "Invalid model configuration {} at line {}, column {}",
                path.display(),
                error.line(),
                error.column()
            ))
        })
    }
}

pub use dim_agent_capabilities::host::ModelProtocol as ProviderKind;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    pub api_key_env: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub compatible: Option<CompatibleConfig>,
    pub models: Vec<ModelEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibleConfig {
    pub preset: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelEntry {
    pub id: String,
    /// The model's context window: the input and output tokens one request may
    /// occupy.
    pub context_window: Option<u64>,
    /// The model's output limit; a request's own `maxTokens` never sends more.
    pub max_output_tokens: Option<u64>,
    pub config: ModelConfig,
}
