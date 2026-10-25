//! Product catalog, static connections and model selection. OAuth plugins own
//! managed credentials; this plugin owns provider configuration and SQLite records.
mod auxiliary;
mod drivers;
mod legacy;
pub(crate) mod management;
mod new_api;
mod preferences;
mod remote;
mod secrets;
pub use legacy::ImportResult;
pub use preferences::ModelPreferences;
use secrets::StaticCredential;
pub use secrets::{ProviderSecret, ProviderSecretHost, ResolveProviderSecret};

use crate::{
    codex_oauth::OpenAICodexOAuthService,
    grok_oauth::GrokOAuthService,
    managed_oauth,
    managed_provider::{AccountSnapshot, CredentialLease, ManagedModel, ManagedProviders},
    model_catalog::{
        BundledCatalog, MAX_MODEL_TOKENS, ModelCapabilities, ModelEdit, ModelEntry, ModelMetadata,
        ModelSource, NewApiEndpoint, ProviderProjection,
    },
    storage::SqliteStorage,
};
use dim_agent_capabilities::host::*;
use dim_agent_runtime::{AgentError, Names, When, names::ServiceCatalog};
use dim_agent_types::{
    ModelCapabilitySnapshot, ModelConfig, ModelId, ModelRequest, ModelSelection,
};
use futures_util::future::BoxFuture as SharedFuture;
use reqwest::Client;
use rusqlite::{OptionalExtension, params};
use rutis::{BoxFuture, CordisError, Ctx, Effect, Keyed, Plugin, TypeKey, TypedPlugin};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModel {
    pub id: String,
    pub name: String,
    pub endpoint: Option<ModelProtocol>,
    pub config: ModelConfig,
    pub capabilities: ModelCapabilities,
    pub unavailable_reason: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub zdr: bool,
    pub metadata: Option<ModelMetadata>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderRecord {
    kind: ManagedRecordKind,
    account: AccountSnapshot,
    models: Vec<ManagedModel>,
    model_config_overrides: Vec<ModelEntry>,
    disabled_model_ids: Vec<String>,
    version: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ManagedRecordKind {
    Managed,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum PersistedProviderRecord {
    Managed(Box<ProviderRecord>),
    Static(Box<StaticProviderRecord>),
}
fn parse_static_row(
    id: &str,
    version: u32,
    body: &str,
) -> Result<Option<StaticProviderRecord>, HostError> {
    let record: PersistedProviderRecord =
        serde_json::from_str(body).map_err(|_| failure("Invalid persisted provider record"))?;
    match record {
        PersistedProviderRecord::Managed(record) => {
            validate_catalog(version, &record).map_err(failure)?;
            Ok(None)
        }
        PersistedProviderRecord::Static(mut record) => {
            if record.version != version {
                return Err(failure("Provider version mismatch"));
            }
            validate_static_record(id, &mut record)?;
            Ok(Some(*record))
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StaticProviderRecord {
    kind: StaticRecordKind,
    display_name: String,
    driver_kind: String,
    auth: StaticAuth,
    #[serde(default)]
    credential_authority: CredentialAuthority,
    base_url: String,
    #[serde(default)]
    network: ModelNetwork,
    #[serde(default)]
    headers: HashMap<String, String>,
    credential: Option<StaticCredential>,
    models: Vec<ModelEntry>,
    #[serde(default)]
    remote_models: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    endpoint_types: BTreeMap<String, NewApiEndpoint>,
    /// Derived at the persistence/input boundary, never read from stored JSON.
    #[serde(skip)]
    routes: HashMap<String, ModelProtocol>,
    /// Derived from an admitted Bedrock endpoint; never trusted from storage.
    #[serde(skip)]
    bedrock_region: Option<String>,
    #[serde(default)]
    model_config_overrides: Vec<ModelEntry>,
    #[serde(default)]
    disabled_model_ids: Vec<String>,
    enabled: bool,
    #[serde(default)]
    definition_fingerprint: Option<String>,
    version: u32,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum StaticRecordKind {
    Builtin,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialAuthority {
    #[default]
    Static,
    OpenaiCodex,
    Grok,
}

fn managed_authority(provider_id: &str) -> CredentialAuthority {
    match provider_id {
        "openai-codex" => CredentialAuthority::OpenaiCodex,
        "grok-build" => CredentialAuthority::Grok,
        _ => CredentialAuthority::Static,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum StaticAuth {
    ApiKey { header_name: String, prefix: String },
    Bearer,
    CustomHeader { header_name: String },
    Anonymous,
    AwsBedrock,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderInput {
    pub provider_id: String,
    pub display_name: String,
    pub driver_kind: String,
    pub base_url: String,
    pub auth: StaticAuth,
    #[serde(default, deserialize_with = "optional_non_null")]
    pub network: Option<ModelNetwork>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default, deserialize_with = "optional_non_null")]
    pub credential: Option<CredentialUpdate>,
    pub models: Option<Vec<ModelEntry>>,
    pub enabled: bool,
}

fn optional_non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}
impl std::fmt::Debug for ProviderInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderInput")
            .field("provider_id", &self.provider_id)
            .field("display_name", &self.display_name)
            .field("driver_kind", &self.driver_kind)
            .field("base_url", &"<redacted until validated>")
            .field("auth", &"<redacted until validated>")
            .field(
                "headers",
                &self.headers.as_ref().map(|h| h.keys().collect::<Vec<_>>()),
            )
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "<redacted>"),
            )
            .field("model_count", &self.models.as_ref().map(Vec::len))
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[derive(Clone, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase", deny_unknown_fields)]
pub enum CredentialUpdate {
    Keep,
    Clear,
    Set {
        value: String,
    },
    #[serde(rename = "set_aws")]
    SetAws {
        value: AwsCredentials,
    },
    #[serde(rename = "set_ref")]
    SetRef {
        reference: String,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummary {
    pub provider_id: String,
    pub source: &'static str,
    pub display_name: String,
    pub driver_kind: String,
    pub auth: StaticAuth,
    pub credential_authority: CredentialAuthority,
    pub base_url: String,
    pub proxy_configured: bool,
    pub timeout_ms: Option<u64>,
    pub header_names: Vec<String>,
    pub credential_configured: bool,
    pub enabled: bool,
    pub version: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProviderList {
    pub providers: Vec<ProviderSummary>,
    pub errors: Vec<ProviderRecordError>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRecordError {
    pub provider_id: String,
    pub message: String,
}

impl ProviderRecord {
    fn catalog(&self, _bundled: &BundledCatalog) -> ProviderCatalog {
        let defaults: Vec<_> = self
            .models
            .iter()
            .map(|model| model.entry.clone())
            .collect();
        let entries: Vec<_> = defaults
            .into_iter()
            .map(|entry| {
                self.model_config_overrides
                    .iter()
                    .find(|m| m.model_id == entry.model_id)
                    .cloned()
                    .unwrap_or(entry)
            })
            .collect();
        ProviderCatalog {
            account: Some(self.account.clone()),
            version: self.version,
            disabled_model_ids: self.disabled_model_ids.clone(),
            models: entries
                .into_iter()
                .filter_map(|entry| {
                    // A disappeared remote model retains its override but cannot
                    // acquire a new account route from stale user metadata.
                    let source = self
                        .models
                        .iter()
                        .find(|model| model.entry.model_id == entry.model_id)?;
                    Some(ProviderModel {
                        id: entry.model_id,
                        name: entry.display_name,
                        endpoint: Some(source.protocol),
                        config: ModelConfig {
                            max_tokens: Some(entry.capabilities.max_output_tokens as f64),
                            ..Default::default()
                        },
                        capabilities: entry.capabilities,
                        unavailable_reason: source.unavailable_reason.clone(),
                        zdr: source.zdr,
                        metadata: entry.metadata,
                    })
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCatalog {
    pub account: Option<AccountSnapshot>,
    pub models: Vec<ProviderModel>,
    pub disabled_model_ids: Vec<String>,
    pub version: u32,
}
impl ProviderCatalog {
    fn selectable(&self, id: &str) -> Option<&ProviderModel> {
        self.models.iter().find(|model| {
            model.id == id
                && model.unavailable_reason.is_none()
                && !self.disabled_model_ids.contains(&model.id)
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderSelection {
    pub provider_id: String,
    pub model_id: String,
    pub version: u32,
}

fn read_static(
    connection: &rusqlite::Connection,
    provider_id: &str,
) -> dim_agent_runtime::Result<Option<StaticProviderRecord>> {
    let row: Option<(u32, String)> = connection
        .query_row(
            "SELECT version,record FROM providers WHERE provider_id=?1",
            [provider_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    row.map(|(version, body)| parse_static(provider_id, version, &body))
        .transpose()
        .map_err(storage)
}

fn parse_static(
    provider_id: &str,
    version: u32,
    body: &str,
) -> Result<StaticProviderRecord, HostError> {
    let mut record: StaticProviderRecord =
        serde_json::from_str(body).map_err(|_| failure("Invalid persisted provider record"))?;
    if record.version != version {
        return Err(failure("Provider version mismatch"));
    }
    validate_static_record(provider_id, &mut record)?;
    Ok(record)
}

fn validate_static_record(
    provider_id: &str,
    record: &mut StaticProviderRecord,
) -> Result<(), HostError> {
    if record.credential_authority != managed_authority(provider_id) {
        return Err(failure("Invalid provider credential authority"));
    }
    let managed = record.credential_authority != CredentialAuthority::Static;
    if managed {
        let (driver, base_url) = match record.credential_authority {
            CredentialAuthority::OpenaiCodex => {
                ("openai-codex", "https://chatgpt.com/backend-api/codex")
            }
            CredentialAuthority::Grok => ("xai-grok-build", "https://api.x.ai/v1"),
            CredentialAuthority::Static => unreachable!(),
        };
        if record.kind != StaticRecordKind::Builtin
            || record.auth != StaticAuth::Bearer
            || record.driver_kind != driver
            || record.base_url != base_url
            || !record.headers.is_empty()
            || record.credential.is_some()
        {
            return Err(failure("Managed provider binding is read-only"));
        }
    } else if matches!(
        record.driver_kind.as_str(),
        "openai-codex" | "xai-grok-build"
    ) {
        return Err(failure(
            "Subscription driver requires its managed authority",
        ));
    }
    if provider_id.trim().is_empty() || record.display_name.trim().is_empty() {
        return Err(failure("Provider identity and name must not be empty"));
    }
    if let Some(proxy) = &record.network.proxy {
        let url = reqwest::Url::parse(proxy).map_err(|_| failure("Invalid provider proxy URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err(failure("Provider proxy must be an HTTP or HTTPS root URL"));
        }
    }
    if record
        .network
        .timeout_ms
        .is_some_and(|value| value == 0 || value > dim_agent_types::JS_MAX_SAFE_INTEGER)
    {
        return Err(failure(
            "Provider timeout must be a positive safe JSON integer",
        ));
    }
    record.bedrock_region = None;
    if record.kind == StaticRecordKind::Custom || !record.base_url.is_empty() || record.enabled {
        let url = reqwest::Url::parse(&record.base_url)
            .map_err(|_| failure("Invalid provider base URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(failure(
                "Provider base URL must be HTTP(S) without credentials, query or fragment",
            ));
        }
        if record.driver_kind == "azure-openai"
            && (url.scheme() != "https"
                || !url.host_str().is_some_and(|host| {
                    host.ends_with(".openai.azure.com") || host.ends_with(".services.ai.azure.com")
                })
                || url.path().trim_end_matches('/') != "/openai")
        {
            return Err(failure(
                "Azure OpenAI base URL must be an official resource /openai endpoint",
            ));
        }
        if record.driver_kind == "amazon-bedrock" {
            let host = url.host_str().unwrap_or_default();
            let region = host
                .strip_prefix("bedrock-runtime.")
                .and_then(|value| value.strip_suffix(".amazonaws.com"))
                .filter(|value| {
                    !value.is_empty()
                        && value.bytes().all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                        })
                });
            if url.scheme() != "https"
                || url.port().is_some()
                || url.path() != "/"
                || region.is_none()
            {
                return Err(failure(
                    "Amazon Bedrock base URL must be an official regional runtime endpoint",
                ));
            }
            record.bedrock_region = region.map(str::to_owned);
        }
    }
    if (record.auth == StaticAuth::AwsBedrock) != (record.driver_kind == "amazon-bedrock") {
        return Err(failure(
            "AWS Bedrock auth is only valid for the Amazon Bedrock driver",
        ));
    }
    if (record.kind == StaticRecordKind::Custom || record.enabled)
        && !drivers::supported(&record.driver_kind)
    {
        return Err(failure("Unsupported provider driver"));
    }
    for models in [&record.models, &record.model_config_overrides] {
        let mut ids = HashSet::new();
        if models.iter().any(|model| {
            model.model_id.trim().is_empty()
                || model.display_name.trim().is_empty()
                || !ids.insert(&model.model_id)
                || model.capabilities.context_window == 0
                || model.capabilities.max_output_tokens == 0
                || model.capabilities.context_window > MAX_MODEL_TOKENS
                || model.capabilities.max_output_tokens > MAX_MODEL_TOKENS
        }) {
            return Err(failure(
                "Provider contains invalid or duplicate model descriptors",
            ));
        }
    }
    let credential_header = match &record.auth {
        StaticAuth::ApiKey { header_name, .. } | StaticAuth::CustomHeader { header_name } => {
            Some(header_name.as_str())
        }
        StaticAuth::Bearer => Some("Authorization"),
        StaticAuth::Anonymous | StaticAuth::AwsBedrock => None,
    };
    let mut names = HashSet::new();
    for (name, value) in &record.headers {
        let header = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| failure("Invalid HTTP header name"))?;
        let signer_owned = record.auth == StaticAuth::AwsBedrock
            && matches!(
                name.to_ascii_lowercase().as_str(),
                "authorization"
                    | "content-type"
                    | "host"
                    | "x-amz-date"
                    | "x-amz-content-sha256"
                    | "x-amz-security-token"
            );
        if !names.insert(header)
            || credential_header.is_some_and(|h| h.eq_ignore_ascii_case(name))
            || signer_owned
        {
            return Err(failure("Duplicate or conflicting credential header"));
        }
        reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| failure("Invalid HTTP header value"))?;
    }
    if let Some(name) = credential_header {
        reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| failure("Invalid credential header"))?;
    }
    if let StaticAuth::ApiKey { prefix, .. } = &record.auth {
        reqwest::header::HeaderValue::from_str(prefix)
            .map_err(|_| failure("Invalid credential header prefix"))?;
    }
    if let Some(value) = &record.credential {
        match value {
            StaticCredential::Inline(value) => value.validate(&record.auth)?,
            StaticCredential::Reference { reference } => {
                if reference.trim().is_empty() || reference.chars().any(char::is_control) {
                    return Err(failure("Invalid provider secret reference"));
                }
            }
        }
    } else if !managed && record.enabled && record.auth != StaticAuth::Anonymous {
        return Err(failure("Enabled provider credential is required"));
    }
    record.routes = drivers::routes(&record.driver_kind, &record.models, &record.endpoint_types)?;
    Ok(())
}

fn static_models(
    bundled: &BundledCatalog,
    provider_id: &str,
    record: &StaticProviderRecord,
    account_available: bool,
) -> Vec<ProviderModel> {
    bundled
        .effective_models(
            &ProviderProjection {
                provider_id: provider_id.to_owned(),
                source: Some(if record.kind == StaticRecordKind::Builtin {
                    ModelSource::Builtin
                } else {
                    ModelSource::Custom
                }),
                ..Default::default()
            },
            &record.models,
            &record.model_config_overrides,
        )
        .into_iter()
        .filter(|entry| record.models.iter().any(|m| m.model_id == entry.model_id))
        .map(|entry| {
            let unavailable =
                matches!(entry.status, crate::model_catalog::ModelStatus::Unavailable)
                    || !entry.capabilities.modalities.iter().any(|m| m == "text")
                    || (record.driver_kind != "new-api-router"
                        && entry
                            .metadata
                            .as_ref()
                            .and_then(|m| m.raw.get("type"))
                            .and_then(Value::as_str)
                            .is_some_and(|kind| kind != "chat"))
                    || !record.enabled
                    || !record.routes.contains_key(&entry.model_id)
                    || !account_available
                    || (record.credential_authority == CredentialAuthority::Static
                        && record.auth != StaticAuth::Anonymous
                        && record.credential.is_none());
            let endpoint = record.routes.get(&entry.model_id).copied();
            ProviderModel {
                id: entry.model_id,
                name: entry.display_name,
                endpoint,
                config: ModelConfig {
                    max_tokens: Some(entry.capabilities.max_output_tokens as f64),
                    ..Default::default()
                },
                capabilities: entry.capabilities,
                unavailable_reason: unavailable.then(|| "provider_unavailable".into()),
                zdr: false,
                metadata: entry.metadata,
            }
        })
        .collect()
}

fn static_summary(provider_id: String, record: StaticProviderRecord) -> ProviderSummary {
    ProviderSummary {
        provider_id,
        source: if record.kind == StaticRecordKind::Builtin {
            "builtin"
        } else {
            "custom"
        },
        display_name: record.display_name,
        driver_kind: record.driver_kind,
        auth: record.auth,
        credential_authority: record.credential_authority,
        base_url: record.base_url,
        header_names: {
            let mut names: Vec<_> = record.headers.into_keys().collect();
            names.sort();
            names
        },
        credential_configured: record.credential.is_some(),
        proxy_configured: record.network.proxy.is_some(),
        timeout_ms: record.network.timeout_ms,
        enabled: record.enabled,
        version: record.version,
    }
}

fn static_catalog(
    bundled: &BundledCatalog,
    provider_id: &str,
    record: &StaticProviderRecord,
    account_available: bool,
) -> ProviderCatalog {
    ProviderCatalog {
        account: None,
        models: static_models(bundled, provider_id, record, account_available),
        disabled_model_ids: record.disabled_model_ids.clone(),
        version: record.version,
    }
}

pub struct ProviderService {
    db: Arc<SqliteStorage>,
    oauth: Arc<ManagedProviders>,
    codex: Arc<OpenAICodexOAuthService>,
    grok: Arc<GrokOAuthService>,
    secret_host: Option<Arc<ProviderSecretHost>>,
    bundled: Arc<BundledCatalog>,
    client: Client,
    stop: CancellationToken,
    tasks: TaskTracker,
    admission: Mutex<()>,
}
fn failure(error: impl std::fmt::Display) -> HostError {
    HostError::failed(error.to_string())
}
fn storage(error: impl std::fmt::Display) -> AgentError {
    AgentError::Storage(error.to_string())
}
fn same_account(a: &AccountSnapshot, b: &AccountSnapshot) -> bool {
    a.credential_epoch == b.credential_epoch && a.generation == b.generation
}
fn read_catalog(
    connection: &rusqlite::Connection,
    provider_id: &str,
) -> dim_agent_runtime::Result<Option<ProviderRecord>> {
    let row: Option<(u32, String)> = connection
        .query_row(
            "SELECT version, record FROM providers WHERE provider_id = ?1",
            [provider_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    row.map(|(version, body)| parse_catalog(version, &body))
        .transpose()
}
fn parse_catalog(version: u32, body: &str) -> dim_agent_runtime::Result<ProviderRecord> {
    let catalog: ProviderRecord = serde_json::from_str(body).map_err(storage)?;
    validate_catalog(version, &catalog)?;
    Ok(catalog)
}
fn validate_catalog(version: u32, catalog: &ProviderRecord) -> dim_agent_runtime::Result<()> {
    if catalog.version != version {
        return Err(storage("Provider version mismatch"));
    }
    let mut ids = HashSet::new();
    if catalog.account.account.is_none()
        || catalog.models.iter().any(|model| {
            model.entry.model_id.trim().is_empty()
                || !ids.insert(&model.entry.model_id)
                || model.entry.capabilities.context_window > MAX_MODEL_TOKENS
                || model.entry.capabilities.max_output_tokens > MAX_MODEL_TOKENS
                || (model.unavailable_reason.is_none()
                    && (model.entry.capabilities.max_output_tokens == 0
                        || model.entry.capabilities.context_window == 0))
        })
    {
        return Err(storage("Invalid persisted provider catalog"));
    }
    let mut overrides = HashSet::new();
    if catalog.model_config_overrides.iter().any(|model| {
        model.model_id.trim().is_empty()
            || !overrides.insert(&model.model_id)
            || model.capabilities.context_window == 0
            || model.capabilities.max_output_tokens == 0
            || model.capabilities.context_window > MAX_MODEL_TOKENS
            || model.capabilities.max_output_tokens > MAX_MODEL_TOKENS
    }) {
        return Err(storage("Invalid persisted model override"));
    }
    Ok(())
}
fn save_catalog(
    connection: &rusqlite::Connection,
    provider_id: &str,
    catalog: &ProviderRecord,
) -> dim_agent_runtime::Result<()> {
    connection.execute("INSERT INTO providers(provider_id,version,record) VALUES (?1,?2,?3) ON CONFLICT(provider_id) DO UPDATE SET version=excluded.version,record=excluded.record",
        params![provider_id, catalog.version, serde_json::to_string(catalog).map_err(storage)?]).map_err(storage)?;
    Ok(())
}
fn selection_scope(cwd: Option<&Path>) -> Result<String, HostError> {
    match cwd {
        None => Ok("global".into()),
        Some(path) if path.is_absolute() => Ok(format!(
            "workspace:{}",
            path.to_str()
                .ok_or_else(|| failure("Workspace path is not UTF-8"))?
        )),
        Some(_) => Err(failure("Workspace path must be absolute")),
    }
}
impl ProviderService {
    async fn account_available(
        self: &Arc<Self>,
        provider_id: &str,
        cancel: &CancellationToken,
    ) -> Result<bool, HostError> {
        self.check(cancel)?;
        let authority = managed_authority(provider_id);
        if authority == CredentialAuthority::Static {
            return Ok(true);
        }
        let owner = self.clone();
        self.owned(cancel, move |cancel| {
            Box::pin(async move {
                let status = match authority {
                    CredentialAuthority::OpenaiCodex => owner.codex.status(&cancel).await,
                    CredentialAuthority::Grok => owner.grok.status(&cancel).await,
                    CredentialAuthority::Static => unreachable!(),
                };
                owner.check(&cancel)?;
                Ok(status.map_err(failure)?.authenticated)
            })
        })
        .await
    }

    async fn managed_credential(
        self: &Arc<Self>,
        authority: CredentialAuthority,
        rejected: Option<managed_oauth::CredentialLease>,
        cancel: &CancellationToken,
    ) -> Result<managed_oauth::CredentialLease, HostError> {
        let owner = self.clone();
        self.owned(cancel, move |cancel| {
            Box::pin(async move {
                let lease = match authority {
                    CredentialAuthority::OpenaiCodex => {
                        owner.codex.credential(rejected.as_ref(), &cancel).await
                    }
                    CredentialAuthority::Grok => {
                        owner.grok.credential(rejected.as_ref(), &cancel).await
                    }
                    CredentialAuthority::Static => unreachable!(),
                };
                owner.check(&cancel)?;
                lease.map_err(failure)
            })
        })
        .await
    }

    pub async fn catalog_for(
        self: &Arc<Self>,
        provider_id: String,
        cancel: &CancellationToken,
    ) -> Result<Option<ProviderCatalog>, HostError> {
        if self
            .oauth
            .contains(&provider_id, cancel)
            .await
            .map_err(failure)?
        {
            return self.catalog(provider_id, cancel).await;
        }
        let available = self.account_available(&provider_id, cancel).await?;
        let db = self.db.clone();
        let bundled = self.bundled.clone();
        self.owned(cancel, move |_| {
            Box::pin(async move {
                db.run(move |connection| {
                    Ok(read_static(connection, &provider_id)?
                        .map(|record| static_catalog(&bundled, &provider_id, &record, available)))
                })
                .await
                .map_err(failure)
            })
        })
        .await
    }
    async fn seed_bundled(self: &Arc<Self>) -> Result<(), HostError> {
        let mut seeds: HashMap<_, _> = self
            .bundled
            .providers()
            .filter(|provider| {
                matches!(provider.auth.kind.as_str(), "api_key" | "aws_bedrock")
                    || managed_authority(&provider.provider_id) != CredentialAuthority::Static
            })
            .map(|provider| {
                let authority = managed_authority(&provider.provider_id);
                let endpoint_types = provider
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.raw.as_ref())
                    .map(|raw| raw.endpoint_type_by_model.clone())
                    .unwrap_or_default();
                let fingerprint = serde_json::to_vec(&(
                    &provider.display_name,
                    &provider.driver_kind,
                    &provider.auth,
                    &provider.default_base_url,
                    &provider.models,
                    &endpoint_types,
                ))
                .map_err(failure)?;
                Ok((
                    provider.provider_id.clone(),
                    StaticProviderRecord {
                        kind: StaticRecordKind::Builtin,
                        display_name: provider.display_name.clone(),
                        driver_kind: provider.driver_kind.clone(),
                        auth: if authority == CredentialAuthority::Static {
                            if provider.auth.kind == "aws_bedrock" {
                                StaticAuth::AwsBedrock
                            } else {
                                StaticAuth::ApiKey {
                                    header_name: provider
                                        .auth
                                        .header_name
                                        .clone()
                                        .unwrap_or_else(|| "Authorization".into()),
                                    prefix: provider.auth.prefix.clone(),
                                }
                            }
                        } else {
                            StaticAuth::Bearer
                        },
                        credential_authority: authority,
                        base_url: provider.default_base_url.clone().unwrap_or_default(),
                        network: ModelNetwork::default(),
                        headers: HashMap::new(),
                        credential: None,
                        models: provider.models.clone(),
                        remote_models: false,
                        endpoint_types,
                        routes: HashMap::new(),
                        bedrock_region: None,
                        model_config_overrides: Vec::new(),
                        disabled_model_ids: Vec::new(),
                        enabled: authority != CredentialAuthority::Static,
                        definition_fingerprint: Some(format!("{:x}", Sha256::digest(fingerprint))),
                        version: 1,
                    },
                ))
            })
            .collect::<Result<_, HostError>>()?;
        let db = self.db.clone();
        self.owned(&CancellationToken::new(), move |_| {
            Box::pin(async move {
                db.run(move |connection| {
                    let tx = connection.transaction().map_err(storage)?;
                    let rows = {
                        let mut query = tx
                            .prepare("SELECT provider_id,version,record FROM providers ")
                            .map_err(storage)?;
                        query
                            .query_map([], |r| {
                                Ok((
                                    r.get::<_, String>(0)?,
                                    r.get::<_, u32>(1)?,
                                    r.get::<_, String>(2)?,
                                ))
                            })
                            .map_err(storage)?
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(storage)?
                    };
                    for (id, version, body) in rows {
                        let definition = seeds.remove(&id);
                        // Preserve unreadable optional records. Their own operations
                        // report the error; SQL failures still abort initialization.
                        let Ok(Some(mut previous)) = parse_static_row(&id, version, &body) else {
                            continue;
                        };
                        if previous.kind != StaticRecordKind::Builtin {
                            continue;
                        }
                        let mut record = match definition {
                            Some(mut definition)
                                if previous.definition_fingerprint
                                    != definition.definition_fingerprint =>
                            {
                                definition.base_url = previous.base_url;
                                definition.network = previous.network;
                                definition.headers = previous.headers;
                                definition.credential = previous.credential;
                                if previous.remote_models {
                                    definition.models = previous.models;
                                    definition.remote_models = true;
                                }
                                definition.model_config_overrides = previous.model_config_overrides;
                                definition.disabled_model_ids = previous.disabled_model_ids;
                                definition.enabled =
                                    previous.enabled && drivers::supported(&definition.driver_kind);
                                definition.version = version
                                    .checked_add(1)
                                    .ok_or_else(|| storage("Provider version overflow"))?;
                                definition
                            }
                            None if previous.enabled => {
                                previous.enabled = false;
                                previous.version = version
                                    .checked_add(1)
                                    .ok_or_else(|| storage("Provider version overflow"))?;
                                previous
                            }
                            _ => continue,
                        };
                        validate_static_record(&id, &mut record).map_err(storage)?;
                        tx.execute(
                            "UPDATE providers SET version=?2,record=?3 WHERE provider_id=?1",
                            params![
                                id,
                                record.version,
                                serde_json::to_string(&record).map_err(storage)?
                            ],
                        )
                        .map_err(storage)?;
                    }
                    for (id, mut definition) in seeds {
                        validate_static_record(&id, &mut definition).map_err(storage)?;
                        tx.execute(
                            "INSERT INTO providers(provider_id,version,record) VALUES (?1,1,?2)",
                            params![id, serde_json::to_string(&definition).map_err(storage)?],
                        )
                        .map_err(storage)?;
                    }
                    tx.commit().map_err(storage)
                })
                .await
                .map_err(failure)
            })
        })
        .await
    }

    pub async fn list(
        self: &Arc<Self>,
        cancel: &CancellationToken,
    ) -> Result<ProviderList, HostError> {
        let db = self.db.clone();
        let mut list = self.owned(cancel, move |_| Box::pin(async move { db.run(move |connection| {
            let mut query = connection.prepare("SELECT provider_id,version,record FROM providers ORDER BY provider_id").map_err(storage)?;
            let rows = query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?, row.get::<_, String>(2)?))).map_err(storage)?;
            let mut result = ProviderList { providers: Vec::new(), errors: Vec::new() };
            for row in rows {
                let (id, version, body) = row.map_err(storage)?;
                match parse_static_row(&id, version, &body) {
                    Ok(None)=>{},
                    Ok(Some(record)) => result.providers.push(static_summary(id, record)),
                    Err(error) => result.errors.push(ProviderRecordError { provider_id: id, message: error.to_string() }),
                }
            }
            Ok(result)
        }).await.map_err(failure) })).await?;
        for provider in &mut list.providers {
            if provider.credential_authority != CredentialAuthority::Static {
                match self.account_available(&provider.provider_id, cancel).await {
                    Ok(available) => provider.credential_configured = available,
                    Err(error) => list.errors.push(ProviderRecordError {
                        provider_id: provider.provider_id.clone(),
                        message: error.to_string(),
                    }),
                }
            }
        }
        self.check(cancel)?;
        Ok(list)
    }

    pub async fn save_static(
        self: &Arc<Self>,
        input: ProviderInput,
        expected_version: Option<u32>,
        cancel: &CancellationToken,
    ) -> Result<ProviderSummary, HostError> {
        let authority = managed_authority(&input.provider_id);
        if self
            .oauth
            .contains(&input.provider_id, cancel)
            .await
            .map_err(failure)?
            || (authority == CredentialAuthority::Static
                && self
                    .bundled
                    .provider(&input.provider_id)
                    .is_some_and(|provider| {
                        !matches!(provider.auth.kind.as_str(), "api_key" | "aws_bedrock")
                    }))
        {
            return Err(failure("Managed provider cannot be replaced"));
        }
        let available = self.account_available(&input.provider_id, cancel).await?;
        if authority != CredentialAuthority::Static
            && !matches!(input.credential, None | Some(CredentialUpdate::Keep))
        {
            return Err(failure("Managed credentials belong to the OAuth service"));
        }
        let db = self.db.clone();
        self.owned(cancel, move |_| Box::pin(async move { db.run(move |connection| {
            let tx = connection.transaction().map_err(storage)?;
            let previous = read_static(&tx, &input.provider_id)?;
            if previous.as_ref().map(|r| r.version) != expected_version { return Err(storage("Provider version conflict")); }
            let credential = match input.credential {
                None | Some(CredentialUpdate::Keep) => previous.as_ref().and_then(|r| r.credential.clone()),
                Some(CredentialUpdate::Clear) => None,
                Some(CredentialUpdate::Set { value }) if !value.is_empty() => Some(StaticCredential::Inline(ProviderSecret::Token(value))),
                Some(CredentialUpdate::Set { .. }) => return Err(storage("Credential must not be empty")),
                Some(CredentialUpdate::SetAws { value }) => Some(StaticCredential::Inline(ProviderSecret::Aws(value))),
                Some(CredentialUpdate::SetRef { reference }) => Some(StaticCredential::Reference { reference }),
            };
            let version = expected_version.unwrap_or(0).checked_add(1).ok_or_else(|| storage("Provider version overflow"))?;
            let kind = previous.as_ref().map_or(StaticRecordKind::Custom, |r| r.kind);
            let overrides = previous.as_ref().map_or_else(Vec::new, |r| r.model_config_overrides.clone());
            let disabled = previous.as_ref().map_or_else(Vec::new, |r| r.disabled_model_ids.clone());
            if let Some(previous) = previous.as_ref().filter(|record| record.kind == StaticRecordKind::Builtin)
                && (input.display_name != previous.display_name
                    || input.driver_kind != previous.driver_kind
                    || input.auth != previous.auth
                    || input.models.is_some())
                {
                    return Err(storage("Builtin provider definition is read-only"));
                }
            let definition_fingerprint = previous.as_ref().and_then(|r| r.definition_fingerprint.clone());
            let remote_models = input.models.is_none() && previous.as_ref().is_some_and(|r| r.remote_models);
            let models = input.models.or_else(|| previous.as_ref().map(|r| r.models.clone())).ok_or_else(|| storage("New provider requires model descriptors"))?;
            let headers = input.headers.unwrap_or_else(|| previous.as_ref().map(|r| r.headers.clone()).unwrap_or_default());
            let network = input.network.unwrap_or_else(|| previous.as_ref().map(|r| r.network.clone()).unwrap_or_default());
            let endpoint_types = previous.map(|r| r.endpoint_types).unwrap_or_default();
            let mut record = StaticProviderRecord { kind, display_name: input.display_name, driver_kind: input.driver_kind, auth: input.auth, credential_authority: authority, base_url: input.base_url, network, headers, credential, models, remote_models, endpoint_types, routes: HashMap::new(), bedrock_region: None, model_config_overrides: overrides, disabled_model_ids: disabled, enabled: input.enabled, definition_fingerprint, version };
            let id = input.provider_id;
            validate_static_record(&id, &mut record).map_err(storage)?;
            tx.execute("INSERT INTO providers(provider_id,version,record) VALUES (?1,?2,?3) ON CONFLICT(provider_id) DO UPDATE SET version=excluded.version,record=excluded.record", params![id, version, serde_json::to_string(&record).map_err(storage)?]).map_err(storage)?;
            tx.commit().map_err(storage)?;
            let mut summary = static_summary(id, record);
            if authority != CredentialAuthority::Static { summary.credential_configured = available; }
            Ok(summary)
        }).await.map_err(failure) })).await
    }

    pub async fn delete_static(
        self: &Arc<Self>,
        provider_id: String,
        expected_version: u32,
        cancel: &CancellationToken,
    ) -> Result<(), HostError> {
        let db = self.db.clone();
        self.owned(cancel, move |_| {
            Box::pin(async move {
                db.run(move |connection| {
                    let tx = connection.transaction().map_err(storage)?;
                    let record = read_static(&tx, &provider_id)?
                        .ok_or_else(|| storage("Provider not found"))?;
                    if record.version != expected_version {
                        return Err(storage("Provider version conflict"));
                    }
                    if record.kind == StaticRecordKind::Builtin {
                        let mut record = record;
                        record.enabled = false;
                        record.version = record
                            .version
                            .checked_add(1)
                            .ok_or_else(|| storage("Provider version overflow"))?;
                        tx.execute(
                            "UPDATE providers SET version=?2,record=?3 WHERE provider_id=?1",
                            params![
                                provider_id,
                                record.version,
                                serde_json::to_string(&record).map_err(storage)?
                            ],
                        )
                        .map_err(storage)?;
                        return tx.commit().map_err(storage);
                    }
                    tx.execute(
                        "DELETE FROM provider_selections WHERE provider_id=?1",
                        [&provider_id],
                    )
                    .map_err(storage)?;
                    tx.execute("DELETE FROM providers WHERE provider_id=?1", [&provider_id])
                        .map_err(storage)?;
                    tx.commit().map_err(storage)
                })
                .await
                .map_err(failure)
            })
        })
        .await
    }

    fn check(&self, cancel: &CancellationToken) -> Result<(), HostError> {
        if self.stop.is_cancelled() {
            return Err(HostError::classified(
                HostErrorKind::OwnerClosed,
                "Provider is closed",
            ));
        }
        if cancel.is_cancelled() {
            return Err(HostError::Cancelled("Provider operation cancelled".into()));
        }
        Ok(())
    }
    async fn owned<T: Send + 'static>(
        self: &Arc<Self>,
        cancel: &CancellationToken,
        operation: impl FnOnce(CancellationToken) -> SharedFuture<'static, Result<T, HostError>>
        + Send
        + 'static,
    ) -> Result<T, HostError> {
        let task = {
            let _guard = self.admission.lock().unwrap();
            self.check(cancel)?;
            let caller = cancel.clone();
            let stop = self.stop.clone();
            self.tasks.spawn(async move {
                let cancel = stop.child_token();
                let mut operation = operation(cancel.clone());
                tokio::select! {
                    result = &mut operation => return result,
                    _ = caller.cancelled() => cancel.cancel(),
                    _ = stop.cancelled() => {},
                }
                // Cancel lock/network waits but drain any admitted transaction.
                operation.await
            })
        };
        let result = task.await.map_err(|_| failure("Provider worker failed"))?;
        if result.is_err() {
            self.check(cancel)?;
        }
        result
    }
    pub async fn catalog(
        self: &Arc<Self>,
        provider_id: String,
        cancel: &CancellationToken,
    ) -> Result<Option<ProviderCatalog>, HostError> {
        let account = self
            .oauth
            .status(&provider_id, cancel)
            .await
            .map_err(failure)?;
        let db = self.db.clone();
        let bundled = self.bundled.clone();
        self.owned(cancel, move |_| {
            Box::pin(async move {
                db.run(move |c| {
                    Ok(read_catalog(c, &provider_id)?
                        .filter(|r| account.account.is_some() && same_account(&r.account, &account))
                        .map(|r| r.catalog(&bundled)))
                })
                .await
                .map_err(failure)
            })
        })
        .await
    }
    pub async fn refresh(
        self: &Arc<Self>,
        provider_id: String,
        cancel: &CancellationToken,
    ) -> Result<ProviderCatalog, HostError> {
        if !self
            .oauth
            .contains(&provider_id, cancel)
            .await
            .map_err(failure)?
        {
            return Err(failure("Managed provider plugin is unavailable"));
        }
        let db = self.db.clone();
        let id = provider_id.clone();
        let expected_version = self
            .owned(cancel, move |_| {
                Box::pin(async move {
                    db.run(move |c| Ok(read_catalog(c, &id)?.map(|r| r.version)))
                        .await
                        .map_err(failure)
                })
            })
            .await?;
        let catalog = self
            .oauth
            .catalog(&provider_id, cancel)
            .await
            .map_err(failure)?;
        let current = self
            .oauth
            .status(&provider_id, cancel)
            .await
            .map_err(failure)?;
        if !same_account(&current, &catalog.account) {
            return Err(failure("Managed account changed"));
        }
        let db = self.db.clone();
        let bundled = self.bundled.clone();
        let id = provider_id.clone();
        let result = self
            .owned(cancel, move |_| {
                Box::pin(async move {
                    db.run(move |c| {
                        let tx = c.transaction().map_err(storage)?;
                        let old = read_catalog(&tx, &provider_id)?;
                        if old.as_ref().map(|r| r.version) != expected_version {
                            return Err(storage("Provider version conflict"));
                        }
                        let version = old
                            .as_ref()
                            .map_or(0, |r| r.version)
                            .checked_add(1)
                            .ok_or_else(|| storage("Provider version overflow"))?;
                        let old = old.filter(|r| same_account(&r.account, &catalog.account));
                        let (disabled_model_ids, model_config_overrides) = old.map_or_else(
                            || (vec![], vec![]),
                            |r| (r.disabled_model_ids, r.model_config_overrides),
                        );
                        let record = ProviderRecord {
                            kind: ManagedRecordKind::Managed,
                            account: catalog.account,
                            models: catalog.models,
                            disabled_model_ids,
                            model_config_overrides,
                            version,
                        };
                        save_catalog(&tx, &provider_id, &record)?;
                        tx.commit().map_err(storage)?;
                        Ok(record.catalog(&bundled))
                    })
                    .await
                    .map_err(failure)
                })
            })
            .await?;
        let settled = self.oauth.status(&id, cancel).await.map_err(failure)?;
        if !same_account(&current, &settled) {
            return Err(failure("Managed account changed"));
        }
        Ok(result)
    }
    pub async fn set_enabled_models(
        self: &Arc<Self>,
        provider_id: String,
        expected_version: u32,
        ids: Vec<String>,
        cancel: &CancellationToken,
    ) -> Result<ProviderCatalog, HostError> {
        self.check(cancel)?;
        if !self
            .oauth
            .contains(&provider_id, cancel)
            .await
            .map_err(failure)?
        {
            let available = self.account_available(&provider_id, cancel).await?;
            let db = self.db.clone();
            let bundled = self.bundled.clone();
            return self
                .owned(cancel, move |_| {
                    Box::pin(async move {
                        db.run(move |connection| {
                            let tx = connection.transaction().map_err(storage)?;
                            let mut record = read_static(&tx, &provider_id)?
                                .ok_or_else(|| storage("Provider not found"))?;
                            if record.version != expected_version {
                                return Err(storage("Provider version conflict"));
                            }
                            if ids
                                .iter()
                                .any(|id| !record.models.iter().any(|model| &model.model_id == id))
                            {
                                return Err(storage("Model is unavailable"));
                            }
                            record.disabled_model_ids.retain(|id| {
                                !record.models.iter().any(|model| &model.model_id == id)
                            });
                            record.disabled_model_ids.extend(
                                record
                                    .models
                                    .iter()
                                    .filter(|model| !ids.contains(&model.model_id))
                                    .map(|model| model.model_id.clone()),
                            );
                            record.version = record
                                .version
                                .checked_add(1)
                                .ok_or_else(|| storage("Provider version overflow"))?;
                            tx.execute(
                                "UPDATE providers SET version=?2,record=?3 WHERE provider_id=?1",
                                params![
                                    provider_id,
                                    record.version,
                                    serde_json::to_string(&record).map_err(storage)?
                                ],
                            )
                            .map_err(storage)?;
                            tx.commit().map_err(storage)?;
                            Ok(static_catalog(&bundled, &provider_id, &record, available))
                        })
                        .await
                        .map_err(failure)
                    })
                })
                .await;
        }
        let owner = self.clone();
        self.owned(cancel, move |cancel| {
            Box::pin(async move {
                let account = owner
                    .oauth
                    .status(&provider_id, &cancel)
                    .await
                    .map_err(failure)?;
                let db = owner.db.clone();
                let bundled = owner.bundled.clone();
                db.run(move |connection| {
                    let tx = connection.transaction().map_err(storage)?;
                    let mut catalog = read_catalog(&tx, &provider_id)?
                        .ok_or_else(|| storage("No managed catalog"))?;
                    if !same_account(&catalog.account, &account)
                        || catalog.version != expected_version
                    {
                        return Err(storage("Provider version or account conflict"));
                    }
                    if ids.iter().any(|id| {
                        !catalog.models.iter().any(|model| {
                            &model.entry.model_id == id && model.unavailable_reason.is_none()
                        })
                    }) {
                        return Err(storage("Model is unavailable"));
                    }
                    // Edits apply to the current catalog, not to models
                    // temporarily absent from discovery.
                    catalog.disabled_model_ids.retain(|id| {
                        !catalog
                            .models
                            .iter()
                            .any(|model| &model.entry.model_id == id)
                    });
                    catalog.disabled_model_ids.extend(
                        catalog
                            .models
                            .iter()
                            .filter(|model| !ids.contains(&model.entry.model_id))
                            .map(|model| model.entry.model_id.clone()),
                    );
                    catalog.version = catalog
                        .version
                        .checked_add(1)
                        .ok_or_else(|| storage("Provider version overflow"))?;
                    save_catalog(&tx, &provider_id, &catalog)?;
                    tx.commit().map_err(storage)?;
                    Ok(catalog.catalog(&bundled))
                })
                .await
                .map_err(failure)
            })
        })
        .await
    }
    pub async fn update_model_config(
        self: &Arc<Self>,
        provider_id: String,
        expected_version: u32,
        model_id: String,
        edit: Option<ModelEdit>,
        cancel: &CancellationToken,
    ) -> Result<ProviderCatalog, HostError> {
        if edit
            .as_ref()
            .and_then(|edit| edit.display_name.as_ref())
            .is_some_and(|name| name.trim().is_empty())
        {
            return Err(failure("Model display name must not be empty"));
        }
        if edit.as_ref().is_some_and(|edit| {
            [edit.context_window, edit.max_output_tokens]
                .into_iter()
                .flatten()
                .any(|limit| limit.get() > MAX_MODEL_TOKENS)
        }) {
            return Err(failure("Model limits must be safe JSON integers"));
        }
        if edit
            .as_ref()
            .and_then(|edit| edit.reasoning.as_ref())
            .and_then(|reasoning| reasoning.effort_options.as_ref())
            .is_some_and(|options| {
                options.iter().any(|option| {
                    !matches!(
                        option.as_str(),
                        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
                    )
                })
            })
        {
            return Err(failure("Invalid reasoning effort option"));
        }
        if !self
            .oauth
            .contains(&provider_id, cancel)
            .await
            .map_err(failure)?
        {
            let available = self.account_available(&provider_id, cancel).await?;
            let db = self.db.clone();
            let bundled = self.bundled.clone();
            return self
                .owned(cancel, move |_| {
                    Box::pin(async move {
                        db.run(move |connection| {
                            let tx = connection.transaction().map_err(storage)?;
                            let mut record = read_static(&tx, &provider_id)?
                                .ok_or_else(|| storage("Provider not found"))?;
                            if record.version != expected_version {
                                return Err(storage("Provider version conflict"));
                            }
                            if !record.models.iter().any(|model| model.model_id == model_id) {
                                return Err(storage("Model is unavailable"));
                            }
                            let index = record
                                .model_config_overrides
                                .iter()
                                .position(|model| model.model_id == model_id);
                            match edit {
                                Some(edit) => {
                                    let current = bundled
                                        .effective_models(
                                            &ProviderProjection {
                                                provider_id: provider_id.clone(),
                                                source: Some(
                                                    if record.kind == StaticRecordKind::Builtin {
                                                        ModelSource::Builtin
                                                    } else {
                                                        ModelSource::Custom
                                                    },
                                                ),
                                                ..Default::default()
                                            },
                                            &record.models,
                                            &record.model_config_overrides,
                                        )
                                        .into_iter()
                                        .find(|model| model.model_id == model_id)
                                        .ok_or_else(|| storage("Model is unavailable"))?;
                                    let next = bundled.merge_visible_edit(&current, edit);
                                    if let Some(index) = index {
                                        record.model_config_overrides[index] = next;
                                    } else {
                                        record.model_config_overrides.push(next);
                                    }
                                }
                                None => {
                                    record.model_config_overrides.remove(
                                        index.ok_or_else(|| storage("Model override not found"))?,
                                    );
                                }
                            }
                            record.version = record
                                .version
                                .checked_add(1)
                                .ok_or_else(|| storage("Provider version overflow"))?;
                            tx.execute(
                                "UPDATE providers SET version=?2,record=?3 WHERE provider_id=?1",
                                params![
                                    provider_id,
                                    record.version,
                                    serde_json::to_string(&record).map_err(storage)?
                                ],
                            )
                            .map_err(storage)?;
                            tx.commit().map_err(storage)?;
                            Ok(static_catalog(&bundled, &provider_id, &record, available))
                        })
                        .await
                        .map_err(failure)
                    })
                })
                .await;
        }
        let owner = self.clone();
        self.owned(cancel, move |cancel| {
            Box::pin(async move {
                let account = owner
                    .oauth
                    .status(&provider_id, &cancel)
                    .await
                    .map_err(failure)?;
                let db = owner.db.clone();
                let bundled = owner.bundled.clone();
                db.run(move |connection| {
                    let tx = connection.transaction().map_err(storage)?;
                    let mut record = read_catalog(&tx, &provider_id)?
                        .ok_or_else(|| storage("No managed catalog"))?;
                    if !same_account(&record.account, &account)
                        || record.version != expected_version
                    {
                        return Err(storage("Provider version or account conflict"));
                    }
                    if !record
                        .models
                        .iter()
                        .any(|model| model.entry.model_id == model_id)
                    {
                        return Err(storage("Model is unavailable"));
                    }
                    let index = record
                        .model_config_overrides
                        .iter()
                        .position(|model| model.model_id == model_id);
                    match edit {
                        Some(edit) => {
                            let current = record
                                .model_config_overrides
                                .iter()
                                .find(|m| m.model_id == model_id)
                                .cloned()
                                .or_else(|| {
                                    record
                                        .models
                                        .iter()
                                        .find(|m| m.entry.model_id == model_id)
                                        .map(|m| m.entry.clone())
                                })
                                .ok_or_else(|| storage("Model is unavailable"))?;
                            let next = bundled.merge_visible_edit(&current, edit);
                            if let Some(index) = index {
                                record.model_config_overrides[index] = next;
                            } else {
                                record.model_config_overrides.push(next);
                            }
                        }
                        None => {
                            let index = index.ok_or_else(|| storage("Model override not found"))?;
                            record.model_config_overrides.remove(index);
                        }
                    }
                    record.version = record
                        .version
                        .checked_add(1)
                        .ok_or_else(|| storage("Provider version overflow"))?;
                    save_catalog(&tx, &provider_id, &record)?;
                    tx.commit().map_err(storage)?;
                    Ok(record.catalog(&bundled))
                })
                .await
                .map_err(failure)
            })
        })
        .await
    }
    pub async fn selection(
        self: &Arc<Self>,
        cwd: Option<&Path>,
        cancel: &CancellationToken,
    ) -> Result<Option<ProviderSelection>, HostError> {
        let scope = selection_scope(cwd)?;
        let db = self.db.clone();
        self.owned(cancel, move |_cancel| {
            Box::pin(async move {
                db.run(move |connection| {
                    connection
                        .query_row(
                            "SELECT provider_id, model_id, version FROM provider_selections WHERE scope=?1",
                            [scope],
                            |row| {
                                Ok(ProviderSelection {
                                    provider_id: row.get(0)?,
                                    model_id: row.get(1)?,
                                    version: row.get(2)?,
                                })
                            },
                        )
                        .optional()
                        .map_err(storage)
                })
                .await
                .map_err(failure)
            })
        })
        .await
    }
    pub async fn select(
        self: &Arc<Self>,
        cwd: Option<&Path>,
        target: ModelId,
        expected_version: Option<u32>,
        cancel: &CancellationToken,
    ) -> Result<ProviderSelection, HostError> {
        let scope = selection_scope(cwd)?;
        let catalog = self
            .catalog_for(target.provider_id.clone(), cancel)
            .await?
            .ok_or_else(|| failure("Provider not found"))?;
        if catalog.selectable(&target.model_id).is_none() {
            return Err(failure("Model is unavailable"));
        }
        let db = self.db.clone();
        self.owned(cancel,move |_| Box::pin(async move { db.run(move |c| {
            let tx=c.transaction().map_err(storage)?;
            let version:Option<u32>=tx.query_row("SELECT version FROM provider_selections WHERE scope=?1",[&scope],|r|r.get(0)).optional().map_err(storage)?;
            if version!=expected_version {return Err(storage("Selection version conflict"));}
            let version=version.unwrap_or(0).checked_add(1).ok_or_else(||storage("Selection version overflow"))?;
            tx.execute("INSERT INTO provider_selections(scope,version,provider_id,model_id) VALUES (?1,?2,?3,?4) ON CONFLICT(scope) DO UPDATE SET version=excluded.version,provider_id=excluded.provider_id,model_id=excluded.model_id",params![scope,version,target.provider_id,target.model_id]).map_err(storage)?;
            tx.commit().map_err(storage)?; Ok(ProviderSelection {provider_id:target.provider_id,model_id:target.model_id,version})
        }).await.map_err(failure) })).await
    }
    pub async fn configuration(
        self: &Arc<Self>,
        explicit: Option<ModelId>,
        cwd: Option<&Path>,
        preferences: ModelPreferences,
        cancel: &CancellationToken,
    ) -> Result<ModelSelection, HostError> {
        self.check(cancel)?;
        let explicit_target = explicit.is_some();
        let mut candidates = vec![];
        if let Some(id) = explicit {
            candidates.push((id.provider_id, Some(id.model_id)));
        } else {
            for scope in [cwd, None] {
                if let Some(selection) = self.selection(scope, cancel).await? {
                    candidates.push((selection.provider_id, Some(selection.model_id)));
                }
            }
            candidates.extend(
                self.oauth
                    .providers(cancel)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| (p.provider_id, None)),
            );
            candidates.extend(
                self.list(cancel)
                    .await?
                    .providers
                    .into_iter()
                    .map(|p| (p.provider_id, None)),
            );
        }
        for (id, model_id) in candidates {
            let catalog = match self.catalog_for(id.clone(), cancel).await {
                Ok(Some(c)) => c,
                Ok(None) => continue,
                Err(_) if !explicit_target => continue,
                Err(e) => return Err(e),
            };
            let model = match model_id {
                Some(ref m) => catalog.selectable(m),
                None => catalog
                    .models
                    .iter()
                    .find(|m| catalog.selectable(&m.id).is_some()),
            };
            let Some(model) = model else { continue };
            let protocol = model
                .endpoint
                .ok_or_else(|| failure("Missing model protocol"))?;
            let config = preferences
                .configure(model, protocol, &model.config)
                .map_err(failure)?;
            return Ok(ModelSelection {
                provider_id: id.clone(),
                model_id: model.id.clone(),
                capability_snapshot: Some(ModelCapabilitySnapshot {
                    target: ModelId {
                        provider_id: id,
                        model_id: model.id.clone(),
                    },
                    capabilities: model.capabilities.clone(),
                    reasoning: model.metadata.as_ref().and_then(|m| m.reasoning.clone()),
                    service_tier: model.metadata.as_ref().and_then(|m| m.service_tier.clone()),
                }),
                config: Some(config),
                metadata: None,
            });
        }
        Err(failure(if explicit_target {
            "Explicit model is unavailable"
        } else {
            "No available model"
        }))
    }

    /// Applies supplied preferences to the adopted model without reselecting defaults.
    pub async fn apply_preferences(
        self: &Arc<Self>,
        current: &dim_agent_types::ModelConfiguration,
        preferences: ModelPreferences,
        cancel: &CancellationToken,
    ) -> Result<ModelSelection, HostError> {
        self.check(cancel)?;
        let catalog = self
            .catalog_for(current.provider_id.clone(), cancel)
            .await?
            .ok_or_else(|| failure("Provider is unavailable"))?;
        let model = catalog
            .selectable(&current.model_id)
            .ok_or_else(|| failure("Model is unavailable"))?;
        let protocol = model
            .endpoint
            .ok_or_else(|| failure("Missing model protocol"))?;
        let base = current.config.clone().unwrap_or_default();
        let config = preferences
            .configure(model, protocol, &base)
            .map_err(failure)?;
        Ok(ModelSelection {
            provider_id: current.provider_id.clone(),
            model_id: current.model_id.clone(),
            capability_snapshot: current.capability_snapshot.clone(),
            config: if config == base {
                current.config.clone()
            } else {
                Some(config)
            },
            metadata: current.metadata.clone(),
        })
    }
}
struct ProductResolver(Arc<ProviderService>);
impl ResolveModelConnection for ProductResolver {
    fn resolve(
        &self,
        request: Arc<ModelRequest>,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ResolvedModelConnection> {
        Box::pin(async move {
            self.0.check(&cancel)?;
            request
                .capability_snapshot
                .as_ref()
                .ok_or_else(|| {
                    HostError::classified(
                        HostErrorKind::InvalidRequest,
                        "Model capabilities are not adopted; select the model again",
                    )
                })?
                .validate_for(&request.provider_id, &request.model_id)
                .map_err(|message| HostError::classified(HostErrorKind::InvalidRequest, message))?;
            if !self
                .0
                .oauth
                .contains(&request.provider_id, &cancel)
                .await
                .map_err(failure)?
            {
                let provider_id = request.provider_id.clone();
                let model_id = request.model_id.clone();
                let db = self.0.db.clone();
                let record = self
                    .0
                    .owned(&cancel, move |_| {
                        Box::pin(async move {
                            db.run(move |connection| read_static(connection, &provider_id))
                                .await
                                .map_err(failure)
                        })
                    })
                    .await?
                    .ok_or_else(|| failure("Provider not found"))?;
                self.0.check(&cancel)?;
                if static_catalog(&self.0.bundled, &request.provider_id, &record, true)
                    .selectable(&model_id)
                    .is_none()
                {
                    return Err(failure("Model is unavailable"));
                }
                if record.credential_authority != CredentialAuthority::Static {
                    let lease = self
                        .0
                        .managed_credential(record.credential_authority, None, &cancel)
                        .await?;
                    let recovered = managed_connection(
                        self.0.clone(),
                        record.credential_authority,
                        lease,
                        record.network,
                    );
                    return Ok(ResolvedModelConnection {
                        connection: recovered.connection,
                        recovery: Some(recovered.recovery),
                        failure_policy: None,
                    });
                }
                let credential = self
                    .0
                    .credential(
                        &request.provider_id,
                        &record.credential,
                        &record.auth,
                        &cancel,
                    )
                    .await?;
                let mut headers = record.headers;
                let mut bearer = None;
                match record.auth {
                    StaticAuth::Anonymous => {}
                    StaticAuth::Bearer => {
                        let ProviderSecret::Token(value) =
                            credential.ok_or_else(|| failure("Provider credential is required"))?
                        else {
                            return Err(failure("Invalid provider credential type"));
                        };
                        bearer = Some(value)
                    }
                    StaticAuth::ApiKey {
                        header_name,
                        prefix,
                    } => {
                        let ProviderSecret::Token(value) =
                            credential.ok_or_else(|| failure("Provider credential is required"))?
                        else {
                            return Err(failure("Invalid provider credential type"));
                        };
                        if header_name.eq_ignore_ascii_case("authorization") && prefix == "Bearer "
                        {
                            bearer = Some(value);
                        } else {
                            headers.insert(header_name, format!("{prefix}{value}"));
                        }
                    }
                    StaticAuth::CustomHeader { header_name } => {
                        let ProviderSecret::Token(value) =
                            credential.ok_or_else(|| failure("Provider credential is required"))?
                        else {
                            return Err(failure("Invalid provider credential type"));
                        };
                        headers.insert(header_name, value);
                    }
                    StaticAuth::AwsBedrock => {
                        let auth = match credential
                            .ok_or_else(|| failure("Provider credential is required"))?
                        {
                            ProviderSecret::Token(value) => BedrockAuth::Bearer(value),
                            ProviderSecret::Aws(credentials) => BedrockAuth::SigV4 {
                                credentials,
                                region: record
                                    .bedrock_region
                                    .ok_or_else(|| failure("Amazon Bedrock region is missing"))?,
                            },
                        };
                        return Ok(ResolvedModelConnection {
                            connection: ModelConnection::Bedrock(BedrockModelConnection {
                                base_url: record.base_url,
                                network: record.network,
                                headers,
                                auth,
                            }),
                            failure_policy: None,
                            recovery: None,
                        });
                    }
                }
                let protocol = record.routes[&model_id];
                let mut connection =
                    drivers::connection(&record.driver_kind, protocol, &record.base_url, headers);
                if request.provider_id == "opencode-go" {
                    let session_id = request
                        .metadata
                        .as_ref()
                        .and_then(|m| m.session_id.as_deref())
                        .filter(|id| {
                            !id.is_empty()
                                && id.trim() == *id
                                && id.bytes().all(|b| (0x20..=0x7e).contains(&b))
                        })
                        .ok_or_else(|| failure("OpenCode Go requires a printable session ID"))?;
                    connection
                        .headers
                        .retain(|name, _| !name.eq_ignore_ascii_case("x-opencode-session"));
                    connection
                        .headers
                        .insert("x-opencode-session".into(), session_id.into());
                }
                connection.bearer = bearer;
                connection.network = record.network;
                return Ok(ResolvedModelConnection {
                    connection: ModelConnection::Http(connection),
                    failure_policy: None,
                    recovery: None,
                });
            }
            let catalog = self
                .0
                .catalog(request.provider_id.clone(), &cancel)
                .await?
                .ok_or_else(|| failure("No current managed catalog"))?;
            let model = catalog
                .selectable(&request.model_id)
                .ok_or_else(|| failure("Model is unavailable"))?;
            let lease = self
                .0
                .oauth
                .credential(&request.provider_id, &request.model_id, &cancel)
                .await
                .map_err(failure)?;
            if catalog.account.as_ref() != Some(&lease.account)
                || model.endpoint != Some(lease.protocol)
            {
                return Err(failure("Managed account or model route changed"));
            }
            let recovered = plugin_connection(self.0.clone(), lease);
            Ok(ResolvedModelConnection {
                connection: recovered.connection,
                recovery: Some(recovered.recovery),
                failure_policy: None,
            })
        })
    }
}
impl ResolveModelSelection for ProductResolver {
    fn resolve(
        &self,
        explicit: Option<ModelId>,
        cwd: Option<std::path::PathBuf>,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelSelection> {
        Box::pin(async move {
            self.0
                .configuration(
                    explicit,
                    cwd.as_deref(),
                    ModelPreferences::default(),
                    &cancel,
                )
                .await
        })
    }
}
struct ManagedRecovery {
    owner: Arc<ProviderService>,
    authority: CredentialAuthority,
    lease: managed_oauth::CredentialLease,
    network: ModelNetwork,
}

fn managed_connection(
    owner: Arc<ProviderService>,
    authority: CredentialAuthority,
    lease: managed_oauth::CredentialLease,
    network: ModelNetwork,
) -> RecoveredModelConnection {
    let mut headers = HashMap::new();
    let (protocol, preset) = match authority {
        CredentialAuthority::OpenaiCodex => {
            headers.insert("OAI-Product-Sku".into(), "codex".into());
            headers.insert("originator".into(), "codex-tui".into());
            headers.insert(
                "User-Agent".into(),
                "codex-tui/0.144.1 (Ubuntu 22.4.0; x86_64) xterm-256color (codex-tui; 0.144.1)"
                    .into(),
            );
            if let Some(account_id) = &lease.account_id {
                headers.insert("ChatGPT-Account-ID".into(), account_id.clone());
            }
            (ModelProtocol::OpenaiResponses, Some("openai-codex".into()))
        }
        CredentialAuthority::Grok => (ModelProtocol::OpenaiCompatible, None),
        CredentialAuthority::Static => unreachable!(),
    };
    RecoveredModelConnection {
        connection: ModelConnection::Http(HttpModelConnection {
            protocol,
            base_url: lease.relay_base_url.clone(),
            headers,
            network: network.clone(),
            bearer: Some(lease.bearer().into()),
            api_key: None,
            preset,
        }),
        recovery: Arc::new(ManagedRecovery {
            owner,
            authority,
            lease,
            network,
        }),
    }
}

impl RecoverModelConnection for ManagedRecovery {
    fn rejection(&self, error: &HostError, phase: CredentialRecoveryPhase) -> CredentialRejection {
        oauth_rejection(error, phase)
    }
    fn recover(&self, cancel: CancellationToken) -> HostFuture<'_, RecoveredModelConnection> {
        Box::pin(async move {
            let lease = self
                .owner
                .managed_credential(self.authority, Some(self.lease.clone()), &cancel)
                .await?;
            Ok(managed_connection(
                self.owner.clone(),
                self.authority,
                lease,
                self.network.clone(),
            ))
        })
    }
    fn finish_rejected(&self, cancel: CancellationToken) -> HostFuture<'_, ()> {
        let owner = self.owner.clone();
        let authority = self.authority;
        let lease = self.lease.clone();
        Box::pin(async move {
            self.owner
                .owned(&cancel, move |cancel| {
                    Box::pin(async move {
                        let result = match authority {
                            CredentialAuthority::OpenaiCodex => {
                                owner.codex.finish_rejected(&lease, &cancel).await
                            }
                            CredentialAuthority::Grok => {
                                owner.grok.finish_rejected(&lease, &cancel).await
                            }
                            CredentialAuthority::Static => unreachable!(),
                        };
                        owner.check(&cancel)?;
                        result.map_err(failure)
                    })
                })
                .await
        })
    }
}

struct PluginRecovery {
    owner: Arc<ProviderService>,
    lease: CredentialLease,
}
fn plugin_connection(
    owner: Arc<ProviderService>,
    lease: CredentialLease,
) -> RecoveredModelConnection {
    RecoveredModelConnection {
        connection: ModelConnection::Http(HttpModelConnection {
            protocol: lease.protocol,
            base_url: lease.base_url.clone(),
            headers: lease.headers.clone(),
            network: ModelNetwork::default(),
            bearer: Some(lease.bearer.clone()),
            api_key: None,
            preset: None,
        }),
        recovery: Arc::new(PluginRecovery { owner, lease }),
    }
}
impl RecoverModelConnection for PluginRecovery {
    fn rejection(&self, error: &HostError, phase: CredentialRecoveryPhase) -> CredentialRejection {
        oauth_rejection(error, phase)
    }
    fn recover(&self, cancel: CancellationToken) -> HostFuture<'_, RecoveredModelConnection> {
        Box::pin(async move {
            self.owner.check(&cancel)?;
            let lease = self
                .owner
                .oauth
                .recover(&self.lease, &cancel)
                .await
                .map_err(failure)?;
            self.owner.check(&cancel)?;
            Ok(plugin_connection(self.owner.clone(), lease))
        })
    }
    fn finish_rejected(&self, cancel: CancellationToken) -> HostFuture<'_, ()> {
        Box::pin(async move {
            self.owner.check(&cancel)?;
            self.owner
                .oauth
                .finish_rejected(&self.lease, &cancel)
                .await
                .map_err(failure)
        })
    }
}

// These authorities retain their existing second-rejection settlement policy.
// A generic model caller must not infer account invalidity from HTTP status.
fn oauth_rejection(error: &HostError, phase: CredentialRecoveryPhase) -> CredentialRejection {
    if !matches!(
        error,
        HostError::Failed {
            kind: HostErrorKind::Http {
                status: Some(401),
                ..
            },
            ..
        }
    ) {
        return CredentialRejection::Unrelated;
    }
    match phase {
        CredentialRecoveryPhase::Initial => CredentialRejection::Recoverable,
        CredentialRecoveryPhase::Recovered => CredentialRejection::Terminal,
    }
}

#[derive(Default)]
pub struct ProviderPlugin {
    secret_host: bool,
}
impl ProviderPlugin {
    /// The supplying plugin must provide ProviderSecretHost before activation.
    pub fn with_secret_host() -> Self {
        Self { secret_host: true }
    }
    /// This part with its keys from `names`: ProviderSecretHost only when
    /// a plugin supplies it.
    pub fn part(self, names: &Names<'_>) -> Result<Box<dyn Plugin>, CordisError> {
        let keys = self.keys(names)?;
        Ok(names.part_with(self, keys))
    }
    /// This part keyed from `catalog`, to mount it directly.
    pub fn typed(self, catalog: &ServiceCatalog) -> Result<rutis::Typed<Self>, CordisError> {
        let build = Default::default();
        let keys = self.keys(&Names::copy(catalog, &build))?;
        Ok(rutis::Typed::with_keys(self, keys))
    }
    fn keys(
        &self,
        names: &Names<'_>,
    ) -> Result<<<Self as TypedPlugin>::Deps as rutis::Deps>::Keys, CordisError> {
        Ok((
            names.key()?,
            names.key()?,
            names.key()?,
            names.key()?,
            names.when::<Keyed<ProviderSecretHost>>(self.secret_host)?,
            names.key()?,
        ))
    }
}
impl TypedPlugin for ProviderPlugin {
    type Deps = (
        Keyed<SqliteStorage>,
        Keyed<ManagedProviders>,
        Keyed<OpenAICodexOAuthService>,
        Keyed<GrokOAuthService>,
        When<Keyed<ProviderSecretHost>>,
        Keyed<ModelHost>,
    );
    fn name(&self) -> &str {
        "Provider"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (
            Keyed(db),
            Keyed(oauth),
            Keyed(codex),
            Keyed(grok),
            When(secret_host),
            Keyed(host),
        ): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Arc::new(ProviderService {
                db,
                oauth,
                codex,
                grok,
                secret_host: secret_host.map(|Keyed(host)| host),
                bundled: Arc::new(
                    BundledCatalog::load()
                        .map_err(|error| CordisError::PluginFailed(error.into()))?,
                ),
                client: remote::client(&ModelNetwork::default())
                    .map_err(|error| CordisError::PluginFailed(error.to_string().into()))?,
                stop: CancellationToken::new(),
                tasks: TaskTracker::new(),
                admission: Mutex::new(()),
            });
            let owner = service.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        {
                            let _guard = owner.admission.lock().unwrap();
                            owner.stop.cancel();
                            owner.tasks.close();
                        }
                        owner.tasks.wait().await;
                        Ok(())
                    })
                }))
            })?;
            service
                .seed_bundled()
                .await
                .map_err(|error| CordisError::PluginFailed(error.to_string().into()))?;
            let resolver = Arc::new(ProductResolver(service.clone()));
            ctx.provide(ProviderConnectionResolver(resolver.clone()))?;
            ctx.provide(ModelSelectionHost(resolver))?;
            let auxiliary = Arc::new(auxiliary::AuxiliaryModel {
                owner: service.clone(),
                host,
            });
            ctx.provide(AuxiliaryModelHost(auxiliary.clone()))?;
            ctx.provide_as::<dyn rutis_bridge::session::HostDispatch>(
                rutis_bridge::session::host_key("auxiliaryModel"),
                auxiliary,
            )?;
            management::provide(ctx, service.clone())?;
            ctx.provide_as(TypeKey::of::<ProviderService>(), service)?;
            Ok(Effect::Done)
        })
    }
}
