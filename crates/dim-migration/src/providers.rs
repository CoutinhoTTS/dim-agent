use crate::{
    Error, Migration, ObjectResult, Outcome, Result as MigrationResult, TaskState, native::Native,
    source::Source,
};
use dim_agent_capabilities::host::{AwsCredentials, ModelNetwork};
use dim_app_server::{
    model_catalog::{
        BundledCatalog, MAX_MODEL_TOKENS, ModelEntry, ModelSource, ModelStatus, NewApiEndpoint,
        ProviderProjection,
    },
    provider::{CredentialAuthority, ModelPreferences, ProviderSecret, StaticAuth},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params, types::ValueRef};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
};
use tokio_util::sync::CancellationToken;
fn failure(value: impl std::fmt::Display) -> String {
    value.to_string()
}
fn managed_authority(id: &str) -> CredentialAuthority {
    match id {
        "openai-codex" => CredentialAuthority::OpenaiCodex,
        "grok-build" => CredentialAuthority::Grok,
        _ => CredentialAuthority::Static,
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum StaticCredential {
    Inline(ProviderSecret),
    Reference {
        #[serde(rename = "secretRef")]
        reference: String,
    },
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
    #[serde(default)]
    model_config_overrides: Vec<ModelEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    user_models: Vec<ModelEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    model_preferences: BTreeMap<String, ModelPreferences>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    endpoint_overrides: BTreeMap<String, NewApiEndpoint>,
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

const PROVIDER_COLUMNS: &str = "providerId source displayName driverKind defaultBaseUrl baseUrl auth credential accountUsageSettings headers proxy timeoutMs retryConfig models modelConfigOverrides capabilityDefaults activeModelId modelsSource catalogVersion modelsUpdatedAt metadata enabled createdAt updatedAt version";
const SELECTION_COLUMNS: &str =
    "selectionId scope cwd providerId modelId createdAt updatedAt version";

struct SourceRow {
    id: String,
    version: i64,
    digest: String,
    fields: Map<String, Value>,
}

struct Snapshot {
    schema: String,
    providers: Vec<SourceRow>,
    selections: Vec<SourceRow>,
}

fn covered_fields(source: &SourceRow) -> Result<BTreeMap<String, String>, String> {
    let mut fields = BTreeMap::new();
    for name in [
        "source",
        "displayName",
        "driverKind",
        "defaultBaseUrl",
        "baseUrl",
        "auth",
        "credential",
        "headers",
        "proxy",
        "timeoutMs",
        "models",
        "modelConfigOverrides",
        "modelsSource",
        "enabled",
    ] {
        fields.insert(
            name.into(),
            format!(
                "{:x}",
                Sha256::digest(
                    serde_json::to_vec(&source.fields[name])
                        .map_err(|_| "Cannot fingerprint Provider field")?
                )
            ),
        );
    }
    let metadata: Option<Map<String, Value>> = json_field(&source.fields, "metadata")?;
    for name in ["enabledModelIds", "disabledModelIds", "modelPreferences"] {
        fields.insert(
            format!("metadata.{name}"),
            format!(
                "{:x}",
                Sha256::digest(
                    serde_json::to_vec(&metadata.as_ref().and_then(|value| value.get(name)))
                        .map_err(|_| "Cannot fingerprint Provider metadata")?
                )
            ),
        );
    }
    Ok(fields)
}

fn read_source(path: PathBuf, cancel: &CancellationToken) -> Result<Snapshot, String> {
    if !path.is_absolute() || !path.is_file() {
        return Err(failure("Import source must be an absolute SQLite file"));
    }
    let mut db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| failure("Cannot open legacy SQLite source read-only"))?;
    db.pragma_update(None, "trusted_schema", false)
        .map_err(|_| failure("Cannot secure legacy SQLite reader"))?;
    let tx = db
        .transaction()
        .map_err(|_| failure("Cannot read legacy snapshot"))?;
    let mut schema = Vec::new();
    let mut bytes = 0usize;
    let mut read = |table: &str, columns: &str| -> Result<Vec<SourceRow>, String> {
        let table_type: String = tx
            .query_row(
                "SELECT type FROM sqlite_master WHERE name=?1",
                [table],
                |row| row.get(0),
            )
            .map_err(|_| failure("Legacy provider tables are missing"))?;
        if table_type != "table" {
            return Err(failure("Legacy provider source must use tables"));
        }
        let columns: Vec<_> = columns.split_whitespace().collect();
        let mut statement = tx
            .prepare(&format!("PRAGMA table_xinfo({table})"))
            .map_err(|_| failure("Cannot inspect legacy schema"))?;
        let mut actual = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|_| failure("Cannot inspect legacy schema"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| failure("Cannot inspect legacy schema"))?;
        if actual.len() != columns.len()
            || columns.iter().any(|expected| {
                !actual.iter().any(|(name, ty, hidden, primary)| {
                    name == expected
                        && *hidden == 0
                        && *primary == i64::from(*expected == columns[0])
                        && ty
                            == if matches!(*expected, "version" | "enabled" | "timeoutMs") {
                                "INTEGER"
                            } else {
                                "TEXT"
                            }
                })
            })
        {
            return Err(failure(
                "Unsupported legacy provider schema; source preserved",
            ));
        }
        actual.sort();
        schema.push(actual);
        let mut statement = tx
            .prepare(&format!(
                "SELECT {} FROM {table} ORDER BY {} LIMIT 1025",
                columns.join(","),
                columns[0]
            ))
            .map_err(|_| failure("Cannot read legacy rows"))?;
        let mut rows = statement
            .query([])
            .map_err(|_| failure("Cannot read legacy rows"))?;
        let mut result = Vec::new();
        while let Some(row) = rows.next().map_err(|_| failure("Cannot read legacy row"))? {
            if cancel.is_cancelled() {
                return Err("Provider import cancelled".into());
            }
            if result.len() == 1024 {
                return Err(failure("Legacy import exceeds 1024 rows per table"));
            }
            let mut fields = Map::new();
            for (index, name) in columns.iter().enumerate() {
                let value = row
                    .get_ref(index)
                    .map_err(|_| failure("Cannot read legacy value"))?;
                bytes += match value {
                    ValueRef::Text(bytes) | ValueRef::Blob(bytes) => bytes.len(),
                    _ => 8,
                };
                if bytes > 8 * 1024 * 1024 {
                    return Err(failure("Legacy import exceeds 8 MiB"));
                }
                let value = match value {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(value) => json!(value),
                    ValueRef::Text(value) => Value::String(
                        std::str::from_utf8(value)
                            .map_err(|_| failure("Invalid legacy text"))?
                            .to_owned(),
                    ),
                    _ => return Err(failure("Invalid legacy SQLite value type")),
                };
                fields.insert((*name).into(), value);
            }
            let id = fields[columns[0]]
                .as_str()
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| failure("Invalid legacy row identity"))?
                .to_owned();
            let version = fields["version"]
                .as_i64()
                .filter(|v| *v > 0 && *v <= dim_agent_types::JS_MAX_SAFE_INTEGER as i64)
                .ok_or_else(|| failure("Invalid legacy row version"))?;
            let digest = format!(
                "{:x}",
                Sha256::digest(
                    serde_json::to_vec(&fields)
                        .map_err(|_| failure("Cannot fingerprint legacy row"))?
                )
            );
            result.push(SourceRow {
                id,
                version,
                digest,
                fields,
            });
        }
        Ok(result)
    };
    let providers = read("providers", PROVIDER_COLUMNS)?;
    let selections = read("provider_selections", SELECTION_COLUMNS)?;
    let schema = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&schema).map_err(failure)?)
    );
    Ok(Snapshot {
        schema,
        providers,
        selections,
    })
}

fn field<T: DeserializeOwned>(fields: &Map<String, Value>, name: &str) -> Result<T, String> {
    serde_json::from_value(fields[name].clone()).map_err(|_| format!("Invalid {name}"))
}
fn json_field<T: DeserializeOwned>(fields: &Map<String, Value>, name: &str) -> Result<T, String> {
    let value = match &fields[name] {
        Value::Null => Value::Null,
        Value::String(text) => {
            serde_json::from_str(text).map_err(|_| format!("Invalid {name} JSON"))?
        }
        _ => return Err(format!("Invalid {name}")),
    };
    serde_json::from_value(value).map_err(|_| format!("Unsupported or invalid {name}"))
}
fn retains(source: &Value, target: &Value) -> bool {
    match (source, target) {
        (Value::Object(source), Value::Object(target)) => source
            .iter()
            .all(|(key, value)| target.get(key).is_some_and(|target| retains(value, target))),
        (Value::Array(source), Value::Array(target)) => {
            source.len() == target.len() && source.iter().zip(target).all(|(s, t)| retains(s, t))
        }
        _ => source == target,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyAuth {
    #[serde(rename = "type")]
    kind: String,
    header_name: Option<String>,
    prefix: Option<String>,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all_fields = "camelCase", deny_unknown_fields)]
enum LegacyCredential {
    #[serde(rename = "apiKey")]
    ApiKey {
        api_key: String,
        header_name: Option<String>,
        prefix: Option<String>,
    },
    #[serde(rename = "bearer")]
    Bearer {
        token: String,
        header_name: Option<String>,
        prefix: Option<String>,
    },
    #[serde(rename = "secretRef")]
    Reference {
        r#ref: String,
        credential_type: Option<String>,
        header_name: Option<String>,
        prefix: Option<String>,
    },
    #[serde(rename = "custom")]
    Custom {
        custom_headers: HashMap<String, String>,
    },
    #[serde(rename = "awsBedrockAccessKey")]
    Aws {
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
    },
}

fn convert_provider(
    source: &SourceRow,
    previous: Option<StaticProviderRecord>,
) -> Result<StaticProviderRecord, String> {
    let fields = &source.fields;
    if managed_authority(&source.id) != CredentialAuthority::Static {
        return Err("Managed OAuth requires its owner's explicit recovery".into());
    }
    for name in ["accountUsageSettings", "retryConfig", "capabilityDefaults"] {
        let value: Option<Value> = json_field(fields, name)?;
        let _ = value;
    }
    let _: Option<String> = field(fields, "activeModelId")?;
    let source_kind: String = field(fields, "source")?;
    let kind = match source_kind.as_str() {
        "custom" if previous.is_none() => StaticRecordKind::Custom,
        "builtin"
            if previous
                .as_ref()
                .is_some_and(|p| p.kind == StaticRecordKind::Builtin && p.version == 1) =>
        {
            StaticRecordKind::Builtin
        }
        "custom" | "builtin" => {
            return Err("Target provider exists or is not a pristine builtin".into());
        }
        _ => return Err("Unsupported source kind".into()),
    };
    let legacy_auth: LegacyAuth = json_field(fields, "auth")?;
    let auth = match legacy_auth.kind.as_str() {
        "api_key" => StaticAuth::ApiKey {
            header_name: legacy_auth
                .header_name
                .unwrap_or_else(|| "Authorization".into()),
            prefix: legacy_auth.prefix.unwrap_or_default(),
        },
        "bearer" if legacy_auth.header_name.is_none() && legacy_auth.prefix.is_none() => {
            StaticAuth::Bearer
        }
        "custom" if legacy_auth.header_name.is_none() && legacy_auth.prefix.is_none() => {
            StaticAuth::Anonymous
        }
        "aws_bedrock" if legacy_auth.header_name.is_none() && legacy_auth.prefix.is_none() => {
            StaticAuth::AwsBedrock
        }
        _ => return Err("Unsupported legacy auth; OAuth credentials must not be imported".into()),
    };
    let mut headers =
        json_field::<Option<HashMap<String, String>>>(fields, "headers")?.unwrap_or_default();
    let credential = json_field::<Option<LegacyCredential>>(fields, "credential")?;
    let check_override = |name: Option<String>, prefix: Option<String>| -> Result<(), String> {
        if let StaticAuth::ApiKey {
            header_name,
            prefix: expected,
        } = &auth
        {
            if name
                .as_ref()
                .is_none_or(|n| n.eq_ignore_ascii_case(header_name))
                && prefix.as_ref().is_none_or(|p| p == expected)
            {
                return Ok(());
            }
        } else if name.is_none() && prefix.is_none() {
            return Ok(());
        }
        Err("Credential auth overrides need explicit reconfiguration".into())
    };
    let credential = match credential {
        None if auth == StaticAuth::Anonymous => {
            return Err("Missing custom credentials cannot become anonymous auth".into());
        }
        None => None,
        Some(LegacyCredential::ApiKey {
            api_key,
            header_name,
            prefix,
        }) if matches!(auth, StaticAuth::ApiKey { .. }) => {
            check_override(header_name, prefix)?;
            Some(StaticCredential::Inline(ProviderSecret::Token(api_key)))
        }
        Some(LegacyCredential::Bearer {
            token,
            header_name,
            prefix,
        }) if matches!(auth, StaticAuth::Bearer | StaticAuth::AwsBedrock) => {
            check_override(header_name, prefix)?;
            Some(StaticCredential::Inline(ProviderSecret::Token(token)))
        }
        Some(LegacyCredential::Reference {
            r#ref,
            credential_type,
            header_name,
            prefix,
        }) => {
            let expected = match auth {
                StaticAuth::ApiKey { .. } => "apiKey",
                StaticAuth::Bearer => "bearer",
                _ => return Err("Unsupported secret reference auth".into()),
            };
            if credential_type
                .as_deref()
                .is_some_and(|kind| kind != expected)
            {
                return Err("Unsupported secret reference type".into());
            }
            check_override(header_name, prefix)?;
            Some(StaticCredential::Reference { reference: r#ref })
        }
        Some(LegacyCredential::Aws {
            access_key_id,
            secret_access_key,
            session_token,
        }) if auth == StaticAuth::AwsBedrock => Some(StaticCredential::Inline(
            ProviderSecret::Aws(AwsCredentials {
                access_key_id,
                secret_access_key,
                session_token,
            }),
        )),
        Some(LegacyCredential::Custom { custom_headers })
            if auth == StaticAuth::Anonymous && !custom_headers.is_empty() =>
        {
            let mut names = HashSet::new();
            for (key, value) in custom_headers {
                if !names.insert(key.to_ascii_lowercase()) {
                    return Err("Ambiguous case-insensitive custom credential headers".into());
                }
                headers.retain(|name, _| !name.eq_ignore_ascii_case(&key));
                headers.insert(key, value);
            }
            None
        }
        _ => return Err("Credential does not match legacy auth".into()),
    };
    let mut models: Vec<ModelEntry> = json_field(fields, "models")?;
    let mut overrides =
        json_field::<Option<Vec<ModelEntry>>>(fields, "modelConfigOverrides")?.unwrap_or_default();
    for (name, models) in [("models", &models), ("modelConfigOverrides", &overrides)] {
        let original: Option<Value> = json_field(fields, name)?;
        if let Some(original) = original
            && !retains(
                &original,
                &serde_json::to_value(models).map_err(|_| "Invalid model serialization")?,
            )
        {
            return Err(format!("Unsupported fields in {name}"));
        }
    }
    let models_source: String = field(fields, "modelsSource")?;
    if !matches!(models_source.as_str(), "bundled" | "remote" | "user") {
        return Err("Invalid modelsSource".into());
    }
    let mut user_models = if models_source == "user" {
        std::mem::take(&mut models)
    } else {
        Vec::new()
    };
    if models_source == "user" {
        if let Some(previous) = &previous {
            models = previous.models.clone();
        }
        overrides.retain(|entry| {
            if let Some(user) = user_models
                .iter_mut()
                .find(|user| user.model_id == entry.model_id)
            {
                *user = entry.clone();
                false
            } else {
                true
            }
        });
    }
    let metadata =
        json_field::<Option<Map<String, Value>>>(fields, "metadata")?.unwrap_or_default();
    let model_preferences: BTreeMap<String, ModelPreferences> = metadata
        .get("modelPreferences")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| "Invalid modelPreferences")?
        .unwrap_or_default();
    let mut disabled: Vec<String> = metadata
        .get("disabledModelIds")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()
        .map_err(|_| "Invalid disabledModelIds")?
        .unwrap_or_default();
    if let Some(enabled) = metadata.get("enabledModelIds") {
        let enabled: Vec<String> =
            serde_json::from_value(enabled.clone()).map_err(|_| "Invalid enabledModelIds")?;
        let enabled: HashSet<_> = enabled
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .collect();
        if disabled.iter().any(|id| enabled.contains(id.as_str())) {
            return Err("Conflicting enabled and disabled model IDs".into());
        }
        disabled.extend(
            models
                .iter()
                .chain(&user_models)
                .filter(|model| !enabled.contains(model.model_id.as_str()))
                .map(|model| model.model_id.clone()),
        );
    }
    disabled.sort();
    disabled.dedup();
    let enabled: i64 = field(fields, "enabled")?;
    if !matches!(enabled, 0 | 1) {
        return Err("Invalid enabled flag".into());
    }
    let mut record = StaticProviderRecord {
        kind,
        display_name: field(fields, "displayName")?,
        driver_kind: field(fields, "driverKind")?,
        auth,
        credential_authority: CredentialAuthority::Static,
        base_url: field::<Option<String>>(fields, "baseUrl")?
            .or(field(fields, "defaultBaseUrl")?)
            .unwrap_or_default(),
        network: ModelNetwork {
            proxy: field(fields, "proxy")?,
            timeout_ms: field(fields, "timeoutMs")?,
        },
        headers,
        credential,
        models,
        remote_models: models_source == "remote",
        endpoint_types: BTreeMap::new(),
        model_config_overrides: overrides,
        user_models,
        model_preferences,
        endpoint_overrides: BTreeMap::new(),
        disabled_model_ids: disabled,
        enabled: enabled == 1,
        definition_fingerprint: previous
            .as_ref()
            .and_then(|p| p.definition_fingerprint.clone()),
        version: if previous.is_some() { 2 } else { 1 },
    };
    if let Some(previous) = previous {
        if record.display_name != previous.display_name
            || record.driver_kind != previous.driver_kind
            || record.auth != previous.auth
        {
            return Err("Legacy builtin definition differs from the current definition".into());
        }
        record.endpoint_types = previous.endpoint_types;
    }
    validate_static_record(&source.id, &mut record)
        .map_err(|_| "Invalid or unsupported provider configuration; source preserved")?;
    Ok(record)
}
fn validate_static_record(
    provider_id: &str,
    record: &mut StaticProviderRecord,
) -> Result<(), String> {
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
        }
    }
    if (record.auth == StaticAuth::AwsBedrock) != (record.driver_kind == "amazon-bedrock") {
        return Err(failure(
            "AWS Bedrock auth is only valid for the Amazon Bedrock driver",
        ));
    }
    if (record.kind == StaticRecordKind::Custom || record.enabled)
        && ![
            "openai-compatible",
            "custom",
            "deepseek",
            "moonshotai",
            "aihubmix",
            "zenmux",
            "openai-responses",
            "aihubmix-responses",
            "azure-openai",
            "xai",
            "xai-responses",
            "anthropic",
            "gemini",
            "amazon-bedrock",
            "new-api-router",
        ]
        .contains(&record.driver_kind.as_str())
    {
        return Err(failure("Unsupported provider driver"));
    }
    for models in [
        &record.models,
        &record.model_config_overrides,
        &record.user_models,
    ] {
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
    if record.user_models.iter().any(|user| {
        record
            .models
            .iter()
            .chain(&record.model_config_overrides)
            .any(|model| model.model_id == user.model_id)
    }) {
        return Err(failure(
            "User models conflict with default models or descriptor overrides",
        ));
    }
    if !record.endpoint_overrides.is_empty()
        && (record.driver_kind != "new-api-router"
            || record.credential_authority != CredentialAuthority::Static
            || record.endpoint_overrides.values().any(|endpoint| {
                !matches!(
                    endpoint,
                    NewApiEndpoint::Openai
                        | NewApiEndpoint::OpenaiResponse
                        | NewApiEndpoint::Anthropic
                        | NewApiEndpoint::Gemini
                )
            }))
    {
        return Err(failure(
            "Endpoint overrides require an executable static New API route",
        ));
    }
    if record.driver_kind == "new-api-router" {
        for model in record
            .models
            .iter()
            .chain(&record.user_models)
            .chain(&record.model_config_overrides)
        {
            if let Some(metadata) = &model.metadata {
                for (name, alias) in [
                    ("endpointType", "endpoint_type"),
                    ("supportedEndpointTypes", "supported_endpoint_types"),
                    ("selectableEndpointTypes", "selectable_endpoint_types"),
                ] {
                    if metadata.raw.contains_key(name) && metadata.raw.contains_key(alias) {
                        return Err("Ambiguous native route metadata".into());
                    }
                    if let Some(value) = metadata.raw.get(name).or_else(|| metadata.raw.get(alias))
                    {
                        let valid = if name == "endpointType" {
                            serde_json::from_value::<Option<NewApiEndpoint>>(value.clone()).is_ok()
                        } else {
                            serde_json::from_value::<Option<Vec<String>>>(value.clone()).is_ok_and(
                                |values| {
                                    values.is_none_or(|values| {
                                        values.iter().all(|value| !value.trim().is_empty())
                                    })
                                },
                            )
                        };
                        if !valid {
                            return Err("Incompatible native route metadata".into());
                        }
                    }
                }
            }
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
            StaticCredential::Inline(value) => validate_secret(value, &record.auth)?,
            StaticCredential::Reference { reference } => {
                if reference.trim().is_empty() || reference.chars().any(char::is_control) {
                    return Err(failure("Invalid provider secret reference"));
                }
            }
        }
    } else if !managed && record.enabled && record.auth != StaticAuth::Anonymous {
        return Err(failure("Enabled provider credential is required"));
    }
    Ok(())
}

fn validate_secret(value: &ProviderSecret, auth: &StaticAuth) -> Result<(), String> {
    match value {
        ProviderSecret::Token(value) => {
            if value.is_empty() {
                return Err("Empty credential".into());
            }
            reqwest::header::HeaderValue::from_str(value).map_err(failure)?;
        }
        ProviderSecret::Aws(value) => {
            if *auth != StaticAuth::AwsBedrock {
                return Err("Invalid AWS credentials".into());
            }
            for field in [
                Some(value.access_key_id.as_str()),
                Some(value.secret_access_key.as_str()),
                value.session_token.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if field.is_empty() || reqwest::header::HeaderValue::from_str(field).is_err() {
                    return Err("Invalid AWS credentials".into());
                }
            }
        }
    }
    Ok(())
}

fn omitted_fields(source: &SourceRow) -> Result<Vec<String>, String> {
    let mut omitted = Vec::new();
    for name in ["accountUsageSettings", "retryConfig", "capabilityDefaults"] {
        let value: Option<Value> = json_field(&source.fields, name)?;
        if value.as_ref().is_some_and(|value| {
            !matches!(value, Value::Object(fields) if fields.is_empty())
                && !matches!(value, Value::Array(items) if items.is_empty())
        }) {
            omitted.push(name.into());
        }
    }
    let metadata: Option<Map<String, Value>> = json_field(&source.fields, "metadata")?;
    for key in metadata
        .into_iter()
        .flat_map(|value| value.into_iter().map(|(key, _)| key))
    {
        if !["enabledModelIds", "disabledModelIds", "modelPreferences"].contains(&key.as_str()) {
            omitted.push(format!("metadata.{key}"));
        }
    }
    Ok(omitted)
}

fn chat_route(record: &StaticProviderRecord, model: &ModelEntry) -> bool {
    if record.driver_kind != "new-api-router" {
        return true;
    }
    let source = record
        .models
        .iter()
        .chain(&record.user_models)
        .find(|source| source.model_id == model.model_id)
        .expect("selected model belongs to the imported provider");
    let model = record
        .model_config_overrides
        .iter()
        .find(|entry| entry.model_id == model.model_id)
        .unwrap_or(source);
    let chat = |endpoint: NewApiEndpoint| {
        matches!(
            endpoint,
            NewApiEndpoint::Openai
                | NewApiEndpoint::OpenaiResponse
                | NewApiEndpoint::Anthropic
                | NewApiEndpoint::Gemini
        )
    };
    let field = |name: &str, alias: &str| {
        model
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.raw.get(name).or_else(|| metadata.raw.get(alias)))
    };
    let explicit = record
        .endpoint_overrides
        .get(&model.model_id)
        .copied()
        .or_else(|| {
            field("endpointType", "endpoint_type")
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .or_else(|| record.endpoint_types.get(&model.model_id).copied())
        });
    if let Some(endpoint) = explicit {
        return chat(endpoint);
    }
    let candidates = field("selectableEndpointTypes", "selectable_endpoint_types")
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
        .or_else(|| {
            field("supportedEndpointTypes", "supported_endpoint_types")
                .and_then(Value::as_array)
                .filter(|values| !values.is_empty())
        })
        .map(|values| {
            values
                .iter()
                .filter_map(|value| serde_json::from_value::<NewApiEndpoint>(value.clone()).ok())
                .fold(Vec::new(), |mut values, endpoint| {
                    if !values.contains(&endpoint) {
                        values.push(endpoint);
                    }
                    values
                })
        });
    let hint = format!(
        "{} {} {}",
        model.model_id,
        field("ownedBy", "owned_by")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        field("capabilityProviderId", "capability_provider_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
    )
    .to_ascii_lowercase();
    for (endpoint, words) in [
        (NewApiEndpoint::Anthropic, &["anthropic", "claude"][..]),
        (NewApiEndpoint::Gemini, &["google", "gemini"][..]),
        (
            NewApiEndpoint::OpenaiResponse,
            &["response", "responses"][..],
        ),
    ] {
        if candidates
            .as_ref()
            .is_none_or(|values| values.contains(&endpoint))
            && hint
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .any(|word| words.contains(&word))
        {
            return true;
        }
    }
    if let Some(endpoint) =
        field("type", "type").and_then(|value| serde_json::from_value(value.clone()).ok())
    {
        return chat(endpoint);
    }
    candidates.is_none_or(|values| {
        values.iter().filter(|endpoint| chat(**endpoint)).count() == 1
            || values.contains(&NewApiEndpoint::Openai)
    })
}

pub(crate) async fn import(
    owner: &Migration,
    task: &TaskState,
    source: &Source,
    target: &Native,
) -> MigrationResult<()> {
    if !source.inventory.tables.contains_key("providers") {
        return Ok(());
    }
    let snapshot = read_source(source.snapshot.clone(), &owner.stopping)
        .map_err(|_| Error::Object("Legacy provider schema is unsupported".into()))?;
    let bundled = BundledCatalog::load().map_err(crate::native::failure)?;
    let mut imported = HashSet::new();
    for (kind, rows) in [
        ("provider", snapshot.providers),
        ("selection", snapshot.selections),
    ] {
        for row in rows {
            crate::engine::check(owner, &task.id).await?;
            let omitted = if kind == "provider" {
                omitted_fields(&row).unwrap_or_default()
            } else {
                Vec::new()
            };
            let result = (|| -> MigrationResult<Outcome> {
                let mut db = target.db.lock().unwrap();
                let tx = db.transaction().map_err(crate::native::failure)?;
                let seen: Option<String> = tx
                    .query_row(
                        "SELECT source_digest FROM provider_imports WHERE kind=?1 AND id=?2",
                        params![kind, row.id],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(crate::native::failure)?;
                let coverage_key = format!(
                    "migration/provider/{}",
                    crate::source::fingerprint(&row.id)?
                );
                if let Some(seen) = seen {
                    if kind == "provider" {
                        let current: Option<String> = tx
                            .query_row(
                                "SELECT record FROM providers WHERE provider_id=?1",
                                [&row.id],
                                |r| r.get(0),
                            )
                            .optional()
                            .map_err(crate::native::failure)?;
                        if current.is_none() {
                            return Ok(Outcome::Deleted);
                        }
                        let coverage: Option<String> = tx
                            .query_row(
                                "SELECT document FROM settings_documents WHERE source=?1",
                                [&coverage_key],
                                |r| r.get(0),
                            )
                            .optional()
                            .map_err(crate::native::failure)?;
                        if let Some(coverage) = coverage {
                            let coverage: Value =
                                serde_json::from_str(&coverage).map_err(crate::native::failure)?;
                            let fields = covered_fields(&row).map_err(Error::Object)?;
                            let previous: BTreeMap<String, String> =
                                serde_json::from_value(coverage["fields"].clone())
                                    .map_err(crate::native::failure)?;
                            if previous
                                .iter()
                                .any(|(field, digest)| fields.get(field) != Some(digest))
                            {
                                return Ok(Outcome::Conflict);
                            }
                            if fields.len() > previous.len() {
                                let current: Value =
                                    serde_json::from_str(current.as_deref().unwrap())
                                        .map_err(crate::native::failure)?;
                                if current != coverage["target"] {
                                    return Ok(Outcome::Conflict);
                                }
                                let current: StaticProviderRecord = serde_json::from_value(current)
                                    .map_err(crate::native::failure)?;
                                let version = current.version.checked_add(1).ok_or_else(|| {
                                    Error::Target("Provider version exhausted".into())
                                })?;
                                let previous = if current.kind == StaticRecordKind::Builtin {
                                    let mut previous = current;
                                    previous.version = 1;
                                    Some(previous)
                                } else {
                                    None
                                };
                                let mut converted =
                                    convert_provider(&row, previous).map_err(Error::Object)?;
                                converted.version = version;
                                let converted = serde_json::to_value(converted)
                                    .map_err(crate::native::failure)?;
                                tx.execute("UPDATE providers SET version=?2,record=?3 WHERE provider_id=?1", params![row.id,version,converted.to_string()]).map_err(crate::native::failure)?;
                                Native::save_document(
                                    &tx,
                                    &coverage_key,
                                    &json!({"fields":fields,"target":converted}),
                                )?;
                                tx.commit().map_err(crate::native::failure)?;
                                return Ok(Outcome::Imported);
                            }
                            return Ok(Outcome::AlreadyImported);
                        }
                    }
                    return Ok(if seen == row.digest {
                        Outcome::AlreadyImported
                    } else {
                        Outcome::Conflict
                    });
                }
                if kind == "provider" {
                    let previous: Option<(u32, String)> = tx
                        .query_row(
                            "SELECT version,record FROM providers WHERE provider_id=?1",
                            [&row.id],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()
                        .map_err(crate::native::failure)?;
                    let previous = previous
                        .map(|(version, body)| -> MigrationResult<StaticProviderRecord> {
                            let record: StaticProviderRecord = serde_json::from_str(&body)
                                .map_err(|_| {
                                    Error::Object("Unreadable native provider is preserved".into())
                                })?;
                            if record.version != version {
                                return Err(Error::Object(
                                    "Native provider version mismatch; target preserved".into(),
                                ));
                            }
                            Ok(record)
                        })
                        .transpose()?;
                    if previous.as_ref().is_some_and(|record| {
                        record.kind != StaticRecordKind::Builtin || record.version != 1
                    }) {
                        return Ok(Outcome::Conflict);
                    }
                    let mut source = row.fields.clone();
                    let fresh_builtin =
                        previous.is_none() && source.get("source") == Some(&json!("builtin"));
                    if fresh_builtin {
                        source.insert("source".into(), json!("custom"));
                    }
                    let mut converted = convert_provider(
                        &SourceRow {
                            id: row.id.clone(),
                            version: row.version,
                            digest: row.digest.clone(),
                            fields: source,
                        },
                        previous,
                    )
                    .map_err(Error::Object)?;
                    if fresh_builtin {
                        converted.kind = StaticRecordKind::Builtin;
                    }
                    let converted =
                        serde_json::to_value(&converted).map_err(crate::native::failure)?;
                    tx.execute(
                        "INSERT INTO providers VALUES (?1,?2,?3) ON CONFLICT(provider_id) DO UPDATE SET version=excluded.version,record=excluded.record",
                        params![
                            row.id,
                            converted["version"].as_i64().unwrap(),
                            converted.to_string()
                        ],
                    )
                    .map_err(crate::native::failure)?;
                    Native::save_document(
                        &tx,
                        &coverage_key,
                        &json!({"fields":covered_fields(&row).map_err(Error::Object)?,"target":converted}),
                    )?;
                } else {
                    let scope = match row.fields["scope"].as_str() {
                        Some("global") => "global".to_owned(),
                        Some("workspace") => {
                            let cwd = row.fields["cwd"].as_str().ok_or_else(|| {
                                Error::Object("Missing workspace selection directory".into())
                            })?;
                            if !std::path::Path::new(cwd).is_absolute() {
                                return Err(Error::Object(
                                    "Invalid workspace selection directory".into(),
                                ));
                            }
                            format!("workspace:{cwd}")
                        }
                        _ => return Err(Error::Object("Unknown provider selection scope".into())),
                    };
                    let exists: bool = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM provider_selections WHERE scope=?1)",
                            [&scope],
                            |r| r.get(0),
                        )
                        .map_err(crate::native::failure)?;
                    if exists {
                        return Ok(Outcome::Conflict);
                    }
                    let provider = row.fields["providerId"]
                        .as_str()
                        .ok_or_else(|| Error::Object("Missing provider selection".into()))?;
                    let model = row.fields["modelId"].as_str().ok_or_else(|| {
                        Error::Object("Implicit model selection is retained in source".into())
                    })?;
                    if !imported.contains(provider) {
                        return Err(Error::Object(
                            "Selected provider was not imported; selection retained in source"
                                .into(),
                        ));
                    }
                    let body: String = tx
                        .query_row(
                            "SELECT record FROM providers WHERE provider_id=?1",
                            [provider],
                            |r| r.get(0),
                        )
                        .map_err(crate::native::failure)?;
                    let record: StaticProviderRecord =
                        serde_json::from_str(&body).map_err(crate::native::failure)?;
                    let models: Vec<_> = record
                        .models
                        .iter()
                        .chain(&record.user_models)
                        .cloned()
                        .collect();
                    let available = record.enabled
                        && !record.disabled_model_ids.iter().any(|id| id == model)
                        && models.iter().any(|source| source.model_id == model)
                        && bundled
                            .effective_models(
                                &ProviderProjection {
                                    provider_id: provider.into(),
                                    source: Some(if record.kind == StaticRecordKind::Builtin {
                                        ModelSource::Builtin
                                    } else {
                                        ModelSource::Custom
                                    }),
                                    models: models.clone(),
                                    ..Default::default()
                                },
                                &models,
                                &record.model_config_overrides,
                            )
                            .iter()
                            .any(|entry| {
                                entry.model_id == model
                                    && entry.status != ModelStatus::Unavailable
                                    && entry
                                        .capabilities
                                        .modalities
                                        .iter()
                                        .any(|kind| kind == "text")
                                    && chat_route(&record, entry)
                                    && (record.driver_kind == "new-api-router"
                                        || entry
                                            .metadata
                                            .as_ref()
                                            .and_then(|metadata| metadata.raw.get("type"))
                                            .and_then(Value::as_str)
                                            .is_none_or(|kind| kind == "chat"))
                            });
                    if !available {
                        return Err(Error::Object(
                            "Selected model is unavailable; selection retained in source".into(),
                        ));
                    }
                    tx.execute(
                        "INSERT INTO provider_selections VALUES (?1,1,?2,?3)",
                        params![scope, provider, model],
                    )
                    .map_err(crate::native::failure)?;
                }
                tx.execute(
                    "INSERT INTO provider_imports VALUES (?1,?2,?3,?4,?5)",
                    params![kind, row.id, snapshot.schema, row.version, row.digest],
                )
                .map_err(crate::native::failure)?;
                tx.commit().map_err(crate::native::failure)?;
                Ok(Outcome::Imported)
            })();
            let (outcome, reason) = match result {
                Ok(outcome) => (outcome, None),
                Err(Error::Object(reason)) => (Outcome::Unsupported, Some(reason)),
                Err(e) => return Err(e),
            };
            if kind == "provider" && matches!(outcome, Outcome::Imported | Outcome::AlreadyImported)
            {
                imported.insert(row.id.clone());
            }
            owner
                .ledger
                .object(
                    &task.id,
                    &ObjectResult {
                        kind: kind.into(),
                        id: row.id,
                        outcome,
                        target_id: None,
                        reason,
                        omitted,
                    },
                )
                .await?;
        }
    }
    Ok(())
}
