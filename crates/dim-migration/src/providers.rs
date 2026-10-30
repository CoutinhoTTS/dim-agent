use crate::{
    Error, Migration, ObjectResult, Outcome, Result as MigrationResult, TaskState, native::Native,
    source::Source,
};
use dim_app_server::{
    model_catalog::BundledCatalog,
    provider::{
        LEGACY_METADATA_FIELDS, LEGACY_PROVIDER_FIELDS, LegacyRow, StaticProviderRecord,
        convert_legacy_provider, convert_legacy_selection, legacy_omitted_fields, parse_static,
        read_legacy_source,
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

fn covered_fields(source: &LegacyRow) -> Result<BTreeMap<String, String>, String> {
    let mut fields = BTreeMap::new();
    for name in LEGACY_PROVIDER_FIELDS {
        fields.insert((*name).to_owned(), digest(&source.fields[*name])?);
    }
    let metadata: Option<Map<String, Value>> = source.json_field("metadata")?;
    for name in LEGACY_METADATA_FIELDS {
        fields.insert(
            format!("metadata.{name}"),
            digest(&metadata.as_ref().and_then(|value| value.get(*name)))?,
        );
    }
    Ok(fields)
}

fn digest(value: &impl serde::Serialize) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|_| "Cannot fingerprint Provider field")?)
    ))
}

/// The target record for `provider_id`, parsed and validated by Provider.
fn read_target(
    connection: &Connection,
    provider_id: &str,
) -> MigrationResult<Option<StaticProviderRecord>> {
    let row: Option<(u32, String)> = connection
        .query_row(
            "SELECT version,record FROM providers WHERE provider_id=?1",
            [provider_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(crate::native::failure)?;
    row.map(|(version, body)| {
        parse_static(provider_id, version, &body)
            .map_err(|_| Error::Object("Unreadable native provider is preserved".into()))
    })
    .transpose()
}

/// `read_target` for a provider a previous run imported; a missing or
/// unreadable record simply leaves it out of the selection mapping.
fn imported_target(connection: &Connection, provider_id: &str) -> Option<StaticProviderRecord> {
    read_target(connection, provider_id).ok().flatten()
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
    let snapshot = read_legacy_source(source.snapshot.clone(), &owner.stopping)
        .map_err(|_| Error::Object("Legacy provider schema is unsupported".into()))?;
    let bundled = BundledCatalog::load().map_err(crate::native::failure)?;
    let mut imported: HashMap<String, StaticProviderRecord> = HashMap::new();
    for (kind, rows) in [
        ("provider", snapshot.providers),
        ("selection", snapshot.selections),
    ] {
        for row in rows {
            crate::engine::check(owner, &task.id).await?;
            let omitted = if kind == "provider" {
                legacy_omitted_fields(&row).unwrap_or_default()
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
                                let current = read_target(&tx, &row.id)?.ok_or_else(|| {
                                    Error::Object("Unreadable native provider is preserved".into())
                                })?;
                                if serde_json::to_value(&current).map_err(crate::native::failure)?
                                    != coverage["target"]
                                {
                                    return Ok(Outcome::Conflict);
                                }
                                let version =
                                    current.version().checked_add(1).ok_or_else(|| {
                                        Error::Target("Provider version exhausted".into())
                                    })?;
                                let previous =
                                    current.is_builtin().then(|| current.with_version(1));
                                let converted = convert_legacy_provider(&row, previous, &bundled)
                                    .map_err(Error::Object)?
                                    .record
                                    .with_version(version);
                                let converted_value = serde_json::to_value(&converted)
                                    .map_err(crate::native::failure)?;
                                tx.execute("UPDATE providers SET version=?2,record=?3 WHERE provider_id=?1", params![row.id,version,converted_value.to_string()]).map_err(crate::native::failure)?;
                                Native::save_document(
                                    &tx,
                                    &coverage_key,
                                    &json!({"fields":fields,"target":converted_value}),
                                )?;
                                tx.commit().map_err(crate::native::failure)?;
                                imported.insert(row.id.clone(), converted);
                                return Ok(Outcome::Imported);
                            }
                            if let Some(record) = imported_target(&tx, &row.id) {
                                imported.insert(row.id.clone(), record);
                            }
                            return Ok(Outcome::AlreadyImported);
                        }
                    }
                    if seen == row.digest {
                        if kind == "provider"
                            && let Some(record) = imported_target(&tx, &row.id)
                        {
                            imported.insert(row.id.clone(), record);
                        }
                        return Ok(Outcome::AlreadyImported);
                    }
                    return Ok(Outcome::Conflict);
                }
                if kind == "provider" {
                    let previous = read_target(&tx, &row.id)?;
                    if previous
                        .as_ref()
                        .is_some_and(|record| !record.is_pristine_builtin())
                    {
                        return Ok(Outcome::Conflict);
                    }
                    let conversion =
                        convert_legacy_provider(&row, previous, &bundled).map_err(Error::Object)?;
                    let converted =
                        serde_json::to_value(&conversion.record).map_err(crate::native::failure)?;
                    tx.execute(
                        "INSERT INTO providers VALUES (?1,?2,?3) ON CONFLICT(provider_id) DO UPDATE SET version=excluded.version,record=excluded.record",
                        params![
                            row.id,
                            conversion.record.version(),
                            converted.to_string()
                        ],
                    )
                    .map_err(crate::native::failure)?;
                    Native::save_document(
                        &tx,
                        &coverage_key,
                        &json!({"fields":covered_fields(&row).map_err(Error::Object)?,"target":converted}),
                    )?;
                    imported.insert(row.id.clone(), conversion.record);
                } else {
                    let selection = convert_legacy_selection(&row, &imported, &bundled)
                        .map_err(Error::Object)?;
                    let exists: bool = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM provider_selections WHERE scope=?1)",
                            [&selection.scope],
                            |r| r.get(0),
                        )
                        .map_err(crate::native::failure)?;
                    if exists {
                        return Ok(Outcome::Conflict);
                    }
                    tx.execute(
                        "INSERT INTO provider_selections VALUES (?1,1,?2,?3)",
                        params![selection.scope, selection.provider_id, selection.model_id],
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
