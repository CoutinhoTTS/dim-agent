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
    let user_home = Home::new();
    let missing = home
        .command()
        .env("HOME", &user_home.0)
        .args(["migration", "--source-stopped"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("No default legacy v2 database"));
    assert!(!home.0.join("migration").exists());
    let default_source = user_home.0.join(".dimcode/v2");
    fs::create_dir_all(&default_source).unwrap();
    fs::copy(
        source.0.join("dimcode.sqlite"),
        default_source.join("dimcode.sqlite"),
    )
    .unwrap();
    fs::copy(
        source.0.join("permissions.json"),
        default_source.join("permissions.json"),
    )
    .unwrap();
    let command = || {
        let mut command = home.command();
        command
            .env("HOME", &user_home.0)
            .arg("--model-config")
            .arg(&model);
        command
    };
    let mut decline = command();
    decline.arg("migration");
    let mut terminal = Terminal::launch(decline);
    terminal.until("Continue? [y/N]");
    assert!(
        terminal
            .transcript
            .contains(default_source.to_str().unwrap())
    );
    assert!(terminal.transcript.contains(home.0.to_str().unwrap()));
    terminal.write(b"n\n");
    assert_eq!(terminal.wait().code(), Some(130));
    assert!(!home.0.join("migration").exists());
    assert!(!home.0.join("rust/daemon.sock").exists());
    let global = user_home.0.join(".dim/v2");
    fs::create_dir_all(&global).unwrap();
    fs::copy(
        source.0.join("dimcode.sqlite"),
        global.join("dimglobal.sqlite"),
    )
    .unwrap();
    let ambiguous = command()
        .args(["migration", "--source-stopped"])
        .output()
        .unwrap();
    assert_eq!(ambiguous.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("Both legacy v2 homes exist"));
    assert!(!home.0.join("migration").exists());
    let inspected = home
        .command()
        .env("HOME", &user_home.0)
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
    let cn_database = default_source.join("dimcode.sqlite");
    let parked = default_source.join("parked.sqlite");
    fs::rename(&cn_database, &parked).unwrap();
    let global_inspection = command()
        .args(["migration", "inspect", "--source-stopped"])
        .output()
        .unwrap();
    assert!(
        global_inspection.status.success(),
        "{}",
        String::from_utf8_lossy(&global_inspection.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&global_inspection.stdout).unwrap()["database"],
        "dimglobal.sqlite"
    );
    fs::rename(parked, cn_database).unwrap();
    fs::remove_dir_all(global).unwrap();
    let run = |args: &[&str]| command().args(args).output().unwrap();
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
    let mut import = command();
    import.arg("migration");
    let mut terminal = Terminal::launch(import);
    terminal.until("Continue? [y/N]");
    terminal.write(b"y\n");
    assert!(terminal.wait().success(), "{}", terminal.transcript);
    let tasks = run(&["migration", "list"]);
    assert!(tasks.status.success());
    let tasks: Value = serde_json::from_slice(&tasks.stdout).unwrap();
    assert_eq!(tasks.as_array().unwrap().len(), 1);
    let first = run(&["migration", "show", tasks[0]["id"].as_str().unwrap()]);
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
    let unaccepted = home
        .0
        .join("migration/tasks/018ec73f-0000-7000-8000-000000000099/results");
    fs::create_dir_all(&unaccepted).unwrap();
    let listed = home.command().args(["migration", "list"]).output().unwrap();
    assert!(listed.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&listed.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    fs::write(unaccepted.join("orphan.json"), b"{}").unwrap();
    let damaged = home.command().args(["migration", "list"]).output().unwrap();
    assert_eq!(
        damaged.status.code(),
        Some(1),
        "Results without state must remain a visible integrity error"
    );
    fs::remove_dir_all(unaccepted.parent().unwrap()).unwrap();
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
    assert_eq!(
        fs::read(default_source.join("dimcode.sqlite")).unwrap(),
        before
    );
}
