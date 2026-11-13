use crate::snapshot::Snapshot;
use dim_agent_capabilities::context_model::output_allowance;
use dim_agent_capabilities::host::HostError;
use dim_agent_types::{
    ModelCapabilities, ModelCapabilitySnapshot, ModelConfig, ModelId, ModelProtocol, ModelSelection,
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

#[derive(Clone, Debug)]
pub struct FunModel {
    pub provider_id: String,
    pub model_id: String,
    pub label: String,
    pub protocol: ModelProtocol,
    pub base_url: String,
    pub headers: HashMap<String, String>,
    pub preset: Option<String>,
    pub key_file: Option<PathBuf>,
    pub api_key_env: Option<String>,
    pub defaults: ModelConfig,
    pub context_window: u64,
    pub max_output_tokens: u64,
    pub output_allowance: u64,
    pub capabilities: ModelCapabilities,
}

#[derive(Clone, Debug)]
pub struct FunCatalog {
    pub revision: u64,
    default_model: ModelId,
    provider_labels: HashMap<String, String>,
    providers: HashMap<String, HashMap<String, FunModel>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSummary {
    pub revision: u64,
    pub providers: usize,
    pub models: usize,
    pub default_model: ModelIdSnapshotOut,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelIdSnapshotOut {
    pub provider_id: String,
    pub model_id: String,
}

impl Default for FunCatalog {
    fn default() -> Self {
        Self {
            revision: 0,
            default_model: ModelId {
                provider_id: String::new(),
                model_id: String::new(),
            },
            provider_labels: HashMap::new(),
            providers: HashMap::new(),
        }
    }
}

impl FunCatalog {
    pub fn from_snapshot(snapshot: Snapshot) -> Result<Self, String> {
        let mut providers: HashMap<String, HashMap<String, FunModel>> = HashMap::new();
        let mut seen_providers = HashSet::new();
        let mut provider_labels = HashMap::new();
        for provider in snapshot.providers {
            let provider_id = provider.id.trim().to_owned();
            if provider_id.is_empty() || !seen_providers.insert(provider_id.clone()) {
                return Err("provider ids must be nonempty and unique".into());
            }
            if !provider.enabled {
                continue;
            }
            let protocol = protocol(&provider.protocol)?;
            if protocol == ModelProtocol::Bedrock {
                return Err(format!("provider {provider_id} cannot use bedrock"));
            }
            let url = url::Url::parse(provider.base_url.trim())
                .map_err(|_| format!("provider {provider_id} has an invalid baseUrl"))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(format!("provider {provider_id} has an invalid baseUrl"));
            }
            for (name, value) in &provider.headers {
                if http::header::HeaderName::from_bytes(name.as_bytes()).is_err()
                    || http::header::HeaderValue::from_str(value).is_err()
                {
                    return Err(format!("provider {provider_id} has an invalid header"));
                }
            }
            let mut models = HashMap::new();
            for model in provider.models {
                let model_id = model.id.trim().to_owned();
                if model_id.is_empty() || models.contains_key(&model_id) {
                    return Err(format!(
                        "model ids for {provider_id} must be nonempty and unique"
                    ));
                }
                if model.context_window == 0 || model.max_output_tokens == 0 {
                    return Err(format!(
                        "{provider_id}/{model_id} needs positive contextWindow and maxOutputTokens"
                    ));
                }
                let allowance = output_allowance(model.context_window, model.max_output_tokens);
                let mut defaults = model.config.unwrap_or_default();
                defaults.max_tokens = Some(
                    defaults
                        .max_tokens
                        .map_or(allowance as f64, |max| max.min(allowance as f64)),
                );
                let mut capabilities =
                    ModelCapabilities::with_limits(model.context_window, model.max_output_tokens);
                if let Some(declared) = model.capabilities {
                    capabilities.tool_calling =
                        declared.tool_calling.unwrap_or(capabilities.tool_calling);
                    capabilities.reasoning = declared.reasoning.unwrap_or(capabilities.reasoning);
                    capabilities.vision = declared.vision.unwrap_or(capabilities.vision);
                }
                models.insert(
                    model_id.clone(),
                    FunModel {
                        provider_id: provider_id.clone(),
                        model_id,
                        label: model.label,
                        protocol,
                        base_url: provider.base_url.trim().to_owned(),
                        headers: provider.headers.clone(),
                        preset: provider.preset.clone(),
                        key_file: provider.key_file.clone(),
                        api_key_env: provider.api_key_env.clone(),
                        defaults,
                        context_window: model.context_window,
                        max_output_tokens: model.max_output_tokens,
                        output_allowance: allowance,
                        capabilities,
                    },
                );
            }
            if models.is_empty() {
                return Err(format!("provider {provider_id} declares no models"));
            }
            provider_labels.insert(provider_id.clone(), provider.label.clone());
            providers.insert(provider_id, models);
        }

        let default_model = ModelId {
            provider_id: snapshot.default_model.provider_id.trim().to_owned(),
            model_id: snapshot.default_model.model_id.trim().to_owned(),
        };
        let catalog = Self {
            revision: snapshot.revision,
            default_model,
            provider_labels,
            providers,
        };
        catalog
            .get(
                &catalog.default_model.provider_id,
                &catalog.default_model.model_id,
            )
            .map_err(|error| error.to_string())?;
        Ok(catalog)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        Self::from_snapshot(Snapshot::parse(bytes)?)
    }

    pub fn from_value(value: serde_json::Value) -> Result<Self, String> {
        Self::from_snapshot(Snapshot::from_value(value)?)
    }

    pub fn default_model(&self) -> &ModelId {
        &self.default_model
    }

    pub fn default_selection(&self) -> ModelSelection {
        let id = &self.default_model;
        let config = self
            .get(&id.provider_id, &id.model_id)
            .ok()
            .map(|model| model.defaults.clone());
        ModelSelection {
            provider_id: id.provider_id.clone(),
            model_id: id.model_id.clone(),
            capability_snapshot: None,
            config,
            metadata: None,
        }
    }

    pub fn get(&self, provider_id: &str, model_id: &str) -> Result<&FunModel, HostError> {
        self.providers
            .get(provider_id)
            .and_then(|models| models.get(model_id))
            .ok_or_else(|| {
                HostError::failed(format!("Model is not configured: {provider_id}/{model_id}"))
            })
    }

    pub fn model_count(&self) -> usize {
        self.providers.values().map(HashMap::len).sum()
    }

    pub fn selectable(&self) -> Vec<ModelCapabilitySnapshot> {
        let mut entries: Vec<(&str, &str, &FunModel)> = self
            .providers
            .iter()
            .flat_map(|(provider, models)| {
                models
                    .iter()
                    .map(move |(model, entry)| (provider.as_str(), model.as_str(), entry))
            })
            .collect();
        entries.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        entries
            .into_iter()
            .map(|(provider_id, model_id, model)| ModelCapabilitySnapshot {
                target: ModelId {
                    provider_id: provider_id.to_owned(),
                    model_id: model_id.to_owned(),
                },
                capabilities: model.capabilities.clone(),
                reasoning: None,
                service_tier: None,
            })
            .collect()
    }

    /// The read-only `provider.management` shape existing clients use for a
    /// model picker. It exposes no write operations and no account providers.
    pub fn management_view(&self) -> serde_json::Value {
        let mut providers = std::collections::BTreeMap::new();
        for (provider_id, models) in &self.providers {
            let mut entries: Vec<(&String, &FunModel)> = models.iter().collect();
            entries.sort_by_key(|(model_id, _)| *model_id);
            let models: Vec<serde_json::Value> = entries
                .into_iter()
                .map(|(model_id, model)| {
                    serde_json::json!({
                        "model": {
                            "modelId": model_id,
                            "displayName": if model.label.trim().is_empty() {
                                model_id.clone()
                            } else {
                                model.label.clone()
                            },
                            "capabilities": {
                                "contextWindow": model.context_window,
                                "maxOutputTokens": model.max_output_tokens,
                            }
                        },
                        "enabled": true,
                        "unavailableReason": null
                    })
                })
                .collect();
            let display_name = self
                .provider_labels
                .get(provider_id)
                .filter(|label| !label.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| provider_id.clone());
            providers.insert(
                provider_id.clone(),
                serde_json::json!({
                    "providerId": provider_id,
                    "displayName": display_name,
                    "source": "fun",
                    "version": 1,
                    "enabled": true,
                    "models": models
                }),
            );
        }
        serde_json::json!({
            "providers": providers.into_values().collect::<Vec<_>>(),
            "drivers": [],
            "errors": []
        })
    }

    pub fn summary(&self) -> CatalogSummary {
        CatalogSummary {
            revision: self.revision,
            providers: self.providers.len(),
            models: self.model_count(),
            default_model: ModelIdSnapshotOut {
                provider_id: self.default_model.provider_id.clone(),
                model_id: self.default_model.model_id.clone(),
            },
        }
    }
}

#[derive(Debug)]
pub struct FunCatalogState {
    current: RwLock<Arc<FunCatalog>>,
}

impl FunCatalogState {
    pub fn new(catalog: FunCatalog) -> Self {
        Self {
            current: RwLock::new(Arc::new(catalog)),
        }
    }

    pub fn current(&self) -> Arc<FunCatalog> {
        self.current.read().expect("fun provider catalog").clone()
    }

    /// Replace the catalog. A snapshot older than the current revision is
    /// refused so an out-of-order sync cannot roll the daemon back.
    pub fn replace(&self, catalog: FunCatalog) -> Result<CatalogSummary, String> {
        let mut current = self.current.write().expect("fun provider catalog");
        if catalog.revision < current.revision {
            return Err(format!(
                "provider snapshot revision {} is older than {}",
                catalog.revision, current.revision
            ));
        }
        let summary = catalog.summary();
        *current = Arc::new(catalog);
        Ok(summary)
    }

    pub fn summary(&self) -> CatalogSummary {
        self.current().summary()
    }
}

pub struct LoadedCatalog {
    pub state: Arc<FunCatalogState>,
    pub bytes: Vec<u8>,
}

pub fn load(path: &Path) -> Result<LoadedCatalog, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read provider snapshot {}: {error}", path.display()))?;
    let catalog = FunCatalog::from_bytes(&bytes)?;
    Ok(LoadedCatalog {
        state: Arc::new(FunCatalogState::new(catalog)),
        bytes,
    })
}

fn protocol(value: &str) -> Result<ModelProtocol, String> {
    serde_json::from_value(serde_json::Value::String(value.trim().to_owned()))
        .map_err(|_| format!("unknown protocol {value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot(revision: u64) -> serde_json::Value {
        json!({
            "version": 1,
            "revision": revision,
            "defaultModel": {"providerId": "local", "modelId": "one"},
            "providers": [{
                "id": "local",
                "protocol": "openai-compatible",
                "baseUrl": "https://example.com/v1",
                "keyFile": "/tmp/local.key",
                "models": [{
                    "id": "one",
                    "contextWindow": 128000,
                    "maxOutputTokens": 8192,
                    "config": {"maxTokens": 8192}
                }]
            }]
        })
    }

    #[test]
    fn parses_and_lists_a_snapshot() {
        let catalog = FunCatalog::from_value(snapshot(1)).unwrap();
        assert_eq!(catalog.default_model().provider_id, "local");
        assert_eq!(catalog.selectable().len(), 1);
        assert_eq!(catalog.get("local", "one").unwrap().output_allowance, 8192);
        assert!(catalog.get("local", "missing").is_err());
    }

    #[test]
    fn replaces_only_with_a_newer_revision() {
        let state = FunCatalogState::new(FunCatalog::from_value(snapshot(2)).unwrap());
        let older = FunCatalog::from_value(snapshot(1)).unwrap();
        assert!(state.replace(older).is_err());
        let newer = FunCatalog::from_value(snapshot(3)).unwrap();
        assert_eq!(state.replace(newer).unwrap().revision, 3);
    }
}
