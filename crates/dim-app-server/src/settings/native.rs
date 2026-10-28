//! User-source SQLite authority. Import commits the document and provenance atomically.
use super::*;
use rusqlite::{Connection, OptionalExtension, params};

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
                let imported = snapshot(&*format, files.read(&path, &cancel).await)?;
                let body = serde_json::to_string(&imported.document)
                    .map_err(|_| SettingsError::Invalid("cannot encode settings"))?;
                db.run(move |connection| {
                    let result = (|| {
                        if stopping.is_cancelled() {
                            return Err(SettingsError::Closed);
                        }
                        if cancel.is_cancelled() {
                            return Err(SettingsError::Cancelled);
                        }
                        let tx = connection.transaction().map_err(|_| SettingsError::Persistence)?;
                        tx.execute(
                            "INSERT INTO settings_documents(source, version, document, imported_revision) VALUES (?1, 1, ?2, ?3) ON CONFLICT(source) DO NOTHING",
                            params![key(format.source()), body, imported.revision],
                        ).map_err(|_| SettingsError::Persistence)?;
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
    pub(super) async fn native_replace<F: Format>(
        self: &Arc<Self>,
        format: Arc<F>,
        path: FilePath,
        expected: String,
        document: F::Document,
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
                    let body = serde_json::to_string(&document)
                        .map_err(|_| SettingsError::Invalid("cannot encode settings"))?;
                    let version = version
                        .checked_add(1)
                        .ok_or(SettingsError::Invalid("settings revision overflow"))?;
                    tx.execute(
                        "UPDATE settings_documents SET version = ?1, document = ?2 WHERE source = ?3",
                        params![version, body, key(format.source())],
                    )
                    .map_err(|_| SettingsError::Persistence)?;
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
