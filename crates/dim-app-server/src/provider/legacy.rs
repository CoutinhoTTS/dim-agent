//! Explicit, read-only TS configuration import. No sessions or OAuth grants.
use super::*;
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use serde::de::DeserializeOwned;
use serde_json::{Map, json};
use std::path::PathBuf;

const PROVIDER_COLUMNS: &str = "providerId source displayName driverKind defaultBaseUrl baseUrl auth credential accountUsageSettings headers proxy timeoutMs retryConfig models modelConfigOverrides capabilityDefaults activeModelId modelsSource catalogVersion modelsUpdatedAt metadata enabled createdAt updatedAt version";
const SELECTION_COLUMNS: &str =
    "selectionId scope cwd providerId modelId createdAt updatedAt version";

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    Imported,
    AlreadyImported,
    Rejected,
}

#[derive(Debug, Serialize)]
pub struct ImportItem {
    pub kind: &'static str,
    pub id: String,
    pub status: ImportStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub items: Vec<ImportItem>,
}

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

fn read_source(path: PathBuf, cancel: &CancellationToken) -> Result<Snapshot, HostError> {
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
    let mut read = |table: &str, columns: &str| -> Result<Vec<SourceRow>, HostError> {
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
                return Err(HostError::Cancelled("Provider import cancelled".into()));
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

// Parsing and loss checks live only at this legacy external-data boundary.
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
    for name in [
        "accountUsageSettings",
        "retryConfig",
        "capabilityDefaults",
        "activeModelId",
    ] {
        if !fields[name].is_null() {
            return Err(format!("Unsupported legacy field: {name}"));
        }
    }
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
    let overrides =
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
    for model in &overrides {
        if !models.iter().any(|m| m.model_id == model.model_id) {
            models.push(model.clone());
        }
    }
    let metadata =
        json_field::<Option<Map<String, Value>>>(fields, "metadata")?.unwrap_or_default();
    if metadata
        .keys()
        .any(|key| !matches!(key.as_str(), "enabledModelIds" | "disabledModelIds"))
    {
        return Err("Unsupported provider metadata; source preserved".into());
    }
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
                .filter(|model| !enabled.contains(model.model_id.as_str()))
                .map(|model| model.model_id.clone()),
        );
    }
    disabled.sort();
    disabled.dedup();
    let models_source: String = field(fields, "modelsSource")?;
    if !matches!(models_source.as_str(), "bundled" | "remote" | "user") {
        return Err("Invalid modelsSource".into());
    }
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
        remote_models: true,
        endpoint_types: BTreeMap::new(),
        routes: HashMap::new(),
        bedrock_region: None,
        model_config_overrides: overrides,
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

impl ProviderService {
    pub async fn import_legacy(
        self: &Arc<Self>,
        source_path: PathBuf,
        cancel: &CancellationToken,
    ) -> Result<ImportResult, HostError> {
        let owner = self.clone();
        self.owned(cancel, move |cancel| Box::pin(async move {
            let read_cancel = cancel.clone();
            let snapshot = tokio::task::spawn_blocking(move || read_source(source_path, &read_cancel)).await.map_err(|_| failure("Legacy reader failed"))??;
            owner.check(&cancel)?;
            let bundled = owner.bundled.clone();
            owner.db.run(move |connection| {
                if cancel.is_cancelled() { return Err(storage("Provider import cancelled")); }
                let tx = connection.transaction().map_err(storage)?;
                let mut items = Vec::new();
                let mut imported = HashSet::new();
                for (kind, rows) in [("provider", snapshot.providers), ("selection", snapshot.selections)] {
                    for source in rows {
                        let seen: Option<String> = tx.query_row("SELECT source_digest FROM provider_imports WHERE kind=?1 AND id=?2", params![kind, source.id], |row| row.get(0)).optional().map_err(storage)?;
                        if let Some(digest) = seen {
                            items.push(ImportItem { kind, id: source.id, status: ImportStatus::AlreadyImported, reason: Some(if digest == source.digest { "Source row was already imported" } else { "Previously imported identity changed; automatic replay refused" }.into()) });
                            continue;
                        }
                        let result: Result<(), String> = if kind == "provider" {
                            let previous = tx.query_row("SELECT version,record FROM providers WHERE provider_id=?1", [&source.id], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))).optional().map_err(storage)?;
                            let previous = previous.map(|(version, body)| parse_static(&source.id, version, &body)).transpose().map_err(|_| "Target provider is unreadable; preserved".to_owned());
                            match previous.and_then(|previous| convert_provider(&source, previous)) {
                                Err(reason) => Err(reason),
                                Ok(record) if record.kind == StaticRecordKind::Custom && bundled.provider(&source.id).is_some() => Err("Builtin identity cannot be imported as custom".into()),
                                Ok(record) => {
                                    tx.execute("INSERT INTO providers(provider_id,version,record) VALUES (?1,?2,?3) ON CONFLICT(provider_id) DO UPDATE SET version=excluded.version,record=excluded.record", params![source.id, record.version, serde_json::to_string(&record).map_err(storage)?]).map_err(storage)?;
                                    imported.insert(source.id.clone());
                                    Ok(())
                                }
                            }
                        } else {
                            match convert_selection(&source, &imported) {
                                Err(reason) => Err(reason),
                                Ok((scope, provider, model)) => {
                                    let existing: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_selections WHERE scope=?1)", [&scope], |row| row.get(0)).map_err(storage)?;
                                    let record = read_static(&tx, &provider)?.ok_or_else(|| storage("Imported provider missing"))?;
                                    if existing { Err("Target selection already exists".into()) }
                                    else if static_catalog(&bundled, &provider, &record, true).selectable(&model).is_none() { Err("Selected model is unavailable".into()) }
                                    else {
                                        tx.execute("INSERT INTO provider_selections(scope,version,provider_id,model_id) VALUES (?1,1,?2,?3)", params![scope, provider, model]).map_err(storage)?;
                                        Ok(())
                                    }
                                }
                            }
                        };
                        match result {
                            Ok(()) => {
                                tx.execute("INSERT INTO provider_imports(kind,id,source_schema,source_version,source_digest) VALUES (?1,?2,?3,?4,?5)", params![kind, source.id, snapshot.schema, source.version, source.digest]).map_err(storage)?;
                                items.push(ImportItem { kind, id: source.id, status: ImportStatus::Imported, reason: None });
                            }
                            Err(reason) => items.push(ImportItem { kind, id: source.id, status: ImportStatus::Rejected, reason: Some(reason) }),
                        }
                    }
                }
                tx.commit().map_err(storage)?;
                Ok(ImportResult { items })
            }).await.map_err(failure)
        })).await
    }
}

fn convert_selection(
    source: &SourceRow,
    imported: &HashSet<String>,
) -> Result<(String, String, String), String> {
    let scope: String = field(&source.fields, "scope")?;
    let cwd: Option<String> = field(&source.fields, "cwd")?;
    let scope = match (scope.as_str(), cwd) {
        ("global", None) => "global".into(),
        ("workspace", Some(cwd)) => {
            let path = Path::new(&cwd);
            if !path.is_absolute() {
                return Err("Workspace path must be absolute".into());
            }
            let canonical = path
                .canonicalize()
                .map_err(|_| "Workspace directory is unavailable")?;
            if !canonical.is_dir() {
                return Err("Workspace is not a directory".into());
            }
            selection_scope(Some(&canonical)).map_err(|_| "Invalid workspace path")?
        }
        _ => return Err("Invalid selection scope".into()),
    };
    let provider: String = field(&source.fields, "providerId")?;
    let model: String =
        field(&source.fields, "modelId").map_err(|_| "Implicit model selection is not imported")?;
    if !imported.contains(&provider) {
        return Err("Selection requires its provider to be imported in this batch".into());
    }
    Ok((scope, provider, model))
}
