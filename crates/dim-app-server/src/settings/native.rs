//! SQLite authority for user sources. A row's existence is the import marker;
//! source bytes and that authority switch commit together, without modifying the original.
use super::*;
use rusqlite::{Connection, OptionalExtension, params};

fn key(source: SettingsSource) -> &'static str {
    match source {
        SettingsSource::UserMcp => "user/mcp",
        SettingsSource::Skills => "user/skills",
        SettingsSource::ProjectMcp(_) => unreachable!("workspace files retain their authority"),
    }
}

fn encode(document: &SettingsDocument) -> Result<String> {
    match document {
        SettingsDocument::Mcp(document) => serde_json::to_string(document),
        SettingsDocument::Skills(document) => serde_json::to_string(document),
    }
    .map_err(|_| SettingsError::Invalid("cannot encode settings"))
}

fn revision(source: SettingsSource, version: i64) -> String {
    format!("native:{}:{version}", key(source))
}

fn read(
    connection: &Connection,
    source: SettingsSource,
) -> Result<Option<(i64, SettingsSnapshot)>> {
    let row: Option<(i64, String)> = connection
        .query_row(
            "SELECT version, document FROM settings_documents WHERE source = ?1",
            [key(source)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| SettingsError::Persistence)?;
    row.map(|(version, document)| {
        if version <= 0 {
            return Err(SettingsError::Invalid(
                "invalid persisted settings revision",
            ));
        }
        let mut snapshot = decode(source, Ok(document.into_bytes()))?;
        snapshot.revision = revision(source, version);
        Ok((version, snapshot))
    })
    .transpose()
}

impl SettingsService {
    pub(super) async fn native_read(
        &self,
        source: SettingsSource,
        cancel: &CancellationToken,
    ) -> Result<SettingsSnapshot> {
        let task = {
            let _guard = self.gate.lock().unwrap();
            self.check_open(cancel)?;
            let db = self
                .database
                .as_ref()
                .expect("native settings binding")
                .clone();
            let files = self.files.clone();
            let path = self.path(source)?;
            let cancel = cancel.clone();
            let stopping = self.stopping.clone();
            self.updates.spawn(async move {
                if let Some((_, snapshot)) = db
                    .run(move |connection| Ok(read(connection, source)))
                    .await
                    .map_err(|_| SettingsError::Persistence)??
                {
                    return Ok(snapshot);
                }
                // Import only on first successful read. Malformed/unreadable legacy
                // files leave no row, so repairing the file permits another attempt.
                let imported = decode(source, files.read(&path, &cancel).await)?;
                let document = encode(&imported.document)?;
                db.run(move |connection| {
                    Ok((|| {
                        if stopping.is_cancelled() { return Err(SettingsError::Closed); }
                        if cancel.is_cancelled() { return Err(SettingsError::Cancelled); }
                        let tx = connection.transaction().map_err(|_| SettingsError::Persistence)?;
                        tx.execute(
                            "INSERT INTO settings_documents(source, version, document, imported_revision) VALUES (?1, 1, ?2, ?3) ON CONFLICT(source) DO NOTHING",
                            params![key(source), document, imported.revision],
                        ).map_err(|_| SettingsError::Persistence)?;
                        let (_, snapshot) = read(&tx, source)?.expect("source inserted or already present");
                        tx.commit().map_err(|_| SettingsError::Persistence)?;
                        Ok(snapshot)
                    })())
                }).await.map_err(|_| SettingsError::Persistence)?
            })
        };
        task.await?
    }

    pub(super) async fn native_replace(
        &self,
        source: SettingsSource,
        expected: &str,
        change: SettingsChange,
        cancel: &CancellationToken,
    ) -> Result<SettingsSnapshot> {
        // Establish authority before mutation, never by falling back after a database error.
        self.native_read(source, cancel).await?;
        self.check_open(cancel)?;
        let expected = expected.to_owned();
        let cancel = cancel.clone();
        let stopping = self.stopping.clone();
        let changed = self.changed.clone();
        let skills_changed = self.skills_changed.clone();
        self.database.as_ref().expect("native settings binding").run(move |connection| {
            Ok((|| {
                if stopping.is_cancelled() { return Err(SettingsError::Closed); }
                if cancel.is_cancelled() { return Err(SettingsError::Cancelled); }
                let tx = connection.transaction().map_err(|_| SettingsError::Persistence)?;
                let (version, current) = read(&tx, source)?
                    .ok_or(SettingsError::Invalid("persisted settings source is missing"))?;
                if current.revision != expected { return Err(SettingsError::Conflict); }
                let Some(document) = change.apply(&current.document) else { return Ok(current) };
                validate(source, &document)?;
                let body = encode(&document)?;
                let version = version.checked_add(1).ok_or(SettingsError::Invalid("settings revision overflow"))?;
                tx.execute("UPDATE settings_documents SET version = ?1, document = ?2 WHERE source = ?3",
                    params![version, body, key(source)]).map_err(|_| SettingsError::Persistence)?;
                tx.commit().map_err(|_| SettingsError::Persistence)?;
                let revision = revision(source, version);
                // Publish under SQLite's connection gate, in durable commit order.
                // A dropped RPC waiter cannot suppress a committed change notification.
                if source == SettingsSource::Skills { skills_changed.set(revision.clone()); }
                changed.update(|value| *value += 1);
                Ok(SettingsSnapshot { revision, document })
            })())
        }).await.map_err(|_| SettingsError::Persistence)?
    }
}
