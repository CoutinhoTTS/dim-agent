#![cfg(unix)]
use dim_agent_capabilities::defaults;
use dim_agent_runtime::{AgentBuilder, AgentConfig};
use dim_agent_types::EventKind;
use dim_app_server::{ServerConfig, serve};
use dim_client::{Client, ConnectionConfig, Server};
use dim_migration::{Migration, MigrationPlugin, Outcome, TaskStatus};
use rusqlite::{Connection, params};
use rutis::host::CtxHost;
use serde_json::{Value, json};
use std::path::Path;

fn fixture(home: &Path) -> Connection {
    std::fs::create_dir_all(home).unwrap();
    let db = Connection::open(home.join("dimcode.sqlite")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
        CREATE TABLE sessions(sessionId TEXT PRIMARY KEY,cwd TEXT,title TEXT,status TEXT,tags TEXT,createdAt TEXT,updatedAt TEXT,version INTEGER);
        CREATE TABLE session_states(sessionId TEXT PRIMARY KEY,selectedProviderId TEXT,selectedModelId TEXT,settings TEXT,permissionSettings TEXT,currentMode TEXT);
        CREATE TABLE messages(messageId TEXT PRIMARY KEY,sessionId TEXT,role TEXT,parts TEXT,attachments TEXT,metadata TEXT,orderKey TEXT,createdAt TEXT);
        CREATE TABLE compaction_states(sessionId TEXT PRIMARY KEY,cursor TEXT,compactionSegments TEXT);
        CREATE TABLE session_blobs(blobId TEXT PRIMARY KEY,sessionId TEXT,kind TEXT,ref TEXT,mimeType TEXT,sizeBytes INTEGER,checksum TEXT);
        CREATE TABLE blob_data(blobId TEXT PRIMARY KEY,data BLOB);").unwrap();
    db.execute_batch("CREATE TABLE background_tasks(taskId TEXT PRIMARY KEY,sessionId TEXT,outputPath TEXT,status TEXT,completion TEXT);
        CREATE TABLE workflow_runs(workflowRunId TEXT PRIMARY KEY,sessionId TEXT,record TEXT);
        CREATE TABLE queue_items(queueItemId TEXT PRIMARY KEY,sessionId TEXT,payload TEXT,status TEXT);
        CREATE TABLE permission_decisions(decisionId TEXT PRIMARY KEY,sessionId TEXT,decision TEXT,remember INTEGER);
        CREATE TABLE usage_ledger(ledgerId TEXT PRIMARY KEY,sessionId TEXT,usage TEXT);").unwrap();
    std::fs::create_dir_all(home.join("data/jobs")).unwrap();
    std::fs::write(
        home.join("data/jobs/old-output.txt"),
        b"historical output bytes",
    )
    .unwrap();
    db.execute("INSERT INTO background_tasks VALUES ('old-task','old','data/jobs/old-output.txt','running','{\"result\":\"old result\"}')",[]).unwrap();
    db.execute("INSERT INTO queue_items VALUES ('old-queue','old','{\"text\":\"never automatically submit this queued input\"}','pending')",[]).unwrap();
    db.execute(
        "INSERT INTO permission_decisions VALUES ('one-shot','old','allow',0)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO usage_ledger VALUES ('old-usage','old','{\"inputTokens\":123}')",
        [],
    )
    .unwrap();
    let workflow = home.join("data/workflows/sessions/old/old-workflow");
    std::fs::create_dir_all(&workflow).unwrap();
    std::fs::write(
        workflow.join("workflow.ts"),
        b"throw new Error('never execute imported Workflow code')",
    )
    .unwrap();
    std::fs::write(
        workflow.join("meta.json"),
        b"{\"workflowRunId\":\"old-workflow\"}",
    )
    .unwrap();
    db.execute("INSERT INTO workflow_runs VALUES ('old-workflow','old','{\"source\":{\"kind\":\"inline\"},\"status\":\"running\",\"result\":\"historical workflow result\"}')",[]).unwrap();
    for (id, status) in [
        ("old", "active"),
        ("archived", "archived"),
        ("broken", "active"),
    ] {
        db.execute(
            "INSERT INTO sessions VALUES (?1,?2,'Old title',?3,'[]','2026-01-01','2026-01-01',1)",
            params![id, home.canonicalize().unwrap().to_str().unwrap(), status],
        )
        .unwrap();
        db.execute(
            "INSERT INTO session_states VALUES (?1,'mock','deterministic','{}','{}','agent')",
            [id],
        )
        .unwrap();
    }
    db.execute(
        "UPDATE sessions SET tags='[\"not-yet-supported\"]' WHERE sessionId='old'",
        [],
    )
    .unwrap();
    db.execute("UPDATE session_states SET settings='{\"__privateUnsupported__\":true}', currentMode='goal' WHERE sessionId='old'", []).unwrap();
    for (id, session, role, text, order) in [
        ("first", "old", "user", "compressed original", "a"),
        ("tail", "old", "assistant", "effective suffix", "b"),
        ("bad", "broken", "unknown", "bad role", "a"),
    ] {
        db.execute(
            "INSERT INTO messages VALUES (?1,?2,?3,?4,'[]','{}',?5,'2026-01-01T00:00:00Z')",
            params![
                id,
                session,
                role,
                json!([{"type":"text","text":text}]).to_string(),
                order
            ],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO compaction_states VALUES ('old','first',?1)",
        [
            json!([{"endMessageId":"first","summary":"retained summary","rawItems":[]}])
                .to_string(),
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO session_blobs VALUES ('attachment','old','text','unused','text/plain',7,NULL)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO blob_data VALUES ('attachment',?1)",
        [b"payload".as_slice()],
    )
    .unwrap();
    std::fs::write(
        home.join("config.json"),
        json!({"locale":"ja","tuiNotifyOnComplete":false,"themeId":"system","subagentModel":{"providerId":"mock","modelId":"deterministic","reasoningEffort":"high"}}).to_string(),
    )
    .unwrap();
    std::fs::write(
        home.join("desktop-config.json"),
        json!({"version":7,"settings":{"locale":"zh-CN"}}).to_string(),
    )
    .unwrap();
    db
}

fn snapshot(home: &Path) -> Vec<(String, Vec<u8>)> {
    [
        "dimcode.sqlite",
        "dimcode.sqlite-wal",
        "dimcode.sqlite-shm",
        "schedule-tui.sqlite",
        "schedule-tui.sqlite-wal",
        "schedule-tui.sqlite-shm",
        "config.json",
        "desktop-config.json",
        "auth.json",
        "mcp-oauth.json",
    ]
    .into_iter()
    .filter(|name| home.join(name).exists())
    .map(|name| (name.into(), std::fs::read(home.join(name)).unwrap()))
    .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugin_imports_wal_history_files_and_settings_then_reconciles_without_duplicates() {
    let directory = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut source = directory.path().join("source");
    let _db = fixture(&source);
    _db.execute_batch("CREATE TABLE scheduled_jobs(incompatible TEXT); INSERT INTO scheduled_jobs VALUES ('bad schema');").unwrap();
    let terminal = Connection::open(source.join("schedule-tui.sqlite")).unwrap();
    terminal.execute_batch("CREATE TABLE scheduled_jobs(jobId TEXT PRIMARY KEY,name TEXT,prompt TEXT,status TEXT,rrule TEXT,cwd TEXT,providerId TEXT,modelId TEXT,reasoningEffort TEXT,sessionMode TEXT,boundSessionId TEXT,nextRunAt TEXT,lastRunAt TEXT,createdAt TEXT);").unwrap();
    terminal.execute("INSERT INTO scheduled_jobs VALUES ('independent','Independent CLI job','Prompt','paused','FREQ=DAILY;BYHOUR=9;BYMINUTE=0',?1,'mock','deterministic',NULL,'new',NULL,NULL,NULL,'2026-01-01T00:00:00Z')", [source.canonicalize().unwrap().to_str().unwrap()]).unwrap();
    drop(terminal);
    std::fs::write(source.join("auth.json"), b"[]").unwrap();
    std::fs::write(
        source.join("mcp-oauth.json"),
        br#"{"version":1,"servers":{"independent":{}}}"#,
    )
    .unwrap();
    let before = snapshot(&source);
    let target = directory.path().join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(
        target.join("config.json"),
        json!({"locale":"en","tuiToolDetailsExpanded":false}).to_string(),
    )
    .unwrap();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::install(&mut builder);
    defaults::mock_hosts(&mut builder);
    let server = serve(
        builder,
        ServerConfig {
            database_path: Some(target.join("store.sqlite")),
            product_home: Some(target.clone()),
            unix_path: Some(directory.path().join("server.sock")),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let agent = server.agent().unwrap();
    let connection = ConnectionConfig {
        endpoint: server.unix_path.clone().unwrap(),
        server: Server {
            program: std::env::current_exe().unwrap(),
            embedded: false,
        },
        profile: None,
        client_kind: "cli",
        client_name: "migration-test",
    };
    let root = rutis::Ctx::root().unwrap();
    let plugin = root.plugin(MigrationPlugin::new(target.clone(), vec!["system".into()]));
    (&plugin).await.unwrap();
    let migration = root.get::<Migration>().unwrap();
    assert!(migration.inspect(source.clone(), false).await.is_err());
    assert_eq!(
        migration
            .inspect(source.clone(), true)
            .await
            .unwrap()
            .tables["sessions"],
        3
    );
    let report = migration
        .run(source.clone(), true, None, connection.clone())
        .await
        .unwrap();
    assert_eq!(report.task.status, TaskStatus::Partial, "{report:?}");
    assert!(
        report
            .objects
            .iter()
            .any(|item| item.id == "config.subagentModel.reasoningEffort"
                && item.outcome == Outcome::Unsupported)
    );
    assert!(
        report
            .objects
            .iter()
            .any(|item| item.id == "auth.json" && item.outcome == Outcome::Failed)
    );
    assert!(report.objects.iter().any(|item| item.kind == "account"
        && item.id == "mcp:independent"
        && item.outcome == Outcome::Unsupported));
    assert!(
        report
            .objects
            .iter()
            .any(|item| item.kind == "scheduled_jobs"
                && item.id == "legacy-desktop"
                && item.outcome == Outcome::Failed)
    );
    assert!(
        report
            .objects
            .iter()
            .any(|item| item.kind == "scheduled_job"
                && item.id == "legacy-cli:independent"
                && item.outcome == Outcome::Unsupported)
    );
    assert!(
        report
            .objects
            .iter()
            .any(|item| item.id == "archived" && item.outcome == Outcome::Deferred)
    );
    assert!(
        report
            .objects
            .iter()
            .any(|item| item.id == "broken" && item.outcome == Outcome::Failed)
    );
    let session = report
        .objects
        .iter()
        .find(|item| item.id == "old" && item.kind == "session")
        .unwrap();
    assert_eq!(session.outcome, Outcome::Imported, "{session:?}");
    assert!(session.omitted.contains(&"tags".into()));
    assert!(
        session
            .omitted
            .contains(&"session.settings.__privateUnsupported__".into())
    );
    let id = session.target_id.clone().unwrap();
    let config: Value =
        serde_json::from_slice(&std::fs::read(target.join("config.json")).unwrap()).unwrap();
    assert_eq!(
        config,
        json!({"locale":"en","tuiNotifyOnComplete":false,"tuiToolDetailsExpanded":false,"themeId":"system"})
    );
    assert_eq!(snapshot(&source), before);
    let mut client = Client::connect(&connection).await.unwrap();
    let listed: Value = client.typed("session.list", json!({})).await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    client
        .rpc("session.attach", json!({"sessionId":id}))
        .await
        .unwrap();
    let session = agent.get_session(&id).unwrap();
    let imported_history = session.history("main", None).await.unwrap();
    let metadata_path = imported_history
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::InputMessage(input) => Some(&input.blocks),
            _ => None,
        })
        .flatten()
        .filter_map(|block| match block {
            dim_agent_types::InputBlock::Messages { messages, .. } => Some(messages),
            _ => None,
        })
        .flatten()
        .find_map(|record| match &record.message.content {
            dim_agent_types::MessageContent::Text(text)
                if text.starts_with("Migration import record:") =>
            {
                assert!(!record.in_context);
                Some(
                    text.split("readable at ")
                        .nth(1)
                        .unwrap()
                        .split(". These records")
                        .next()
                        .unwrap()
                        .to_owned(),
                )
            }
            _ => None,
        })
        .unwrap();
    let archive: Value = serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
    assert_eq!(archive["source"], "legacy-session");
    assert!(!archive.to_string().contains("not-yet-supported"));
    assert!(!archive.to_string().contains("__privateUnsupported__"));
    assert_eq!(archive["records"].as_array().unwrap().len(), 5);
    assert!(archive["records"].as_array().unwrap().iter().any(|record|record["table"]=="queue_items" && record["data"]["status"]=="pending"));
    assert_eq!(archive["artifacts"].as_array().unwrap().len(), 3);
    let output = archive["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["source"] == "data/jobs/old-output.txt")
        .unwrap();
    assert_eq!(
        std::fs::read(output["path"].as_str().unwrap()).unwrap(),
        b"historical output bytes"
    );
    assert_eq!(
        report
            .objects
            .iter()
            .filter(|item| item.kind == "historical_record" && item.outcome == Outcome::Imported)
            .count(),
        5
    );
    assert!(
        session
            .history("main", None)
            .await
            .unwrap()
            .iter()
            .all(|event| !matches!(event.kind, EventKind::ModelCall(_) | EventKind::ToolCall(_)))
    );
    client
        .rpc(
            "session.submit",
            json!({"sessionId":id,"branchId":"main","text":"continue"}),
        )
        .await
        .unwrap();
    client
        .rpc("session.idle", json!({"sessionId":id,"branchId":"main"}))
        .await
        .unwrap();
    let history = session.history("main", None).await.unwrap();
    let request = history
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(call) => Some(&call.request),
            _ => None,
        })
        .unwrap();
    let context = serde_json::to_string(&request.messages).unwrap();
    assert!(
        context.contains("retained summary") && context.contains("effective suffix"),
        "{context}"
    );
    assert!(!context.contains("compressed original"));
    assert!(!context.contains("never automatically submit this queued input"));
    assert!(!context.contains("Migration import record:"));
    let retried = migration
        .run(
            source.clone(),
            true,
            Some(report.task.id),
            connection.clone(),
        )
        .await
        .unwrap();
    assert!(
        retried
            .objects
            .iter()
            .any(|item| item.id == "old" && item.outcome == Outcome::AlreadyImported)
    );
    let listed: Value = client.typed("session.list", json!({})).await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let moved = directory.path().join("moved-source");
    std::fs::rename(&source, &moved).unwrap();
    source = moved;
    let moved_report = migration
        .run(source.clone(), true, None, connection.clone())
        .await
        .unwrap();
    assert!(
        moved_report
            .objects
            .iter()
            .any(|item| item.id == "old" && item.outcome == Outcome::AlreadyImported),
        "{moved_report:?}"
    );
    client
        .rpc("session.delete", json!({"sessionId":id}))
        .await
        .unwrap();
    let owned = rusqlite::Connection::open(target.join("store.sqlite"))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM session_file_owners WHERE session_id=?1",
            [&id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(
        owned, 0,
        "historical data must follow Session deletion ownership"
    );
    let replay = migration
        .run(source.clone(), true, None, connection.clone())
        .await
        .unwrap();
    assert!(
        replay
            .objects
            .iter()
            .any(|item| item.id == "old" && item.outcome == Outcome::Deleted)
    );
    assert_eq!(snapshot(&source), before);
    plugin.dispose().await.unwrap();
    assert!(root.get::<Migration>().is_none());
    root.shutdown().await.unwrap();
    server.shutdown();
    server.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unloading_during_file_transfer_settles_cancelled_and_retry_resumes_hidden_session() {
    let directory = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = directory.path().join("source");
    let db = fixture(&source);
    let bytes = vec![42_u8; 3 * 1024 * 1024];
    db.execute(
        "UPDATE session_blobs SET sizeBytes=?1 WHERE blobId='attachment'",
        [bytes.len() as i64],
    )
    .unwrap();
    db.execute(
        "UPDATE blob_data SET data=?1 WHERE blobId='attachment'",
        [&bytes],
    )
    .unwrap();
    let before = snapshot(&source);
    let target = directory.path().join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("config.json"), b"{invalid").unwrap();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::install(&mut builder);
    defaults::mock_hosts(&mut builder);
    let server = serve(
        builder,
        ServerConfig {
            database_path: Some(target.join("store.sqlite")),
            product_home: Some(target.clone()),
            unix_path: Some(directory.path().join("server.sock")),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let connection = ConnectionConfig {
        endpoint: server.unix_path.clone().unwrap(),
        server: Server {
            program: std::env::current_exe().unwrap(),
            embedded: false,
        },
        profile: None,
        client_kind: "cli",
        client_name: "migration-test",
    };
    let root = rutis::Ctx::root().unwrap();
    let plugin = root.plugin(MigrationPlugin::new(target.clone(), vec!["system".into()]));
    (&plugin).await.unwrap();
    let migration = root.get::<Migration>().unwrap();
    let mut progress = migration.progress();
    let mut worker = {
        let owner = migration.clone();
        let source = source.clone();
        let connection = connection.clone();
        tokio::spawn(async move { owner.run(source, true, None, connection).await.unwrap() })
    };
    let interrupted = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            tokio::select! {
                changed = progress.changed() => changed.unwrap(),
                result = &mut worker => panic!("Migration stopped before file transfer: {result:?}"),
            }
            if let Some(state) = progress.borrow_and_update().clone()
                && state.stage == "file-bytes"
                && state.processed > 0
            {
                break state;
            }
        }
    })
    .await
    .unwrap();
    plugin.dispose().await.unwrap();
    let cancelled = worker.await.unwrap();
    assert_eq!(
        cancelled.task.status,
        TaskStatus::Cancelled,
        "{cancelled:?}"
    );
    assert_eq!(cancelled.task.id, interrupted.id);
    assert!(root.get::<Migration>().is_none());
    let mut client = Client::connect(&connection).await.unwrap();
    let listed: Value = client.typed("session.list", json!({})).await.unwrap();
    assert!(
        listed.as_array().unwrap().is_empty(),
        "A partly transferred Session must stay hidden"
    );
    let plugin = root.plugin(MigrationPlugin::new(target.clone(), vec!["system".into()]));
    (&plugin).await.unwrap();
    let migration = root.get::<Migration>().unwrap();
    assert_eq!(
        migration
            .report(&cancelled.task.id)
            .await
            .unwrap()
            .task
            .status,
        TaskStatus::Cancelled
    );
    let retried = migration
        .run(
            source.clone(),
            true,
            Some(cancelled.task.id.clone()),
            connection,
        )
        .await
        .unwrap();
    assert_eq!(
        retried.task.retry_of.as_deref(),
        Some(cancelled.task.id.as_str())
    );
    assert_ne!(retried.task.id, cancelled.task.id);
    assert!(
        retried.objects.iter().any(|result| result.kind == "session"
            && result.id == "old"
            && result.outcome == Outcome::Imported),
        "{retried:?}"
    );
    let listed: Value = client.typed("session.list", json!({})).await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(
        migration
            .report(&cancelled.task.id)
            .await
            .unwrap()
            .task
            .status,
        TaskStatus::Cancelled
    );
    assert_eq!(
        std::fs::read(target.join("config.json")).unwrap(),
        b"{invalid"
    );
    assert_eq!(snapshot(&source), before);
    plugin.dispose().await.unwrap();
    root.shutdown().await.unwrap();
    server.shutdown();
    server.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn native_web_and_skills_import_keep_disabled_state_secrets_and_source_id_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let source = directory.path().join("source");
    let _db = fixture(&source);
    std::fs::create_dir_all(source.join("data/plugins/web")).unwrap();
    let web = json!({"version":1,"enabled":false,"activeProvider":"brave","activeProviderExplicit":true,"providers":{"brave":{"apiKey":"legacy-brave-fixture","apiEndpoint":"https://api.search.brave.com/res/v1/web/search"},"tavily":{"apiKey":"inactive-fixture"}},"numResults":5});
    std::fs::write(source.join("data/plugins/web/state.json"), web.to_string()).unwrap();
    std::fs::write(source.join("config.json"), b"[]").unwrap();
    let skills = source.join("skills.json");
    std::fs::write(
        &skills,
        json!({"disabledSkillIds":["user:shared"]}).to_string(),
    )
    .unwrap();
    let target = directory.path().join("target");
    std::fs::create_dir_all(&target).unwrap();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let defaults_file = target.join("defaults.json");
    std::fs::write(&defaults_file, json!({"plugins":{"web":repository.join("runtime/plugins/web")},"locked":["PluginManager","Rpc","Transport"]}).to_string()).unwrap();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::install(&mut builder);
    defaults::mock_hosts(&mut builder);
    let server = serve(
        builder,
        ServerConfig {
            database_path: Some(target.join("store.sqlite")),
            product_home: Some(target.clone()),
            unix_path: Some(directory.path().join("server.sock")),
            plugin_defaults: Some(defaults_file),
            session_typescript: Some(
                dim_app_server::session_typescript::SessionTypeScriptConfig {
                    node: std::env::var_os("DIM_TEST_NODE").unwrap().into(),
                    js_runtime: std::env::var_os("DIM_TEST_JS_RUNTIME").unwrap().into(),
                    anchor: repository.join("runtime/package.json"),
                    python: None,
                    acquire: None,
                    rows: vec![],
                },
            ),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let connection = ConnectionConfig {
        endpoint: server.unix_path.clone().unwrap(),
        server: Server {
            program: std::env::current_exe().unwrap(),
            embedded: false,
        },
        profile: None,
        client_kind: "cli",
        client_name: "migration-test",
    };
    let mut client = Client::connect(&connection).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while client
            .rpc("settings.read", json!({"source":"web","scope":"user"}))
            .await
            .is_err()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let root = rutis::Ctx::root().unwrap();
    let plugin = root.plugin(MigrationPlugin::new(target.clone(), vec!["system".into()]));
    (&plugin).await.unwrap();
    let migration = root.get::<Migration>().unwrap();
    let report = migration
        .run(source.clone(), true, None, connection.clone())
        .await
        .unwrap();
    assert!(
        report
            .objects
            .iter()
            .any(|item| item.id == "web.enabled" && item.outcome == Outcome::Imported),
        "{report:?}"
    );
    assert!(
        report
            .objects
            .iter()
            .any(|item| item.id == "web.searchService" && item.outcome == Outcome::Imported),
        "{report:?}"
    );
    assert!(
        report
            .objects
            .iter()
            .any(|item| item.id == "skills.disabledSkillIds" && item.outcome == Outcome::Imported),
        "{report:?}"
    );
    let record: Value =
        serde_json::from_slice(&std::fs::read(target.join("plugins.json")).unwrap()).unwrap();
    assert_eq!(record["names"]["web"]["off"], true);
    let native = Connection::open_with_flags(
        target.join("store.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let read = |source: &str| {
        let document: String = native
            .query_row(
                "SELECT document FROM settings_documents WHERE source=?1",
                [source],
                |row| row.get(0),
            )
            .unwrap();
        serde_json::from_str::<Value>(&document).unwrap()
    };
    assert_eq!(
        read("user/web")["searchService"],
        json!({"kind":"brave","apiKey":"legacy-brave-fixture","endpoint":"https://api.search.brave.com/res/v1/web/search"})
    );
    assert_eq!(
        read("user/skills")["disabledSkillIds"],
        json!(["user:shared"])
    );
    let encoded = serde_json::to_string(&report).unwrap();
    assert!(!encoded.contains("legacy-brave-fixture") && !encoded.contains("inactive-fixture"));
    std::fs::write(
        &skills,
        json!({"disabledSkillIds":["host:legacy","user:ambiguous/nested"]}).to_string(),
    )
    .unwrap();
    let retry = migration
        .run(source, true, Some(report.task.id), connection)
        .await
        .unwrap();
    assert!(
        retry
            .objects
            .iter()
            .any(|item| item.id == "skills.disabledSkillIds" && item.outcome == Outcome::Deferred)
    );
    assert_eq!(
        read("user/skills")["disabledSkillIds"],
        json!(["user:shared"])
    );
    plugin.dispose().await.unwrap();
    root.shutdown().await.unwrap();
    server.shutdown();
    server.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_context_boundaries_and_changed_sources_reach_native_continuation() {
    let directory = tempfile::tempdir().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = directory.path().join("target");
    std::fs::create_dir_all(&target).unwrap();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::install(&mut builder);
    defaults::mock_hosts(&mut builder);
    let server = serve(
        builder,
        ServerConfig {
            database_path: Some(target.join("store.sqlite")),
            product_home: Some(target.clone()),
            unix_path: Some(directory.path().join("server.sock")),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let agent = server.agent().unwrap();
    let connection = ConnectionConfig {
        endpoint: server.unix_path.clone().unwrap(),
        server: Server {
            program: std::env::current_exe().unwrap(),
            embedded: false,
        },
        profile: None,
        client_kind: "cli",
        client_name: "migration-test",
    };
    let root = rutis::Ctx::root().unwrap();
    let plugin = root.plugin(MigrationPlugin::new(target, vec!["system".into()]));
    (&plugin).await.unwrap();
    let migration = root.get::<Migration>().unwrap();
    let mut client = Client::connect(&connection).await.unwrap();
    for case in [
        "revert",
        "remote",
        "tools",
        "retry",
        "image",
        "oversize-message",
        "oversize-file",
    ] {
        let source = directory.path().join(case);
        let db = fixture(&source);
        let source_id = format!("old-{case}");
        for table in [
            "sessions",
            "session_states",
            "messages",
            "compaction_states",
            "session_blobs",
            "background_tasks",
            "workflow_runs",
            "queue_items",
            "permission_decisions",
            "usage_ledger",
        ] {
            db.execute(
                &format!("UPDATE {table} SET sessionId=?1 WHERE sessionId='old'"),
                [&source_id],
            )
            .unwrap();
        }
        match case {
            "revert" => {
                db.execute(
                    "UPDATE session_states SET settings=?1 WHERE sessionId=?2",
                    params![
                        json!({"__revertState__":{"anchorOrderKey":"b"}}).to_string(),
                        source_id
                    ],
                )
                .unwrap();
            }
            "remote" => {
                db.execute("UPDATE compaction_states SET compactionSegments=?1 WHERE sessionId=?2", params![json!([{"endMessageId":"first","summary":"[remote-compacted]","rawItems":[{"type":"compaction","encrypted_content":"legacy-opaque"}]}]).to_string(),source_id]).unwrap();
            }
            "tools" => {
                db.execute("UPDATE messages SET role='assistant',parts=?1 WHERE messageId='first'", [json!([{"type":"tool_use","id":"historical-call","name":"legacy-never-execute","input":{"value":1}}]).to_string()]).unwrap();
                db.execute("UPDATE messages SET role='tool_result',parts=?1 WHERE messageId='tail'", [json!([{"type":"tool_result","tool_use_id":"historical-call","content":"historical output"}]).to_string()]).unwrap();
            }
            "retry" => {
                db.execute("UPDATE messages SET metadata=?1 WHERE messageId='tail'", [json!({"_dimTranscript":{"kind":"provider_retry","visibility":"visible","owner":"runtime"}}).to_string()]).unwrap();
            }
            "image" => {
                use base64::Engine;
                let image = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aFZkAAAAASUVORK5CYII=").unwrap();
                db.execute("UPDATE session_blobs SET mimeType='image/png',sizeBytes=?1 WHERE blobId='attachment'", [image.len() as i64]).unwrap();
                db.execute(
                    "UPDATE blob_data SET data=?1 WHERE blobId='attachment'",
                    [&image],
                )
                .unwrap();
                db.execute("UPDATE messages SET attachments=?1 WHERE messageId='first'", [json!([{"attachmentId":"legacy-image","kind":"blob","data":{"blobId":"attachment","mediaType":"image/png","source":"paste"}}]).to_string()]).unwrap();
                db.execute(
                    "DELETE FROM compaction_states WHERE sessionId=?1",
                    [&source_id],
                )
                .unwrap();
            }
            "oversize-message" => {
                db.execute(
                    "UPDATE messages SET parts=?1 WHERE messageId='first'",
                    [json!([{"type":"text","text":"x".repeat(1024*1024)}]).to_string()],
                )
                .unwrap();
            }
            "oversize-file" => {
                db.execute(
                    "UPDATE session_blobs SET sizeBytes=?1 WHERE blobId='attachment'",
                    [16 * 1024 * 1024 + 1],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let before = snapshot(&source);
        let report = migration
            .run(source.clone(), true, None, connection.clone())
            .await
            .unwrap();
        let imported = report
            .objects
            .iter()
            .find(|item| item.kind == "session" && item.id == source_id)
            .unwrap();
        if case.starts_with("oversize-") {
            assert_eq!(imported.outcome, Outcome::Failed, "{report:?}");
            assert!(imported.target_id.is_none());
            assert_eq!(snapshot(&source), before);
            continue;
        }
        assert_eq!(imported.outcome, Outcome::Imported, "{report:?}");
        let id = imported.target_id.as_ref().unwrap();
        client
            .rpc("session.attach", json!({"sessionId":id}))
            .await
            .unwrap();
        let session = agent.get_session(id).unwrap();
        let history = session.history("main", None).await.unwrap();
        assert!(
            history.iter().all(|event| !matches!(
                event.kind,
                EventKind::ModelCall(_) | EventKind::ToolCall(_)
            ))
        );
        let original_ids = history
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::InputMessage(input) => Some(&input.blocks),
                _ => None,
            })
            .flatten()
            .filter_map(|block| match block {
                dim_agent_types::InputBlock::Messages { messages, .. } => Some(messages),
                _ => None,
            })
            .flatten()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>();
        assert!(original_ids.contains(&"first") && original_ids.contains(&"tail"));
        client
            .rpc(
                "session.submit",
                json!({"sessionId":id,"branchId":"main","text":"continue"}),
            )
            .await
            .unwrap();
        client
            .rpc("session.idle", json!({"sessionId":id,"branchId":"main"}))
            .await
            .unwrap();
        let history = session.history("main", None).await.unwrap();
        let request = history.iter().find_map(|event| match &event.kind {
            EventKind::ModelCall(call) => Some(&call.request),
            _ => None,
        });
        if case == "remote" {
            assert!(request.is_none());
            assert!(history.iter().any(|event| matches!(&event.kind,EventKind::TurnEnd(end) if matches!(end.outcome,dim_agent_types::TurnOutcome::Failed{..}))));
        } else {
            let request = request.unwrap();
            let context = serde_json::to_string(&request.messages).unwrap();
            match case {
                "revert" => {
                    assert!(context.contains("compressed original"), "{context}");
                    assert!(
                        !context.contains("retained summary")
                            && !context.contains("effective suffix"),
                        "{context}"
                    );
                }
                "tools" => {
                    assert!(
                        request
                            .messages
                            .iter()
                            .any(|message| message.role == dim_agent_types::Role::Assistant)
                    );
                    assert!(
                        request
                            .messages
                            .iter()
                            .any(|message| message.role == dim_agent_types::Role::ToolResult)
                    );
                    assert!(
                        context.contains("historical-call")
                            && context.contains("historical output")
                    );
                }
                "retry" => {
                    assert!(
                        context.contains("retained summary")
                            && !context.contains("effective suffix"),
                        "{context}"
                    );
                }
                "image" => {
                    assert!(
                        context.contains("image/png") && context.contains("iVBORw0KGgo"),
                        "{context}"
                    );
                }
                _ => unreachable!(),
            }
        }
        assert!(
            history
                .iter()
                .all(|event| !matches!(event.kind, EventKind::ToolCall(_)))
        );
        assert_eq!(snapshot(&source), before);
        db.execute("UPDATE messages SET parts='[{\"type\":\"text\",\"text\":\"changed after import\"}]' WHERE messageId='first'", []).unwrap();
        let changed = migration
            .run(source, true, Some(report.task.id), connection.clone())
            .await
            .unwrap();
        let conflict = changed
            .objects
            .iter()
            .find(|item| item.kind == "session" && item.id == source_id)
            .unwrap();
        assert_eq!(conflict.outcome, Outcome::Conflict, "{changed:?}");
        assert_eq!(conflict.target_id.as_deref(), Some(id.as_str()));
    }
    plugin.dispose().await.unwrap();
    root.shutdown().await.unwrap();
    server.shutdown();
    server.join().await.unwrap();
}
