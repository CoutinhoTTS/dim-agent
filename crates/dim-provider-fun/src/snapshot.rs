use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The provider/model snapshot Fun writes to `~/.fun/dim/providers.json`.
///
/// Fun is the only writer. The daemon validates the document and swaps it in
/// without restarting; a malformed document never replaces a working catalog.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    pub version: u32,
    #[serde(default)]
    pub revision: u64,
    pub default_model: ModelIdSnapshot,
    #[serde(default)]
    pub providers: Vec<ProviderSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelIdSnapshot {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSnapshot {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub protocol: String,
    pub base_url: String,
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Path to the 0600 key file. The snapshot never carries the key itself.
    #[serde(default)]
    pub key_file: Option<PathBuf>,
    /// Fallback environment variable for installations that still export a
    /// key. A key file wins over it.
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSnapshot {
    pub id: String,
    #[serde(default)]
    pub label: String,
    pub context_window: u64,
    pub max_output_tokens: u64,
    #[serde(default)]
    pub config: Option<dim_agent_types::ModelConfig>,
    #[serde(default)]
    pub capabilities: Option<CapabilitiesSnapshot>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilitiesSnapshot {
    #[serde(default)]
    pub tool_calling: Option<bool>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub vision: Option<bool>,
}

fn default_true() -> bool {
    true
}

impl Snapshot {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let snapshot: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid provider snapshot: {error}"))?;
        if snapshot.version != 1 {
            return Err(format!(
                "unsupported provider snapshot version {}",
                snapshot.version
            ));
        }
        Ok(snapshot)
    }

    pub fn from_value(value: serde_json::Value) -> Result<Self, String> {
        let snapshot: Self = serde_json::from_value(value)
            .map_err(|error| format!("invalid provider snapshot: {error}"))?;
        if snapshot.version != 1 {
            return Err(format!(
                "unsupported provider snapshot version {}",
                snapshot.version
            ));
        }
        Ok(snapshot)
    }
}
