//! Workflow's actual process/Session rows through Settings, files, S2 and C1.
use super::*;
use std::path::{Path, PathBuf};

fn rows() -> dim_app_server::session_typescript::SessionTypeScriptConfig {
    let mut config = session_typescript::fixture();
    config.process_rows = vec![
        serde_json::from_str(include_str!(
            "../../../../runtime/plugins/workflow/process.json"
        ))
        .unwrap(),
    ];
    config.rows = vec![
        serde_json::from_str(include_str!(
            "../../../../runtime/plugins/workflow/session.json"
        ))
        .unwrap(),
    ];
    config
}

async fn ready(client: &mut Client) {
    process_typescript::files_available(client, "workflow.status", json!({})).await;
}

async fn settled(client: &mut Client, id: &Value) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let reply = client.rpc("workflow.status", json!({"run_id":id})).await;
            assert_ok(&reply);
            if matches!(
                reply["result"]["state"].as_str(),
                Some("completed" | "failed" | "stopped" | "interrupted")
            ) {
                return reply["result"].clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Workflow settles its owned stages")
}

fn stored(home: &Path) -> Value {
    let outer: Value =
        serde_json::from_slice(&std::fs::read(home.join("workflow/runs.json")).unwrap()).unwrap();
    assert_eq!(outer["version"], 1);
    serde_json::from_str(outer["text"].as_str().unwrap()).unwrap()
}

async fn report_count(session: &dim_agent_runtime::Session, run: &str) -> usize {
    session
        .local("main", ..)
        .await
        .unwrap()
        .iter()
        .filter(|event| {
            matches!(&event.kind, EventKind::InputMessage(data) if data.blocks.iter().any(|block|
            matches!(block, dim_agent_types::InputBlock::Raw { text, .. }
                if text.starts_with("<dim-workflow kind=\"report\"") && text.contains(run))))
        })
        .count()
}

fn probe_rows() -> dim_app_server::session_typescript::SessionTypeScriptConfig {
    let mut parts = rows();
    parts.process_rows[0]["name"] = json!(
        parts
            .anchor
            .parent()
            .unwrap()
            .join("fixtures/workflow-package.ts")
    );
    parts.process_rows[0]["provides"]["managementMethods:workflowprobe"] =
        json!({"describe":"async","call":"async"});
    parts
}

async fn report_restart(kill_runtime: bool) {
    use tokio::io::AsyncWriteExt;
    let home = std::env::temp_dir().join(format!("dim-workflow-report-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(home.join("workflow")).unwrap();
    let home = home.canonicalize().unwrap();
    let (send, mut controls) = tokio::sync::mpsc::unbounded_channel();
    let mut config = ServerConfig {
        product_home: Some(home.clone()),
        database_path: Some(home.join("native.sqlite")),
        session_typescript: Some(probe_rows()),
        ..Default::default()
    };
    config.session_defaults.tools = vec!["echo".into(), "workflow".into()];
    let (server, agent) = launch_with(Arc::new(MockTools), config, |builder| {
        builder
            .session_plugin(move |scope| session_typescript::ControlledGate(*scope, send.clone()));
    })
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    process_typescript::select_model(&mut client).await;
    let created = client
        .rpc(
            "session.create",
            json!({"cwd":home,"tools":["echo"],
        "permissionPolicy":{"preset":"full-access"}}),
        )
        .await;
    assert_ok(&created);
    let starter = created["result"]["sessionId"].as_str().unwrap();
    let target = agent.get_session(starter).unwrap();
    let (_, _, loader, _) = controls.recv().await.unwrap();
    ready(&mut client).await;
    let before = client.rpc("workflowprobe.status", json!({})).await;
    assert_ok(&before);
    let gate = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert_ok(
        &client
            .rpc(
                "workflowprobe.configure",
                json!({"phase":"before_input",
        "host":"127.0.0.1","port":gate.local_addr().unwrap().port()}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc(
                "workflow.save",
                json!({"source":"user","document":{
        "schema":"dim.workflow/v1","name":"report","description":"Report recovery",
        "steps":[{"stage":{"id":"one","prompt":"completed"}}]}}),
            )
            .await,
    );
    let started = client
        .rpc(
            "workflow.start",
            json!({"name":"report","cwd":home,
        "report_to":{"session_id":starter,"branch_id":"main"}}),
        )
        .await;
    assert_ok(&started);
    let run = started["result"]["run_id"].as_str().unwrap();
    let (mut release, _) = tokio::time::timeout(Duration::from_secs(10), gate.accept())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored(&home)["runs"][0]["report"]["attempt_open"], true);
    let connection = rusqlite::Connection::open(home.join("native.sqlite")).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    release.write_all(b"continue").await.unwrap();
    drop(release);
    tokio::time::timeout(Duration::from_secs(3), async {
        while !target.publication("main").unwrap().has_pending() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let owner = loader.get("workflow").unwrap().view.unwrap();
    if kill_runtime {
        assert!(
            std::process::Command::new("kill")
                .args([
                    "-KILL",
                    &before["result"]["pid"].as_u64().unwrap().to_string()
                ])
                .status()
                .unwrap()
                .success()
        );
    }
    let name = if kill_runtime {
        "SessionNodeRuntime"
    } else {
        "workflow"
    };
    let (restarted, ()) = tokio::join!(loader.restart(name), async {
        let mut state = owner.watch();
        tokio::time::timeout(Duration::from_secs(3), async {
            while state.borrow_and_update().state != rutis::FiberState::Unloading {
                state.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(
            owner
                .effects()
                .iter()
                .any(|effect| effect.label == "sessions.input"),
            "the real background report retains its original-row native writer"
        );
        assert_eq!(stored(&home)["runs"][0]["report"]["attempt_open"], true);
        connection.execute_batch("COMMIT").unwrap();
    });
    restarted.unwrap();
    ready(&mut client).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while stored(&home)["runs"][0]["report"]["state"] != "delivered" {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(report_count(&target, run).await, 1);
    loader.restart("workflow").await.unwrap();
    ready(&mut client).await;
    assert_eq!(report_count(&target, run).await, 1);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_workflow_background_report_drains_on_row_and_runtime_replacement() {
    report_restart(false).await;
    report_restart(true).await;
}

async fn report_crash(phase: &str, fail_replacement: bool) {
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncWriteExt;
    struct RestoreMode(PathBuf, std::fs::Permissions);
    impl Drop for RestoreMode {
        fn drop(&mut self) {
            std::fs::set_permissions(&self.0, self.1.clone()).unwrap();
        }
    }
    let home = std::env::temp_dir().join(format!("dim-workflow-crash-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(home.join("workflow")).unwrap();
    let home = home.canonicalize().unwrap();
    let (send, mut controls) = tokio::sync::mpsc::unbounded_channel();
    let mut config = ServerConfig {
        product_home: Some(home.clone()),
        database_path: Some(home.join("native.sqlite")),
        session_typescript: Some(probe_rows()),
        ..Default::default()
    };
    config.session_defaults.tools = vec!["echo".into(), "workflow".into()];
    let (server, agent) = launch_with(Arc::new(MockTools), config, |builder| {
        builder
            .session_plugin(move |scope| session_typescript::ControlledGate(*scope, send.clone()));
    })
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    process_typescript::select_model(&mut client).await;
    let created = client
        .rpc(
            "session.create",
            json!({"cwd":home,"tools":["echo"],
        "permissionPolicy":{"preset":"full-access"}}),
        )
        .await;
    assert_ok(&created);
    let starter = created["result"]["sessionId"].as_str().unwrap();
    let target = agent.get_session(starter).unwrap();
    let (_, _, loader, _) = controls.recv().await.unwrap();
    ready(&mut client).await;
    let before = client.rpc("workflowprobe.status", json!({})).await;
    assert_ok(&before);
    let gate = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert_ok(
        &client
            .rpc(
                "workflowprobe.configure",
                json!({"phase":phase,
        "host":"127.0.0.1","port":gate.local_addr().unwrap().port()}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc(
                "workflow.save",
                json!({"source":"user","document":{
        "schema":"dim.workflow/v1","name":"report","description":"Report recovery",
        "steps":[{"stage":{"id":"one","prompt":"completed"}}]}}),
            )
            .await,
    );
    let started = client
        .rpc(
            "workflow.start",
            json!({"name":"report","cwd":home,
        "report_to":{"session_id":starter,"branch_id":"main"}}),
        )
        .await;
    assert_ok(&started);
    let run = started["result"]["run_id"].as_str().unwrap();
    let (mut release, _) = tokio::time::timeout(Duration::from_secs(10), gate.accept())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored(&home)["runs"][0]["report"]["attempt_open"], true);
    assert_eq!(
        report_count(&target, run).await,
        usize::from(phase == "reply_write")
    );
    if fail_replacement {
        let directory = home.join("workflow");
        let mode = RestoreMode(
            directory.clone(),
            std::fs::metadata(&directory).unwrap().permissions(),
        );
        let bytes = std::fs::read(directory.join("runs.json")).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
        release.write_all(b"continue").await.unwrap();
        drop(release);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let reply = client.rpc("workflowprobe.status", json!({})).await;
                assert_ok(&reply);
                if reply["result"]["replacementError"] == "io" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(directory.join("runs.json")).unwrap(),
            bytes,
            "a failed actual file.replace preserves the persisted open attempt"
        );
        drop(mode);
        loader.restart("workflow").await.unwrap();
    } else {
        assert!(
            std::process::Command::new("kill")
                .args([
                    "-KILL",
                    &before["result"]["pid"].as_u64().unwrap().to_string()
                ])
                .status()
                .unwrap()
                .success()
        );
        drop(release);
        loader.restart("SessionNodeRuntime").await.unwrap();
    }
    ready(&mut client).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while stored(&home)["runs"][0]["report"]["state"] != "delivered" {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(report_count(&target, run).await, 1);
    loader.restart("workflow").await.unwrap();
    ready(&mut client).await;
    assert_eq!(report_count(&target, run).await, 1);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_workflow_reconciles_crashes_and_a_failed_reply_record_replacement() {
    report_crash("before_input", false).await;
    report_crash("reply_write", false).await;
    report_crash("reply_write", true).await;
}

async fn host(home: &Path) -> (tokio::process::Child, Client) {
    use std::process::Stdio;
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "workflow::node_workflow_reports_recover_after_complete_host_restart",
            "--ignored",
            "--nocapture",
        ])
        .env("DIM_WORKFLOW_PACKAGE_HOST", home)
        .env("DIMCODE_HOME", home)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let socket = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let mut line = String::new();
            assert_ne!(
                output.read_line(&mut line).await.unwrap(),
                0,
                "child exited before startup"
            );
            if let Ok(value) = serde_json::from_str::<Value>(&line)
                && let Some(socket) = value["socket"].as_str()
            {
                break PathBuf::from(socket);
            }
        }
    })
    .await
    .unwrap();
    child.stdout = Some(output.into_inner());
    let mut client = Client {
        transport: Transport::Unix(BufReader::new(UnixStream::connect(socket).await.unwrap())),
        id: 0,
        notices: vec![],
        instance: Value::Null,
        features: Value::Null,
    };
    assert_ok(
        &client
            .rpc("hello", json!({"protocolVersion":3,"clientKind":"other"}))
            .await,
    );
    ready(&mut client).await;
    (child, client)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_workflow_reports_recover_after_complete_host_restart() {
    if let Some(home) = std::env::var_os("DIM_WORKFLOW_PACKAGE_HOST") {
        let home = PathBuf::from(home);
        let mut config = ServerConfig {
            product_home: Some(home.clone()),
            database_path: Some(home.join("native.sqlite")),
            session_typescript: Some(probe_rows()),
            ..Default::default()
        };
        config.session_defaults.tools = vec!["echo".into(), "workflow".into()];
        let server = launch_with_agent(Arc::new(MockTools), config).await.0;
        println!("\n{}", json!({"socket":server.unix_path}));
        server.join().await.unwrap();
        return;
    }
    for phase in ["before_input", "reply_write"] {
        let home = std::env::temp_dir().join(format!("dim-workflow-host-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(home.join("workflow")).unwrap();
        let home = home.canonicalize().unwrap();
        let (mut process, mut client) = host(&home).await;
        process_typescript::select_model(&mut client).await;
        let created = client
            .rpc("session.create", json!({"cwd":home,"tools":["echo"]}))
            .await;
        assert_ok(&created);
        let starter = created["result"]["sessionId"].as_str().unwrap();
        let gate = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert_ok(
            &client
                .rpc(
                    "workflowprobe.configure",
                    json!({"phase":phase,
            "host":"127.0.0.1","port":gate.local_addr().unwrap().port()}),
                )
                .await,
        );
        assert_ok(
            &client
                .rpc(
                    "workflow.save",
                    json!({"source":"user","document":{
            "schema":"dim.workflow/v1","name":"report","description":"Host recovery",
            "steps":[{"stage":{"id":"one","prompt":"completed"}}]}}),
                )
                .await,
        );
        let started = client
            .rpc(
                "workflow.start",
                json!({"name":"report","cwd":home,
            "report_to":{"session_id":starter,"branch_id":"main"}}),
            )
            .await;
        assert_ok(&started);
        let (_release, _) = tokio::time::timeout(Duration::from_secs(10), gate.accept())
            .await
            .unwrap()
            .unwrap();
        let pending = stored(&home)["runs"][0]["report"].clone();
        assert_eq!(pending["attempt_open"], true);
        let pid = process.id().unwrap();
        process.kill().await.unwrap();
        process.wait().await.unwrap();
        drop(client);
        let (mut process, mut client) = host(&home).await;
        assert_ne!(process.id().unwrap(), pid);
        tokio::time::timeout(Duration::from_secs(10), async {
            while stored(&home)["runs"][0]["report"]["state"] != "delivered" {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let history = client
            .rpc(
                "session.history",
                json!({"sessionId":starter,
            "branchId":"main","after":0,"limit":1000}),
            )
            .await;
        assert_ok(&history);
        assert_eq!(history["result"]["done"], true);
        assert_eq!(
            history["result"]["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|event| event["type"] == "input"
                    && event["name"] == "message"
                    && event["data"]["submission_id"] == pending["submission_id"])
                .count(),
            1,
            "{history}"
        );
        assert_ok(&client.rpc("daemon.shutdown", json!({"force":true})).await);
        tokio::time::timeout(Duration::from_secs(10), process.wait())
            .await
            .unwrap()
            .unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_workflow_sources_stages_policy_and_report_use_the_real_package() {
    let home = std::env::temp_dir().join(format!("dim-workflow-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(home.join("workflow")).unwrap();
    let home: PathBuf = home.canonicalize().unwrap();
    let mut config = ServerConfig {
        product_home: Some(home.clone()),
        database_path: Some(home.join("native.sqlite")),
        session_typescript: Some(rows()),
        ..Default::default()
    };
    config.session_defaults.tools = vec!["echo".into(), "wait".into(), "workflow".into()];
    let (server, agent) = launch_with_agent(Arc::new(MockTools), config).await;
    let mut client = Client::new_version(&server, false, 3).await;
    process_typescript::select_model(&mut client).await;
    let created = client
        .rpc(
            "session.create",
            json!({"cwd":home,
        "tools":["workflow","wait","echo"],"permissionPolicy":{"preset":"full-access"}}),
        )
        .await;
    assert_ok(&created);
    let starter = created["result"]["sessionId"].as_str().unwrap();
    ready(&mut client).await;
    let document = json!({"schema":"dim.workflow/v1","name":"review","description":"Review in parallel",
        "parameters":[{"name":"target","required":true}],
        "steps":[{"stage":{"id":"analyze","prompt":"Analyze {{params.target}}"}},
          {"parallel":[{"id":"a","prompt":"A: {{stages.analyze.result}}"},
                       {"id":"b","prompt":"B: {{stages.analyze.result}}"}]}]});
    assert_ok(
        &client
            .rpc(
                "workflow.save",
                json!({"source":"user","document":document}),
            )
            .await,
    );
    let listed = client.rpc("workflow.list", json!({"cwd":home})).await;
    assert_ok(&listed);
    assert_eq!(listed["result"][0]["valid"], true);
    let started = client
        .rpc(
            "workflow.start",
            json!({"name":"review","cwd":home,"params":{"target":"file.ts"},
        "report_to":{"session_id":starter,"branch_id":"main"}}),
        )
        .await;
    assert_ok(&started);
    let run = settled(&mut client, &started["result"]["run_id"]).await;
    assert_eq!(run["state"], "completed", "{run}");
    assert_eq!(run["stages"].as_array().unwrap().len(), 3);
    assert_eq!(run["policy_cap"]["preset"], "read-only");
    for stage in run["stages"].as_array().unwrap() {
        let session = agent
            .get_session(stage["session_id"].as_str().unwrap())
            .unwrap();
        assert!(!session.config().interactive);
        assert_eq!(
            session.config().permission_policy.preset,
            dim_agent_types::PermissionPreset::ReadOnly
        );
        assert!(stage["result_text"].as_str().unwrap().contains("file.ts"));
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        while stored(&home)["runs"][0]["report"]["state"] != "delivered" {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let reports = agent
        .get_session(starter)
        .unwrap()
        .local("main", ..)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| {
            if let EventKind::InputMessage(data) = &event.kind
                && data.blocks.iter().any(|block| {
                    matches!(block,
                    dim_agent_types::InputBlock::Raw { text, .. }
                    if text.starts_with("<dim-workflow kind=\"report\"")
                        && text.contains(started["result"]["run_id"].as_str().unwrap()))
                })
            {
                assert!(
                    data.user.is_none(),
                    "the report is a single raw context block"
                );
                true
            } else {
                false
            }
        })
        .count();
    assert_eq!(
        reports, 1,
        "one published report in the actual starter Timeline"
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

async fn tool_result(
    client: &mut Client,
    session: &dim_agent_runtime::Session,
    arguments: Value,
) -> ToolResult {
    let id = session.id();
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":id,"branchId":"main",
        "text":format!("tool:workflow {arguments}")}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc("session.idle", json!({"sessionId":id,"branchId":"main"}))
            .await,
    );
    session
        .local("main", ..)
        .await
        .unwrap()
        .into_iter()
        .rev()
        .find_map(|event| {
            if let EventKind::ToolResult(batch) = &event.kind {
                batch.results.first().map(|entry| entry.result.clone())
            } else {
                None
            }
        })
        .expect("Workflow tool result")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_workflow_project_enablement_invalid_sources_and_tool_policy() {
    let home = std::env::temp_dir().join(format!("dim-workflow-sources-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(home.join("workflow")).unwrap();
    let home = home.canonicalize().unwrap();
    let mut config = ServerConfig {
        product_home: Some(home.clone()),
        database_path: Some(home.join("native.sqlite")),
        session_typescript: Some(rows()),
        ..Default::default()
    };
    config.session_defaults.tools = vec!["echo".into(), "workflow".into()];
    let (server, agent) = launch_with_agent(Arc::new(MockTools), config).await;
    let mut client = Client::new_version(&server, false, 3).await;
    process_typescript::select_model(&mut client).await;
    let created = client.rpc("session.create", json!({"cwd":home,"tools":["workflow"],
        "permissionPolicy":{"preset":"workspace-write","allowedTools":["workflow"],"disallowedTools":[]}})).await;
    assert_ok(&created);
    let starter = agent
        .get_session(created["result"]["sessionId"].as_str().unwrap())
        .unwrap();
    ready(&mut client).await;
    let document = json!({"schema":"dim.workflow/v1","name":"project","description":"Project source",
        "steps":[{"stage":{"id":"one","prompt":"first contents"}}]});
    assert_ok(
        &client
            .rpc(
                "workflow.save",
                json!({"source":"project","cwd":home,"document":document}),
            )
            .await,
    );
    let listed = client.rpc("workflow.list", json!({"cwd":home})).await;
    assert_ok(&listed);
    assert_eq!(listed["result"][0]["enabled"], false);
    let refused = client
        .rpc("workflow.start", json!({"name":"project","cwd":home}))
        .await;
    assert_eq!(refused["error"]["data"]["appError"], "definition_not_found");
    assert_eq!(stored(&home)["runs"], json!([]));
    let settings = client
        .rpc("settings.read", json!({"source":"workflow","scope":"user"}))
        .await;
    assert_ok(&settings);
    let mut source = settings["result"]["document"].clone();
    source["projectEnabled"] = json!({home.to_str().unwrap():{"project":true}});
    source["max_concurrent_runs"] = json!(2);
    source["max_concurrent_stages"] = json!(1);
    let mut invalid = document.clone();
    invalid["name"] = json!("invalid");
    invalid["steps"][0]["stage"]["prompt"] = json!("{{stages.missing.result}}");
    source["definitions"] = json!({"invalid":invalid,"mismatch":document});
    assert_ok(
        &client
            .rpc(
                "settings.update",
                json!({"source":"workflow","scope":"user",
        "expectedRevision":settings["result"]["revision"],"document":source}),
            )
            .await,
    );
    let revision = client
        .rpc("settings.read", json!({"source":"workflow","scope":"user"}))
        .await;
    let listed = client.rpc("workflow.list", json!({"cwd":home})).await;
    assert_ok(&listed);
    for name in ["invalid", "mismatch"] {
        let entry = listed["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap();
        assert_eq!(entry["valid"], false);
        assert!(entry["issue_count"].as_u64().unwrap() > 0);
        assert_eq!(
            client
                .rpc("workflow.start", json!({"name":name,"cwd":home}))
                .await["error"]["data"]["appError"],
            "invalid_definition"
        );
    }
    let result = tool_result(&mut client, &starter, json!({"action":"list"})).await;
    let dim_agent_types::ToolContent::Text(content) = &result.content else {
        panic!("Workflow returns text")
    };
    assert!(content.contains("invalid reference"));
    assert!(content.contains("Document name must match mismatch"));
    assert_eq!(
        client
            .rpc("settings.read", json!({"source":"workflow","scope":"user"}))
            .await["result"]["revision"],
        revision["result"]["revision"]
    );
    let started = tool_result(
        &mut client,
        &starter,
        json!({"action":"start","name":"project"}),
    )
    .await;
    assert_ne!(started.is_error, Some(true), "{started:?}");
    let id = started.structured_content.as_ref().unwrap()["run_id"].clone();
    let dim_agent_types::ToolContent::Text(content) = &started.content else {
        panic!("Workflow returns text")
    };
    assert!(content.contains(id.as_str().unwrap()));
    let run = settled(&mut client, &id).await;
    assert_eq!(run["policy_cap"]["preset"], "workspace-write");
    assert!(
        run["stages"][0]["result_text"]
            .as_str()
            .unwrap()
            .contains("first contents")
    );
    let mut nested = document.clone();
    nested["name"] = json!("nested");
    nested["policy"] = json!("full_access");
    nested["steps"][0]["stage"]["prompt"] = json!("tool:workflow {\"action\":\"list\"}");
    assert_ok(
        &client
            .rpc("workflow.save", json!({"source":"user","document":nested}))
            .await,
    );
    let started = tool_result(
        &mut client,
        &starter,
        json!({"action":"start","name":"nested"}),
    )
    .await;
    assert_ne!(started.is_error, Some(true), "{started:?}");
    let run = settled(
        &mut client,
        &started.structured_content.as_ref().unwrap()["run_id"],
    )
    .await;
    assert!(
        run["stages"][0]["result_text"]
            .as_str()
            .unwrap()
            .contains("only in interactive Sessions"),
        "{run}"
    );
    let denied = client
        .rpc(
            "session.create",
            json!({"cwd":home,"tools":["workflow"],
        "permissionPolicy":{"preset":"read-only"}}),
        )
        .await;
    assert_ok(&denied);
    let session = agent
        .get_session(denied["result"]["sessionId"].as_str().unwrap())
        .unwrap();
    let result = tool_result(&mut client, &session, json!({"action":"list"})).await;
    assert_eq!(
        result.is_error,
        Some(true),
        "ReadOnly denies Workflow's unclassified tool risk"
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_workflow_rejects_invalid_inner_records_without_changing_bytes() {
    let record = json!({
        "run_id":"record", "name":"record", "source":"user", "params":{}, "cwd":"/tmp",
        "definition":{"schema":"dim.workflow/v1","name":"record","description":"Records",
            "steps":[{"stage":{"id":"one","prompt":"done"}}]},
        "started_by":"management", "created_at":"2026-10-22T00:00:00.000Z",
        "ended_at":"2026-10-22T00:00:01.000Z", "state":"completed", "log":[],
        "stages":[{"id":"one","step":0,"state":"completed","result_text":"done"}],
        "report_to":{"session_id":"starter","branch_id":"main"},
        "report":{"state":"pending","submission_id":"workflow:record"}
    });
    for inner in [
        json!({"version":1,"runs":[]}).to_string(),
        json!({"version":99,"runs":[]}).to_string(),
        json!({"version":2,"runs":[record]}).to_string(),
        "{broken".into(),
    ] {
        let home =
            std::env::temp_dir().join(format!("dim-workflow-invalid-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(home.join("workflow")).unwrap();
        let bytes = serde_json::to_vec(&json!({"version":1,"text":inner})).unwrap();
        let path = home.join("workflow/runs.json");
        std::fs::write(&path, &bytes).unwrap();
        let (server, _) = launch_with_agent(
            Arc::new(MockTools),
            ServerConfig {
                product_home: Some(home.clone()),
                database_path: Some(home.join("native.sqlite")),
                session_typescript: Some(rows()),
                ..Default::default()
            },
        )
        .await;
        let mut client = Client::new_version(&server, false, 3).await;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let reply = client.rpc("workflow.status", json!({})).await;
                let code = &reply["error"]["data"]["appError"];
                if code == "service_reloading" {
                    tokio::task::yield_now().await;
                    continue;
                }
                assert_eq!(code, "invalid_data", "{reply}");
                break;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "invalid inner bytes are never reset or migrated"
        );
        server.shutdown();
        server.join().await.unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}
