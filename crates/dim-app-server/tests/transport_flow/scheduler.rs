//! Scheduler's optional package through actual Settings, C4, S2 and native history.
use super::*;
use std::path::{Path, PathBuf};

fn rows() -> dim_app_server::session_typescript::SessionTypeScriptConfig {
    let mut parts = session_typescript::fixture();
    parts.process_rows = vec![
        serde_json::from_str(include_str!(
            "../../../../runtime/plugins/scheduler/process.json"
        ))
        .unwrap(),
    ];
    parts.rows = vec![
        serde_json::from_str(include_str!(
            "../../../../runtime/plugins/scheduler/session.json"
        ))
        .unwrap(),
    ];
    parts
}

fn home() -> PathBuf {
    let path = std::env::temp_dir().join(format!("dim-scheduler-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(path.join("scheduler")).unwrap();
    path.canonicalize().unwrap()
}

fn config(home: &Path) -> ServerConfig {
    let mut config = ServerConfig {
        product_home: Some(home.to_owned()),
        database_path: Some(home.join("native.sqlite")),
        session_typescript: Some(rows()),
        ..Default::default()
    };
    config.session_defaults.tools = vec!["echo".into(), "schedule".into()];
    config.session_defaults.permission_policy = dim_agent_types::PermissionPolicy {
        preset: dim_agent_types::PermissionPreset::FullAccess,
        ..Default::default()
    };
    config
}

async fn settled(client: &mut Client, job: &Value, run: &Value) -> Value {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let reply = client
                .rpc("schedule.history", json!({"job_id":job,"limit":50}))
                .await;
            assert_ok(&reply);
            if let Some(found) = reply["result"]["runs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|value| value["run_id"] == *run)
                && !matches!(
                    found["state"].as_str(),
                    Some("launching" | "submitted" | "running")
                )
            {
                return found.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Scheduler settles its accepted run")
}

async fn tool(
    client: &mut Client,
    session: &dim_agent_runtime::Session,
    arguments: Value,
) -> ToolResult {
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":session.id(),"branchId":"main",
                    "text":format!("tool:schedule {arguments}")}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc(
                "session.idle",
                json!({"sessionId":session.id(),"branchId":"main"}),
            )
            .await,
    );
    let events = session.local("main", ..).await.unwrap();
    events
        .iter()
        .rev()
        .find_map(|event| {
            if let EventKind::ToolResult(batch) = &event.kind {
                batch.results.first().map(|entry| entry.result.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| panic!("schedule tool result missing: {events:#?}"))
}

async fn reports(session: &dim_agent_runtime::Session, id: &str) -> usize {
    session
        .local("main", ..)
        .await
        .unwrap()
        .iter()
        .filter(|event| {
            if let EventKind::InputMessage(data) = &event.kind
                && data.blocks.iter().any(|block| {
                    matches!(block, dim_agent_types::InputBlock::Raw { text, .. }
                        if text.starts_with("Scheduled job ") && text.contains(id))
                })
            {
                assert!(
                    data.user.is_none(),
                    "one raw report, without duplicate user text"
                );
                assert_eq!(data.mode, Some(dim_agent_types::InputMode::Steer));
                true
            } else {
                false
            }
        })
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_saves_live_authority_runs_noninteractive_and_reports_to_the_latest_saver() {
    let home = home();
    let (server, agent) = launch_with_agent(Arc::new(MockTools), config(&home)).await;
    let mut client = Client::new_version(&server, false, 3).await;
    process_typescript::select_model(&mut client).await;
    let ready = process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
    assert_eq!(ready["jobs"], json!([]));
    let settings = client
        .rpc("settings.read", json!({"source":"schedule","scope":"user"}))
        .await;
    assert_ok(&settings);
    assert_eq!(
        settings["result"]["document"],
        json!({"jobs":[],"max_concurrent_runs":4})
    );
    assert_ok(&client.rpc("settings.update", json!({"source":"schedule","scope":"user",
        "expectedRevision":settings["result"]["revision"],"document":{"jobs":[],"max_concurrent_runs":2}})).await);
    let created = client.rpc("session.create", json!({"cwd":home,"tools":["schedule","echo"],
        "permissionPolicy":{"preset":"workspace-write","allowedTools":["schedule"],"disallowedTools":["blocked"]}})).await;
    assert_ok(&created);
    let starter = agent
        .get_session(created["result"]["sessionId"].as_str().unwrap())
        .unwrap();
    let result = tool(
        &mut client,
        &starter,
        json!({"action":"create","job":{
        "name":"review","schedule":{"at":"2030-01-01T00:00:00Z"},"prompt":"Review the project"}}),
    )
    .await;
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let dim_agent_types::ToolContent::Text(content) = &result.content else {
        panic!("schedule returns text")
    };
    let saved: Value = serde_json::from_str(content).unwrap();
    let job = &saved["job"]["id"];
    assert_eq!(saved["job"]["cwd"], json!(home));
    assert_eq!(
        saved["job"]["created_by"]["tool"]["session_id"],
        starter.id()
    );
    assert_eq!(
        saved["job"]["policy_cap"],
        json!({"preset":"workspace-write","allowedTools":["schedule"],"disallowedTools":["blocked"]})
    );
    let started = client.rpc("schedule.run_now", json!({"job_id":job})).await;
    assert_ok(&started);
    let run = settled(&mut client, job, &started["result"]["run_id"]).await;
    assert_eq!(run["state"], "completed", "{run}");
    assert_eq!(run["session_origin"], "launch");
    assert_eq!(run["policy"], saved["job"]["policy_cap"]);
    let executing = agent
        .get_session(run["session_id"].as_str().unwrap())
        .unwrap();
    assert!(!executing.config().interactive);
    assert_eq!(
        executing.config().permission_policy.preset,
        dim_agent_types::PermissionPreset::WorkspaceWrite
    );
    let history = executing.local("main", ..).await.unwrap();
    assert!(
        history
            .iter()
            .any(|event| matches!(&event.kind, EventKind::InputMessage(data)
        if data.blocks.iter().any(|block| matches!(block, dim_agent_types::InputBlock::Raw{text,..}
            if text.starts_with("<dim-scheduler kind=\"scheduled-run\"")))))
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while reports(&starter, run["run_id"].as_str().unwrap()).await == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(reports(&starter, run["run_id"].as_str().unwrap()).await, 1);
    let refused = tool(&mut client, &executing, json!({"action":"list"})).await;
    assert_eq!(refused.is_error, Some(true));
    let dim_agent_types::ToolContent::Text(content) = &refused.content else {
        panic!("schedule returns text")
    };
    assert!(content.contains("interactive leader"));

    let management = client
        .rpc("schedule.save", json!({"job":{"id":job,"name":"operator"}}))
        .await;
    assert_ok(&management);
    assert_eq!(management["result"]["job"]["created_by"], "user");
    assert_eq!(
        management["result"]["transition"]["to"]["effective_policy"]["preset"],
        "read-only"
    );
    let managed = client.rpc("schedule.run_now", json!({"job_id":job})).await;
    assert_ok(&managed);
    let managed = settled(&mut client, job, &managed["result"]["run_id"]).await;
    assert_eq!(managed["policy"]["preset"], "read-only");
    assert!(managed.get("report_to").is_none());
    assert_eq!(
        reports(&starter, managed["run_id"].as_str().unwrap()).await,
        0
    );
    let created_b = client
        .rpc(
            "session.create",
            json!({"cwd":home,"tools":["schedule","echo"],
        "permissionPolicy":{"preset":"full-access"}}),
        )
        .await;
    assert_ok(&created_b);
    let branch_b = agent
        .get_session(created_b["result"]["sessionId"].as_str().unwrap())
        .unwrap();
    let updated = tool(
        &mut client,
        &branch_b,
        json!({"action":"update","job_id":job,"job":{"name":"branch B"}}),
    )
    .await;
    assert_ne!(updated.is_error, Some(true));
    let dim_agent_types::ToolContent::Text(content) = &updated.content else {
        panic!("schedule returns text")
    };
    let updated: Value = serde_json::from_str(content).unwrap();
    assert_eq!(
        updated["transition"]["from"]["effective_policy"]["preset"],
        "read-only"
    );
    assert_eq!(
        updated["transition"]["to"]["effective_policy"]["preset"],
        "full-access"
    );
    let current = client
        .rpc("settings.read", json!({"source":"schedule","scope":"user"}))
        .await;
    assert_ok(&current);
    assert_eq!(current["result"]["document"]["max_concurrent_runs"], 2);
    let mut invalid = current["result"]["document"].clone();
    invalid["jobs"][0]
        .as_object_mut()
        .unwrap()
        .remove("enabled_at");
    let refused = client
        .rpc(
            "settings.update",
            json!({"source":"schedule","scope":"user",
        "expectedRevision":current["result"]["revision"],"document":invalid}),
        )
        .await;
    assert!(
        refused.get("error").is_some(),
        "enabled jobs need their persisted anchor: {refused}"
    );
    let unchanged = client
        .rpc("settings.read", json!({"source":"schedule","scope":"user"}))
        .await;
    assert_eq!(unchanged["result"], current["result"]);
    let outer: Value =
        serde_json::from_slice(&std::fs::read(home.join("scheduler/runs.json")).unwrap()).unwrap();
    assert_eq!(outer["version"], 1);
    let inner: Value = serde_json::from_str(outer["text"].as_str().unwrap()).unwrap();
    assert_eq!(inner["version"], 1);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_owner_reload_tracks_the_existing_native_run_without_replaying_input_or_reports()
 {
    let home = home();
    let calls = Arc::new(AtomicUsize::new(0));
    let (send, mut controls) = tokio::sync::mpsc::unbounded_channel();
    let mut settings = config(&home);
    settings.session_defaults.tools = vec!["write".into(), "schedule".into()];
    let (server, agent) = launch_with(Arc::new(NamedTools(calls.clone())), settings, |builder| {
        builder
            .session_plugin(move |scope| session_typescript::ControlledGate(*scope, send.clone()));
    })
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    process_typescript::select_model(&mut client).await;
    process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
    let created = client.rpc("session.create", json!({"cwd":home})).await;
    assert_ok(&created);
    let starter = agent
        .get_session(created["result"]["sessionId"].as_str().unwrap())
        .unwrap();
    let (_, _, loader, _) = controls.recv().await.unwrap();
    let saved = tool(
        &mut client,
        &starter,
        json!({"action":"create","job":{
        "name":"pending approval","schedule":{"at":"2030-01-01T00:00:00Z"},
        "prompt":"tool:write {}","policy":"workspace-write"}}),
    )
    .await;
    let dim_agent_types::ToolContent::Text(content) = &saved.content else {
        panic!("schedule returns text")
    };
    assert_ne!(saved.is_error, Some(true), "{saved:?}");
    let saved: Value = serde_json::from_str(content).unwrap();
    let job = &saved["job"]["id"];
    let accepted = client.rpc("schedule.run_now", json!({"job_id":job})).await;
    assert_ok(&accepted);
    let run_id = &accepted["result"]["run_id"];
    let before = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let reply = client.rpc("schedule.history", json!({"job_id":job})).await;
            assert_ok(&reply);
            if reply["result"]["runs"][0]["state"] == "running" {
                return reply["result"]["runs"][0].clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let executing = agent
        .get_session(before["session_id"].as_str().unwrap())
        .unwrap();
    let request = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let events = executing.local("main", ..).await.unwrap();
            if let Some(request) = events.iter().find_map(|event| match &event.kind {
                EventKind::ApprovalRequest(request) => Some(request.approval_id.clone()),
                _ => None,
            }) {
                return request;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("WorkspaceWrite run requires approval for write");
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    loader.restart("scheduler").await.unwrap();
    process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
    let current = client.rpc("schedule.history", json!({"job_id":job})).await;
    assert_ok(&current);
    assert_eq!(
        current["result"]["runs"][0]["deadline_at"],
        before["deadline_at"]
    );
    assert_eq!(
        current["result"]["runs"][0]["input_event_id"],
        before["input_event_id"]
    );
    let events = executing.local("main", ..).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::InputMessage(_)))
            .count(),
        1
    );
    assert!(!events.iter().any(|event| matches!(
        event.kind,
        EventKind::StopInterrupt(_) | EventKind::TurnEnd(_) | EventKind::ApprovalCancelled(_)
    )));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_ok(
        &client
            .rpc(
                "approval.respond",
                json!({"sessionId":executing.id(),"branchId":"main",
        "requestId":request,"decision":"allow"}),
            )
            .await,
    );
    let completed = settled(&mut client, job, run_id).await;
    assert_eq!(completed["state"], "completed", "{completed}");
    assert_eq!(completed["session_id"], before["session_id"]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    tokio::time::timeout(Duration::from_secs(5), async {
        while reports(&starter, run_id.as_str().unwrap()).await == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    loader.restart("scheduler").await.unwrap();
    process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
    assert_eq!(reports(&starter, run_id.as_str().unwrap()).await, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_invalid_inner_records_preserve_bytes_and_do_not_block_native_sessions() {
    for text in [
        "{}",
        "{\"version\":2,\"runs\":[]}",
        "{\"version\":1,\"runs\":[{\"run_id\":\"incomplete\"}]}",
        "{\"version\":1,\"runs\":[{\"run_id\":\"done\",\"job_id\":\"job\",\"window\":\"manual\",\"state\":\"completed\",\"submission_id\":\"scheduler:done\",\"created_at\":\"2026-10-22T00:00:00.000Z\",\"deadline_at\":\"2026-10-22T01:00:00.000Z\",\"ended_at\":\"2026-10-22T00:00:01.000Z\",\"detail\":\"completed\"}]}",
    ] {
        let home = home();
        let bytes = serde_json::to_vec(&json!({"version":1,"text":text})).unwrap();
        std::fs::write(home.join("scheduler/runs.json"), &bytes).unwrap();
        let (server, _) = launch_with_agent(Arc::new(MockTools), config(&home)).await;
        let mut client = Client::new_version(&server, false, 3).await;
        process_typescript::select_model(&mut client).await;
        let failure = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let reply = client.rpc("schedule.list", json!({})).await;
                if reply["error"]["data"]["appError"] != "service_reloading" {
                    return reply;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("failed Scheduler owner: {:?}", server.diagnostics().plugins));
        assert_eq!(
            failure["error"]["data"]["appError"], "service_unavailable",
            "{failure}"
        );
        let diagnostics = server.diagnostics();
        let owner = diagnostics
            .plugins
            .iter()
            .find(|part| part.name == "@dimcode/scheduler")
            .unwrap();
        assert_eq!(owner.state, rutis::FiberState::Failed, "{owner:?}");
        assert!(
            format!("{:?}", owner.error).contains("invalid_data"),
            "{owner:?}"
        );
        for name in ["sessions", "sessionPermissions", "settings", "pluginFiles"] {
            assert!(
                owner
                    .injects
                    .iter()
                    .any(|dependency| dependency.key == rutis_bridge::session::host_key(name)),
                "module metadata retains required {name}: {owner:?}"
            );
        }
        assert_ok(
            &client
                .rpc("session.create", json!({"cwd":home,"tools":["echo"]}))
                .await,
        );
        server.shutdown();
        server.join().await.unwrap();
        assert_eq!(
            std::fs::read(home.join("scheduler/runs.json")).unwrap(),
            bytes
        );
        std::fs::remove_dir_all(home).unwrap();
    }
}
