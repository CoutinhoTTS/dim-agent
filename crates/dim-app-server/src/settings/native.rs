//! User-source SQLite authority. Import commits the document and provenance atomically.
use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;

fn key(source: &str) -> String {
    format!("user/{source}")
}
fn token(source: &str, version: i64) -> String {
    format!("native:{}:{version}", key(source))
}
fn read<F: Format>(
    connection: &Connection,
    format: &F,
) -> Result<Option<(i64, SettingsSnapshot<F::Document>)>> {
    let row: Option<(i64, String)> = connection
        .query_row(
            "SELECT version, document FROM settings_documents WHERE source = ?1",
            [key(format.source())],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| SettingsError::Persistence)?;
    row.map(|(version, body)| {
        if version <= 0 {
            return Err(SettingsError::Invalid(
                "invalid persisted settings revision",
            ));
        }
        let mut snapshot = snapshot(format, Ok(body.into_bytes()))?;
        snapshot.revision = token(format.source(), version);
        Ok((version, snapshot))
    })
    .transpose()
}
impl SettingsService {
    pub(crate) async fn fill_missing(
        self: &Arc<Self>,
        source: String,
        document: Value,
        cancel: &CancellationToken,
    ) -> Result<Value> {
        self.check_open(cancel)?;
        let fields = document
            .as_object()
            .ok_or(SettingsError::Invalid("fill document must be an object"))?
            .clone();
        let registered = self
            .sources
            .get(&source)
            .ok_or(SettingsError::UnknownSource)?;
        let path = self
            .files
            .path(self.home_root, &registered.value.user_file)?;
        let db = self
            .database
            .as_ref()
            .ok_or(SettingsError::Invalid("fill requires native storage"))?
            .clone();
        let file = self.files.read(&path, cancel).await;
        let validate = move |value: &Value| registered.value.prepare(value);
        let owner = self.clone();
        let cancel = cancel.clone();
        let source_key = key(&source);
        db.run(move |connection| {
            let result = (|| {
                owner.check_open(&cancel)?;
                let tx = connection.transaction().map_err(|_| SettingsError::Persistence)?;
                let stored: Option<(i64, String, String)> = tx.query_row(
                    "SELECT version,document,imported_revision FROM settings_documents WHERE source=?1", [&source_key],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                ).optional().map_err(|_| SettingsError::Persistence)?;
                let (version, mut current, imported, mut configured) = match stored {
                    Some((version, body, origin)) => {
                        let value: Value = serde_json::from_str(&body).map_err(|_| SettingsError::Invalid("invalid persisted document"))?;
                        let presence: Option<String> = tx.query_row("SELECT fields FROM settings_import_fields WHERE source=?1",[&source_key],|row|row.get(0))
                            .optional().map_err(|_|SettingsError::Persistence)?;
                        let configured = match presence {
                            Some(fields) => serde_json::from_str::<BTreeSet<String>>(&fields).map_err(|_|SettingsError::Invalid("invalid settings field provenance"))?,
                            None if version==1 && origin=="missing" => BTreeSet::new(),
                            None => value.as_object().ok_or(SettingsError::Invalid("source document must be an object"))?.keys().cloned().collect(),
                        };
                        (version, value, origin, configured)
                    },
                    None => {
                        let bytes = match file {
                            Ok(bytes) => Some(bytes),
                            Err(FileStorageError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
                            Err(error) => return Err(error.into()),
                        };
                        let imported = revision(bytes.as_deref());
                        let initial: Value = bytes.map(|bytes| serde_json::from_slice(&bytes)
                            .map_err(|_| SettingsError::Invalid("invalid source document")))
                            .transpose()?.unwrap_or_else(|| serde_json::json!({}));
                        let configured = initial.as_object().ok_or(SettingsError::Invalid("source document must be an object"))?.keys().cloned().collect();
                        (0, validate(&initial)?, imported, configured)
                    },
                };
                let mut results = serde_json::Map::new();
                let mut candidate = current.clone();
                let candidates = candidate.as_object_mut().ok_or(SettingsError::Invalid("source document must be an object"))?;
                for (field,value) in &fields { if !configured.contains(field) { candidates.insert(field.clone(),value.clone()); } }
                let valid_group = validate(&candidate).ok();
                for (field, value) in &fields {
                    if configured.contains(field) {
                        results.insert(field.clone(), serde_json::json!(if current.get(field) == Some(value) { "unchanged" } else { "conflict" }));
                        continue;
                    }
                    let mut candidate = current.clone();
                    candidate[field] = value.clone();
                    let prepared = if valid_group.is_some() { Ok(candidate) } else { validate(&candidate) };
                    let Ok(candidate) = prepared else {
                        results.insert(field.clone(), serde_json::json!("rejected"));
                        continue;
                    };
                    current = candidate;
                    configured.insert(field.clone());
                    results.insert(field.clone(), serde_json::json!("imported"));
                }
                if results.values().any(|value| value == "imported") {
                    if let Some(prepared) = valid_group { current = prepared; }
                    let version = version.checked_add(1).ok_or(SettingsError::Invalid("settings revision overflow"))?;
                    let body = serde_json::to_string(&current).map_err(|_| SettingsError::Invalid("cannot encode source document"))?;
                    tx.execute("INSERT INTO settings_documents(source,version,document,imported_revision) VALUES (?1,?2,?3,?4) ON CONFLICT(source) DO UPDATE SET version=excluded.version,document=excluded.document,imported_revision=excluded.imported_revision", params![source_key,version,body,imported]).map_err(|_| SettingsError::Persistence)?;
                    let fields=serde_json::to_string(&configured).map_err(|_|SettingsError::Invalid("cannot encode settings field provenance"))?;
                    tx.execute("INSERT INTO settings_import_fields(source,fields) VALUES (?1,?2) ON CONFLICT(source) DO UPDATE SET fields=excluded.fields",params![source_key,fields]).map_err(|_|SettingsError::Persistence)?;
                    tx.commit().map_err(|_| SettingsError::Persistence)?;
                    owner.publish(&source, SettingsScope::User, &token(&source, version));
                }
                Ok(Value::Object(results))
            })();
            Ok(result)
        }).await.map_err(|_| SettingsError::Persistence)?
    }
    pub(super) async fn native_read<F: Format>(
        &self,
        format: Arc<F>,
        path: FilePath,
        cancel: &CancellationToken,
    ) -> Result<SettingsSnapshot<F::Document>> {
        let task = {
            let _guard = self.gate.lock().unwrap();
            self.check_open(cancel)?;
            let db = self
                .database
                .as_ref()
                .expect("native settings binding")
                .clone();
            let files = self.files.clone();
            let cancel = cancel.clone();
            let stopping = self.stopping.clone();
            self.updates.spawn(async move {
                let stored = format.clone();
                if let Some((_, snapshot)) = db
                    .run(move |connection| Ok(read(connection, &*stored)))
                    .await
                    .map_err(|_| SettingsError::Persistence)??
                {
                    return Ok(snapshot);
                }
                let file = files.read(&path, &cancel).await;
                let original = file.as_ref().ok().cloned();
                let imported = snapshot(&*format, file)?;
                let mut document = serde_json::to_value(&imported.document)
                    .map_err(|_| SettingsError::Invalid("cannot encode settings"))?;
                let fields = match original {
                    Some(bytes) => {
                        let Value::Object(fields) = serde_json::from_slice::<Value>(&bytes)
                            .map_err(|_| SettingsError::Invalid("invalid settings document"))?
                            else { return Err(SettingsError::Invalid("source document must be an object")); };
                        let configured = fields.keys().cloned().collect::<BTreeSet<_>>();
                        document.as_object_mut().ok_or(SettingsError::Invalid("source document must be an object"))?.extend(fields);
                        configured
                    },
                    None => BTreeSet::new(),
                };
                let body = serde_json::to_string(&document).map_err(|_| SettingsError::Invalid("cannot encode settings"))?;
                let fields = serde_json::to_string(&fields).map_err(|_| SettingsError::Invalid("cannot encode settings field provenance"))?;
                db.run(move |connection| {
                    let result = (|| {
                        if stopping.is_cancelled() {
                            return Err(SettingsError::Closed);
                        }
                        if cancel.is_cancelled() {
                            return Err(SettingsError::Cancelled);
                        }
                        let tx = connection.transaction().map_err(|_| SettingsError::Persistence)?;
                        let inserted = tx.execute(
                            "INSERT INTO settings_documents(source, version, document, imported_revision) VALUES (?1, 1, ?2, ?3) ON CONFLICT(source) DO NOTHING",
                            params![key(format.source()), body, imported.revision],
                        ).map_err(|_| SettingsError::Persistence)?;
                        if inserted > 0 {
                            tx.execute("INSERT INTO settings_import_fields(source,fields) VALUES (?1,?2)",params![key(format.source()),fields]).map_err(|_| SettingsError::Persistence)?;
                        }
                        let (_, snapshot) = read(&tx, &*format)?.expect("source inserted or already present");
                        tx.commit().map_err(|_| SettingsError::Persistence)?;
                        Ok(snapshot)
                    })();
                    Ok(result)
                }).await.map_err(|_| SettingsError::Persistence)?
            })
        };
        task.await?
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn native_replace<F: Format>(
        self: &Arc<Self>,
        format: Arc<F>,
        path: FilePath,
        expected: String,
        document: F::Document,
        configured_fields: Option<BTreeSet<String>>,
        check: impl FnOnce(&F::Document, &F::Document) -> Result<()> + Send + 'static,
        cancel: &CancellationToken,
    ) -> Result<SettingsSnapshot<F::Document>> {
        self.native_read(format.clone(), path, cancel).await?;
        self.check_open(cancel)?;
        let owner = self.clone();
        let cancel = cancel.clone();
        self.database
            .as_ref()
            .expect("native settings binding")
            .run(move |connection| {
                let result = (|| {
                    owner.check_open(&cancel)?;
                    let tx = connection
                        .transaction()
                        .map_err(|_| SettingsError::Persistence)?;
                    let (version, current) = read(&tx, &*format)?.ok_or(
                        SettingsError::Invalid("persisted settings source is missing"),
                    )?;
                    if current.revision != expected {
                        return Err(SettingsError::Conflict);
                    }
                    check(&current.document, &document)?;
                    let scoped = configured_fields.is_some();
                    let mut presence_changed = false;
                    let presence = if let Some(fields) = configured_fields {
                        let (origin, saved, body): (String, Option<String>, String) = tx.query_row(
                            "SELECT d.imported_revision,p.fields,d.document FROM settings_documents d LEFT JOIN settings_import_fields p ON p.source=d.source WHERE d.source=?1",
                            [key(format.source())], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
                        ).map_err(|_|SettingsError::Persistence)?;
                        let mut configured = match saved {
                            Some(saved) => serde_json::from_str::<BTreeSet<String>>(&saved).map_err(|_|SettingsError::Invalid("invalid settings field provenance"))?,
                            None if version==1 && origin=="missing" => BTreeSet::new(),
                            None => serde_json::from_str::<Value>(&body).map_err(|_|SettingsError::Invalid("invalid persisted document"))?
                                .as_object().ok_or(SettingsError::Invalid("source document must be an object"))?.keys().cloned().collect(),
                        };
                        presence_changed = fields.iter().any(|field| !configured.contains(field));
                        configured.extend(fields);
                        Some(configured)
                    } else { None };
                    let body = serde_json::to_string(&document)
                        .map_err(|_| SettingsError::Invalid("cannot encode settings"))?;
                    if scoped && !presence_changed
                        && serde_json::to_value(&current.document).map_err(|_|SettingsError::Invalid("cannot encode settings"))?
                            == serde_json::to_value(&document).map_err(|_|SettingsError::Invalid("cannot encode settings"))?
                    {
                        return Ok(current);
                    }
                    let version = version
                        .checked_add(1)
                        .ok_or(SettingsError::Invalid("settings revision overflow"))?;
                    tx.execute(
                        "UPDATE settings_documents SET version = ?1, document = ?2 WHERE source = ?3",
                        params![version, body, key(format.source())],
                    )
                    .map_err(|_| SettingsError::Persistence)?;
                    if let Some(fields) = presence {
                        let fields=serde_json::to_string(&fields).map_err(|_|SettingsError::Invalid("cannot encode settings field provenance"))?;
                        tx.execute("INSERT INTO settings_import_fields(source,fields) VALUES (?1,?2) ON CONFLICT(source) DO UPDATE SET fields=excluded.fields",params![key(format.source()),fields]).map_err(|_|SettingsError::Persistence)?;
                    } else {
                        tx.execute("DELETE FROM settings_import_fields WHERE source=?1", [key(format.source())]).map_err(|_| SettingsError::Persistence)?;
                    }
                    tx.commit().map_err(|_| SettingsError::Persistence)?;
                    let revision = token(format.source(), version);
                    owner.publish(format.source(), SettingsScope::User, &revision);
                    Ok(SettingsSnapshot { revision, document })
                })();
                Ok(result)
            })
            .await
            .map_err(|_| SettingsError::Persistence)?
    }
}
