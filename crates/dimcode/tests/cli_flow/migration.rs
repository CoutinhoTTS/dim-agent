use super::*;

#[test]
fn migration_commands_load_on_demand_and_reconcile_retries_without_a_live_plugin() {
    let home = Home::new();
    let source = Home::new();
    let model = home.0.join("mock-model.json");
    fs::write(
        &model,
        include_str!("../../../dim-agent-aimux/examples/offline.json"),
    )
    .unwrap();
    let db = rusqlite::Connection::open(source.0.join("dimcode.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE sessions(sessionId TEXT PRIMARY KEY,cwd TEXT,title TEXT,status TEXT,tags TEXT,createdAt TEXT,updatedAt TEXT,version INTEGER);
        CREATE TABLE session_states(sessionId TEXT PRIMARY KEY,selectedProviderId TEXT,selectedModelId TEXT,settings TEXT,permissionSettings TEXT,currentMode TEXT);
        CREATE TABLE messages(messageId TEXT PRIMARY KEY,sessionId TEXT,role TEXT,parts TEXT,attachments TEXT,metadata TEXT,orderKey TEXT,createdAt TEXT);
        CREATE TABLE compaction_states(sessionId TEXT PRIMARY KEY,cursor TEXT,compactionSegments TEXT);").unwrap();
    db.execute("INSERT INTO sessions VALUES ('old',?1,'Legacy session','active','[]','2026-01-01','2026-01-01',1)",[source.0.canonicalize().unwrap().to_str().unwrap()]).unwrap();
    db.execute(
        "INSERT INTO session_states VALUES ('old','local','diagram-model','{}','{}','agent')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO messages VALUES ('old-message','old','user','[{\"type\":\"text\",\"text\":\"legacy user message\"}]','[]','{}','a','2026-01-01T00:00:00Z')",[]).unwrap();
    drop(db);
    fs::write(source.0.join("permissions.json"),json!({"version":1,"execCommandPermissions":{"rules":{"cargo test *":"allow","git push *":"deny"}}}).to_string()).unwrap();
    let before = fs::read(source.0.join("dimcode.sqlite")).unwrap();
    for args in [
        vec!["migration", "--help"],
        vec!["migration", "run", "--help"],
        vec!["migration", "retry", "--help"],
    ] {
        let output = home.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(!home.0.join("migration").exists());
    assert!(!home.0.join("rust/daemon.sock").exists());
    let overlapping = source
        .command()
        .args([
            "migration",
            "run",
            "--source",
            source.0.to_str().unwrap(),
            "--source-stopped",
        ])
        .output()
        .unwrap();
    assert_eq!(overlapping.status.code(), Some(1));
    assert!(!source.0.join("migration").exists());
    assert!(!source.0.join("rust/daemon.sock").exists());
    let stopped = home
        .command()
        .args(["migration", "run", "--source", source.0.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(stopped.status.code(), Some(1));
    assert!(!home.0.join("migration").exists());
    let inspected = home
        .command()
        .args([
            "migration",
            "inspect",
            "--source",
            source.0.to_str().unwrap(),
            "--source-stopped",
        ])
        .output()
        .unwrap();
    assert!(
        inspected.status.success(),
        "{}",
        String::from_utf8_lossy(&inspected.stderr)
    );
    let inventory: Value = serde_json::from_slice(&inspected.stdout).unwrap();
    assert_eq!(inventory["tables"]["messages"], 1);
    assert!(!home.0.join("rust/daemon.sock").exists());
    let run = |args: &[&str]| {
        home.command()
            .args(["--model-config", model.to_str().unwrap()])
            .args(args)
            .output()
            .unwrap()
    };
    struct Daemon<'a>(&'a Home, &'a std::path::Path);
    impl Drop for Daemon<'_> {
        fn drop(&mut self) {
            let _ = self
                .0
                .command()
                .arg("--model-config")
                .arg(self.1)
                .args(["daemon", "stop"])
                .output();
        }
    }
    let _daemon = Daemon(&home, &model);
    let first = run(&[
        "migration",
        "run",
        "--source",
        source.0.to_str().unwrap(),
        "--source-stopped",
        "--json",
    ]);
    assert!(
        first.status.success(),
        "{}\n{}\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
        fs::read_to_string(home.0.join("rust/daemon.log")).unwrap_or_default()
    );
    let first: Value = serde_json::from_slice(&first.stdout).unwrap();
    let task = first["task"]["id"].as_str().unwrap();
    let session = first["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "session")
        .unwrap()["targetId"]
        .as_str()
        .unwrap();
    let exported = run(&["session", "export", session, "--format", "json"]);
    assert!(
        exported.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&exported.stderr),
        fs::read_to_string(home.0.join("rust/daemon.log")).unwrap_or_default()
    );
    assert!(String::from_utf8_lossy(&exported.stdout).contains("legacy user message"));
    assert!(run(&["daemon", "stop"]).status.success());
    let read_rules = |home: &Home| {
        let native = rusqlite::Connection::open_with_flags(
            home.0.join("rust/dimcode.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let rules: String = native
            .query_row(
                "SELECT document FROM settings_documents WHERE source='user/permission'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        serde_json::from_str::<Value>(&rules).unwrap()["commandRules"].clone()
    };
    assert_eq!(
        read_rules(&home),
        json!([
            {"command":"cargo test","match":"prefix","decision":"allow"},{"command":"git push","match":"prefix","decision":"deny"}
        ])
    );
    let shown = home
        .command()
        .args(["migration", "show", task])
        .output()
        .unwrap();
    assert!(shown.status.success());
    assert!(!home.0.join("rust/daemon.sock").exists());
    assert_eq!(
        serde_json::from_slice::<Value>(&shown.stdout).unwrap(),
        first
    );
    let retried = run(&["migration", "retry", task, "--source-stopped", "--json"]);
    assert!(
        retried.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&retried.stdout),
        String::from_utf8_lossy(&retried.stderr)
    );
    let retried: Value = serde_json::from_slice(&retried.stdout).unwrap();
    assert_eq!(retried["task"]["retryOf"], task);
    assert_ne!(retried["task"]["id"], task);
    assert!(
        retried["objects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["kind"] == "session"
                && item["outcome"] == "alreadyImported"
                && item["targetId"] == session)
    );
    assert!(run(&["daemon", "stop"]).status.success());
    let cancelled = home
        .command()
        .args(["migration", "cancel", task])
        .output()
        .unwrap();
    assert!(cancelled.status.success());
    let listed = home.command().args(["migration", "list"]).output().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&listed.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    fs::write(
        source.0.join("permissions.json"),
        json!({"version":1,"execCommandPermissions":{"rules":{
            "git *":"allow","git push *":"deny","git push --dry-run *":"allow",
            "npm *":"allow","npm publish":"ask","cargo test *":"allow","cargo test --all *":"deny"
        }}})
        .to_string(),
    )
    .unwrap();
    let fresh = Home::new();
    let _fresh_daemon = Daemon(&fresh, &model);
    let partial = fresh
        .command()
        .arg("--model-config")
        .arg(&model)
        .args([
            "migration",
            "run",
            "--source",
            source.0.to_str().unwrap(),
            "--source-stopped",
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(
        partial.status.code(),
        Some(2),
        "{}\n{}",
        String::from_utf8_lossy(&partial.stdout),
        String::from_utf8_lossy(&partial.stderr)
    );
    assert!(
        fresh
            .command()
            .arg("--model-config")
            .arg(&model)
            .args(["daemon", "stop"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        read_rules(&fresh),
        json!([
            {"command":"cargo test","match":"prefix","decision":"allow"},{"command":"cargo test --all","match":"prefix","decision":"deny"}
        ])
    );
    let partial: Value = serde_json::from_slice(&partial.stdout).unwrap();
    assert_eq!(
        partial["objects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("permission.rule:"))
                && item["outcome"] == "unsupported")
            .count(),
        5
    );
    assert_eq!(fs::read(source.0.join("dimcode.sqlite")).unwrap(), before);
}
