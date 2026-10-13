use dim_agent_runtime::{AgentError, Capability, KeysOf, Result};
use rusqlite::{Connection, TransactionBehavior};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{
    fs::File,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

pub(super) const SCHEMA: &[(&str, &str)] = &[
    (
        "agent_store_format",
        "CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 12))",
    ),
    (
        "provider_imports",
        "CREATE TABLE provider_imports (kind TEXT NOT NULL, id TEXT NOT NULL, source_schema TEXT NOT NULL, source_version INTEGER NOT NULL, source_digest TEXT NOT NULL, PRIMARY KEY (kind, id))",
    ),
    (
        "providers",
        "CREATE TABLE providers (provider_id TEXT NOT NULL PRIMARY KEY, version INTEGER NOT NULL CHECK(version > 0), record TEXT NOT NULL)",
    ),
    (
        "provider_selections",
        "CREATE TABLE provider_selections (scope TEXT NOT NULL PRIMARY KEY, version INTEGER NOT NULL CHECK(version > 0), provider_id TEXT NOT NULL, model_id TEXT NOT NULL)",
    ),
    (
        "agent_sessions",
        "CREATE TABLE agent_sessions (session_id TEXT NOT NULL PRIMARY KEY, config TEXT NOT NULL, created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0))",
    ),
    (
        "agents",
        "CREATE TABLE agents(agent_id TEXT PRIMARY KEY, profile TEXT NOT NULL, is_default INTEGER NOT NULL CHECK(is_default IN (0, 1)))",
    ),
    (
        "agents_default_unique",
        "CREATE UNIQUE INDEX agents_default_unique ON agents(is_default) WHERE is_default = 1",
    ),
    (
        "session_owners",
        "CREATE TABLE session_owners(session_id TEXT PRIMARY KEY REFERENCES agent_sessions(session_id), agent_id TEXT NOT NULL REFERENCES agents(agent_id), protocol_version INTEGER NOT NULL CHECK(protocol_version IN (1, 2, 3)))",
    ),
    (
        "timeline_branches",
        "CREATE TABLE timeline_branches (session_id TEXT NOT NULL, branch_id TEXT NOT NULL, info TEXT NOT NULL, sequence INTEGER NOT NULL CHECK (sequence >= 0), PRIMARY KEY (session_id, branch_id), FOREIGN KEY (session_id) REFERENCES agent_sessions(session_id))",
    ),
    (
        "timeline_content",
        "CREATE TABLE timeline_content (session_id TEXT NOT NULL, hash TEXT NOT NULL, kind TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (session_id, hash), FOREIGN KEY (session_id) REFERENCES agent_sessions(session_id))",
    ),
    (
        "timeline_events",
        "CREATE TABLE timeline_events (session_id TEXT NOT NULL, branch_id TEXT NOT NULL, sequence INTEGER NOT NULL CHECK (sequence > 0), last_sequence INTEGER NOT NULL CHECK (last_sequence >= sequence), kind TEXT NOT NULL, body TEXT NOT NULL, event_id TEXT, PRIMARY KEY (session_id, branch_id, sequence), FOREIGN KEY (session_id, branch_id) REFERENCES timeline_branches(session_id, branch_id))",
    ),
    (
        "timeline_snapshots",
        "CREATE TABLE timeline_snapshots (session_id TEXT NOT NULL, branch_id TEXT NOT NULL, sequence INTEGER NOT NULL CHECK (sequence > 0), interpreter TEXT NOT NULL, event_id TEXT NOT NULL, state TEXT NOT NULL, PRIMARY KEY (session_id, branch_id, interpreter, sequence), FOREIGN KEY (session_id, branch_id, sequence) REFERENCES timeline_events(session_id, branch_id, sequence))",
    ),
    (
        "session_owners_by_agent",
        "CREATE INDEX session_owners_by_agent ON session_owners(agent_id, protocol_version, session_id)",
    ),
    (
        "timeline_events_by_end",
        "CREATE INDEX timeline_events_by_end ON timeline_events(session_id, branch_id, last_sequence, sequence)",
    ),
    (
        "timeline_events_by_id",
        "CREATE INDEX timeline_events_by_id ON timeline_events(session_id, event_id) WHERE event_id IS NOT NULL",
    ),
    (
        "timeline_tool_use_starts",
        "CREATE INDEX timeline_tool_use_starts ON timeline_events(session_id, json_extract(body, '$.record.event.data.request_id'), json_extract(body, '$.record.event.data.call_id')) WHERE branch_id = 'main' AND kind = 'step/tool-use-start'",
    ),
];

fn storage(error: impl std::fmt::Display) -> AgentError {
    AgentError::Storage(error.to_string())
}

/// The process connection and maintenance admission. Business modules never
/// open a second connection to this database.
pub struct SqliteStorage {
    pub(super) connection: Mutex<Option<Connection>>,
    admission: Mutex<Admission>,
    idle: tokio::sync::Notify,
    pub(super) path: PathBuf,
}
struct Admission {
    closed: bool,
    running: usize,
    lease: Option<File>,
}
struct Operation(Arc<SqliteStorage>);
impl Drop for Operation {
    fn drop(&mut self) {
        let mut admission = self.0.admission.lock().unwrap();
        admission.running -= 1;
        if admission.running == 0 {
            self.0.idle.notify_waiters();
        }
    }
}
impl SqliteStorage {
    pub fn open(path: PathBuf) -> Result<Arc<Self>> {
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        std::fs::create_dir_all(parent).map_err(storage)?;
        let path = if path.exists() {
            path.canonicalize().map_err(storage)?
        } else {
            parent.canonicalize().map_err(storage)?.join(
                path.file_name()
                    .ok_or_else(|| storage("database path has no file name"))?,
            )
        };
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let admission = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(storage)?;
        admission.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => storage(format!(
                "another process owns the SQLite database: {}",
                path.display()
            )),
            std::fs::TryLockError::Error(error) => storage(error),
        })?;
        let mut connection = Connection::open(&path).map_err(storage)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(storage)?;
        // Admission precedes any migration or writable PRAGMA. This native
        // database never opens or migrates the legacy TypeScript store.
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let actual: Vec<(String, String)> = {
            let mut query = tx.prepare("SELECT name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name").map_err(storage)?;
            query
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(storage)?
                .collect::<rusqlite::Result<_>>()
                .map_err(storage)?
        };
        if actual.is_empty() {
            for (_, sql) in SCHEMA {
                tx.execute_batch(sql).map_err(storage)?;
            }
            tx.execute("INSERT INTO agent_store_format VALUES (12)", [])
                .map_err(storage)?;
        } else {
            if actual.len() != SCHEMA.len()
                || actual.iter().any(|(name, sql)| {
                    !SCHEMA.iter().any(|(expected_name, expected_sql)| {
                        *expected_name == name && *expected_sql == sql
                    })
                })
            {
                return Err(storage(format!(
                    "unsupported SQLite schema at {}; the database was preserved. Use a compatible daemon to access it, or choose a new database path for a fresh start.",
                    path.display()
                )));
            }
            let versions = tx
                .prepare("SELECT version FROM agent_store_format")
                .map_err(storage)?
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(storage)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(storage)?;
            if versions != [12] {
                return Err(storage(format!(
                    "unsupported Timeline storage format version at {}; the database was preserved. Use a compatible daemon to access it, or choose a new database path for a fresh start.",
                    path.display()
                )));
            }
        }
        tx.commit().map_err(storage)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(storage)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(storage)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(storage)?;
        #[cfg(target_os = "macos")]
        connection
            .pragma_update(None, "fullfsync", true)
            .map_err(storage)?;
        Ok(Arc::new(Self {
            connection: Mutex::new(Some(connection)),
            admission: Mutex::new(Admission {
                closed: false,
                running: 0,
                lease: Some(admission),
            }),
            idle: tokio::sync::Notify::new(),
            path,
        }))
    }
    pub(crate) async fn run<T: Send + 'static>(
        self: &Arc<Self>,
        operation: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        {
            let mut admission = self.admission.lock().unwrap();
            if admission.closed {
                return Err(storage("SQLite storage is closed"));
            }
            admission.running += 1;
        }
        let admitted = Operation(self.clone());
        tokio::task::spawn_blocking(move || {
            let mut guard = admitted.0.connection.lock().unwrap();
            let connection = guard
                .as_mut()
                .ok_or_else(|| storage("SQLite storage is closed"))?;
            operation(connection)
        })
        .await
        .map_err(storage)?
    }
    pub async fn close(self: &Arc<Self>) -> Result<()> {
        self.admission.lock().unwrap().closed = true;
        let db = self.clone();
        // Closing must finish even when its caller stops waiting.
        tokio::spawn(async move {
            loop {
                let idle = db.idle.notified();
                tokio::pin!(idle);
                idle.as_mut().enable();
                if db.admission.lock().unwrap().running == 0 {
                    break;
                }
                idle.await;
            }
            tokio::task::spawn_blocking(move || {
                let mut guard = db.connection.lock().unwrap();
                let Some(connection) = guard.take() else {
                    return Ok(());
                };
                match connection.close() {
                    Ok(()) => {
                        drop(db.admission.lock().unwrap().lease.take());
                        Ok(())
                    }
                    Err((connection, failure)) => {
                        *guard = Some(connection);
                        Err(storage(failure))
                    }
                }
            })
            .await
            .map_err(storage)?
        })
        .await
        .map_err(storage)?
    }
}

pub struct SqliteStoragePlugin {
    path: PathBuf,
}
impl SqliteStoragePlugin {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}
impl TypedPlugin for SqliteStoragePlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "SqliteStorage"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let path = self.path.clone();
            let storage = tokio::task::spawn_blocking(move || SqliteStorage::open(path))
                .await
                .map_err(|error| CordisError::PluginFailed(error.into()))??;
            let owned = storage.clone();
            if let Err(error) = ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move { owned.close().await.map_err(Into::into) })
                }))
            }) {
                storage.close().await?;
                return Err(error);
            }
            ctx.provide_as(rutis::TypeKey::of::<SqliteStorage>(), storage)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for SqliteStoragePlugin {
    fn keys(&self) -> KeysOf<Self> {}
}

pub(super) fn error(error: impl std::fmt::Display) -> AgentError {
    storage(error)
}
