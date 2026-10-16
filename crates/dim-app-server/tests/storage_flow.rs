use dim_agent_capabilities::history::{HistoryRenderer, read_context};
use dim_agent_runtime::{
    AgentBuilder, AgentConfig, AgentError, SessionConfig, StoredEvent, Timeline, TimelineKey,
    TimelineStore, TimelineTrace, TraceStep, capability_plugin,
};
use dim_agent_types::*;
use dim_app_server::storage::{SqliteStorage, SqliteTimelineStore};
use rusqlite::{Connection, params};
use std::{path::PathBuf, sync::Arc, time::Duration};

const INTERPRETER: &str = "dim/context-projection/v3";
fn snapshot(sequence: u64, checkpoint: &ProjectionCheckpoint) -> RecoverySnapshot {
    let mut snapshot = RecoverySnapshot {
        sequence,
        event_id: format!("event-{sequence}"),
        interpreter: INTERPRETER.into(),
        state: String::new(),
    };
    snapshot.state = (HistoryRenderer::default().encode_checkpoint)(&snapshot, checkpoint).unwrap();
    snapshot
}

struct Database(PathBuf);
impl Database {
    fn new() -> Self {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/storage-flow")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory.join("dimcode.sqlite"))
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}
/// A completed compaction replacing everything before `retain_from` with one
/// summary message.
fn completed(retain_from: &str, summary: &str, through: u64) -> ContextCompactionData {
    ContextCompactionData {
        turn_id: Some("completed-turn".into()),
        request_id: Some("completed-request".into()),
        command: None,
        start: None,
        through,
        outcome: ContextCompactionResult::Completed {
            retain_from: retain_from.into(),
            replacement: vec![dim_agent_capabilities::history::summary_message(summary)],
            strategy: "summary".into(),
            diagnostics: serde_json::from_value(serde_json::json!({
                "reason": "auto", "trigger": "threshold", "estimatedInputTokens": 1,
                "entriesReplaced": 1, "modelCallCount": 1, "durationMs": 1,
                "strategy": {"kind": "summary"}
            }))
            .unwrap(),
            context_baseline: vec![],
        },
    }
}

fn key(session: &str, branch: &str) -> TimelineKey {
    TimelineKey {
        session_id: session.into(),
        branch_id: branch.into(),
    }
}
fn record(sequence: u64, kind: EventKind) -> StoredEvent {
    StoredEvent {
        event: Arc::new(AgentEvent {
            id: format!("event-{sequence}"),
            sequence,
            create_at: 1000 + sequence,
            tags: Some(vec!["stored".into()]),
            kind,
        }),
        trace: None,
        batch_end: sequence,
    }
}
fn text(sequence: u64, delta: &str) -> StoredEvent {
    record(
        sequence,
        EventKind::Text(DeltaData {
            turn_id: "turn".into(),
            request_id: "request".into(),
            delta: delta.into(),
        }),
    )
}
async fn append(
    store: &SqliteTimelineStore,
    key: TimelineKey,
    records: Vec<StoredEvent>,
) -> dim_agent_runtime::Result<()> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    store.append(
        key,
        records,
        Box::new(move |result| {
            let _ = sender.send(result);
        }),
    )?;
    tokio::time::timeout(Duration::from_secs(5), receiver)
        .await
        .unwrap()
        .unwrap()
}

/// The tables of the documented record example that hold business rows, in
/// an order foreign keys accept.
const BUSINESS_TABLES: &[&str] = &[
    "settings_documents",
    "providers",
    "provider_selections",
    "provider_imports",
    "agents",
    "agent_sessions",
    "session_owners",
    "timeline_branches",
    "timeline_content",
    "timeline_events",
    "timeline_snapshots",
    "session_files",
    "session_file_owners",
];
/// Inserts the rows of `tables` from the complete record example in
/// native-storage-format.md.
fn insert_documented_records(connection: &Connection, tables: &[&str]) {
    use rusqlite::{params_from_iter, types::Value as SqlValue};
    let document = include_str!("../../../docs/design/appserver/native-storage-format.md");
    let json = document
        .split_once("## 完整记录示例")
        .unwrap()
        .1
        .split_once("```json\n")
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0;
    let example: serde_json::Value = serde_json::from_str(json).unwrap();
    for table in tables {
        for row in example[table].as_array().unwrap() {
            let row = row.as_object().unwrap();
            let columns = row.keys().cloned().collect::<Vec<_>>();
            let values = row
                .values()
                .map(|value| match value {
                    serde_json::Value::Null => SqlValue::Null,
                    serde_json::Value::Number(value) => SqlValue::Integer(value.as_i64().unwrap()),
                    serde_json::Value::String(value) => SqlValue::Text(value.clone()),
                    value => SqlValue::Text(value.to_string()),
                })
                .collect::<Vec<_>>();
            connection
                .execute(
                    &format!(
                        "INSERT INTO {table} ({}) VALUES ({})",
                        columns.join(","),
                        vec!["?"; columns.len()].join(",")
                    ),
                    params_from_iter(values),
                )
                .unwrap();
        }
    }
}

/// The v13 schema exactly as v13 daemons wrote it, frozen here so that tests
/// can build disposable v13 stores after the daemon moved on.
const V13_SCHEMA: &[&str] = &[
    "CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 13))",
    "CREATE TABLE settings_documents (source TEXT NOT NULL PRIMARY KEY, version INTEGER NOT NULL CHECK(version > 0), document TEXT NOT NULL, imported_revision TEXT NOT NULL)",
    "CREATE TABLE provider_imports (kind TEXT NOT NULL, id TEXT NOT NULL, source_schema TEXT NOT NULL, source_version INTEGER NOT NULL, source_digest TEXT NOT NULL, PRIMARY KEY (kind, id))",
    "CREATE TABLE providers (provider_id TEXT NOT NULL PRIMARY KEY, version INTEGER NOT NULL CHECK(version > 0), record TEXT NOT NULL)",
    "CREATE TABLE provider_selections (scope TEXT NOT NULL PRIMARY KEY, version INTEGER NOT NULL CHECK(version > 0), provider_id TEXT NOT NULL, model_id TEXT NOT NULL)",
    "CREATE TABLE agent_sessions (session_id TEXT NOT NULL PRIMARY KEY, config TEXT NOT NULL, created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0))",
    "CREATE TABLE agents(agent_id TEXT PRIMARY KEY, profile TEXT NOT NULL, is_default INTEGER NOT NULL CHECK(is_default IN (0, 1)))",
    "CREATE UNIQUE INDEX agents_default_unique ON agents(is_default) WHERE is_default = 1",
    "CREATE TABLE session_owners(session_id TEXT PRIMARY KEY REFERENCES agent_sessions(session_id), agent_id TEXT NOT NULL REFERENCES agents(agent_id), protocol_version INTEGER NOT NULL CHECK(protocol_version IN (1, 2, 3)))",
    "CREATE TABLE timeline_branches (session_id TEXT NOT NULL, branch_id TEXT NOT NULL, info TEXT NOT NULL, sequence INTEGER NOT NULL CHECK (sequence >= 0), PRIMARY KEY (session_id, branch_id), FOREIGN KEY (session_id) REFERENCES agent_sessions(session_id))",
    "CREATE TABLE timeline_content (session_id TEXT NOT NULL, hash TEXT NOT NULL, kind TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (session_id, hash), FOREIGN KEY (session_id) REFERENCES agent_sessions(session_id))",
    "CREATE TABLE timeline_events (session_id TEXT NOT NULL, branch_id TEXT NOT NULL, sequence INTEGER NOT NULL CHECK (sequence > 0), last_sequence INTEGER NOT NULL CHECK (last_sequence >= sequence), kind TEXT NOT NULL, body TEXT NOT NULL, event_id TEXT, PRIMARY KEY (session_id, branch_id, sequence), FOREIGN KEY (session_id, branch_id) REFERENCES timeline_branches(session_id, branch_id))",
    "CREATE TABLE timeline_snapshots (session_id TEXT NOT NULL, branch_id TEXT NOT NULL, sequence INTEGER NOT NULL CHECK (sequence > 0), interpreter TEXT NOT NULL, event_id TEXT NOT NULL, state TEXT NOT NULL, PRIMARY KEY (session_id, branch_id, interpreter, sequence), FOREIGN KEY (session_id, branch_id, sequence) REFERENCES timeline_events(session_id, branch_id, sequence))",
    "CREATE INDEX session_owners_by_agent ON session_owners(agent_id, protocol_version, session_id)",
    "CREATE INDEX timeline_events_by_end ON timeline_events(session_id, branch_id, last_sequence, sequence)",
    "CREATE INDEX timeline_events_by_id ON timeline_events(session_id, event_id) WHERE event_id IS NOT NULL",
    "CREATE INDEX timeline_tool_use_starts ON timeline_events(session_id, json_extract(body, '$.record.event.data.request_id'), json_extract(body, '$.record.event.data.call_id')) WHERE branch_id = 'main' AND kind = 'step/tool-use-start'",
];
/// What v15 adds to v13 besides the marker.
const V15_ADDITIONS: &[&str] = &[
    "CREATE TABLE session_files (file_id TEXT NOT NULL PRIMARY KEY, node TEXT, writer TEXT)",
    "CREATE TABLE session_file_owners (session_id TEXT NOT NULL REFERENCES agent_sessions(session_id), file_id TEXT NOT NULL REFERENCES session_files(file_id), PRIMARY KEY (session_id, file_id))",
    "CREATE INDEX session_file_owners_by_file ON session_file_owners(file_id)",
];
/// Builds a disposable v13 store, as a v13 daemon created it.
fn v13_store(path: &std::path::Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    for sql in V13_SCHEMA {
        connection.execute_batch(sql).unwrap();
    }
    connection
        .execute("INSERT INTO agent_store_format VALUES (13)", [])
        .unwrap();
    connection
}
/// The schema statements of a store, sorted.
fn schema(connection: &Connection) -> Vec<String> {
    let mut statements: Vec<String> = connection
        .prepare("SELECT sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    statements.sort();
    statements
}
/// Every row of `table`, rendered and sorted.
fn rows(connection: &Connection, table: &str) -> Vec<String> {
    let mut query = connection
        .prepare(&format!("SELECT * FROM {table}"))
        .unwrap();
    let columns = query.column_count();
    let mut rows: Vec<String> = query
        .query_map([], |row| {
            (0..columns)
                .map(|index| {
                    row.get::<_, rusqlite::types::Value>(index)
                        .map(|value| format!("{value:?}"))
                })
                .collect::<rusqlite::Result<Vec<_>>>()
                .map(|values| values.join("|"))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    rows.sort();
    rows
}

#[tokio::test]
async fn documented_native_records_restore_with_legacy_snapshot_bypassed() {
    use dim_agent_capabilities::history::{
        project_context_history, project_history, projection_checkpoint,
    };
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let connection = Connection::open(&database.0).unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys = ON")
        .unwrap();
    connection
        .execute("DELETE FROM agent_store_format", [])
        .unwrap();
    insert_documented_records(&connection, &["agent_store_format"]);
    insert_documented_records(&connection, BUSINESS_TABLES);
    drop(connection);

    assert_eq!(store.branches("s1").await.unwrap().len(), 2);
    let main = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("s1", "main"),
        vec![],
        store.clone(),
    )
    .await
    .unwrap();
    let generated = projection_checkpoint(&main.context(2).await.unwrap(), "e2").unwrap();
    let saved = store
        .latest_snapshot(&key("s1", "main"), 7, "dim/context-projection/v1")
        .await
        .unwrap()
        .unwrap();
    let saved: ProjectionCheckpoint = serde_json::from_str(&saved.state).unwrap();
    assert_eq!(saved, generated);
    let full = project_history(&main.history(Some(7)).await.unwrap()).unwrap();
    let recovered = read_context(&main, 7).await.unwrap();
    assert!(recovered.checkpoint.is_none());
    let compacted = project_context_history(&recovered).unwrap();
    assert_eq!(
        full.iter().map(|entry| &entry.id).collect::<Vec<_>>(),
        ["e2", "e7"]
    );
    assert_eq!(compacted, full);
    let child = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("s1", "child"),
        vec![("main".into(), 7)],
        store.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        project_context_history(&read_context(&child, 2).await.unwrap()).unwrap(),
        compacted
    );

    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    builder.timeline_store(store.clone());
    let agent = builder.build();
    let session = agent.restore_session("s1".into()).await.unwrap();
    assert!(session.sequence("child").unwrap() >= 2);
    agent.close().await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn committed_history_round_trips_requests_outputs_traces_and_branch_directory() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    assert!(
        SqliteStorage::open(database.0.clone()).is_err(),
        "only one process writer may own the database"
    );
    let store = SqliteTimelineStore::new(db.clone());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let mut request = ModelRequest::new(
        "mock",
        "test",
        vec![Message::text(Role::User, "shared message")],
    );
    request.tools = Some(Arc::new(vec![serde_json::from_value(serde_json::json!({"name":"echo", "description":"echo", "inputSchema":{"type":"object"}, "source":"static"})).unwrap()]));
    request.system = Some(Arc::new(SystemPrompt::Text("shared system".into())));
    let request = Arc::new(request);
    let prepare = EventKind::ContextPrepare(ContextPrepareData {
        turn_id: "turn".into(),
        request_id: "request".into(),
        through: 0,
        request: request.clone(),
        configuration: None,
        prompt: None,
        recovery: None,
    });
    let call = EventKind::ModelCall(ModelCallData {
        turn_id: "turn".into(),
        request_id: "request".into(),
        request,
        configuration: None,
        prompt: None,
    });
    let mut first = record(1, prepare.clone());
    first.batch_end = 2;
    first.trace = Some(TimelineTrace {
        event_id: first.event.id.clone(),
        original: EventDraft::new(prepare.clone()),
        steps: vec![TraceStep {
            handler: "before".into(),
            result: Some(EventDraft {
                tags: first.event.tags.clone(),
                kind: prepare,
            }),
        }],
    });
    let second = record(2, call);
    let mut expected = vec![first.clone(), second.clone()];
    append(&store, key("session", "main"), expected.clone())
        .await
        .unwrap();
    append(&store, key("session", "main"), vec![text(3, "a")])
        .await
        .unwrap();
    expected.push(text(3, "a"));
    let mut fourth = text(4, "b");
    fourth.batch_end = 5;
    let mut fifth = text(5, "c");
    fifth.trace = Some(TimelineTrace {
        event_id: fifth.event.id.clone(),
        original: EventDraft::new(EventKind::Text(DeltaData {
            turn_id: "turn".into(),
            request_id: "request".into(),
            delta: "before".into(),
        })),
        steps: vec![TraceStep {
            handler: "transform".into(),
            result: Some(EventDraft {
                tags: fifth.event.tags.clone(),
                kind: fifth.event.kind.clone(),
            }),
        }],
    });
    append(
        &store,
        key("session", "main"),
        vec![fourth.clone(), fifth.clone()],
    )
    .await
    .unwrap();
    expected.extend([fourth, fifth]);
    assert!(matches!(
        store.read(&key("session", "main"), 3..=4).await,
        Err(AgentError::InvalidInput(_))
    ));
    assert!(store.read(&key("session", "main"), 1..=1).await.is_err());
    assert_eq!(
        store
            .read(&key("session", "main"), 2..=2)
            .await
            .unwrap()
            .len(),
        1
    );
    let branch = record(
        6,
        EventKind::BranchCreate(BranchCreateData {
            at: BranchCreateAt::Sequence { sequence: 2 },
            source: BranchSource::Host {
                operation_id: "fork".into(),
            },
            branch: "child".into(),
            prompt: "child prompt".into(),
        }),
    );
    append(&store, key("session", "main"), vec![branch.clone()])
        .await
        .unwrap();
    expected.push(branch);
    let mut child = record(
        1,
        EventKind::InputMessage(InputMessageData::text("child", None)),
    );
    Arc::make_mut(&mut child.event).id = "child-event-1".into();
    append(&store, key("session", "child"), vec![child.clone()])
        .await
        .unwrap();
    let mut child_request = second.clone();
    Arc::make_mut(&mut child_request.event).id = "child-event-2".into();
    append(&store, key("session", "child"), vec![child_request])
        .await
        .unwrap();
    for sequence in [3, 4] {
        let mut thinking = record(
            sequence,
            EventKind::Thinking(DeltaData {
                turn_id: "child-turn".into(),
                request_id: "child-request".into(),
                delta: "thought".into(),
            }),
        );
        Arc::make_mut(&mut thinking.event).id = format!("child-event-{sequence}");
        append(&store, key("session", "child"), vec![thinking])
            .await
            .unwrap();
    }
    for entry in [
        text(7, "after branch"),
        text(8, &"x".repeat(4100)),
        text(9, "after size"),
        {
            let mut entry = text(10, "after time");
            Arc::make_mut(&mut entry.event).create_at = 3000;
            entry
        },
    ] {
        append(&store, key("session", "main"), vec![entry.clone()])
            .await
            .unwrap();
        expected.push(entry);
    }
    store.close().await.unwrap();
    db.close().await.unwrap();
    drop(store);
    drop(db);
    let connection = Connection::open(&database.0).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM timeline_content", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM timeline_events WHERE kind = 'step/text'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        4
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM timeline_events WHERE kind = 'step/thinking'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    drop(connection);
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let branches = store.branches("session").await.unwrap();
    let child_info = &branches
        .iter()
        .find(|branch| branch.info.id == "child")
        .unwrap()
        .info;
    assert_eq!(child_info.parent.as_deref(), Some("main"));
    assert_eq!(child_info.branch_point, Some(2));
    assert_eq!(child_info.create_anchor, Some(6));
    let restored = store.read(&key("session", "main"), 1..=10).await.unwrap();
    for (actual, expected) in restored.iter().zip(&expected) {
        assert_eq!(actual.event, expected.event);
        assert_eq!(actual.batch_end, expected.batch_end);
        assert_eq!(
            serde_json::to_value(&actual.trace).unwrap(),
            serde_json::to_value(&expected.trace).unwrap()
        );
    }
    let EventKind::ContextPrepare(prepare) = &restored[0].event.kind else {
        panic!()
    };
    let EventKind::ModelCall(call) = &restored[1].event.kind else {
        panic!()
    };
    let segments = store
        .read_segments(
            "session",
            vec![("main".into(), 1..=2), ("child".into(), 2..=2)],
        )
        .await
        .unwrap();
    let EventKind::ContextPrepare(ancestor_request) = &segments[0][0].event.kind else {
        panic!()
    };
    let EventKind::ModelCall(child_request) = &segments[1][0].event.kind else {
        panic!()
    };
    assert!(Arc::ptr_eq(
        &ancestor_request.request.messages[0],
        &child_request.request.messages[0]
    ));
    assert!(Arc::ptr_eq(
        &prepare.request.messages[0],
        &call.request.messages[0]
    ));
    assert!(Arc::ptr_eq(
        prepare.request.tools.as_ref().unwrap(),
        call.request.tools.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        prepare.request.system.as_ref().unwrap(),
        call.request.system.as_ref().unwrap()
    ));
    assert_eq!(
        store.read(&key("session", "child"), 1..=1).await.unwrap()[0].event,
        child.event
    );
    let timeline = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("session", "main"),
        vec![],
        store.clone(),
    )
    .await
    .unwrap();
    assert_eq!(timeline.sequence(), 10);
    assert_eq!(timeline.history(None).await.unwrap().len(), 10);
    timeline
        .append(EventDraft::new(EventKind::StopInterrupt(
            StopInterruptData {
                reason: "filler".into(),
                scope: InterruptScope::Turn,
            },
        )))
        .unwrap();
    timeline.close().await.unwrap();
    assert_eq!(store.head(&key("session", "main")).await.unwrap(), 11);
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn failed_session_rolls_back_directory_and_does_not_block_other_sessions() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    for session in ["warm", "bad", "good", "other"] {
        store
            .create_session(session, &SessionConfig::default())
            .await
            .unwrap();
    }
    append(&store, key("warm", "main"), vec![text(1, "warm")])
        .await
        .unwrap();
    let connection = Connection::open(&database.0).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_bad BEFORE INSERT ON timeline_events WHEN NEW.session_id = 'bad' BEGIN SELECT RAISE(ABORT, 'injected failure'); END").unwrap();
    let bad_event = record(
        1,
        EventKind::BranchCreate(BranchCreateData {
            at: BranchCreateAt::Create {},
            source: BranchSource::Host {
                operation_id: "fork".into(),
            },
            branch: "missing".into(),
            prompt: String::new(),
        }),
    );
    let (bad, good, other) = tokio::join!(
        append(&store, key("bad", "main"), vec![bad_event]),
        append(&store, key("good", "main"), vec![text(1, "good")]),
        append(&store, key("other", "main"), vec![text(1, "other")])
    );
    assert!(matches!(bad, Err(AgentError::Storage(_))));
    good.unwrap();
    other.unwrap();
    assert_eq!(store.head(&key("bad", "main")).await.unwrap(), 0);
    assert_eq!(store.branches("bad").await.unwrap().len(), 1);
    assert!(
        store
            .append(
                key("bad", "main"),
                vec![text(1, "late")],
                Box::new(|_| panic!("rejected append callback must not run"))
            )
            .is_err()
    );
    assert_eq!(store.head(&key("good", "main")).await.unwrap(), 1);
    assert_eq!(store.head(&key("other", "main")).await.unwrap(), 1);
    connection.execute_batch("DROP TRIGGER fail_bad").unwrap();
    drop(connection);
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_output_commit_closes_only_its_session_and_recovery_settles_the_orphan() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let database = Database::new();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        let mut builder = AgentBuilder::new(AgentConfig::default());
        dim_agent_capabilities::defaults::mock_hosts(&mut builder);
        dim_agent_capabilities::defaults::install(&mut builder);
        builder.timeline_store(store.clone());
        let agent = builder.build();
        let bad = agent.create_session(SessionConfig::default()).await.unwrap();
        let good = agent.create_session(SessionConfig::default()).await.unwrap();
        let bad_id = bad.id().to_owned();
        let connection = Connection::open(&database.0).unwrap();
        connection.execute_batch("CREATE TRIGGER fail_output BEFORE INSERT ON timeline_events WHEN NEW.kind = 'step/text' AND NEW.session_id = (SELECT session_id FROM injected_session) BEGIN SELECT RAISE(ABORT, 'injected output commit failure'); END; CREATE TABLE injected_session(session_id TEXT)").unwrap();
        connection.execute("INSERT INTO injected_session VALUES (?1)", [&bad_id]).unwrap();
        let (admitted_bad, admitted_good) = tokio::join!(
            bad.submit("failed output".into(), InputMode::Queue, "main"),
            good.submit("healthy output".into(), InputMode::Queue, "main"),
        );
        admitted_bad.unwrap();
        admitted_good.unwrap();
        let error = bad.closed().await.unwrap_err();
        assert!(error.to_string().contains("injected output commit failure"));
        good.idle("main").await.unwrap();
        assert!(agent.get_session(&bad_id).is_none());
        assert!(agent.get_session(good.id()).is_some());
        let bad_history = bad.history("main", None).await.unwrap();
        assert!(bad_history.iter().any(|event| matches!(event.kind, EventKind::ModelCall(_))));
        assert!(!bad_history.iter().any(|event| matches!(event.kind, EventKind::Text(_) | EventKind::ModelOutputEnd(_) | EventKind::TurnEnd(_))));
        assert!(bad.submit("late".into(), InputMode::Queue, "main").await.unwrap_err().to_string().contains("injected output commit failure"));
        assert!(good.history("main", None).await.unwrap().iter().any(|event| matches!(&event.kind, EventKind::Text(data) if data.delta.contains("healthy output"))));
        connection.execute_batch("DROP TRIGGER fail_output; DROP TABLE injected_session").unwrap();
        drop(connection);
        let _ = agent.close().await;
        drop(bad);
        drop(good);
        drop(agent);
        store.close().await.unwrap();
        db.close().await.unwrap();
        drop(builder);
        drop(store);
        drop(db);

        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        let mut builder = AgentBuilder::new(AgentConfig::default());
        dim_agent_capabilities::defaults::mock_hosts(&mut builder);
        dim_agent_capabilities::defaults::install(&mut builder);
        builder.timeline_store(store.clone());
        let restored = builder.build();
        let session = restored.restore_session(bad_id).await.unwrap();
        session.idle("main").await.unwrap();
        let history = session.history("main", None).await.unwrap();
        assert!(history.iter().any(|event| matches!(&event.kind, EventKind::ModelOutputEnd(data) if matches!(data.outcome, ModelOutputOutcome::Failed { .. }))));
        assert!(history.iter().any(|event| matches!(&event.kind, EventKind::TurnEnd(data) if matches!(data.outcome, TurnOutcome::Failed { .. }))));
        session.submit("after recovery".into(), InputMode::Queue, "main").await.unwrap();
        session.idle("main").await.unwrap();
        assert!(session.history("main", None).await.unwrap().iter().any(|event| matches!(&event.kind, EventKind::Text(data) if data.delta.contains("after recovery"))));
        restored.close().await.unwrap();
        store.close().await.unwrap();
        db.close().await.unwrap();
    }).await.expect("Session failure and recovery completed");
}

#[tokio::test]
async fn recovery_rejects_corrupt_records_and_preserves_them() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    append(&store, key("session", "main"), vec![text(1, "saved")])
        .await
        .unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
    drop(store);
    drop(db);
    let connection = Connection::open(&database.0).unwrap();
    let original: String = connection
        .query_row("SELECT body FROM timeline_events", [], |row| row.get(0))
        .unwrap();
    let mut corrupt: serde_json::Value = serde_json::from_str(&original).unwrap();
    corrupt["records"][0]["batch_end"] = 2.into();
    let corrupt = serde_json::to_string(&corrupt).unwrap();
    connection
        .execute("UPDATE timeline_events SET body = ?1", [&corrupt])
        .unwrap();
    drop(connection);
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    assert!(matches!(
        store.branches("session").await,
        Err(AgentError::Storage(_))
    ));
    store.close().await.unwrap();
    db.close().await.unwrap();
    let connection = Connection::open(&database.0).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT body FROM timeline_events", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        corrupt
    );
    drop(store);
    drop(db);
    connection
        .execute_batch("PRAGMA foreign_keys = OFF")
        .unwrap();
    connection
        .execute(
            "UPDATE timeline_events SET body = ?1, branch_id = 'missing'",
            [&original],
        )
        .unwrap();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    assert!(matches!(
        store.branches("session").await,
        Err(AgentError::Storage(_))
    ));
    store.close().await.unwrap();
    db.close().await.unwrap();
    assert_eq!(
        connection
            .query_row("SELECT branch_id FROM timeline_events", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "missing"
    );
}

#[tokio::test]
async fn invalid_ancestor_directory_rejects_only_its_session() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    for session in ["broken", "healthy"] {
        store
            .create_session(session, &SessionConfig::default())
            .await
            .unwrap();
    }
    append(
        &store,
        key("broken", "main"),
        vec![record(
            1,
            EventKind::BranchCreate(BranchCreateData {
                at: BranchCreateAt::Create {},
                source: BranchSource::Host {
                    operation_id: "first".into(),
                },
                branch: "z".into(),
                prompt: "first".into(),
            }),
        )],
    )
    .await
    .unwrap();
    let mut nested = record(
        1,
        EventKind::BranchCreate(BranchCreateData {
            at: BranchCreateAt::Create {},
            source: BranchSource::Host {
                operation_id: "second".into(),
            },
            branch: "a".into(),
            prompt: "second".into(),
        }),
    );
    Arc::make_mut(&mut nested.event).id = "nested-create".into();
    append(&store, key("broken", "z"), vec![nested])
        .await
        .unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
    drop(store);
    drop(db);

    let connection = Connection::open(&database.0).unwrap();
    connection
        .execute(
            "UPDATE timeline_branches SET info = json_set(info, '$.parent', 'missing') WHERE session_id = 'broken' AND branch_id = 'z'",
            [],
        )
        .unwrap();
    drop(connection);
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    assert!(matches!(
        store.branches("broken").await,
        Err(AgentError::Storage(_))
    ));
    assert_eq!(store.branches("healthy").await.unwrap().len(), 1);
    store.close().await.unwrap();
    db.close().await.unwrap();
    let connection = Connection::open(&database.0).unwrap();
    let info: String = connection
        .query_row(
            "SELECT info FROM timeline_branches WHERE session_id = 'broken' AND branch_id = 'z'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&info).unwrap()["parent"],
        "missing"
    );
}

#[test]
fn unsupported_legacy_or_changed_schema_is_rejected_without_migration() {
    /// The store a case changes.
    enum Base {
        Empty,
        V13,
        V15,
    }
    for (base, sql) in [
        (Base::Empty, "CREATE TABLE sessions (id TEXT PRIMARY KEY)"),
        (
            Base::Empty,
            "CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 1)); INSERT INTO agent_store_format VALUES (1)",
        ),
        (
            Base::Empty,
            "CREATE TABLE agent_store_format (version INTEGER PRIMARY KEY); INSERT INTO agent_store_format VALUES (99)",
        ),
        (
            Base::V13,
            r#"DROP TABLE agent_store_format;
            CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 1));
            INSERT INTO agent_store_format VALUES (1);
            INSERT INTO agents VALUES ('legacy', '{"name":"legacy","description":"","systemPrompt":"","tools":[]}', 1)"#,
        ),
        (
            Base::V13,
            r#"DROP TABLE agent_store_format;
            CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 2));
            INSERT INTO agent_store_format VALUES (2);"#,
        ),
        // v4 records keep input state, business blocks and prompt deliveredBy.
        (
            Base::V13,
            r#"DROP TABLE agent_store_format;
            CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 4));
            INSERT INTO agent_store_format VALUES (4);"#,
        ),
        // v5 calls closed only by projection fill-ins have no closing fact (agent-029).
        (
            Base::V13,
            r#"DROP TABLE agent_store_format;
            CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 5));
            INSERT INTO agent_store_format VALUES (5);"#,
        ),
        // v6 stores a derived account catalog, without canonical model overrides.
        (
            Base::V13,
            r#"DROP TABLE agent_store_format;
            CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 6));
            INSERT INTO agent_store_format VALUES (6);"#,
        ),
        // v7 selections identify only a model; v8 fixes the full provider/model identity.
        (
            Base::V13,
            r#"DROP TABLE agent_store_format;
            CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 7));
            INSERT INTO agent_store_format VALUES (7);"#,
        ),
        // v8 has no managed OAuth binding in the provider record.
        (
            Base::V13,
            r#"DROP TABLE agent_store_format;
            CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 8));
            INSERT INTO agent_store_format VALUES (8);"#,
        ),
        // v10 has a private account catalog JSON variant, not normalized managed records.
        (
            Base::V13,
            r#"DROP TABLE agent_store_format;
            CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 10));
            INSERT INTO agent_store_format VALUES (10);"#,
        ),
        // v11's inline checkpoint schema must be preserved and rejected.
        (
            Base::V13,
            r#"DROP TABLE agent_store_format;
            CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 11));
            INSERT INTO agent_store_format VALUES (11);
            DROP TABLE timeline_snapshots;
            ALTER TABLE timeline_events ADD COLUMN checkpoint INTEGER NOT NULL DEFAULT 0;
            CREATE INDEX timeline_events_checkpoints ON timeline_events(session_id,branch_id,checkpoint,sequence);"#,
        ),
        // v9 is an unpublished development format without import markers.
        (
            Base::V13,
            r#"DROP TABLE provider_imports; DROP TABLE agent_store_format;
            CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 9));
            INSERT INTO agent_store_format VALUES (9);
            INSERT INTO providers VALUES ('preserve',7,'{"credential":"original"}');"#,
        ),
        // The removed branch-only v14 envelope must not be downgraded to v13.
        (
            Base::V13,
            r#"DROP TABLE settings_documents; DROP TABLE agent_store_format;
            CREATE TABLE agent_store_format (version INTEGER NOT NULL PRIMARY KEY CHECK (version = 14));
            INSERT INTO agent_store_format VALUES (14);
            CREATE TABLE user_configuration (layer TEXT NOT NULL PRIMARY KEY CHECK(layer = 'user'), version INTEGER NOT NULL CHECK(version > 0), envelope TEXT NOT NULL);
            INSERT INTO user_configuration VALUES ('user', 7, '{"format":1,"patches":[],"documents":{},"imports":{}}');"#,
        ),
        // A matching v13 schema cannot disguise a v14 format marker either.
        (
            Base::V13,
            "PRAGMA ignore_check_constraints=ON; UPDATE agent_store_format SET version=14",
        ),
        // A matching schema still requires a valid format marker; a v13
        // schema without marker 13 is never upgraded.
        (Base::V13, "DELETE FROM agent_store_format"),
        (
            Base::V13,
            "PRAGMA ignore_check_constraints=ON; UPDATE agent_store_format SET version=99",
        ),
        // A v15 schema requires marker 15; it is never read as v13.
        (Base::V15, "DELETE FROM agent_store_format"),
        (
            Base::V15,
            "PRAGMA ignore_check_constraints=ON; UPDATE agent_store_format SET version=13",
        ),
        // A partly upgraded v13 store is neither format.
        (
            Base::V13,
            "CREATE TABLE session_files (file_id TEXT NOT NULL PRIMARY KEY, node TEXT, writer TEXT)",
        ),
        (Base::V15, "DROP INDEX session_file_owners_by_file"),
    ] {
        let database = Database::new();
        match base {
            Base::Empty => {}
            // v1 to v13 used the same tables; only their markers differ.
            Base::V13 => drop(v13_store(&database.0)),
            Base::V15 => drop(SqliteStorage::open(database.0.clone()).unwrap()),
        }
        let connection = Connection::open(&database.0).unwrap();
        connection.execute_batch(sql).unwrap();
        drop(connection);
        let original = std::fs::read(&database.0).unwrap();
        for _ in 0..2 {
            // A failed startup must release its maintenance lease for retry.
            let Err(AgentError::Storage(message)) = SqliteStorage::open(database.0.clone()) else {
                panic!("unsupported database must return a storage error");
            };
            assert!(message.starts_with("unsupported "), "{message}");
            assert!(
                message.contains(&database.0.canonicalize().unwrap().display().to_string()),
                "{message}"
            );
            assert!(message.contains("database was preserved"), "{message}");
            assert!(message.contains("compatible daemon"), "{message}");
            assert!(message.contains("new database path"), "{message}");
            assert_eq!(std::fs::read(&database.0).unwrap(), original);
        }
    }
}

fn marker(connection: &Connection) -> Vec<i64> {
    connection
        .prepare("SELECT version FROM agent_store_format")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// S5a's one-way upgrade: an exact v13 store becomes the exact v15 store in
/// place, with every business row unchanged, and only that open reports it.
#[tokio::test]
async fn a_v13_store_upgrades_in_place_once() {
    let mut v15: Vec<String> = V13_SCHEMA
        .iter()
        .chain(V15_ADDITIONS)
        .map(|sql| sql.replace("(version = 13)", "(version = 15)"))
        .collect();
    v15.sort();

    // An empty store is created at v15.
    let fresh = Database::new();
    let db = SqliteStorage::open(fresh.0.clone()).unwrap();
    assert_eq!(db.upgraded_from(), None);
    db.close().await.unwrap();
    let connection = Connection::open(&fresh.0).unwrap();
    assert_eq!(schema(&connection), v15);
    assert_eq!(marker(&connection), [15]);
    drop(connection);

    // A v13 store holding every documented business row.
    let database = Database::new();
    let connection = v13_store(&database.0);
    connection
        .execute_batch("PRAGMA foreign_keys = ON")
        .unwrap();
    let v13_tables = &BUSINESS_TABLES[..BUSINESS_TABLES.len() - 2];
    insert_documented_records(&connection, v13_tables);
    let before: Vec<_> = v13_tables
        .iter()
        .map(|table| rows(&connection, table))
        .collect();
    assert!(before.iter().filter(|rows| !rows.is_empty()).count() >= 7);
    drop(connection);

    let db = SqliteStorage::open(database.0.clone()).unwrap();
    assert_eq!(db.upgraded_from(), Some(13));
    let store = SqliteTimelineStore::new(db.clone());
    assert_eq!(store.branches("s1").await.unwrap().len(), 2);
    store.close().await.unwrap();
    db.close().await.unwrap();
    let connection = Connection::open(&database.0).unwrap();
    assert_eq!(schema(&connection), v15);
    assert_eq!(marker(&connection), [15]);
    let after: Vec<_> = v13_tables
        .iter()
        .map(|table| rows(&connection, table))
        .collect();
    assert_eq!(after, before);
    for table in ["session_files", "session_file_owners"] {
        assert!(rows(&connection, table).is_empty());
    }
    drop(connection);

    // The upgraded store reopens as v15 and reports nothing.
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    assert_eq!(db.upgraded_from(), None);
    db.close().await.unwrap();
}

/// The daemon names the one-way upgrade on stderr, where its default log
/// filter would hide a tracing event, and only on the start that upgrades.
#[cfg(unix)]
#[tokio::test]
async fn the_daemon_reports_a_v13_upgrade_on_stderr_once() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    let database = Database::new();
    drop(v13_store(&database.0));
    let path = database.0.canonicalize().unwrap();
    let socket = PathBuf::from("/tmp").join(format!("dim-upgrade-{}/s", uuid::Uuid::new_v4()));
    let mut reports = vec![];
    for _ in 0..2 {
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_dim-app-server"))
            .args(["--mock-model", "--no-ws", "--unix"])
            .arg(&socket)
            .arg("--database")
            .arg(&database.0)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut ready = String::new();
        tokio::time::timeout(
            Duration::from_secs(10),
            BufReader::new(child.stdout.take().unwrap()).read_line(&mut ready),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(ready.contains("unixPath"), "{ready}");
        child.kill().await.unwrap();
        let mut stderr = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .await
            .unwrap();
        reports.push(stderr);
    }
    let line = format!(
        "Upgraded native store {} from format v13 to v15; earlier dimcode versions cannot open it.",
        path.display()
    );
    assert!(
        reports[0].lines().any(|report| report == line),
        "{reports:?}"
    );
    assert!(!reports[1].contains("Upgraded native store"), "{reports:?}");
    let _ = std::fs::remove_dir_all(socket.parent().unwrap());
}

#[tokio::test]
async fn owned_session_creation_and_plugin_shutdown_keep_one_resource_owner() {
    let database = Database::new();
    let root = rutis::Ctx::root().unwrap();
    let sqlite = root.plugin(capability_plugin(
        dim_app_server::storage::SqliteStoragePlugin::new(database.0.clone()),
    ));
    let session = root.plugin(capability_plugin(
        dim_app_server::storage::SessionStorePlugin::new(0),
    ));
    (&sqlite).await.unwrap();
    (&session).await.unwrap();
    let connection = Connection::open(&database.0).unwrap();
    connection
        .execute(
            "INSERT INTO agents VALUES ('owner', ?1, 1)",
            [serde_json::to_string(&AgentProfile {
                name: "owner".into(),
                description: String::new(),
                system_prompt: String::new(),
                tools: vec![],
                model: None,
            })
            .unwrap()],
        )
        .unwrap();
    let directory = root.get::<dim_app_server::storage::SessionStore>().unwrap();
    let retained_db = root.get::<SqliteStorage>().unwrap();
    assert!(
        directory
            .create_owned_session("missing-owner", &SessionConfig::default(), "unknown", 2)
            .await
            .is_err()
    );
    assert!(directory.sessions().await.unwrap().is_empty());
    directory
        .create_owned_session("owned", &SessionConfig::default(), "owner", 2)
        .await
        .unwrap();
    assert_eq!(directory.sessions().await.unwrap(), ["owned"]);
    assert_eq!(
        connection
            .query_row(
                "SELECT agent_id FROM session_owners WHERE session_id = ?1",
                params!["owned"],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "owner"
    );
    let store = directory.timeline_store();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    store
        .append(
            key("owned", "main"),
            vec![text(1, "shutdown pending")],
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        )
        .unwrap();
    root.shutdown().await.unwrap();
    receiver.await.unwrap().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT sequence FROM timeline_branches WHERE session_id = 'owned'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(matches!(
        directory.sessions().await,
        Err(AgentError::Storage(_))
    ));
    drop(connection);
    let replacement = SqliteStorage::open(database.0.clone()).unwrap();
    retained_db.close().await.unwrap();
    assert!(SqliteStorage::open(database.0.clone()).is_err());
    replacement.close().await.unwrap();

    let plugin = dim_agent_runtime::capability_plugin(
        dim_app_server::storage::SqliteStoragePlugin::new(database.0.clone()),
    );
    assert!(rutis::Plugin::apply(&plugin, &root).await.is_err());
    let reopened = SqliteStorage::open(database.0.clone()).unwrap();
    reopened.close().await.unwrap();
}

// Hold a child before exec, when close-on-exec descriptors are still inherited.
// The database must release ownership on close, not when that child reaches exec.
#[cfg(unix)]
#[tokio::test]
async fn sqlite_close_releases_ownership_while_a_spawned_child_inherits_descriptors() {
    use std::{
        io::{Read, Write},
        os::{fd::AsRawFd, unix::net::UnixStream, unix::process::CommandExt},
    };
    unsafe extern "C" {
        fn read(fd: i32, buffer: *mut u8, count: usize) -> isize;
        fn write(fd: i32, buffer: *const u8, count: usize) -> isize;
    }

    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let (mut parent, child) = UnixStream::pair().unwrap();
    parent
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let spawning = std::thread::spawn(move || {
        let fd = child.as_raw_fd();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.arg("--list").stdout(std::process::Stdio::null());
        // SAFETY: the child uses only async-signal-safe read/write before exec;
        // its socket and both one-byte buffers remain live throughout the calls.
        unsafe {
            command.pre_exec(move || {
                let mut byte = 1_u8;
                if write(fd, &byte, 1) != 1 || read(fd, &mut byte, 1) != 1 {
                    return Err(std::io::Error::from_raw_os_error(5));
                }
                Ok(())
            });
        }
        command.spawn().unwrap().wait().unwrap()
    });
    let mut ready = [0];
    parent.read_exact(&mut ready).unwrap();
    assert_eq!(ready, [1]);
    let closed = db.close().await;
    let replacement = SqliteStorage::open(database.0.clone());
    // Always release and reap the child before asserting the observed result.
    parent.write_all(&[1]).unwrap();
    assert!(spawning.join().unwrap().success());
    closed.unwrap();
    let replacement = replacement.expect("successful close must release native ownership");
    db.close().await.unwrap();
    assert!(SqliteStorage::open(database.0.clone()).is_err());
    replacement.close().await.unwrap();
}

#[test]
fn sqlite_shutdown_drains_cancelled_and_queued_operations_before_releasing_ownership() {
    use std::{future::Future, task::Poll};
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let database = Database::new();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        let config = SessionConfig::default();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        ready.await.unwrap();

        let mut accepted = store.create_session("accepted", &config);
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(accepted.as_mut().poll(cx).is_pending())).await
        );
        let mut cancelled = store.create_session("cancelled", &config);
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(cancelled.as_mut().poll(cx).is_pending())).await
        );
        drop(cancelled);
        let mut cancelled_close = Box::pin(db.close());
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(cancelled_close.as_mut().poll(cx).is_pending()))
                .await
        );
        drop(cancelled_close);
        let mut late = store.create_session("late", &config);
        assert!(matches!(
            std::future::poll_fn(|cx| Poll::Ready(late.as_mut().poll(cx))).await,
            Poll::Ready(Err(AgentError::Storage(_)))
        ));
        assert!(SqliteStorage::open(database.0.clone()).is_err());
        release.send(()).unwrap();
        blocker.await.unwrap();
        accepted.await.unwrap();
        store.close().await.unwrap();

        let replacement = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(reopened) = SqliteStorage::open(database.0.clone()) {
                    break reopened;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("cancelled shutdown must finish without another close caller");
        let replacement_store = SqliteTimelineStore::new(replacement.clone());
        for session in ["accepted", "cancelled"] {
            assert!(
                replacement_store
                    .session_config(session)
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        assert!(
            replacement_store
                .session_config("late")
                .await
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            store.create_session("stale", &config).await,
            Err(AgentError::Storage(_))
        ));
        db.close().await.unwrap();
        assert!(SqliteStorage::open(database.0.clone()).is_err());
        replacement_store.close().await.unwrap();
        replacement.close().await.unwrap();
    });
}

#[tokio::test]
async fn concurrent_and_cancelled_close_waits_for_the_same_durable_round() {
    use std::{future::Future, task::Poll};
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    append(&store, key("session", "main"), vec![text(1, "warm")])
        .await
        .unwrap();
    let mut connection = Connection::open(&database.0).unwrap();
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    store
        .append(
            key("session", "main"),
            vec![text(2, "pending")],
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        )
        .unwrap();
    let mut cancelled = Box::pin(store.close());
    let completed =
        std::future::poll_fn(|cx| Poll::Ready(cancelled.as_mut().poll(cx).is_ready())).await;
    assert!(!completed);
    drop(cancelled);
    let mut close = Box::pin(store.close());
    assert!(
        tokio::time::timeout(Duration::from_millis(10), close.as_mut())
            .await
            .is_err()
    );
    transaction.commit().unwrap();
    close.await.unwrap();
    receiver.await.unwrap().unwrap();
    assert_eq!(store.head(&key("session", "main")).await.unwrap(), 2);
    db.close().await.unwrap();
}

#[tokio::test]
async fn damaged_snapshot_cannot_silently_remove_retained_output_after_restart() {
    use dim_agent_capabilities::history::{
        context_messages, project_context_history, projection_checkpoint, summary_message,
    };

    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    append(
        &store,
        key("session", "main"),
        vec![
            record(
                1,
                EventKind::InputMessage(InputMessageData::text("question", None)),
            ),
            record(
                2,
                EventKind::InputConsumed(InputConsumedData {
                    input_id: "event-1".into(),
                }),
            ),
            record(
                3,
                EventKind::ModelStart(ModelStartData {
                    turn_id: "turn".into(),
                    request_id: "request".into(),
                }),
            ),
            text(4, "RETAINED ANSWER"),
            record(
                5,
                EventKind::ModelOutputEnd(ModelOutputEndData {
                    turn_id: "turn".into(),
                    request_id: "request".into(),
                    outcome: ModelOutputOutcome::Completed {
                        finish_reason: FinishReason::EndTurn,
                    },
                }),
            ),
        ],
    )
    .await
    .unwrap();
    let timeline = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("session", "main"),
        vec![],
        store.clone(),
    )
    .await
    .unwrap();
    // The cut keeps the answer: the model-start of its request.
    let checkpoint = projection_checkpoint(&timeline.context(5).await.unwrap(), "event-3").unwrap();
    assert_eq!(checkpoint.resume.sequence, 5);
    assert!(!checkpoint.state.output.is_empty());
    let compaction = completed("event-3", "summary", 5);
    timeline.close().await.unwrap();
    append(
        &store,
        key("session", "main"),
        vec![record(6, EventKind::ContextCompaction(compaction))],
    )
    .await
    .unwrap();
    let saved = snapshot(6, &checkpoint);
    store
        .save_snapshot(&key("session", "main"), saved.clone())
        .await
        .unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();

    let connection = Connection::open(&database.0).unwrap();
    let facts = || {
        connection
            .prepare("SELECT body FROM timeline_events ORDER BY sequence")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let original = facts();
    let mut damaged: serde_json::Value = serde_json::from_str(&saved.state).unwrap();
    let mut payload: serde_json::Value =
        serde_json::from_str(damaged["payload"].as_str().unwrap()).unwrap();
    payload["state"]["output"] = serde_json::json!([]);
    damaged["payload"] = payload.to_string().into();
    for (state, intact) in [(saved.state, true), (damaged.to_string(), false)] {
        connection
            .execute("UPDATE timeline_snapshots SET state=?1", [state])
            .unwrap();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        store.branches("session").await.unwrap();
        let timeline = Timeline::restore(
            rutis::Ctx::root().unwrap(),
            key("session", "main"),
            vec![],
            store.clone(),
        )
        .await
        .unwrap();
        let full = timeline.context(6).await.unwrap();
        let recovered = read_context(&timeline, 6).await.unwrap();
        let expected = vec![
            summary_message("summary"),
            Message {
                role: Role::Assistant,
                content: MessageContent::Parts(vec![MessagePart::Text {
                    provider_metadata: None,
                    text: "RETAINED ANSWER".into(),
                    start_time: None,
                    end_time: None,
                }]),
            },
        ];
        assert_eq!(
            context_messages(&full, &project_context_history(&full).unwrap()),
            expected
        );
        assert_eq!(
            context_messages(&recovered, &project_context_history(&recovered).unwrap()),
            expected
        );
        assert_eq!(recovered.checkpoint.is_some(), intact);
        timeline.close().await.unwrap();
        store.close().await.unwrap();
        db.close().await.unwrap();
        assert_eq!(facts(), original);
    }
}

#[tokio::test]
async fn recovery_ignores_disposable_snapshots_but_preserves_and_validates_facts() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let mut input_batch = vec![
        record(
            1,
            EventKind::InputMessage(InputMessageData::text("history", Some(InputMode::Queue))),
        ),
        record(
            2,
            EventKind::InputConsumed(InputConsumedData {
                input_id: "event-1".into(),
            }),
        ),
    ];
    for event in &mut input_batch {
        event.batch_end = 2;
    }
    append(&store, key("session", "main"), input_batch)
        .await
        .unwrap();
    // Nothing is kept: the cut is the fact's own event, after every entry.
    let compaction = completed("event-3", "retained summary", 2);
    let original = record(3, EventKind::ContextCompaction(compaction));
    append(&store, key("session", "main"), vec![original.clone()])
        .await
        .unwrap();
    assert!(
        store
            .latest_snapshot(&key("session", "main"), 3, INTERPRETER)
            .await
            .unwrap()
            .is_none()
    );
    for (sequence, event_id) in [(3, "wrong"), (4, "event-4")] {
        assert!(
            store
                .save_snapshot(
                    &key("session", "main"),
                    RecoverySnapshot {
                        sequence,
                        event_id: event_id.into(),
                        interpreter: INTERPRETER.into(),
                        state: "opaque".into(),
                    }
                )
                .await
                .is_err()
        );
    }
    let opaque = RecoverySnapshot {
        sequence: 3,
        event_id: "event-3".into(),
        interpreter: INTERPRETER.into(),
        state: "not json".into(),
    };
    store
        .save_snapshot(&key("session", "main"), opaque.clone())
        .await
        .unwrap();
    assert_eq!(
        store
            .latest_snapshot(&key("session", "main"), 3, INTERPRETER)
            .await
            .unwrap(),
        Some(opaque)
    );
    assert!(
        store
            .latest_snapshot(&key("session", "main"), 2, INTERPRETER)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .latest_snapshot(&key("session", "main"), 3, "other")
            .await
            .unwrap()
            .is_none()
    );
    // Anchoring is structural (#130): any existing event with its id and
    // sequence anchors a snapshot; its owner decides whether it is usable.
    let input_anchored = RecoverySnapshot {
        sequence: 1,
        event_id: "event-1".into(),
        interpreter: INTERPRETER.into(),
        state: "opaque".into(),
    };
    store
        .save_snapshot(&key("session", "main"), input_anchored.clone())
        .await
        .unwrap();
    assert_eq!(
        store
            .latest_snapshot(&key("session", "main"), 2, INTERPRETER)
            .await
            .unwrap(),
        Some(input_anchored)
    );
    store.close().await.unwrap();
    db.close().await.unwrap();
    drop(store);
    drop(db);
    let connection = Connection::open(&database.0).unwrap();
    let body: String = connection
        .query_row(
            "SELECT body FROM timeline_events WHERE sequence = 3",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut states = vec![
        None,
        Some(("unsupported/v99", "not json".to_owned())),
        Some((INTERPRETER, "not json".to_owned())),
    ];
    // Sequence 2 is a legal resume address, but this state omits the pending
    // input consumed there. Decoding and anchor checks alone cannot accept it.
    for (branch, sequence) in [("unknown", 1), ("main", 0), ("main", 3), ("main", 2)] {
        states.push(Some((
            INTERPRETER,
            snapshot(
                3,
                &ProjectionCheckpoint {
                    resume: TimelinePosition {
                        branch_id: branch.into(),
                        sequence,
                    },
                    state: ProjectionCheckpointState {
                        retain_from: "event-3".into(),
                        ..Default::default()
                    },
                },
            )
            .state,
        )));
    }
    for state in states {
        connection
            .execute("DELETE FROM timeline_snapshots", [])
            .unwrap();
        if let Some((interpreter, state)) = state {
            connection
                .execute(
                    "INSERT INTO timeline_snapshots VALUES ('session','main',3,?1,'event-3',?2)",
                    params![interpreter, state],
                )
                .unwrap();
        }
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        store.branches("session").await.unwrap();
        let timeline = Timeline::restore(
            rutis::Ctx::root().unwrap(),
            key("session", "main"),
            vec![],
            store.clone(),
        )
        .await
        .unwrap();
        let full = timeline.context(3).await.unwrap();
        assert!(full.checkpoint.is_none());
        let accelerated = read_context(&timeline, 3).await.unwrap();
        assert!(accelerated.checkpoint.is_none());
        use dim_agent_capabilities::history::{
            context_messages, project_context_history, summary_message,
        };
        assert_eq!(
            context_messages(
                &accelerated,
                &project_context_history(&accelerated).unwrap()
            ),
            [summary_message("retained summary")]
        );
        assert_eq!(
            project_context_history(&accelerated).unwrap(),
            project_context_history(&full).unwrap()
        );
        let mut builder = AgentBuilder::new(AgentConfig::default());
        dim_agent_capabilities::defaults::mock_hosts(&mut builder);
        dim_agent_capabilities::defaults::install(&mut builder);
        builder.timeline_store(store.clone());
        let agent = builder.build();
        agent.restore_session("session".into()).await.unwrap();
        agent.close().await.unwrap();
        timeline.close().await.unwrap();
        store.close().await.unwrap();
        db.close().await.unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT body FROM timeline_events WHERE sequence=3",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            body
        );
    }
    for through in [1, 3, 4] {
        let mut value: serde_json::Value = serde_json::from_str(&body).unwrap();
        value["record"]["event"]["data"]["through"] = through.into();
        let bad = serde_json::to_string(&value).unwrap();
        connection
            .execute(
                "UPDATE timeline_events SET body = ?1 WHERE sequence = 3",
                [&bad],
            )
            .unwrap();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        assert!(matches!(
            store.branches("session").await,
            Err(AgentError::Storage(_))
        ));
        store.close().await.unwrap();
        db.close().await.unwrap();
        drop(store);
        drop(db);
        assert_eq!(
            connection
                .query_row(
                    "SELECT body FROM timeline_events WHERE sequence = 3",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            bad
        );
    }
    connection
        .execute(
            "UPDATE timeline_events SET body = ?1 WHERE sequence = 3",
            [&body],
        )
        .unwrap();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    store.branches("session").await.unwrap();
    assert_eq!(
        store.read(&key("session", "main"), 3..=3).await.unwrap()[0].event,
        original.event
    );
    connection
        .execute("DELETE FROM timeline_snapshots", [])
        .unwrap();
    // Inject malformed cache metadata through this disposable connection;
    // ordinary writes retain their foreign-key enforcement.
    connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    // An anchor of another kind is structurally valid (#130) and is left to
    // the owner's decode, so only identity mismatches are injected here.
    for (branch, sequence, event_id) in [("main", 3, "wrong"), ("orphan", 99, "missing")] {
        connection
            .execute(
                "INSERT INTO timeline_snapshots VALUES ('session',?1,?2,?3,?4,'not json')",
                params![branch, sequence, INTERPRETER, event_id],
            )
            .unwrap();
    }
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    assert!(
        store
            .latest_snapshot(&key("session", "main"), 99, INTERPRETER)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .latest_snapshot(&key("session", "orphan"), 99, INTERPRETER)
            .await
            .unwrap()
            .is_none()
    );
    store.branches("session").await.unwrap();
    store.rollback_session("session").await.unwrap();
    for table in [
        "timeline_snapshots",
        "timeline_events",
        "timeline_branches",
        "timeline_content",
        "agent_sessions",
    ] {
        assert_eq!(
            connection
                .query_row(
                    &format!("SELECT count(*) FROM {table} WHERE session_id='session'"),
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn nested_branches_read_fixed_ancestor_checkpoints_after_their_parents_close() {
    use dim_agent_capabilities::history::{
        context_messages, project_context_history, projection_checkpoint,
    };
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    store
        .create_session("nested", &SessionConfig::default())
        .await
        .unwrap();
    for stored in [
        record(
            1,
            EventKind::InputMessage(InputMessageData::text(
                "earlier input",
                Some(InputMode::Queue),
            )),
        ),
        record(
            2,
            EventKind::InputConsumed(InputConsumedData {
                input_id: "event-1".into(),
            }),
        ),
        record(
            3,
            EventKind::InputMessage(InputMessageData::text(
                "queued across compaction",
                Some(InputMode::Queue),
            )),
        ),
    ] {
        append(&store, key("nested", "main"), vec![stored])
            .await
            .unwrap();
    }
    let prefix = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("nested", "main"),
        vec![],
        store.clone(),
    )
    .await
    .unwrap();
    // The cut replaces the earlier input; the queued one is not placed yet.
    let checkpoint = projection_checkpoint(&prefix.context(3).await.unwrap(), "event-3").unwrap();
    append(
        &store,
        key("nested", "main"),
        vec![record(
            4,
            EventKind::ContextCompaction(completed("event-3", "earlier summary", 3)),
        )],
    )
    .await
    .unwrap();
    store
        .save_snapshot(&key("nested", "main"), snapshot(4, &checkpoint))
        .await
        .unwrap();
    for (sequence, branch, cut) in [(5, "early", 2), (6, "child", 4)] {
        append(
            &store,
            key("nested", "main"),
            vec![record(
                sequence,
                EventKind::BranchCreate(BranchCreateData {
                    at: BranchCreateAt::Sequence { sequence: cut },
                    source: BranchSource::Host {
                        operation_id: branch.into(),
                    },
                    branch: branch.into(),
                    prompt: String::new(),
                }),
            )],
        )
        .await
        .unwrap();
    }
    let main = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("nested", "main"),
        vec![],
        store.clone(),
    )
    .await
    .unwrap();
    main.close().await.unwrap();
    let early = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("nested", "early"),
        vec![("main".into(), 2)],
        store.clone(),
    )
    .await
    .unwrap();
    let early_context = read_context(&early, 0).await.unwrap();
    assert!(early_context.checkpoint.is_none());
    assert_eq!(early_context.events.len(), 2);
    assert_eq!(
        project_context_history(&early_context).unwrap()[0].message,
        Message::text(Role::User, "earlier input")
    );
    let child = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("nested", "child"),
        vec![("main".into(), 4)],
        store.clone(),
    )
    .await
    .unwrap();
    child
        .append(
            EventKind::BranchStart(BranchStartData {
                parent: Some("main".into()),
                create_event: Some(6),
            })
            .into(),
        )
        .unwrap();
    child
        .append(
            EventKind::InputConsumed(InputConsumedData {
                input_id: "event-3".into(),
            })
            .into(),
        )
        .unwrap();
    child
        .append(
            EventKind::BranchCreate(BranchCreateData {
                at: BranchCreateAt::Sequence { sequence: 2 },
                source: BranchSource::Host {
                    operation_id: "grandchild".into(),
                },
                branch: "grandchild".into(),
                prompt: String::new(),
            })
            .into(),
        )
        .unwrap();
    child.close().await.unwrap();
    let grandchild = Timeline::restore(
        rutis::Ctx::root().unwrap(),
        key("nested", "grandchild"),
        vec![("main".into(), 4), ("child".into(), 2)],
        store.clone(),
    )
    .await
    .unwrap();
    let context = read_context(&grandchild, 0).await.unwrap();
    assert!(context.checkpoint.is_some());
    assert_eq!(
        context
            .positions
            .iter()
            .map(|position| (position.branch_id.as_str(), position.sequence))
            .collect::<Vec<_>>(),
        [("main", 3), ("main", 4), ("child", 1), ("child", 2)]
    );
    assert_eq!(
        context_messages(&context, &project_context_history(&context).unwrap()),
        [
            dim_agent_capabilities::history::summary_message("earlier summary"),
            Message::text(Role::User, "queued across compaction")
        ]
    );
    let history = grandchild.history(None).await.unwrap();
    assert_eq!(history.len(), 6);
    assert_eq!(history[..4], main.local(..=4).await.unwrap());
    assert_eq!(history[4..], child.local(..=2).await.unwrap());
    assert_eq!(store.branches("nested").await.unwrap().len(), 4);
    early.close().await.unwrap();
    grandchild.close().await.unwrap();
    prefix.close().await.unwrap();
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn failed_branch_cuts_remain_readable_as_original_durable_facts() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    let mut originals = vec![];
    for bad_cut in [1, 90] {
        let session = format!("failed-cut-{bad_cut}");
        store
            .create_session(&session, &SessionConfig::default())
            .await
            .unwrap();
        let ctx = rutis::Ctx::root().unwrap();
        let timeline = Timeline::new(ctx.clone(), key(&session, "main"), vec![], store.clone());
        timeline
            .append_batch_registered(
                vec![
                    EventKind::StopInterrupt(StopInterruptData {
                        reason: "filler".into(),
                        scope: InterruptScope::Turn,
                    })
                    .into(),
                    EventKind::StopInterrupt(StopInterruptData {
                        reason: "filler".into(),
                        scope: InterruptScope::Turn,
                    })
                    .into(),
                ],
                |_| Ok(()),
            )
            .unwrap();
        let before = timeline
            .before(&ctx, "invalid-cut", &["branch/create"], move |mut draft| {
                let EventKind::BranchCreate(data) = &mut draft.kind else {
                    panic!()
                };
                data.at = BranchCreateAt::Sequence { sequence: bad_cut };
                Ok(draft)
            })
            .unwrap();
        let created = timeline
            .append(
                EventKind::BranchCreate(BranchCreateData {
                    at: BranchCreateAt::Sequence { sequence: 2 },
                    source: BranchSource::Host {
                        operation_id: "cut-check".into(),
                    },
                    branch: "failed-child".into(),
                    prompt: "candidate".into(),
                })
                .into(),
            )
            .unwrap();
        let failed = timeline
            .append(
                EventKind::BranchCreateFailed(BranchCreateFailedData::Failed {
                    branch: "failed-child".into(),
                    create_event: created.sequence,
                    error: "invalid branch cut".into(),
                })
                .into(),
            )
            .unwrap();
        timeline.close().await.unwrap();
        originals.push((session, created, failed));
        before.dispose().await.unwrap();
        drop(timeline);
    }
    store.close().await.unwrap();
    db.close().await.unwrap();
    drop(store);
    drop(db);
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    for ((session, original, failed), bad_cut) in originals.iter().zip([1, 90]) {
        let branches = store.branches(session).await.unwrap();
        let child = branches
            .iter()
            .find(|branch| branch.info.id == "failed-child")
            .unwrap();
        assert_eq!(child.info.branch_point, Some(bad_cut));
        assert_eq!(child.sequence, 0);
        let events = store.read(&key(session, "main"), 3..=4).await.unwrap();
        assert_eq!(events[0].event, *original);
        assert_eq!(events[1].event, *failed);
        let trace = store
            .trace(&key(session, "main"), 3)
            .await
            .unwrap()
            .unwrap();
        let EventKind::BranchCreate(original) = trace.original.kind else {
            panic!()
        };
        assert_eq!(original.at, BranchCreateAt::Sequence { sequence: 2 });
    }
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cold_read_failure_preserves_the_record_and_other_session_progress() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let database = Database::new();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        let mut builder = dim_agent_runtime::AgentBuilder::new(dim_agent_runtime::AgentConfig::default());
        dim_agent_capabilities::defaults::mock_hosts(&mut builder);
        dim_agent_capabilities::defaults::install(&mut builder);
        builder.timeline_store(store.clone());
        let agent = builder.build();
        let bad = agent.create_session(SessionConfig::default()).await.unwrap();
        let good = agent.create_session(SessionConfig::default()).await.unwrap();
        bad.submit("stored request".into(), InputMode::Queue, "main").await.unwrap();
        bad.idle("main").await.unwrap();
        let connection = Connection::open(&database.0).unwrap();
        assert!(connection.execute("UPDATE timeline_content SET body = 'unreadable external bytes' WHERE session_id = ?1", [bad.id()]).unwrap() > 0);
        assert!(matches!(bad.history("main", None).await, Err(AgentError::Storage(_))));
        assert!(bad.failure().is_none(), "a read error belongs to its operation");
        good.submit("healthy progress".into(), InputMode::Queue, "main").await.unwrap();
        good.idle("main").await.unwrap();
        assert!(good.history("main", None).await.unwrap().iter().any(|event| matches!(&event.kind, EventKind::Text(data) if data.delta.contains("healthy progress"))));
        assert_eq!(connection.query_row("SELECT body FROM timeline_content WHERE session_id = ?1 LIMIT 1", [bad.id()], |row| row.get::<_, String>(0)).unwrap(), "unreadable external bytes");
        agent.close().await.unwrap();
        assert!(matches!(store.branches(bad.id()).await, Err(AgentError::Storage(_))));
        store.branches(good.id()).await.unwrap();
        drop(connection);
        store.close().await.unwrap();
        db.close().await.unwrap();
    }).await.expect("the read failure remained local to the operation");
}

#[tokio::test]
async fn directory_history_pages_keep_a_fixed_head_and_resolve_prior_tool_calls() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let sessions = dim_app_server::storage::SessionStore::new(db.clone());
    let store = sessions.timeline_store();
    store
        .create_session("paged", &SessionConfig::default())
        .await
        .unwrap();
    let mut records = vec![record(
        1,
        EventKind::ToolCall(ToolCallData {
            turn_id: "turn".into(),
            request_id: "request".into(),
            calls: vec![ToolCall {
                id: "call".into(),
                name: "echo".into(),
                input: serde_json::Map::new(),
            }],
        }),
    )];
    records.extend((2..=600).map(|sequence| text(sequence, "delta")));
    let (done, wait) = tokio::sync::oneshot::channel();
    store
        .append(
            key("paged", "main"),
            records,
            Box::new(move |result| {
                let _ = done.send(result);
            }),
        )
        .unwrap();
    wait.await.unwrap().unwrap();
    let through = store.head(&key("paged", "main")).await.unwrap();
    let first = sessions.read_page("paged", 0, 200, through).await.unwrap();
    assert_eq!(first.len(), 200);
    let (done, wait) = tokio::sync::oneshot::channel();
    store
        .append(
            key("paged", "main"),
            vec![text(601, "later")],
            Box::new(move |result| {
                let _ = done.send(result);
            }),
        )
        .unwrap();
    wait.await.unwrap().unwrap();
    let second = sessions
        .read_page("paged", 200, 200, through)
        .await
        .unwrap();
    let third = sessions
        .read_page("paged", 400, 200, through)
        .await
        .unwrap();
    let sequences: Vec<_> = first
        .iter()
        .chain(&second)
        .chain(&third)
        .map(|event| event.sequence)
        .collect();
    assert_eq!(sequences, (1..=600).collect::<Vec<_>>());
    assert!(
        sessions
            .read_page("paged", 600, 200, through)
            .await
            .unwrap()
            .is_empty()
    );
    let call = sessions
        .read_event("paged", "event-1")
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(&call.kind, EventKind::ToolCall(data) if data.calls[0].name == "echo"));
    let connection = Connection::open(&database.0).unwrap();
    let plan: String = connection.query_row(
        "EXPLAIN QUERY PLAN SELECT sequence FROM timeline_events WHERE session_id='paged' AND branch_id='main' AND last_sequence>=201 AND sequence<=400 ORDER BY last_sequence",
        [], |row| row.get(3)).unwrap();
    assert!(plan.contains("timeline_events_by_end"), "{plan}");
    drop(connection);
    drop(store);
    drop(sessions);
    db.close().await.unwrap();
}

/// A call's `step/tool-use-start` is found by request and call id through its
/// index, however far it is from the result closing the call.
#[tokio::test]
async fn tool_use_starts_are_found_by_request_and_call_id() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let sessions = dim_app_server::storage::SessionStore::new(db.clone());
    let store = sessions.timeline_store();
    store
        .create_session("starts", &SessionConfig::default())
        .await
        .unwrap();
    let start = |sequence, request_id: &str, call_id: &str, name: &str| {
        record(
            sequence,
            EventKind::ToolUseStart(ToolUseStartData {
                turn_id: "turn".into(),
                request_id: request_id.into(),
                call_id: call_id.into(),
                name: name.into(),
            }),
        )
    };
    let mut records = vec![
        start(1, "request", "call", "echo"),
        start(2, "other", "call", "wait"),
    ];
    records.extend((3..=600).map(|sequence| text(sequence, "delta")));
    let (done, wait) = tokio::sync::oneshot::channel();
    store
        .append(
            key("starts", "main"),
            records,
            Box::new(move |result| {
                let _ = done.send(result);
            }),
        )
        .unwrap();
    wait.await.unwrap().unwrap();
    let name = async |request_id, call_id| {
        sessions
            .read_tool_use_start("starts", request_id, call_id)
            .await
            .unwrap()
            .map(|event| match &event.kind {
                EventKind::ToolUseStart(data) => data.name.clone(),
                kind => panic!("unexpected {}", kind.key()),
            })
    };
    assert_eq!(name("request", "call").await.as_deref(), Some("echo"));
    assert_eq!(name("other", "call").await.as_deref(), Some("wait"));
    assert_eq!(name("request", "missing").await, None);
    let connection = Connection::open(&database.0).unwrap();
    let plan: String = connection.query_row(
        "EXPLAIN QUERY PLAN SELECT sequence FROM timeline_events WHERE session_id='starts' AND branch_id='main' AND kind='step/tool-use-start' AND json_extract(body, '$.record.event.data.request_id')='request' AND json_extract(body, '$.record.event.data.call_id')='call'",
        [], |row| row.get(3)).unwrap();
    assert!(plan.contains("timeline_tool_use_starts"), "{plan}");
    drop(connection);
    drop(store);
    drop(sessions);
    db.close().await.unwrap();
}

#[tokio::test]
async fn history_pages_preserve_batches_in_memory_and_sqlite() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let sqlite = SqliteTimelineStore::new(db.clone());
    let stores: Vec<Arc<dyn TimelineStore>> = vec![
        Arc::new(dim_agent_runtime::MemoryTimelineStore::default()),
        sqlite.clone(),
    ];
    for store in stores {
        let key = key("batches", "main");
        store
            .create_session("batches", &SessionConfig::default())
            .await
            .unwrap();
        let records = (1..=303)
            .map(|sequence| {
                let mut event = if sequence == 1 {
                    record(
                        sequence,
                        EventKind::InputMessage(InputMessageData::text(
                            "first batch",
                            Some(InputMode::Queue),
                        )),
                    )
                } else {
                    text(sequence, "delta")
                };
                event.batch_end = if sequence <= 300 { 300 } else { 303 };
                event
            })
            .collect();
        let (done, wait) = tokio::sync::oneshot::channel();
        store
            .append(
                key.clone(),
                records,
                Box::new(move |result| {
                    let _ = done.send(result);
                }),
            )
            .unwrap();
        wait.await.unwrap().unwrap();
        let first = store.read_page(&key, 0, 2, 303).await.unwrap();
        assert_eq!(
            first.len(),
            300,
            "one batch may cross rows and internal read segments"
        );
        let second = store.read_page(&key, 300, 1, 303).await.unwrap();
        assert_eq!(
            second.iter().map(|r| r.event.sequence).collect::<Vec<_>>(),
            vec![301, 302, 303]
        );
        assert!(matches!(
            store.read_page(&key, 1, 2, 303).await,
            Err(AgentError::InvalidInput(_))
        ));
        assert!(matches!(
            store.read_page(&key, 0, 2, 299).await,
            Err(AgentError::InvalidInput(_))
        ));
        assert!(store.read_page(&key, 0, 0, 303).await.unwrap().is_empty());
        assert!(store.read_page(&key, 303, 2, 303).await.unwrap().is_empty());
    }
    sqlite.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn creation_rollback_drains_accepted_writes_and_deletes_atomically() {
    let database = Database::new();
    let db = SqliteStorage::open(database.0.clone()).unwrap();
    let store = SqliteTimelineStore::new(db.clone());
    store
        .create_session("new", &SessionConfig::default())
        .await
        .unwrap();
    let connection = Connection::open(&database.0).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_rollback BEFORE DELETE ON timeline_branches BEGIN SELECT RAISE(ABORT, 'rollback failure'); END").unwrap();
    let (done, wait) = tokio::sync::oneshot::channel();
    store
        .append(
            key("new", "main"),
            vec![text(1, "pending before rollback")],
            Box::new(move |result| {
                let _ = done.send(result);
            }),
        )
        .unwrap();
    let error = store.rollback_session("new").await.unwrap_err();
    assert!(error.to_string().contains("rollback failure"));
    wait.await.unwrap().unwrap();
    assert!(store.session_config("new").await.unwrap().is_some());
    assert_eq!(
        store.read(&key("new", "main"), 1..=1).await.unwrap().len(),
        1,
        "failed deletion must roll back the preceding event deletion"
    );
    connection
        .execute_batch("DROP TRIGGER fail_rollback")
        .unwrap();
    store.rollback_session("new").await.unwrap();
    for table in [
        "agent_sessions",
        "timeline_branches",
        "timeline_events",
        "timeline_content",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
    assert!(store.session_config("new").await.unwrap().is_none());
    drop(connection);
    store.close().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
async fn native_continuation_survives_sqlite_checkpoint_request_and_child_restore() {
    use dim_agent_capabilities::history::{project_context_history, projection_checkpoint};
    use serde_json::json;
    for metadata in [
        json!({"protocol":"anthropic","signature":"signature-only"}),
        json!({"protocol":"gemini","thoughtSignature":"thought-signature"}),
        json!({"protocol":"openai-responses","itemId":"rs-1","encryptedContent":"encrypted-final","summaryIndex":0}),
    ] {
        let content: Vec<MessagePart> = serde_json::from_value(json!([
            {"type":"thinking","thinking":"","providerMetadata":metadata},
            {"type":"text","text":"before"},
            {"type":"text","text":"after"},
        ]))
        .unwrap();
        let database = Database::new();
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        store
            .create_session("native", &SessionConfig::default())
            .await
            .unwrap();
        append(
            &store,
            key("native", "main"),
            vec![
                record(
                    1,
                    EventKind::InputMessage(InputMessageData::text(
                        "input",
                        Some(InputMode::Queue),
                    )),
                ),
                record(
                    2,
                    EventKind::InputConsumed(InputConsumedData {
                        input_id: "event-1".into(),
                    }),
                ),
                record(
                    3,
                    EventKind::ModelStart(ModelStartData {
                        turn_id: "turn".into(),
                        request_id: "request".into(),
                    }),
                ),
                text(4, "beforeafter"),
                record(
                    5,
                    EventKind::ModelContent(ModelContentData {
                        turn_id: "turn".into(),
                        request_id: "request".into(),
                        content: content.clone(),
                    }),
                ),
                record(
                    6,
                    EventKind::ModelOutputEnd(ModelOutputEndData {
                        turn_id: "turn".into(),
                        request_id: "request".into(),
                        outcome: ModelOutputOutcome::Completed {
                            finish_reason: FinishReason::EndTurn,
                        },
                    }),
                ),
            ],
        )
        .await
        .unwrap();
        let timeline = Timeline::restore(
            rutis::Ctx::root().unwrap(),
            key("native", "main"),
            vec![],
            store.clone(),
        )
        .await
        .unwrap();
        let checkpoint =
            projection_checkpoint(&timeline.context(6).await.unwrap(), "event-3").unwrap();
        assert_eq!(checkpoint.state.output, content);
        let compaction = completed("event-3", "summary", 6);
        append(
            &store,
            key("native", "main"),
            vec![record(7, EventKind::ContextCompaction(compaction))],
        )
        .await
        .unwrap();
        store
            .save_snapshot(&key("native", "main"), snapshot(7, &checkpoint))
            .await
            .unwrap();
        let assistant = Message {
            role: Role::Assistant,
            content: MessageContent::Parts(content.clone()),
        };
        append(
            &store,
            key("native", "main"),
            vec![
                record(
                    8,
                    EventKind::ModelCall(ModelCallData {
                        turn_id: "next".into(),
                        request_id: "next".into(),
                        request: Arc::new(ModelRequest::new(
                            "native",
                            "model",
                            vec![assistant.clone()],
                        )),
                        configuration: None,
                        prompt: None,
                    }),
                ),
                record(
                    9,
                    EventKind::BranchCreate(BranchCreateData {
                        at: BranchCreateAt::Sequence { sequence: 8 },
                        source: BranchSource::Host {
                            operation_id: "child".into(),
                        },
                        branch: "child".into(),
                        prompt: String::new(),
                    }),
                ),
            ],
        )
        .await
        .unwrap();
        timeline.close().await.unwrap();
        store.close().await.unwrap();
        db.close().await.unwrap();
        drop(timeline);
        drop(store);
        drop(db);
        let db = SqliteStorage::open(database.0.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        for (branch, ancestors, through) in
            [("main", vec![], 9), ("child", vec![("main".into(), 8)], 0)]
        {
            let timeline = Timeline::restore(
                rutis::Ctx::root().unwrap(),
                key("native", branch),
                ancestors,
                store.clone(),
            )
            .await
            .unwrap();
            let history =
                project_context_history(&read_context(&timeline, through).await.unwrap()).unwrap();
            assert_eq!(
                history
                    .iter()
                    .map(|entry| &entry.message)
                    .collect::<Vec<_>>(),
                vec![&assistant]
            );
            let events = timeline.history(None).await.unwrap();
            assert!(events.iter().any(|e|matches!(&e.kind,EventKind::ModelCall(call) if call.request.messages[0].as_ref()==&assistant)));
            timeline.close().await.unwrap();
        }
        let saved = store
            .latest_snapshot(&key("native", "main"), 9, INTERPRETER)
            .await
            .unwrap()
            .unwrap();
        let timeline = Timeline::restore(
            rutis::Ctx::root().unwrap(),
            key("native", "main"),
            vec![],
            store.clone(),
        )
        .await
        .unwrap();
        let anchor = timeline
            .local(saved.sequence..=saved.sequence)
            .await
            .unwrap()
            .pop()
            .unwrap();
        timeline.close().await.unwrap();
        let checkpoint = (HistoryRenderer::default().decode_checkpoint)(&saved, &anchor).unwrap();
        assert_eq!(checkpoint.state.output, content);
        store.close().await.unwrap();
        db.close().await.unwrap();
    }
}
