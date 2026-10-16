use super::{
    SqliteStorage,
    codec::{self, ContentCache, WireEvent},
    files::{self, SessionFiles},
    sqlite::error,
};
use dim_agent_capabilities::host::SessionFilesHost;
use dim_agent_runtime::{
    AgentError, BoxFuture, Capability, KeysOf, PublishCallback, Result, SessionConfig,
    StoredBranch, StoredEvent, TimelineInfo, TimelineKey, TimelineStore, TimelineTrace, scoped,
};
use dim_agent_types::{BranchCreateAt, ContextCompactionResult, EventKind, RecoverySnapshot};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use rutis::{CordisError, Ctx, Effect, TypeKey, TypedPlugin};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::RangeInclusive,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

struct Pending {
    key: TimelineKey,
    events: Vec<StoredEvent>,
    publish: PublishCallback,
}
enum Command {
    Append(Pending),
    Rollback {
        session_id: String,
        done: tokio::sync::oneshot::Sender<Result<()>>,
    },
    Close,
}

/// One process writer batches every Session; only pending events reside here.
pub struct SqliteTimelineStore {
    db: Arc<SqliteStorage>,
    /// Schedules a Session-file sweep after a commit released owner rows.
    sweep_files: Arc<tokio::sync::Notify>,
    admission: Mutex<Admission>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    closing: Mutex<Option<tokio::sync::watch::Receiver<Option<Result<()>>>>>,
}
struct Admission {
    sender: mpsc::Sender<Command>,
    failures: BTreeMap<String, AgentError>,
    closed: bool,
}
impl SqliteTimelineStore {
    pub fn new(db: Arc<SqliteStorage>) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel();
        let store = Arc::new(Self {
            db,
            sweep_files: Arc::new(tokio::sync::Notify::new()),
            admission: Mutex::new(Admission {
                sender,
                failures: BTreeMap::new(),
                closed: false,
            }),
            worker: Mutex::new(None),
            closing: Mutex::new(None),
        });
        let weak = Arc::downgrade(&store);
        let writer = thread::spawn(move || {
            let mut last_round: Option<Instant> = None;
            let mut control = None;
            loop {
                let command = match control.take().or_else(|| receiver.recv().ok()) {
                    Some(command) => command,
                    None => break,
                };
                let first = match command {
                    Command::Append(first) => first,
                    Command::Rollback { session_id, done } => {
                        let Some(store) = weak.upgrade() else { break };
                        let _ = done.send(store.rollback_creation(&session_id));
                        continue;
                    }
                    Command::Close => break,
                };
                if let Some(started) = last_round {
                    thread::sleep(Duration::from_millis(20).saturating_sub(started.elapsed()));
                }
                let mut pending = vec![first];
                while let Ok(command) = receiver.try_recv() {
                    match command {
                        Command::Append(batch) => pending.push(batch),
                        command => {
                            control = Some(command);
                            break;
                        }
                    }
                }
                last_round = Some(Instant::now());
                let Some(store) = weak.upgrade() else { break };
                store.commit_round(pending);
            }
        });
        *store.worker.lock().unwrap() = Some(writer);
        store
    }
    fn commit_round(&self, pending: Vec<Pending>) {
        let mut sessions: BTreeMap<String, Vec<Pending>> = BTreeMap::new();
        for batch in pending {
            let failure = self
                .admission
                .lock()
                .unwrap()
                .failures
                .get(&batch.key.session_id)
                .cloned();
            if let Some(failure) = failure {
                (batch.publish)(Err(failure));
            } else {
                sessions
                    .entry(batch.key.session_id.clone())
                    .or_default()
                    .push(batch);
            }
        }
        if sessions.is_empty() {
            return;
        }
        let mut guard = self.db.connection.lock().unwrap();
        let result = guard
            .as_mut()
            .ok_or_else(|| error("SQLite storage is closed"))
            .and_then(|connection| {
                transaction(connection, &self.db.path, sessions.values().flatten())
            });
        if result.is_ok() {
            drop(guard);
            for batch in sessions.into_values().flatten() {
                (batch.publish)(Ok(()));
            }
            return;
        }
        for (session, batches) in sessions {
            let existing = self
                .admission
                .lock()
                .unwrap()
                .failures
                .get(&session)
                .cloned();
            let result = match existing {
                Some(failure) => Err(failure),
                None => guard
                    .as_mut()
                    .ok_or_else(|| error("SQLite storage is closed"))
                    .and_then(|connection| transaction(connection, &self.db.path, batches.iter())),
            };
            if let Err(failure) = &result {
                self.admission
                    .lock()
                    .unwrap()
                    .failures
                    .insert(session, failure.clone());
            }
            // Callbacks may read or append; never run them under the database lock.
            drop(guard);
            for batch in batches {
                (batch.publish)(result.clone());
            }
            guard = self.db.connection.lock().unwrap();
        }
    }
    fn rollback_creation(&self, session_id: &str) -> Result<()> {
        let mut guard = self.db.connection.lock().unwrap();
        let connection = guard
            .as_mut()
            .ok_or_else(|| error("SQLite storage is closed"))?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(error)?;
        // Owner rows go before the Session record they reference.
        let released = files::release(&tx, session_id).map_err(error)?;
        for table in [
            "timeline_snapshots",
            "timeline_events",
            "timeline_branches",
            "timeline_content",
            "session_owners",
            "agent_sessions",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE session_id=?1"),
                [session_id],
            )
            .map_err(error)?;
        }
        tx.commit().map_err(error)?;
        if released > 0 {
            self.sweep_files.notify_one();
        }
        self.admission.lock().unwrap().failures.remove(session_id);
        Ok(())
    }
    pub async fn close(self: &Arc<Self>) -> Result<()> {
        let mut receiver = {
            let mut closing = self.closing.lock().unwrap();
            closing
                .get_or_insert_with(|| {
                    let (sender, receiver) = tokio::sync::watch::channel(None);
                    let mut admission = self.admission.lock().unwrap();
                    admission.closed = true;
                    let requested = admission.sender.send(Command::Close).map_err(error);
                    let worker = self.worker.lock().unwrap().take();
                    let owned = self.clone();
                    tokio::spawn(async move {
                        let result = match worker {
                            Some(worker) => tokio::task::spawn_blocking(move || {
                                worker.join().map_err(|_| error("Timeline writer panicked"))
                            })
                            .await
                            .map_err(error)
                            .and_then(|result| result)
                            .and(requested),
                            None => requested,
                        };
                        sender.send_replace(Some(result));
                        drop(owned);
                    });
                    receiver
                })
                .clone()
        };
        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            receiver.changed().await.map_err(error)?;
        }
    }
}

/// Session directory service; Agent hosting owns the profile and owner meaning.
#[derive(Clone)]
pub struct SessionStore {
    timeline: Arc<SqliteTimelineStore>,
}
impl SessionStore {
    pub fn new(db: Arc<SqliteStorage>) -> Self {
        Self {
            timeline: SqliteTimelineStore::new(db),
        }
    }
    pub fn timeline_store(&self) -> Arc<dyn TimelineStore> {
        self.timeline.clone()
    }
    pub async fn sessions(&self) -> Result<Vec<String>> {
        self.timeline
            .db
            .run(|connection| {
                connection
                    .prepare("SELECT session_id FROM agent_sessions ORDER BY session_id")
                    .map_err(error)?
                    .query_map([], |row| row.get(0))
                    .map_err(error)?
                    .collect::<rusqlite::Result<_>>()
                    .map_err(error)
            })
            .await
    }
    /// Directory-only reads never validate or decode Timeline events.
    pub async fn directory(
        &self,
        agent: Option<String>,
        version: u32,
    ) -> Result<Vec<crate::protocol::SessionInfo>> {
        self.timeline
            .db
            .run(move |connection| {
                let filter = if agent.is_some() {
                    " AND o.agent_id=?2"
                } else {
                    ""
                };
                let sql = format!(
                    "SELECT s.session_id, s.config, s.created_at_ms, b.sequence, o.agent_id \
                 FROM session_owners o JOIN agent_sessions s USING(session_id) \
                 JOIN timeline_branches b ON b.session_id=s.session_id AND b.branch_id='main' \
                 WHERE o.protocol_version=?1{filter} ORDER BY s.session_id"
                );
                let mut query = connection.prepare(&sql).map_err(error)?;
                let mut parameters = vec![rusqlite::types::Value::Integer(i64::from(version))];
                if let Some(agent) = agent {
                    parameters.push(agent.into());
                }
                let mut rows = query
                    .query(rusqlite::params_from_iter(parameters))
                    .map_err(error)?;
                let mut result = vec![];
                while let Some(row) = rows.next().map_err(error)? {
                    let id: String = row.get(0).map_err(error)?;
                    let parsed = (|| -> Result<_> {
                        let config: String = row.get(1).map_err(error)?;
                        let config: SessionConfig = serde_json::from_str(&config).map_err(error)?;
                        Ok(crate::protocol::SessionInfo {
                            session_id: id.clone(),
                            cwd: config.cwd,
                            model: dim_agent_types::ModelId {
                                provider_id: config.model.provider_id,
                                model_id: config.model.model_id,
                            },
                            created_at: read_sequence(row, 2).map_err(error)?,
                            sequence: read_sequence(row, 3).map_err(error)?,
                            running: false,
                            attached_clients: 0,
                            agent_id: Some(row.get(4).map_err(error)?),
                        })
                    })();
                    match parsed {
                        Ok(record) => result.push(record),
                        Err(error) => {
                            tracing::error!(session_id=%id, %error, "Invalid Session directory")
                        }
                    }
                }
                Ok(result)
            })
            .await
    }
    pub async fn read_page(
        &self,
        id: &str,
        after: u64,
        limit: usize,
        through: u64,
    ) -> Result<Vec<Arc<dim_agent_types::AgentEvent>>> {
        let key = TimelineKey {
            session_id: id.into(),
            branch_id: "main".into(),
        };
        Ok(self
            .timeline
            .read_page(&key, after, limit, through)
            .await?
            .into_iter()
            .map(|record| record.event)
            .collect())
    }
    pub async fn read_event(
        &self,
        id: &str,
        event_id: &str,
    ) -> Result<Option<Arc<dim_agent_types::AgentEvent>>> {
        let id = id.to_owned();
        let event_id = event_id.to_owned();
        self.timeline
            .db
            .run(move |connection| {
                let key: Option<(String, u64)> = connection
                    .query_row(
                        "SELECT branch_id, sequence FROM timeline_events WHERE session_id=?1 AND event_id=?2",
                        params![id, event_id],
                        |row| Ok((row.get(0)?, read_sequence(row, 1)?)),
                    )
                    .optional()
                    .map_err(error)?;
                let Some((branch_id, sequence)) = key else {
                    return Ok(None);
                };
                let key = TimelineKey { session_id: id, branch_id };
                let record = read_events(connection, &key, sequence..=sequence, false)?
                    .pop()
                    .ok_or_else(|| error("indexed event is missing"))?;
                if record.event.id != event_id {
                    return Err(error("event identity index does not match row"));
                }
                Ok(Some(record.event))
            })
            .await
    }
    /// The main-branch `step/tool-use-start` of call `call_id` in request
    /// `request_id`, found through `timeline_tool_use_starts`.
    pub async fn read_tool_use_start(
        &self,
        id: &str,
        request_id: &str,
        call_id: &str,
    ) -> Result<Option<Arc<dim_agent_types::AgentEvent>>> {
        let id = id.to_owned();
        let request_id = request_id.to_owned();
        let call_id = call_id.to_owned();
        self.timeline
            .db
            .run(move |connection| {
                let sequence: Option<u64> = connection
                    .query_row(
                        "SELECT sequence FROM timeline_events WHERE session_id=?1 AND branch_id='main' AND kind='step/tool-use-start' AND json_extract(body, '$.record.event.data.request_id')=?2 AND json_extract(body, '$.record.event.data.call_id')=?3",
                        params![id, request_id, call_id],
                        |row| read_sequence(row, 0),
                    )
                    .optional()
                    .map_err(error)?;
                let Some(sequence) = sequence else {
                    return Ok(None);
                };
                let key = TimelineKey {
                    session_id: id,
                    branch_id: "main".into(),
                };
                let record = read_events(connection, &key, sequence..=sequence, false)?
                    .pop()
                    .ok_or_else(|| error("indexed event is missing"))?;
                Ok(Some(record.event))
            })
            .await
    }
    pub async fn has_records(&self, id: &str) -> Result<bool> {
        let id = id.to_owned();
        self.timeline
            .db
            .run(move |connection| {
                connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE session_id=?1) \
                 OR EXISTS(SELECT 1 FROM session_owners WHERE session_id=?1) \
                 OR EXISTS(SELECT 1 FROM timeline_branches WHERE session_id=?1) \
                 OR EXISTS(SELECT 1 FROM timeline_events WHERE session_id=?1) \
                 OR EXISTS(SELECT 1 FROM timeline_content WHERE session_id=?1)",
                        [id],
                        |row| row.get(0),
                    )
                    .map_err(error)
            })
            .await
    }
    pub async fn create_owned_session(
        &self,
        session_id: &str,
        config: &SessionConfig,
        agent_id: &str,
        protocol_version: u32,
    ) -> Result<()> {
        let session = session_id.to_owned();
        let config = config.clone();
        let agent = agent_id.to_owned();
        self.timeline
            .db
            .run(move |connection| {
                let tx = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(error)?;
                create_session(&tx, &session, &config)?;
                tx.execute(
                    "INSERT INTO session_owners VALUES (?1, ?2, ?3)",
                    params![session, agent, protocol_version],
                )
                .map_err(error)?;
                tx.commit().map_err(error)
            })
            .await
    }
}

/// The Session directory and the `sessionFiles` service. Session files live
/// below the FileStorage root `files_root`, which is the database directory.
pub struct SessionStorePlugin {
    files_root: usize,
}
impl SessionStorePlugin {
    pub fn new(files_root: usize) -> Self {
        Self { files_root }
    }
}
impl TypedPlugin for SessionStorePlugin {
    type Deps = (Arc<SqliteStorage>,);
    fn name(&self) -> &str {
        "SessionStore"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (db,): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let store = Arc::new(SessionStore::new(db.clone()));
            let files = SessionFiles::new(
                db,
                ctx.clone(),
                self.files_root,
                store.timeline.sweep_files.clone(),
            );
            let (timeline, owned_files) = (store.timeline.clone(), files.clone());
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        // Puts settle before the native store stops writing.
                        owned_files.close().await;
                        timeline.close().await.map_err(Into::into)
                    })
                }))
            })?;
            files.start();
            ctx.provide_as(TypeKey::of::<SessionStore>(), store)?;
            ctx.provide_as(
                TypeKey::of::<SessionFilesHost>(),
                Arc::new(SessionFilesHost(Arc::new(files))),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for SessionStorePlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}

fn create_session(connection: &Connection, session: &str, config: &SessionConfig) -> Result<()> {
    let config = serde_json::to_string(config).map_err(error)?;
    connection
        .execute(
            "INSERT INTO agent_sessions VALUES (?1, ?2, ?3)",
            params![
                session,
                config,
                sql_sequence(dim_agent_runtime::core::unix_millis())?
            ],
        )
        .map_err(error)?;
    let info = TimelineInfo {
        id: "main".into(),
        parent: None,
        branch_point: None,
        create_anchor: None,
    };
    insert_branch(connection, session, &info)
}
fn insert_branch(connection: &Connection, session: &str, info: &TimelineInfo) -> Result<()> {
    connection
        .execute(
            "INSERT INTO timeline_branches VALUES (?1, ?2, ?3, 0)",
            params![
                session,
                info.id,
                serde_json::to_string(info).map_err(error)?
            ],
        )
        .map_err(error)?;
    Ok(())
}
fn transaction<'a>(
    connection: &mut Connection,
    path: &std::path::Path,
    mut batches: impl Iterator<Item = &'a Pending>,
) -> Result<()> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|failure| {
        tracing::error!(db_path = %path.display(), adapter = "rust", phase = "BEGIN", pid = std::process::id(), sqlite_code = ?failure.sqlite_error_code(), %failure, "storage.transaction");
        error(failure)
    })?;
    match batches.try_for_each(|batch| write_batch(&tx, batch)) {
        Ok(()) => tx.commit().map_err(|failure| {
            tracing::error!(db_path = %path.display(), adapter = "rust", phase = "COMMIT", pid = std::process::id(), sqlite_code = ?failure.sqlite_error_code(), %failure, "storage.transaction");
            error(failure)
        }),
        Err(failure) => {
            tracing::error!(db_path = %path.display(), adapter = "rust", phase = "callback", pid = std::process::id(), %failure, "storage.transaction");
            if let Err(rollback) = tx.rollback() {
                tracing::error!(db_path = %path.display(), adapter = "rust", phase = "ROLLBACK", pid = std::process::id(), sqlite_code = ?rollback.sqlite_error_code(), %rollback, "storage.transaction");
                return Err(failure.with_cleanup(error(rollback)));
            }
            Err(failure)
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "encoding", rename_all = "lowercase", deny_unknown_fields)]
enum Row {
    Event {
        record: Box<WireEvent>,
    },
    Output {
        turn_id: String,
        request_id: String,
        started_at: u64,
        records: Vec<WireEvent>,
    },
}
fn head(connection: &Connection, key: &TimelineKey) -> Result<u64> {
    connection
        .query_row(
            "SELECT sequence FROM timeline_branches WHERE session_id = ?1 AND branch_id = ?2",
            params![key.session_id, key.branch_id],
            |row| read_sequence(row, 0),
        )
        .map_err(error)
}
fn read_sequence(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
fn sql_sequence(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(error)
}
fn write_batch(connection: &Connection, batch: &Pending) -> Result<()> {
    let mut sequence = head(connection, &batch.key)?;
    for record in &batch.events {
        if record.event.sequence != sequence + 1 {
            return Err(error("Timeline sequence is not contiguous"));
        }
        sequence += 1;
        let mut wire = codec::encode(connection, &batch.key.session_id, record)?;
        let kind = record.event.kind.key();
        match &record.event.kind {
            EventKind::Text(data) | EventKind::Thinking(data) => {
                let previous: Option<(u64, String, String)> = connection.query_row("SELECT sequence, kind, body FROM timeline_events WHERE session_id = ?1 AND branch_id = ?2 ORDER BY sequence DESC LIMIT 1", params![batch.key.session_id, batch.key.branch_id], |row| Ok((read_sequence(row, 0)?, row.get(1)?, row.get(2)?))).optional().map_err(error)?;
                let output_data = wire.event["data"]
                    .as_object_mut()
                    .ok_or_else(|| error("output data is missing in event encoding"))?;
                output_data.remove("turn_id");
                output_data.remove("request_id");
                if let Some((start, previous_kind, body)) = previous
                    && previous_kind == kind
                    && body.len() < 4096
                    && let Row::Output {
                        turn_id,
                        request_id,
                        started_at,
                        mut records,
                    } = serde_json::from_str(&body).map_err(error)?
                    && turn_id == data.turn_id
                    && request_id == data.request_id
                    && record.event.create_at.saturating_sub(started_at) < 1000
                {
                    records.push(wire);
                    let body = serde_json::to_string(&Row::Output {
                        turn_id,
                        request_id,
                        started_at,
                        records,
                    })
                    .map_err(error)?;
                    connection.execute("UPDATE timeline_events SET last_sequence = ?3, body = ?4 WHERE session_id = ?1 AND branch_id = ?2 AND sequence = ?5", params![batch.key.session_id, batch.key.branch_id, sql_sequence(sequence)?, body, sql_sequence(start)?]).map_err(error)?;
                    continue;
                }
                let row = Row::Output {
                    turn_id: data.turn_id.clone(),
                    request_id: data.request_id.clone(),
                    started_at: record.event.create_at,
                    records: vec![wire],
                };
                insert_event(connection, &batch.key, sequence, kind, &row)?;
            }
            _ => {
                insert_event(
                    connection,
                    &batch.key,
                    sequence,
                    kind,
                    &Row::Event {
                        record: Box::new(wire),
                    },
                )?;
            }
        }
        if let EventKind::BranchCreate(data) = &record.event.kind {
            let info = TimelineInfo {
                id: data.branch.clone(),
                parent: Some(batch.key.branch_id.clone()),
                branch_point: Some(match data.at {
                    BranchCreateAt::Sequence { sequence } => sequence,
                    BranchCreateAt::Create {} => sequence,
                }),
                create_anchor: Some(sequence),
            };
            insert_branch(connection, &batch.key.session_id, &info)?;
        }
    }
    connection
        .execute(
            "UPDATE timeline_branches SET sequence = ?3 WHERE session_id = ?1 AND branch_id = ?2",
            params![
                batch.key.session_id,
                batch.key.branch_id,
                sql_sequence(sequence)?
            ],
        )
        .map_err(error)?;
    Ok(())
}
fn insert_event(
    connection: &Connection,
    key: &TimelineKey,
    sequence: u64,
    kind: &str,
    row: &Row,
) -> Result<()> {
    connection
        .execute(
            "INSERT INTO timeline_events VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6)",
            params![
                key.session_id,
                key.branch_id,
                sql_sequence(sequence)?,
                kind,
                serde_json::to_string(row).map_err(error)?,
                match row {
                    Row::Event { record } => record.event["id"].as_str(),
                    Row::Output { .. } => None,
                }
            ],
        )
        .map_err(error)?;
    Ok(())
}
fn decode_row(
    connection: &Connection,
    session: &str,
    kind: &str,
    body: &str,
    cache: &mut ContentCache,
) -> Result<Vec<StoredEvent>> {
    let row: Row = serde_json::from_str(body).map_err(error)?;
    let wires = match row {
        Row::Event { record } => vec![*record],
        Row::Output {
            turn_id,
            request_id,
            records,
            ..
        } => {
            if kind != "step/text" && kind != "step/thinking" {
                return Err(error("output row has invalid event kind"));
            }
            records
                .into_iter()
                .map(|mut wire| {
                    let data = wire.event["data"]
                        .as_object_mut()
                        .ok_or_else(|| error("output data is missing"))?;
                    data.insert("turn_id".into(), turn_id.clone().into());
                    data.insert("request_id".into(), request_id.clone().into());
                    Ok(wire)
                })
                .collect::<Result<Vec<_>>>()?
        }
    };
    let records = wires
        .into_iter()
        .map(|wire| cache.decode(connection, session, wire))
        .collect::<Result<Vec<_>>>()?;
    if records.is_empty() || records.iter().any(|record| record.event.kind.key() != kind) {
        return Err(error("stored event kind does not match row"));
    }
    Ok(records)
}
fn read_events(
    connection: &Connection,
    key: &TimelineKey,
    range: RangeInclusive<u64>,
    boundary: bool,
) -> Result<Vec<StoredEvent>> {
    read_events_with_cache(
        connection,
        key,
        range,
        boundary,
        &mut ContentCache::default(),
    )
}
fn read_events_with_cache(
    connection: &Connection,
    key: &TimelineKey,
    range: RangeInclusive<u64>,
    boundary: bool,
    cache: &mut ContentCache,
) -> Result<Vec<StoredEvent>> {
    if range.is_empty() {
        return Ok(vec![]);
    }
    let mut query = connection.prepare("SELECT sequence, last_sequence, kind, body FROM timeline_events WHERE session_id = ?1 AND branch_id = ?2 AND sequence <= ?4 AND last_sequence >= ?3 ORDER BY last_sequence").map_err(error)?;
    let rows = query
        .query_map(
            params![
                key.session_id,
                key.branch_id,
                sql_sequence(*range.start())?,
                sql_sequence(*range.end())?
            ],
            |row| {
                Ok((
                    read_sequence(row, 0)?,
                    read_sequence(row, 1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(error)?;
    let mut records = vec![];
    for row in rows {
        let (first, last, kind, body) = row.map_err(error)?;
        let decoded = decode_row(connection, &key.session_id, &kind, &body, cache)?;
        if decoded.first().unwrap().event.sequence != first
            || decoded.last().unwrap().event.sequence != last
            || decoded
                .windows(2)
                .any(|pair| pair[0].event.sequence + 1 != pair[1].event.sequence)
        {
            return Err(error("stored output sequence range is invalid"));
        }
        records.extend(
            decoded
                .into_iter()
                .filter(|record| range.contains(&record.event.sequence)),
        );
    }
    if boundary
        && records
            .last()
            .is_some_and(|record| record.batch_end > *range.end())
    {
        return Err(AgentError::InvalidInput(format!(
            "sequence {} is inside an atomic batch",
            range.end()
        )));
    }
    if boundary && *range.start() > 0 && records.len() as u64 != range.end() - range.start() + 1 {
        return Err(error("Timeline read contains a sequence gap"));
    }
    Ok(records)
}

impl TimelineStore for SqliteTimelineStore {
    fn create_session<'a>(
        &'a self,
        session_id: &'a str,
        config: &'a SessionConfig,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let session = session_id.to_owned();
            let config = config.clone();
            self.db
                .run(move |connection| {
                    let tx = connection
                        .transaction_with_behavior(TransactionBehavior::Immediate)
                        .map_err(error)?;
                    create_session(&tx, &session, &config)?;
                    tx.commit().map_err(error)
                })
                .await
        })
    }
    fn rollback_session<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let (done, wait) = tokio::sync::oneshot::channel();
            {
                let admission = self.admission.lock().unwrap();
                if admission.closed {
                    return Err(error("Timeline storage is closed"));
                }
                admission
                    .sender
                    .send(Command::Rollback {
                        session_id: session_id.into(),
                        done,
                    })
                    .map_err(error)?;
            }
            wait.await.map_err(error)?
        })
    }
    fn session_config<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<SessionConfig>>> {
        Box::pin(async move {
            let session = session_id.to_owned();
            self.db
                .run(move |connection| {
                    let config: Option<String> = connection
                        .query_row(
                            "SELECT config FROM agent_sessions WHERE session_id = ?1",
                            [&session],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(error)?;
                    config
                        .map(|config| serde_json::from_str(&config).map_err(error))
                        .transpose()
                })
                .await
        })
    }
    fn branches<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, Result<Vec<StoredBranch>>> {
        Box::pin(async move {
            let session = session_id.to_owned();
            validate_session(&self.db, &session).await
        })
    }
    fn head<'a>(&'a self, key: &'a TimelineKey) -> BoxFuture<'a, Result<u64>> {
        Box::pin(async move {
            let key = key.clone();
            self.db.run(move |connection| head(connection, &key)).await
        })
    }
    fn append(
        &self,
        key: TimelineKey,
        events: Vec<StoredEvent>,
        publish: PublishCallback,
    ) -> Result<()> {
        let admission = self.admission.lock().unwrap();
        if admission.closed {
            return Err(error("Timeline storage is closed"));
        }
        if let Some(failure) = admission.failures.get(&key.session_id) {
            return Err(failure.clone());
        }
        admission
            .sender
            .send(Command::Append(Pending {
                key,
                events,
                publish,
            }))
            .map_err(error)
    }
    fn read_page<'a>(
        &'a self,
        key: &'a TimelineKey,
        after: u64,
        limit: usize,
        through: u64,
    ) -> BoxFuture<'a, Result<Vec<StoredEvent>>> {
        Box::pin(async move {
            if after > through {
                return Err(AgentError::InvalidInput("future Timeline cursor".into()));
            }
            // Validate both caller-supplied boundaries, including empty pages.
            self.read(key, through..=through).await?;
            if after > 0 && after != through {
                self.read(key, after..=after).await?;
            }
            let mut end = through.min(after.saturating_add(limit as u64));
            if end <= after {
                return Ok(vec![]);
            }
            let mut next = after + 1;
            let mut events = vec![];
            while next <= end {
                let last = end.min(next.saturating_add(255));
                let key = key.clone();
                let page = self
                    .db
                    .run(move |connection| read_events(connection, &key, next..=last, false))
                    .await?;
                if page.len() as u64 != last - next + 1 {
                    return Err(error("Timeline read contains a sequence gap"));
                }
                if last == end {
                    end = page.last().unwrap().batch_end;
                    if end < last || end > through {
                        return Err(error("invalid Timeline batch boundary"));
                    }
                }
                events.extend(page);
                next = last + 1;
            }
            Ok(events)
        })
    }
    fn read_segments<'a>(
        &'a self,
        session_id: &'a str,
        segments: Vec<(String, RangeInclusive<u64>)>,
    ) -> BoxFuture<'a, Result<Vec<Vec<StoredEvent>>>> {
        Box::pin(async move {
            let session = session_id.to_owned();
            let mut result = Vec::with_capacity(segments.len());
            let mut cache = ContentCache::default();
            for (branch_id, range) in segments {
                let key = TimelineKey {
                    session_id: session.clone(),
                    branch_id,
                };
                let through = self.head(&key).await?;
                if *range.end() > through {
                    return Err(AgentError::InvalidInput("future Timeline sequence".into()));
                }
                let mut records = vec![];
                let mut next = *range.start();
                while next <= *range.end() {
                    let end = next.saturating_add(255).min(*range.end());
                    let key = key.clone();
                    let (page, returned_cache) = self
                        .db
                        .run(move |connection| {
                            let page = read_events_with_cache(
                                connection,
                                &key,
                                next..=end,
                                false,
                                &mut cache,
                            )?;
                            Ok((page, cache))
                        })
                        .await?;
                    cache = returned_cache;
                    let expected = if end >= next.max(1) {
                        end - next.max(1) + 1
                    } else {
                        0
                    };
                    if page.len() as u64 != expected {
                        return Err(error("Timeline read contains a sequence gap"));
                    }
                    records.extend(page);
                    next = end + 1;
                }
                if records
                    .last()
                    .is_some_and(|record| record.batch_end > *range.end())
                {
                    return Err(AgentError::InvalidInput(format!(
                        "sequence {} is inside an atomic batch",
                        range.end()
                    )));
                }
                result.push(records);
            }
            Ok(result)
        })
    }
    fn trace<'a>(
        &'a self,
        key: &'a TimelineKey,
        sequence: u64,
    ) -> BoxFuture<'a, Result<Option<TimelineTrace>>> {
        Box::pin(async move {
            let key = key.clone();
            self.db
                .run(move |connection| {
                    Ok(read_events(connection, &key, sequence..=sequence, false)?
                        .pop()
                        .and_then(|record| record.trace))
                })
                .await
        })
    }
    fn save_snapshot<'a>(
        &'a self,
        key: &'a TimelineKey,
        snapshot: RecoverySnapshot,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let key = key.clone();
            self.db.run(move |connection| {
                let anchor = read_events(connection, &key, snapshot.sequence..=snapshot.sequence, false)?
                    .pop().ok_or_else(|| error("snapshot anchor is missing"))?;
                if anchor.event.id != snapshot.event_id
                    || !matches!(&anchor.event.kind, EventKind::ContextCompaction(data)
                        if matches!(data.outcome, ContextCompactionResult::Completed { .. }))
                {
                    return Err(error("snapshot anchor is not the identified completed compaction"));
                }
                connection.execute(
                    "INSERT OR REPLACE INTO timeline_snapshots VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![key.session_id, key.branch_id, sql_sequence(snapshot.sequence)?, snapshot.interpreter, snapshot.event_id, snapshot.state],
                ).map_err(error)?;
                Ok(())
            }).await
        })
    }
    fn latest_snapshot<'a>(
        &'a self,
        key: &'a TimelineKey,
        through: u64,
        interpreter: &'a str,
    ) -> BoxFuture<'a, Result<Option<RecoverySnapshot>>> {
        Box::pin(async move {
            let key = key.clone();
            let interpreter = interpreter.to_owned();
            self.db.run(move |connection| {
                connection.query_row(
                    "SELECT s.sequence, s.event_id, s.interpreter, s.state FROM timeline_snapshots s JOIN timeline_events e ON e.session_id=s.session_id AND e.branch_id=s.branch_id AND e.sequence=s.sequence AND e.event_id=s.event_id WHERE s.session_id=?1 AND s.branch_id=?2 AND s.sequence<=?3 AND s.interpreter=?4 AND typeof(s.sequence)='integer' AND s.sequence>0 AND typeof(s.event_id)='text' AND typeof(s.state)='text' AND e.kind='context/compaction' AND json_valid(e.body) AND json_extract(e.body, '$.record.event.data.status')='completed' AND json_extract(e.body, '$.record.event.id')=s.event_id AND json_extract(e.body, '$.record.event.sequence')=s.sequence ORDER BY s.sequence DESC LIMIT 1",
                    params![key.session_id, key.branch_id, sql_sequence(through)?, interpreter],
                    |row| Ok(RecoverySnapshot { sequence: read_sequence(row, 0)?, event_id: row.get(1)?, interpreter: row.get(2)?, state: row.get(3)? }),
                ).optional().map_err(error)
            }).await
        })
    }
}

async fn validate_session(db: &Arc<SqliteStorage>, session: &str) -> Result<Vec<StoredBranch>> {
    let id = session.to_owned();
    let directory = db.run(move |connection| {
        let config: String = connection.query_row("SELECT config FROM agent_sessions WHERE session_id=?1", [&id], |row| row.get(0)).map_err(error)?;
        let _: SessionConfig = serde_json::from_str(&config).map_err(error)?;
        let orphan: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM timeline_events e LEFT JOIN timeline_branches b \
             ON b.session_id=e.session_id AND b.branch_id=e.branch_id \
             WHERE e.session_id=?1 AND b.branch_id IS NULL)", [&id], |row| row.get(0)).map_err(error)?;
        if orphan { return Err(error("Timeline event references a missing branch")); }
        connection.prepare("SELECT branch_id, info, sequence FROM timeline_branches WHERE session_id=?1 ORDER BY branch_id")
            .map_err(error)?.query_map([id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, read_sequence(row, 2)?)))
            .map_err(error)?.collect::<rusqlite::Result<Vec<_>>>().map_err(error)
    }).await?;
    let mut branches = BTreeMap::new();
    let mut event_ids = BTreeSet::new();
    let mut created = BTreeSet::new();
    let mut completions = vec![];
    for (id, info, sequence) in directory {
        let info: TimelineInfo = serde_json::from_str(&info).map_err(error)?;
        if info.id != id {
            return Err(error("branch directory identity mismatch"));
        }
        let mut next = 1;
        let mut batch_end = 0;
        loop {
            let key = TimelineKey {
                session_id: session.into(),
                branch_id: id.clone(),
            };
            let page = db.run(move |connection| {
                let mut query = connection.prepare("SELECT sequence, last_sequence, kind, body, event_id FROM timeline_events WHERE session_id=?1 AND branch_id=?2 AND sequence>=?3 ORDER BY sequence LIMIT 64").map_err(error)?;
                let rows = query.query_map(params![key.session_id, key.branch_id, sql_sequence(next)?], |row| Ok((
                    read_sequence(row,0)?, read_sequence(row,1)?, row.get::<_,String>(2)?, row.get::<_,String>(3)?, row.get::<_,Option<String>>(4)?
                ))).map_err(error)?;
                let mut cache = ContentCache::default();
                rows.map(|row| {
                    let (first,last,kind,body,event_id) = row.map_err(error)?;
                    let records = decode_row(connection, &key.session_id, &kind, &body, &mut cache)?;
                    let indexed = !matches!(kind.as_str(), "step/text" | "step/thinking");
                    if indexed && event_id.as_deref() != Some(records[0].event.id.as_str()) || !indexed && event_id.is_some() {
                        return Err(error("event identity index does not match row"));
                    }
                    Ok((first,last,kind,records))
                }).collect::<Result<Vec<_>>>()
            }).await?;
            if page.is_empty() {
                break;
            }
            for (first, last, kind, records) in page {
                if first != next || records.last().unwrap().event.sequence != last {
                    return Err(error("Timeline contains a sequence gap"));
                }
                for record in records {
                    if !event_ids.insert(record.event.id.clone()) {
                        return Err(error("duplicate Timeline event identity"));
                    }
                    if record.event.sequence != next
                        || record.batch_end < next
                        || record.batch_end > sequence
                    {
                        return Err(error("invalid Timeline sequence or batch boundary"));
                    }
                    if next > batch_end {
                        batch_end = record.batch_end
                    } else if batch_end != record.batch_end {
                        return Err(error("inconsistent atomic batch boundary"));
                    }
                    if record.trace.as_ref().is_some_and(|trace| {
                        trace.event_id != record.event.id
                            || trace.original.kind.key() != kind
                            || trace.steps.iter().any(|step| {
                                step.result
                                    .as_ref()
                                    .is_some_and(|draft| draft.kind.key() != kind)
                            })
                    }) {
                        return Err(error("before trace does not match event"));
                    }
                    if let EventKind::BranchCreate(data) = &record.event.kind {
                        created.insert(data.branch.clone());
                    }
                    if let EventKind::ContextCompaction(data) = &record.event.kind
                        && matches!(data.outcome, ContextCompactionResult::Completed { .. })
                    {
                        completions.push((id.clone(), record.event.sequence, data.through));
                    }
                    next += 1;
                }
            }
        }
        if next != sequence + 1 || batch_end != sequence {
            return Err(error("Timeline head or final batch boundary is invalid"));
        }
        branches.insert(id, StoredBranch { info, sequence });
    }
    let main = branches
        .get("main")
        .ok_or_else(|| error("Session main directory is missing"))?;
    if main.info.parent.is_some()
        || main.info.branch_point.is_some()
        || main.info.create_anchor.is_some()
    {
        return Err(error("invalid main branch directory"));
    }
    if created.iter().any(|id| !branches.contains_key(id)) {
        return Err(error("branch create event has no directory"));
    }
    for (id, branch) in &branches {
        if id == "main" {
            continue;
        }
        let parent = branch
            .info
            .parent
            .as_ref()
            .ok_or_else(|| error("child branch has no parent"))?;
        let anchor = branch
            .info
            .create_anchor
            .ok_or_else(|| error("child branch has no create anchor"))?;
        let cut = branch
            .info
            .branch_point
            .ok_or_else(|| error("child branch has no branch point"))?;
        let parent_branch = branches
            .get(parent)
            .ok_or_else(|| error("child branch parent is missing"))?;
        if anchor > parent_branch.sequence {
            return Err(error("child branch references future create anchor"));
        }
        let parent_key = TimelineKey {
            session_id: session.into(),
            branch_id: parent.clone(),
        };
        let create = db
            .run(move |connection| read_events(connection, &parent_key, anchor..=anchor, false))
            .await?
            .pop()
            .ok_or_else(|| error("branch create event is missing"))?;
        match &create.event.kind {
            EventKind::BranchCreate(data)
                if data.branch == *id
                    && cut
                        == match data.at {
                            BranchCreateAt::Create {} => anchor,
                            BranchCreateAt::Sequence { sequence } => sequence,
                        } => {}
            _ => return Err(error("branch directory does not match create event")),
        }
        let mut seen = BTreeSet::new();
        let mut cursor = Some(id);
        while let Some(current) = cursor {
            if !seen.insert(current) {
                return Err(error("branch directory cycle"));
            }
            cursor = branches
                .get(current)
                .ok_or_else(|| error("child branch ancestor is missing"))?
                .info
                .parent
                .as_ref();
        }
    }
    for (branch, at, through) in completions {
        if through >= at || through > branches[&branch].sequence {
            return Err(error("compaction refers to a future preparation boundary"));
        }
        let key = TimelineKey {
            session_id: session.into(),
            branch_id: branch.clone(),
        };
        if through > 0 {
            db.run(move |connection| read_events(connection, &key, through..=through, true))
                .await
                .map_err(|_| error("compaction preparation boundary is inside an atomic batch"))?;
        }
    }
    Ok(branches.into_values().collect())
}
