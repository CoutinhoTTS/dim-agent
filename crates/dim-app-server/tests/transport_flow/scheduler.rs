//! Scheduler's optional package through actual Settings, C4, S2 and native history.
use super::*;
use dim_agent_capabilities::input_queue::InputQueue;
use dim_agent_runtime::{BranchGate, BranchScope, InBranch, SessionIdentity};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

struct AppliedInputGate {
    scope: BranchScope,
    held: Arc<AtomicBool>,
    entered: tokio::sync::mpsc::UnboundedSender<(String, String)>,
    release: Arc<tokio::sync::Notify>,
}
impl TypedPlugin for AppliedInputGate {
    type Deps = (
        InBranch<dim_agent_runtime::core::Dispatcher>,
        BranchGate<InputQueue>,
        InSession<SessionIdentity>,
    );
    fn name(&self) -> &str {
        "test.scheduler-applied-input"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (InBranch(dispatcher), _, InSession(identity)): Self::Deps,
    ) -> rutis::BoxFuture<'a, Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            let release = self.release.clone();
            ctx.effect(move || {
                release.notify_waiters();
                rutis::Effect::Done
            })?;
            let (held, entered, release) = (
                self.held.clone(),
                self.entered.clone(),
                self.release.clone(),
            );
            dispatcher.subscribe(ctx, "test.scheduler-applied-input", &["input/message"], move |event| {
                let (held, entered, release, session) = (held.clone(), entered.clone(), release.clone(), identity.session_id.clone());
                Box::pin(async move {
                    let scheduled = matches!(&event.kind, EventKind::InputMessage(data) if data.blocks.iter().any(|block|
                        matches!(block, dim_agent_types::InputBlock::Raw { text, .. } if text.starts_with("<dim-scheduler kind=\"scheduled-run\""))));
                    // Hold delivery after InputQueue appends applied, before Loop can consume it.
                    if scheduled && !held.swap(true, Ordering::SeqCst) {
                        entered.send((session, event.id.clone())).unwrap();
                        release.notified().await;
                    }
                    Ok(())
                })
            })?;
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for AppliedInputGate {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

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

fn probe_rows() -> dim_app_server::session_typescript::SessionTypeScriptConfig {
    let mut parts = rows();
    parts.process_rows[0]["name"] = json!(
        parts
            .anchor
            .parent()
            .unwrap()
            .join("fixtures/scheduler-package.ts")
    );
    parts.process_rows[0]["provides"]["managementMethods:schedulerprobe"] =
        json!({"describe":"async","call":"async"});
    parts
}

fn stored(home: &Path) -> Value {
    let outer: Value =
        serde_json::from_slice(&std::fs::read(home.join("scheduler/runs.json")).unwrap()).unwrap();
    assert_eq!(outer["version"], 1);
    serde_json::from_str(outer["text"].as_str().unwrap()).unwrap()
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

async fn window(client: &mut Client, job: &Value, at: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let reply = client.rpc("schedule.history", json!({"job_id":job})).await;
            assert_ok(&reply);
            if let Some(run) = reply["result"]["runs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|run| {
                    run["window"] == at
                        && !matches!(
                            run["state"].as_str(),
                            Some("launching" | "submitted" | "running")
                        )
                })
            {
                return run.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a due native Scheduler window settles")
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

async fn input_count(client: &mut Client, session: &Value, submission: &str) -> usize {
    let history = client
        .rpc(
            "session.history",
            json!({"sessionId":session,"branchId":"main","after":0,"limit":1000}),
        )
        .await;
    assert_ok(&history);
    assert_eq!(history["result"]["done"], true);
    history["result"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| {
            event["type"] == "input"
                && event["name"] == "message"
                && event["data"]["submission_id"] == submission
        })
        .count()
}

async fn host(home: &Path) -> (tokio::process::Child, Client) {
    use std::process::Stdio;
    let mut process = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "scheduler::node_scheduler_complete_host_restart_reconciles_input_and_never_replays_terminal_reports", "--ignored", "--nocapture"])
        .env("DIM_SCHEDULER_PACKAGE_HOST", home)
        .env("DIMCODE_HOME", home)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn().unwrap();
    let mut output = BufReader::new(process.stdout.take().unwrap());
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
                return PathBuf::from(socket);
            }
        }
    })
    .await
    .unwrap();
    process.stdout = Some(output.into_inner());
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
    process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
    (process, client)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_complete_host_restart_reconciles_input_and_never_replays_terminal_reports()
{
    if let Some(home) = std::env::var_os("DIM_SCHEDULER_PACKAGE_HOST") {
        let home = PathBuf::from(home);
        let mut config = config(&home);
        config.session_typescript = Some(probe_rows());
        let server = launch_with_agent(Arc::new(MockTools), config).await.0;
        println!("\n{}", json!({"socket":server.unix_path}));
        server.join().await.unwrap();
        return;
    }
    for phase in ["input_reply", "report_input", "report_reply"] {
        let home = home();
        let (mut process, mut client) = host(&home).await;
        process_typescript::select_model(&mut client).await;
        let starter = client
            .rpc("session.create", json!({"cwd":home,"tools":["echo"]}))
            .await;
        assert_ok(&starter);
        let starter = &starter["result"]["sessionId"];
        let job = client
            .rpc(
                "schedule.save",
                json!({"job":{"name":phase,"cwd":home,
            "prompt":"completed","schedule":{"at":"2030-01-01T00:00:00Z"}}}),
            )
            .await;
        assert_ok(&job);
        let job = &job["result"]["job"]["id"];
        let settings = client
            .rpc("settings.read", json!({"source":"schedule","scope":"user"}))
            .await;
        assert_ok(&settings);
        let mut document = settings["result"]["document"].clone();
        document["jobs"][0]["created_by"] =
            json!({"tool":{"session_id":starter,"branch_id":"main"}});
        document["jobs"][0]["revision"] = json!(2);
        assert_ok(
            &client
                .rpc(
                    "settings.update",
                    json!({"source":"schedule","scope":"user",
            "expectedRevision":settings["result"]["revision"],"document":document}),
                )
                .await,
        );
        let gate = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert_ok(
            &client
                .rpc(
                    "schedulerprobe.configure",
                    json!({"gate":{"phase":phase,
            "host":"127.0.0.1","port":gate.local_addr().unwrap().port()}}),
                )
                .await,
        );
        let accepted = client.rpc("schedule.run_now", json!({"job_id":job})).await;
        assert_ok(&accepted);
        let id = &accepted["result"]["run_id"];
        let (_release, _) = tokio::time::timeout(Duration::from_secs(10), gate.accept())
            .await
            .unwrap()
            .unwrap();
        let before = stored(&home)["runs"][0].clone();
        assert_eq!(before["report_to"]["session_id"], *starter);
        if phase == "input_reply" {
            assert_eq!(before["state"], "launching");
            assert!(before.get("input_event_id").is_none());
            assert_ok(
                &client
                    .rpc(
                        "session.idle",
                        json!({"sessionId":before["session_id"],"branchId":"main"}),
                    )
                    .await,
            );
        } else {
            assert_eq!(before["state"], "completed");
        }
        let submission = format!("scheduler:{}", id.as_str().unwrap());
        let report_submission = format!("scheduler-report:{}", id.as_str().unwrap());
        assert_eq!(
            input_count(&mut client, &before["session_id"], &submission).await,
            1
        );
        assert_eq!(
            input_count(&mut client, starter, &report_submission).await,
            usize::from(phase == "report_reply")
        );
        let old_pid = process.id().unwrap();
        process.kill().await.unwrap();
        process.wait().await.unwrap();
        drop(client);
        let (mut process, mut client) = host(&home).await;
        assert_ne!(process.id().unwrap(), old_pid);
        let recovered = settled(&mut client, job, id).await;
        assert_eq!(
            recovered["state"],
            "completed",
            "phase={phase}; run={recovered}; native diagnostic={}",
            client.rpc("schedulerprobe.status", json!({})).await
        );
        assert_eq!(recovered["session_id"], before["session_id"]);
        assert_eq!(recovered["deadline_at"], before["deadline_at"]);
        assert_eq!(
            input_count(&mut client, &before["session_id"], &submission).await,
            1
        );
        if phase == "input_reply" {
            tokio::time::timeout(Duration::from_secs(10), async {
                while input_count(&mut client, starter, &report_submission).await == 0 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        assert_eq!(
            input_count(&mut client, starter, &report_submission).await,
            usize::from(phase != "report_input")
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
    let permission = client
        .rpc(
            "session.permission.get",
            json!({"sessionId":starter.id(),"branchId":"main"}),
        )
        .await;
    assert_ok(&permission);
    assert_ok(&client.rpc("session.permission.update", json!({"sessionId":starter.id(),"branchId":"main",
        "expectedVersion":permission["result"]["version"],"policy":{"preset":"full-access","allowedTools":[],"disallowedTools":[]}})).await);
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
async fn node_scheduler_job_edits_cas_and_local_source_writes_preserve_accepted_snapshots() {
    use tokio::io::AsyncWriteExt;
    for mode in [
        "before_launch",
        "before_input",
        "other_job",
        "same_revision",
        "cas_conflict",
    ] {
        let home = home();
        let mut config = config(&home);
        config.session_typescript = Some(probe_rows());
        let (server, agent) = launch_with_agent(Arc::new(MockTools), config).await;
        let mut client = Client::new_version(&server, false, 3).await;
        process_typescript::select_model(&mut client).await;
        process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
        let created = client
            .rpc(
                "session.create",
                json!({"cwd":home,"permissionPolicy":{
            "preset":"workspace-write","allowedTools":["schedule"],"disallowedTools":["blocked"]}}),
            )
            .await;
        assert_ok(&created);
        let starter = agent
            .get_session(created["result"]["sessionId"].as_str().unwrap())
            .unwrap();
        let saved = tool(
            &mut client,
            &starter,
            json!({"action":"create","job":{"name":"original",
            "prompt":"original snapshot prompt","schedule":{"at":"2030-01-01T00:00:00Z"}}}),
        )
        .await;
        assert_ne!(saved.is_error, Some(true));
        let original = &saved.structured_content.as_ref().unwrap()["job"];
        let job = &original["id"];
        let other = if matches!(mode, "other_job" | "cas_conflict") {
            let reply = client
                .rpc(
                    "schedule.save",
                    json!({"job":{"name":"other","cwd":home,
                "prompt":"other job","schedule":{"at":"2030-01-01T00:00:00Z"}}}),
                )
                .await;
            assert_ok(&reply);
            Some(reply["result"]["job"]["id"].clone())
        } else {
            None
        };
        let next_saver = if mode == "same_revision" {
            let workspace = home.join("another-workspace");
            std::fs::create_dir_all(&workspace).unwrap();
            let reply = client.rpc("session.create", json!({"cwd":workspace})).await;
            assert_ok(&reply);
            Some(
                agent
                    .get_session(reply["result"]["sessionId"].as_str().unwrap())
                    .unwrap(),
            )
        } else {
            None
        };
        let sessions_before = client.rpc("session.summaries", json!({})).await;
        assert_ok(&sessions_before);
        let phase = match mode {
            "before_launch" => "launching_commit",
            "cas_conflict" => "policy_reply",
            _ => "launch_reply",
        };
        let gate = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert_ok(
            &client
                .rpc(
                    "schedulerprobe.configure",
                    json!({"gate":{"phase":phase,
            "host":"127.0.0.1","port":gate.local_addr().unwrap().port()}}),
                )
                .await,
        );
        let pending = if mode == "cas_conflict" {
            client
                .send(
                    "schedule.save",
                    json!({"job":{"id":job,"name":"must not commit"}}),
                )
                .await
        } else {
            client.send("schedule.run_now", json!({"job_id":job})).await
        };
        let (mut release, _) = tokio::time::timeout(Duration::from_secs(10), gate.accept())
            .await
            .unwrap()
            .unwrap();
        match mode {
            "other_job" | "cas_conflict" => {
                assert_ok(
                    &client
                        .rpc(
                            "schedule.save",
                            json!({"job":{"id":other.as_ref().unwrap(),"name":"changed other"}}),
                        )
                        .await,
                );
            }
            "same_revision" => {
                let settings = client
                    .rpc("settings.read", json!({"source":"schedule","scope":"user"}))
                    .await;
                assert_ok(&settings);
                let mut document = settings["result"]["document"].clone();
                let selected = document["jobs"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|item| item["id"] == *job)
                    .unwrap();
                selected["name"] = json!("configured next saver");
                selected["prompt"] = json!("new configured prompt");
                selected["created_by"] = json!({"tool":{"session_id":next_saver.as_ref().unwrap().id(),"branch_id":"main"}});
                selected["policy_cap"] =
                    json!({"preset":"full-access","allowedTools":[],"disallowedTools":[]});
                assert_eq!(selected["revision"], original["revision"]);
                assert_ok(
                    &client
                        .rpc(
                            "settings.update",
                            json!({"source":"schedule","scope":"user",
                    "expectedRevision":settings["result"]["revision"],"document":document}),
                        )
                        .await,
                );
            }
            _ => {
                assert_ok(
                    &client
                        .rpc(
                            "schedule.save",
                            json!({"job":{"id":job,"name":"changed selected"}}),
                        )
                        .await,
                );
            }
        }
        release.write_all(b"continue").await.unwrap();
        drop(release);
        let reply = client.response(pending).await;
        if mode == "cas_conflict" {
            assert_eq!(
                reply["error"]["data"]["appError"], "schedule_conflict",
                "{reply}"
            );
            let current = client
                .rpc("settings.read", json!({"source":"schedule","scope":"user"}))
                .await;
            assert_ok(&current);
            let selected = current["result"]["document"]["jobs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == *job)
                .unwrap();
            assert_eq!(
                selected, original,
                "a stale authorizing save preserves the old origin/cap and job"
            );
        } else {
            assert_ok(&reply);
            let run = settled(&mut client, job, &reply["result"]["run_id"]).await;
            if matches!(mode, "before_launch" | "before_input") {
                assert_eq!(run["state"], "skipped");
                assert_eq!(run["detail"], "changed");
                assert_eq!(reports(&starter, run["run_id"].as_str().unwrap()).await, 0);
                assert!(run.get("input_event_id").is_none());
                if mode == "before_launch" {
                    assert!(run.get("session_id").is_none());
                    let sessions_after = client.rpc("session.summaries", json!({})).await;
                    assert_ok(&sessions_after);
                    assert_eq!(
                        sessions_after["result"].as_array().unwrap().len(),
                        sessions_before["result"].as_array().unwrap().len()
                    );
                } else {
                    assert_eq!(run["session_origin"], "launch");
                    let executing = agent
                        .get_session(run["session_id"].as_str().unwrap())
                        .unwrap();
                    assert!(
                        !executing
                            .local("main", ..)
                            .await
                            .unwrap()
                            .iter()
                            .any(|event| matches!(event.kind, EventKind::InputMessage(_)))
                    );
                }
            } else {
                assert_eq!(run["state"], "completed", "{run}");
                assert_eq!(run["policy"], original["policy_cap"]);
                assert_eq!(run["report_to"]["session_id"], starter.id());
                tokio::time::timeout(Duration::from_secs(10), async {
                    while reports(&starter, run["run_id"].as_str().unwrap()).await == 0 {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .unwrap();
                let executing = agent
                    .get_session(run["session_id"].as_str().unwrap())
                    .unwrap();
                assert!(executing.local("main", ..).await.unwrap().iter().any(|event| matches!(&event.kind,
                    EventKind::InputMessage(data) if data.user.as_ref().is_some_and(|user| matches!(user.content.as_slice(),
                        [dim_agent_types::ContentBlock::Text { text }] if text == "original snapshot prompt")))));
                if let Some(next_saver) = next_saver {
                    let global = tool(
                        &mut client,
                        &next_saver,
                        json!({"action":"get","job_id":job}),
                    )
                    .await;
                    assert_ne!(global.is_error, Some(true));
                    assert_eq!(
                        global.structured_content.as_ref().unwrap()["job"]["created_by"]["tool"]["session_id"],
                        next_saver.id()
                    );
                    let next = client.rpc("schedule.run_now", json!({"job_id":job})).await;
                    assert_ok(&next);
                    let next = settled(&mut client, job, &next["result"]["run_id"]).await;
                    assert_eq!(next["state"], "completed");
                    assert_eq!(next["policy"]["preset"], "full-access");
                    assert_eq!(next["report_to"]["session_id"], next_saver.id());
                    tokio::time::timeout(Duration::from_secs(10), async {
                        while reports(&next_saver, next["run_id"].as_str().unwrap()).await == 0 {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .unwrap();
                    assert_eq!(reports(&starter, next["run_id"].as_str().unwrap()).await, 0);
                }
            }
        }
        server.shutdown();
        server.join().await.unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_shared_policy_vectors_reach_native_save_launch_and_reuse() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/permission-policy-cap.json"
    ))
    .unwrap();
    for vector in vectors["cap"].as_array().unwrap() {
        let home = home();
        let mut config = config(&home);
        let authorizing_save = vector["usage"] == "job-save-and-launch";
        if authorizing_save {
            config.session_defaults.permission_policy =
                serde_json::from_value(vector["bound"].clone()).unwrap();
        }
        let (server, _) = launch_with_agent(Arc::new(MockTools), config).await;
        let mut client = Client::new_version(&server, false, 3).await;
        process_typescript::select_model(&mut client).await;
        process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
        let before = client
            .rpc("settings.read", json!({"source":"schedule","scope":"user"}))
            .await;
        assert_ok(&before);
        let mut job = json!({"name":vector["name"],"cwd":home,"prompt":"policy check",
            "schedule":{"at":"2030-01-01T00:00:00Z"}});
        if authorizing_save {
            job["policy"] = vector["requested"]["preset"].clone();
        }
        let saved = client.rpc("schedule.save", json!({"job":job})).await;
        if authorizing_save && vector["presetWithin"] == false {
            assert_eq!(
                saved["error"]["data"]["appError"], "schedule_invalid",
                "{saved}"
            );
            let after = client
                .rpc("settings.read", json!({"source":"schedule","scope":"user"}))
                .await;
            assert_ok(&after);
            assert_eq!(after["result"], before["result"]);
        } else {
            assert_ok(&saved);
            let job = &saved["result"]["job"]["id"];
            if authorizing_save {
                assert_eq!(saved["result"]["job"]["policy_cap"], vector["bound"]);
                assert_eq!(
                    saved["result"]["transition"]["to"]["effective_policy"],
                    vector["capped"]
                );
            } else {
                let starter = client
                    .rpc(
                        "session.create",
                        json!({"cwd":home,"tools":[],"permissionPolicy":vector["bound"]}),
                    )
                    .await;
                assert_ok(&starter);
                let settings = client
                    .rpc("settings.read", json!({"source":"schedule","scope":"user"}))
                    .await;
                assert_ok(&settings);
                let mut document = settings["result"]["document"].clone();
                document["jobs"][0]["policy_cap"] = vector["requested"].clone();
                document["jobs"][0]["created_by"] = json!({"tool":{"session_id":starter["result"]["sessionId"],"branch_id":"main"}});
                document["jobs"][0]["revision"] = json!(2);
                assert_ok(
                    &client
                        .rpc(
                            "settings.update",
                            json!({"source":"schedule","scope":"user",
                    "expectedRevision":settings["result"]["revision"],"document":document}),
                        )
                        .await,
                );
            }
            let accepted = client.rpc("schedule.run_now", json!({"job_id":job})).await;
            assert_ok(&accepted);
            let run = settled(&mut client, job, &accepted["result"]["run_id"]).await;
            assert_eq!(run["state"], "completed", "{vector}: {run}");
            assert_eq!(run["policy"], vector["capped"], "{vector}");
            let actual = client
                .rpc(
                    "session.permission.get",
                    json!({"sessionId":run["session_id"],"branchId":"main"}),
                )
                .await;
            assert_ok(&actual);
            assert_eq!(actual["result"]["policy"], vector["capped"]);
        }
        server.shutdown();
        server.join().await.unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
    for vector in vectors["reuse"].as_array().unwrap() {
        let home = home();
        let mut config = config(&home);
        config.session_defaults.permission_policy =
            serde_json::from_value(vector["liveBound"].clone()).unwrap();
        let (server, _) = launch_with_agent(Arc::new(MockTools), config).await;
        let mut client = Client::new_version(&server, false, 3).await;
        process_typescript::select_model(&mut client).await;
        process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
        let target = client
            .rpc(
                "session.create",
                json!({"cwd":home,"tools":[],"permissionPolicy":vector["x"]}),
            )
            .await;
        assert_ok(&target);
        let target = &target["result"]["sessionId"];
        let saved = client.rpc("schedule.save", json!({"job":{"name":vector["name"],"cwd":home,
            "prompt":"reuse policy check","schedule":{"at":"2030-01-01T00:00:00Z"},"mode":"reuse","session_id":target}})).await;
        assert_ok(&saved);
        let job = &saved["result"]["job"]["id"];
        let settings = client
            .rpc("settings.read", json!({"source":"schedule","scope":"user"}))
            .await;
        assert_ok(&settings);
        let mut document = settings["result"]["document"].clone();
        document["jobs"][0]["policy_cap"] = vector["savedEffective"].clone();
        document["jobs"][0]["policy"] = vector["savedEffective"]["preset"].clone();
        document["jobs"][0]["revision"] = json!(2);
        assert_ok(
            &client
                .rpc(
                    "settings.update",
                    json!({"source":"schedule","scope":"user",
            "expectedRevision":settings["result"]["revision"],"document":document}),
                )
                .await,
        );
        let accepted = client.rpc("schedule.run_now", json!({"job_id":job})).await;
        assert_ok(&accepted);
        let run = settled(&mut client, job, &accepted["result"]["run_id"]).await;
        let allowed = vector["withinSaved"] == true && vector["withinLive"] == true;
        assert_eq!(
            run["state"],
            if allowed { "completed" } else { "skipped" },
            "{vector}: {run}"
        );
        if allowed {
            assert_eq!(run["session_origin"], "reuse");
        } else {
            assert_eq!(run["detail"], "policy");
        }
        assert_eq!(
            input_count(&mut client, target, run["submission_id"].as_str().unwrap()).await,
            usize::from(allowed)
        );
        let actual = client
            .rpc(
                "session.permission.get",
                json!({"sessionId":target,"branchId":"main"}),
            )
            .await;
        assert_ok(&actual);
        assert_eq!(
            actual["result"]["policy"], vector["x"],
            "reuse must not change the target policy"
        );
        server.shutdown();
        server.join().await.unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_not_held_rereads_native_deletion_completion_and_consumption() {
    use tokio::io::AsyncWriteExt;
    for outcome in ["deleted", "completed", "consumed"] {
        let home = home();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut config = config(&home);
        config.session_typescript = Some(probe_rows());
        config.session_defaults.tools = vec!["write".into(), "schedule".into()];
        let (server, agent) = launch_with_agent(Arc::new(NamedTools(calls.clone())), config).await;
        let mut client = Client::new_version(&server, false, 3).await;
        process_typescript::select_model(&mut client).await;
        process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
        let target = client
            .rpc(
                "session.create",
                json!({"cwd":home,"permissionPolicy":{"preset":"workspace-write"}}),
            )
            .await;
        assert_ok(&target);
        let target = &target["result"]["sessionId"];
        let executing = agent.get_session(target.as_str().unwrap()).unwrap();
        assert_ok(
            &client
                .rpc("session.attach", json!({"sessionId":target}))
                .await,
        );
        assert_ok(
            &client
                .rpc(
                    "session.submit",
                    json!({"sessionId":target,"branchId":"main","text":"tool:write {}"}),
                )
                .await,
        );
        let first = client.notice("approval.request").await;
        let saved = client.rpc("schedule.save", json!({"job":{"name":outcome,"cwd":home,
            "prompt":if outcome == "consumed" { "tool:write {}" } else { "queued completion" },
            "schedule":{"at":"2030-01-01T00:00:00Z"},"max_run_minutes":1,"mode":"reuse","session_id":target,"policy":"workspace-write"}})).await;
        assert_ok(&saved);
        let job = &saved["result"]["job"]["id"];
        let accepted = client.rpc("schedule.run_now", json!({"job_id":job})).await;
        assert_ok(&accepted);
        let submitted = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let history = client.rpc("schedule.history", json!({"job_id":job})).await;
                assert_ok(&history);
                let run = &history["result"]["runs"][0];
                if run["input_event_id"].is_string() {
                    return run.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let input = submitted["input_event_id"].as_str().unwrap();
        let gate = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert_ok(
            &client
                .rpc(
                    "schedulerprobe.configure",
                    json!({"now":submitted["deadline_at"],"gate":{
            "phase":"history_reply","host":"127.0.0.1","port":gate.local_addr().unwrap().port()}}),
                )
                .await,
        );
        let (mut release, _) = tokio::time::timeout(Duration::from_secs(10), gate.accept())
            .await
            .unwrap()
            .unwrap();
        if outcome == "deleted" {
            let withdrawn = client
                .rpc(
                    "session.input.withdraw",
                    json!({"sessionId":target,"branchId":"main","inputId":input}),
                )
                .await;
            assert_ok(&withdrawn);
            assert_eq!(withdrawn["result"]["status"], "deleted");
        } else {
            assert_ok(
                &client
                    .rpc(
                        "approval.respond",
                        json!({"sessionId":target,"branchId":"main",
                "requestId":first["params"]["requestId"],"decision":"allow"}),
                    )
                    .await,
            );
            if outcome == "consumed" {
                client.notice("approval.request").await;
            } else {
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let events = executing.local("main", ..).await.unwrap();
                        if let Some(index) = events.iter().position(|event| matches!(&event.kind, EventKind::InputConsumed(data) if data.input_id == input))
                            && events[index + 1..].iter().any(|event| matches!(event.kind, EventKind::TurnEnd(_))) { break; }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }).await.unwrap();
            }
        }
        release.write_all(b"continue").await.unwrap();
        drop(release);
        let ended = settled(&mut client, job, &accepted["result"]["run_id"]).await;
        assert_eq!(
            ended["state"],
            match outcome {
                "deleted" => "cancelled",
                "completed" => "completed",
                _ => "timed_out",
            },
            "{ended}"
        );
        if outcome == "deleted" {
            assert_eq!(ended["detail"], "withdrawn");
        }
        let events = executing.local("main", ..).await.unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::StopInterrupt(_)))
                .count(),
            usize::from(outcome == "consumed")
        );
        assert_eq!(events.iter().filter(|event| matches!(&event.kind, EventKind::InputWithdraw(data) if data.input_id == input)).count(), 1 + usize::from(outcome == "deleted"));
        assert_eq!(events.iter().filter(|event| matches!(&event.kind, EventKind::InputDeleted(data) if data.input_id == input)).count(), usize::from(outcome == "deleted"));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            usize::from(outcome != "deleted")
        );
        server.shutdown();
        server.join().await.unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_applied_inputs_interrupted_before_consumption_expire_without_another_stop()
{
    for (reuse, stale_prefix) in [(false, false), (false, true), (true, false), (true, true)] {
        let home = home();
        let mut config = config(&home);
        config.session_typescript = Some(probe_rows());
        let (entered, mut events) = tokio::sync::mpsc::unbounded_channel();
        let held = Arc::new(AtomicBool::new(false));
        let release = Arc::new(tokio::sync::Notify::new());
        let for_plugin = release.clone();
        let (server, agent) = launch_with(Arc::new(MockTools), config, |builder| {
            builder.timeline_plugin(move |scope| AppliedInputGate {
                scope: *scope,
                held: held.clone(),
                entered: entered.clone(),
                release: for_plugin.clone(),
            });
        })
        .await;
        let mut client = Client::new_version(&server, false, 3).await;
        process_typescript::select_model(&mut client).await;
        process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
        let mut job = json!({"name":"retained input","cwd":home,"prompt":"retained scheduled input",
            "schedule":{"at":"2030-01-01T00:00:00Z"},"max_run_minutes":1});
        if reuse {
            let target = client
                .rpc(
                    "session.create",
                    json!({"cwd":home,"permissionPolicy":{"preset":"read-only"}}),
                )
                .await;
            assert_ok(&target);
            job["mode"] = json!("reuse");
            job["session_id"] = target["result"]["sessionId"].clone();
        }
        let saved = client.rpc("schedule.save", json!({"job":job})).await;
        assert_ok(&saved);
        let job = &saved["result"]["job"]["id"];
        let accepted = client.rpc("schedule.run_now", json!({"job_id":job})).await;
        assert_ok(&accepted);
        let (session, input) = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .unwrap()
            .unwrap();
        let executing = agent.get_session(&session).unwrap();
        let history_reply = if stale_prefix {
            let gate = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            assert_ok(&client.rpc("schedulerprobe.configure", json!({"gate":{
                "phase":"history_reply","host":"127.0.0.1","port":gate.local_addr().unwrap().port()}})).await);
            Some(
                tokio::time::timeout(Duration::from_secs(10), gate.accept())
                    .await
                    .unwrap()
                    .unwrap()
                    .0,
            )
        } else {
            None
        };
        executing
            .interrupt("main", "user interrupt before consumption".into())
            .unwrap();
        release.notify_one();
        assert_ok(
            &client
                .rpc(
                    "session.idle",
                    json!({"sessionId":session,"branchId":"main"}),
                )
                .await,
        );
        assert_ok(
            &client
                .rpc(
                    "schedulerprobe.configure",
                    json!({"now":accepted["result"]["deadline_at"]}),
                )
                .await,
        );
        if let Some(mut reply) = history_reply {
            reply.write_all(b"prefix before interrupt").await.unwrap();
        }
        let ended = settled(&mut client, job, &accepted["result"]["run_id"]).await;
        assert_eq!(ended["state"], "timed_out", "{ended}");
        assert_eq!(ended["detail"], "interrupted_before_consumption");
        let before = executing.local("main", ..).await.unwrap();
        assert!(before.iter().any(
            |event| matches!(&event.kind, EventKind::InputApplied(data) if data.input_id == input)
        ));
        assert!(!before.iter().any(
            |event| matches!(&event.kind, EventKind::InputConsumed(data) if data.input_id == input)
        ));
        let unexpected: Vec<_> = before
            .iter()
            .filter_map(|event| match event.kind {
                EventKind::InputDeleted(_) => Some(("input deleted", event.sequence)),
                EventKind::ModelCall(_) => Some(("model called", event.sequence)),
                _ => None,
            })
            .collect();
        assert!(
            unexpected.is_empty(),
            "applied input changed: {unexpected:?}"
        );
        assert_eq!(
            before
                .iter()
                .filter(|event| matches!(event.kind, EventKind::StopInterrupt(_)))
                .count(),
            1
        );
        if reuse {
            assert_ok(
                &client
                    .rpc(
                        "session.submit",
                        json!({"sessionId":session,"branchId":"main","text":"later driving input"}),
                    )
                    .await,
            );
            assert_ok(
                &client
                    .rpc(
                        "session.idle",
                        json!({"sessionId":session,"branchId":"main"}),
                    )
                    .await,
            );
            let after = executing.local("main", ..).await.unwrap();
            assert!(after.iter().any(|event| matches!(&event.kind, EventKind::InputConsumed(data) if data.input_id == input)));
            let current = settled(&mut client, job, &ended["run_id"]).await;
            assert_eq!(
                current, ended,
                "later consumption cannot change a terminal run"
            );
        } else {
            let next = client.rpc("schedule.run_now", json!({"job_id":job})).await;
            assert_ok(&next);
            assert_eq!(
                settled(&mut client, job, &next["result"]["run_id"]).await["state"],
                "completed"
            );
        }
        server.shutdown();
        server.join().await.unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_overlap_capacity_and_busy_reuse_follow_native_input_boundaries() {
    let home = home();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut config = config(&home);
    config.session_typescript = Some(probe_rows());
    config.session_defaults.tools = vec!["write".into(), "schedule".into()];
    let (server, _) = launch_with_agent(Arc::new(NamedTools(calls.clone())), config).await;
    let mut client = Client::new_version(&server, false, 3).await;
    process_typescript::select_model(&mut client).await;
    process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-10-22T00:00:00Z"}),
            )
            .await,
    );
    let settings = client
        .rpc("settings.read", json!({"source":"schedule","scope":"user"}))
        .await;
    assert_ok(&settings);
    assert_ok(&client.rpc("settings.update", json!({"source":"schedule","scope":"user",
        "expectedRevision":settings["result"]["revision"],"document":{"jobs":[],"max_concurrent_runs":1}})).await);
    let saved = client.rpc("schedule.save", json!({"job":{"name":"overlap","cwd":home,
        "prompt":"tool:write {}","policy":"workspace-write","max_run_minutes":3,"schedule":{"every_minutes":1}}})).await;
    assert_ok(&saved);
    let job = &saved["result"]["job"]["id"];
    let first = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let history = client.rpc("schedule.history", json!({"job_id":job})).await;
            assert_ok(&history);
            if history["result"]["runs"][0]["input_event_id"].is_string() {
                return history["result"]["runs"][0].clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_ok(
        &client
            .rpc("session.attach", json!({"sessionId":first["session_id"]}))
            .await,
    );
    client.notice("approval.request").await;
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-10-22T00:01:00Z"}),
            )
            .await,
    );
    let skipped = window(&mut client, job, "2026-10-22T00:01:00.000Z").await;
    assert_eq!(skipped["state"], "skipped");
    assert_eq!(skipped["detail"], "overlap");
    let other = client
        .rpc(
            "schedule.save",
            json!({"job":{"name":"capacity","cwd":home,
        "prompt":"later","schedule":{"at":"2030-01-01T00:00:00Z"}}}),
        )
        .await;
    assert_ok(&other);
    let other = &other["result"]["job"]["id"];
    let capacity = client
        .rpc("schedule.run_now", json!({"job_id":other}))
        .await;
    assert_ok(&capacity);
    assert_eq!(capacity["result"]["state"], "skipped");
    assert_eq!(capacity["result"]["detail"], "capacity");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_ok(
        &client
            .rpc(
                "session.stop",
                json!({"sessionId":first["session_id"],"branchId":"main"}),
            )
            .await,
    );
    assert_eq!(
        settled(&mut client, job, &first["run_id"]).await["state"],
        "cancelled"
    );
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-10-22T00:02:00Z"}),
            )
            .await,
    );
    let next = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let history = client.rpc("schedule.history", json!({"job_id":job})).await;
            assert_ok(&history);
            if let Some(run) = history["result"]["runs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|run| {
                    run["window"] == "2026-10-22T00:02:00.000Z" && run["input_event_id"].is_string()
                })
            {
                return run.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_ok(
        &client
            .rpc("session.attach", json!({"sessionId":next["session_id"]}))
            .await,
    );
    let approval = client.notice("approval.request").await;
    assert_ok(
        &client
            .rpc(
                "approval.respond",
                json!({"sessionId":next["session_id"],"branchId":"main",
        "requestId":approval["params"]["requestId"],"decision":"allow"}),
            )
            .await,
    );
    assert_eq!(
        settled(&mut client, job, &next["run_id"]).await["state"],
        "completed"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let allowed = client
        .rpc("schedule.run_now", json!({"job_id":other}))
        .await;
    assert_ok(&allowed);
    assert_eq!(
        settled(&mut client, other, &allowed["result"]["run_id"]).await["state"],
        "completed"
    );

    let target = client
        .rpc(
            "session.create",
            json!({"cwd":home,"permissionPolicy":{"preset":"workspace-write"}}),
        )
        .await;
    assert_ok(&target);
    let target = &target["result"]["sessionId"];
    assert_ok(
        &client
            .rpc("session.attach", json!({"sessionId":target}))
            .await,
    );
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":target,"branchId":"main","text":"tool:write {}"}),
            )
            .await,
    );
    let first_approval = client.notice("approval.request").await;
    let queued = client.rpc("schedule.save", json!({"job":{"name":"queued","cwd":home,"prompt":"tool:write {}",
        "policy":"workspace-write","schedule":{"at":"2030-01-01T00:00:00Z"},"mode":"reuse","session_id":target}})).await;
    assert_ok(&queued);
    let queued = &queued["result"]["job"]["id"];
    let accepted = client
        .rpc("schedule.run_now", json!({"job_id":queued}))
        .await;
    assert_ok(&accepted);
    let waiting = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let history = client
                .rpc("schedule.history", json!({"job_id":queued}))
                .await;
            assert_ok(&history);
            let run = &history["result"]["runs"][0];
            if run["input_event_id"].is_string() {
                return run.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(waiting["state"], "submitted");
    let history = client
        .rpc(
            "session.history",
            json!({"sessionId":target,"branchId":"main","after":0,"limit":1000}),
        )
        .await;
    assert_ok(&history);
    assert!(
        !history["result"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| {
                event["type"] == "input"
                    && event["name"] == "consumed"
                    && event["data"]["input_id"] == waiting["input_event_id"]
            })
    );
    assert_ok(
        &client
            .rpc(
                "approval.respond",
                json!({"sessionId":target,"branchId":"main",
        "requestId":first_approval["params"]["requestId"],"decision":"allow"}),
            )
            .await,
    );
    let second_approval = client.notice("approval.request").await;
    assert_ne!(
        second_approval["params"]["requestId"],
        first_approval["params"]["requestId"]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let active = client
        .rpc("schedule.history", json!({"job_id":queued}))
        .await;
    assert_ok(&active);
    assert!(
        matches!(
            active["result"]["runs"][0]["state"].as_str(),
            Some("submitted" | "running")
        ),
        "the earlier turn end is not this run's outcome: {active}"
    );
    assert_ok(
        &client
            .rpc(
                "approval.respond",
                json!({"sessionId":target,"branchId":"main",
        "requestId":second_approval["params"]["requestId"],"decision":"allow"}),
            )
            .await,
    );
    let done = settled(&mut client, queued, &accepted["result"]["run_id"]).await;
    assert_eq!(done["state"], "completed");
    assert_eq!(done["session_id"], *target);
    assert_eq!(done["session_origin"], "reuse");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_late_native_admission_and_policy_replies_preserve_identity_without_input() {
    use tokio::io::AsyncWriteExt;
    for phase in ["launch_reply", "reuse_reply", "policy_reply"] {
        let home = home();
        let mut config = config(&home);
        config.session_typescript = Some(probe_rows());
        let (server, agent) = launch_with_agent(Arc::new(MockTools), config).await;
        let mut client = Client::new_version(&server, false, 3).await;
        process_typescript::select_model(&mut client).await;
        process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
        let mut job = json!({"name":phase,"cwd":home,"prompt":"must not be submitted",
            "schedule":{"at":"2030-01-01T00:00:00Z"},"max_run_minutes":1});
        if phase != "launch_reply" {
            let target = client
                .rpc(
                    "session.create",
                    json!({"cwd":home,"permissionPolicy":{"preset":"read-only"}}),
                )
                .await;
            assert_ok(&target);
            job["mode"] = json!("reuse");
            job["session_id"] = target["result"]["sessionId"].clone();
        }
        let job = client.rpc("schedule.save", json!({"job":job})).await;
        assert_ok(&job);
        let job = &job["result"]["job"]["id"];
        let gate = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert_ok(
            &client
                .rpc(
                    "schedulerprobe.configure",
                    json!({"gate":{"phase":phase,
            "host":"127.0.0.1","port":gate.local_addr().unwrap().port()}}),
                )
                .await,
        );
        let accepted = client.rpc("schedule.run_now", json!({"job_id":job})).await;
        assert_ok(&accepted);
        let (mut release, _) = tokio::time::timeout(Duration::from_secs(10), gate.accept())
            .await
            .unwrap()
            .unwrap();
        assert_ok(
            &client
                .rpc(
                    "schedulerprobe.configure",
                    json!({"now":accepted["result"]["deadline_at"]}),
                )
                .await,
        );
        release.write_all(b"continue").await.unwrap();
        drop(release);
        let run = settled(&mut client, job, &accepted["result"]["run_id"]).await;
        assert_eq!(run["state"], "timed_out", "{run}");
        assert_eq!(run["detail"], "deadline_before_input");
        assert_eq!(
            run["session_origin"],
            if phase == "launch_reply" {
                "launch"
            } else {
                "reuse"
            }
        );
        assert_eq!(run["policy"]["preset"], "read-only");
        assert_eq!(run["deadline_at"], accepted["result"]["deadline_at"]);
        assert!(run.get("input_event_id").is_none());
        let session = agent
            .get_session(run["session_id"].as_str().unwrap())
            .unwrap();
        let events = session.local("main", ..).await.unwrap();
        assert!(!events.iter().any(|event| matches!(
            event.kind,
            EventKind::InputMessage(_) | EventKind::InputWithdraw(_) | EventKind::StopInterrupt(_)
        )));
        server.shutdown();
        server.join().await.unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_reports_drain_native_writers_before_row_or_runtime_cleanup() {
    use tokio::io::AsyncWriteExt;
    for kill_runtime in [false, true] {
        let home = home();
        let (send, mut controls) = tokio::sync::mpsc::unbounded_channel();
        let mut config = config(&home);
        config.session_typescript = Some(probe_rows());
        let (server, agent) = launch_with(Arc::new(MockTools), config, |builder| {
            builder.session_plugin(move |scope| {
                session_typescript::ControlledGate(*scope, send.clone())
            });
        })
        .await;
        let mut client = Client::new_version(&server, false, 3).await;
        process_typescript::select_model(&mut client).await;
        process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
        let created = client.rpc("session.create", json!({"cwd":home})).await;
        assert_ok(&created);
        let target = agent
            .get_session(created["result"]["sessionId"].as_str().unwrap())
            .unwrap();
        let (_, _, loader, _) = controls.recv().await.unwrap();
        let saved = tool(
            &mut client,
            &target,
            json!({"action":"create","job":{"name":"drain",
            "prompt":"completed","schedule":{"at":"2030-01-01T00:00:00Z"}}}),
        )
        .await;
        assert_ne!(saved.is_error, Some(true));
        let job = &saved.structured_content.as_ref().unwrap()["job"]["id"];
        let before = client.rpc("schedulerprobe.status", json!({})).await;
        assert_ok(&before);
        let gate = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        assert_ok(
            &client
                .rpc(
                    "schedulerprobe.configure",
                    json!({"gate":{"phase":"report_input",
            "host":"127.0.0.1","port":gate.local_addr().unwrap().port()}}),
                )
                .await,
        );
        let accepted = client.rpc("schedule.run_now", json!({"job_id":job})).await;
        assert_ok(&accepted);
        let id = accepted["result"]["run_id"].as_str().unwrap();
        let (mut release, _) = tokio::time::timeout(Duration::from_secs(10), gate.accept())
            .await
            .unwrap()
            .unwrap();
        let run = stored(&home)["runs"][0].clone();
        assert_eq!(run["state"], "completed");
        assert_ok(&client.rpc("schedule.delete", json!({"job_id":job})).await);
        assert_eq!(stored(&home)["runs"][0]["run_id"], id);
        let connection = rusqlite::Connection::open(home.join("native.sqlite")).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        release.write_all(b"continue").await.unwrap();
        drop(release);
        tokio::time::timeout(Duration::from_secs(5), async {
            while !target.publication("main").unwrap().has_pending() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let owner = loader.get("scheduler").unwrap().view.unwrap();
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
        let (restarted, ()) = tokio::join!(
            loader.restart(if kill_runtime {
                "SessionNodeRuntime"
            } else {
                "scheduler"
            }),
            async {
                let mut state = owner.watch();
                tokio::time::timeout(Duration::from_secs(5), async {
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
                    "native writer remains owned during cleanup"
                );
                assert_eq!(stored(&home)["runs"][0]["run_id"], id);
                connection.execute_batch("COMMIT").unwrap();
            }
        );
        restarted.unwrap();
        process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
        assert_eq!(reports(&target, id).await, 1);
        tokio::time::timeout(Duration::from_secs(10), async {
            while !stored(&home)["runs"].as_array().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let summaries = client.rpc("session.summaries", json!({})).await;
        assert_ok(&summaries);
        assert!(
            !summaries["result"]
                .as_array()
                .unwrap()
                .iter()
                .any(|session| session["sessionId"] == run["session_id"])
        );
        server.shutdown();
        server.join().await.unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_retention_retries_real_delete_and_record_failures_with_no_jobs() {
    use std::os::unix::fs::PermissionsExt;
    for fault in ["delete", "record"] {
        let home = home();
        let mut config = config(&home);
        config.session_typescript = Some(probe_rows());
        let (server, _) = launch_with_agent(Arc::new(MockTools), config).await;
        let mut client = Client::new_version(&server, false, 3).await;
        process_typescript::select_model(&mut client).await;
        process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
        let saved = client
            .rpc(
                "schedule.save",
                json!({"job":{"name":"owner","cwd":home,"prompt":"completed",
            "schedule":{"at":"2030-01-01T00:00:00Z"}}}),
            )
            .await;
        assert_ok(&saved);
        let job = &saved["result"]["job"]["id"];
        let accepted = client.rpc("schedule.run_now", json!({"job_id":job})).await;
        assert_ok(&accepted);
        let run = settled(&mut client, job, &accepted["result"]["run_id"]).await;
        let owned = &run["session_id"];
        let reused = client
            .rpc(
                "schedule.save",
                json!({"job":{"name":"reference","cwd":home,"prompt":"later",
            "schedule":{"at":"2030-01-01T00:00:00Z"},"mode":"reuse","session_id":owned}}),
            )
            .await;
        assert_ok(&reused);
        let reused = &reused["result"]["job"]["id"];
        assert_ok(&client.rpc("schedule.delete", json!({"job_id":job})).await);
        assert_eq!(stored(&home)["runs"][0]["session_id"], *owned);
        let external = client
            .rpc("session.create", json!({"cwd":home,"tools":[]}))
            .await;
        assert_ok(&external);
        let external = &external["result"]["sessionId"];
        let output = home
            .join("command-output")
            .join(owned.as_str().unwrap())
            .join("locked");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("exec.log"), b"owned output").unwrap();
        let locked = if fault == "delete" {
            output.clone()
        } else {
            home.join("scheduler")
        };
        let permissions = std::fs::metadata(&locked).unwrap().permissions();
        let bytes = std::fs::read(home.join("scheduler/runs.json")).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let denied = std::fs::write(locked.join("probe"), b"")
            .unwrap_err()
            .kind();
        let deleted = client
            .rpc("schedule.delete", json!({"job_id":reused}))
            .await;
        let retained = std::fs::read(home.join("scheduler/runs.json")).unwrap();
        let observed = client.rpc("schedulerprobe.status", json!({})).await;
        let availability = if fault == "delete" {
            client
                .rpc("session.attach", json!({"sessionId":owned}))
                .await
        } else {
            Value::Null
        };
        std::fs::set_permissions(&locked, permissions).unwrap();
        assert_eq!(denied, std::io::ErrorKind::PermissionDenied);
        assert_ok(&deleted);
        assert_ok(&observed);
        assert_eq!(retained, bytes);
        if fault == "record" {
            assert_eq!(observed["result"]["replacementError"], "io");
        } else {
            assert!(output.join("exec.log").exists());
            assert_eq!(availability["error"]["data"]["appError"], "session_closed");
        }
        let listed = client.rpc("schedule.list", json!({})).await;
        assert_eq!(listed["result"]["jobs"], json!([]));
        assert_ok(
            &client
                .rpc(
                    "schedulerprobe.configure",
                    json!({"now":run["deadline_at"]}),
                )
                .await,
        );
        tokio::time::timeout(Duration::from_secs(10), async {
            while !stored(&home)["runs"].as_array().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(!output.exists());
        let summaries = client.rpc("session.summaries", json!({})).await;
        assert_ok(&summaries);
        let sessions = summaries["result"].as_array().unwrap();
        assert!(
            !sessions
                .iter()
                .any(|session| session["sessionId"] == *owned)
        );
        assert!(
            sessions
                .iter()
                .any(|session| session["sessionId"] == *external)
        );
        server.shutdown();
        server.join().await.unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_native_deadlines_withdraw_held_input_and_stop_only_consumed_work() {
    for mode in ["held", "consumed", "user_stop"] {
        let home = home();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut config = config(&home);
        config.session_typescript = Some(probe_rows());
        config.session_defaults.tools = vec!["write".into(), "schedule".into()];
        let (server, agent) = launch_with_agent(Arc::new(NamedTools(calls.clone())), config).await;
        let mut client = Client::new_version(&server, false, 3).await;
        process_typescript::select_model(&mut client).await;
        process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
        let mut job = json!({"name":mode,"cwd":home,"prompt":"tool:write {}",
            "schedule":{"at":"2030-01-01T00:00:00Z"},"policy":"workspace-write","max_run_minutes":1});
        let mut first_approval = Value::Null;
        if mode == "held" {
            let target = client
                .rpc(
                    "session.create",
                    json!({"cwd":home,"permissionPolicy":{"preset":"workspace-write"}}),
                )
                .await;
            assert_ok(&target);
            let target = &target["result"]["sessionId"];
            assert_ok(
                &client
                    .rpc("session.attach", json!({"sessionId":target}))
                    .await,
            );
            assert_ok(
                &client
                    .rpc(
                        "session.submit",
                        json!({"sessionId":target,"branchId":"main","text":"tool:write {}"}),
                    )
                    .await,
            );
            first_approval = client.notice("approval.request").await["params"]["requestId"].clone();
            job["mode"] = json!("reuse");
            job["session_id"] = target.clone();
            job["prompt"] = json!("queued run");
        }
        let job = client.rpc("schedule.save", json!({"job":job})).await;
        assert_ok(&job);
        let job = &job["result"]["job"]["id"];
        let accepted = client.rpc("schedule.run_now", json!({"job_id":job})).await;
        assert_ok(&accepted);
        let id = &accepted["result"]["run_id"];
        let submitted = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let history = client.rpc("schedule.history", json!({"job_id":job})).await;
                assert_ok(&history);
                let run = &history["result"]["runs"][0];
                if run["input_event_id"].is_string() {
                    return run.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let session = &submitted["session_id"];
        let executing = agent.get_session(session.as_str().unwrap()).unwrap();
        if mode != "held" {
            assert_ok(
                &client
                    .rpc("session.attach", json!({"sessionId":session}))
                    .await,
            );
            client.notice("approval.request").await;
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let own_input = submitted["input_event_id"].as_str().unwrap();
        let events = executing.local("main", ..).await.unwrap();
        assert_eq!(events.iter().any(|event| matches!(&event.kind, EventKind::InputConsumed(data) if data.input_id == own_input)), mode != "held");
        if mode == "user_stop" {
            assert_ok(
                &client
                    .rpc(
                        "session.stop",
                        json!({"sessionId":session,"branchId":"main"}),
                    )
                    .await,
            );
        } else {
            assert_ok(
                &client
                    .rpc(
                        "schedulerprobe.configure",
                        json!({"now":submitted["deadline_at"]}),
                    )
                    .await,
            );
        }
        let ended = settled(&mut client, job, id).await;
        assert_eq!(
            ended["state"],
            if mode == "user_stop" {
                "cancelled"
            } else {
                "timed_out"
            },
            "{ended}"
        );
        assert_eq!(ended["deadline_at"], submitted["deadline_at"]);
        assert!(ended.get("report_to").is_none());
        let events = executing.local("main", ..).await.unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::StopInterrupt(_)))
                .count(),
            usize::from(mode != "held")
        );
        if mode == "held" {
            assert!(events.iter().any(|event| matches!(&event.kind, EventKind::InputDeleted(data) if data.input_id == own_input)));
            assert_ok(
                &client
                    .rpc(
                        "approval.respond",
                    json!({"sessionId":session,"branchId":"main","requestId":first_approval,"decision":"allow"}),
                    )
                    .await,
            );
            assert_ok(
                &client
                    .rpc(
                        "session.idle",
                        json!({"sessionId":session,"branchId":"main"}),
                    )
                    .await,
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            let events = executing.local("main", ..).await.unwrap();
            assert!(!events.iter().any(|event| matches!(&event.kind, EventKind::InputConsumed(data) if data.input_id == own_input)));
        }
        server.shutdown();
        server.join().await.unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires DIM_TEST_NODE and DIM_TEST_JS_RUNTIME"]
async fn node_scheduler_native_clock_preserves_anchors_collapses_gaps_and_handles_dst_once() {
    let home = home();
    let mut config = config(&home);
    config.session_typescript = Some(probe_rows());
    let (server, _) = launch_with_agent(Arc::new(MockTools), config).await;
    let mut client = Client::new_version(&server, false, 3).await;
    process_typescript::select_model(&mut client).await;
    process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-03-08T06:59:00Z"}),
            )
            .await,
    );
    let gap = client
        .rpc(
            "schedule.save",
            json!({"job":{"name":"gap","cwd":home,"prompt":"clock run",
        "schedule":{"cron":"30 2 * * *","tz":"America/New_York"}}}),
        )
        .await;
    assert_ok(&gap);
    let gap = &gap["result"]["job"]["id"];
    let listed = client.rpc("schedule.list", json!({})).await;
    assert_eq!(
        listed["result"]["jobs"][0]["next_window"],
        "2026-03-09T06:30:00.000Z"
    );
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-03-08T07:30:00Z"}),
            )
            .await,
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let history = client.rpc("schedule.history", json!({"job_id":gap})).await;
    assert_eq!(history["result"]["runs"], json!([]));
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-03-09T06:30:00Z"}),
            )
            .await,
    );
    assert_eq!(
        window(&mut client, gap, "2026-03-09T06:30:00.000Z").await["state"],
        "completed"
    );
    assert_ok(
        &client
            .rpc(
                "schedule.set_enabled",
                json!({"job_id":gap,"enabled":false}),
            )
            .await,
    );

    let once = client
        .rpc(
            "schedule.save",
            json!({"job":{"name":"once","cwd":home,"prompt":"one run",
        "schedule":{"at":"2026-03-09T06:31:00Z"}}}),
        )
        .await;
    assert_ok(&once);
    let once = &once["result"]["job"]["id"];
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-03-09T06:31:00Z"}),
            )
            .await,
    );
    assert_eq!(
        window(&mut client, once, "2026-03-09T06:31:00.000Z").await["state"],
        "completed"
    );
    let listed = client.rpc("schedule.list", json!({})).await;
    let job = listed["result"]["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["id"] == *once)
        .unwrap();
    assert_eq!(job["enabled"], false);
    assert!(job.get("enabled_at").is_none());

    let every = client
        .rpc(
            "schedule.save",
            json!({"job":{"name":"every","cwd":home,"prompt":"interval run",
        "schedule":{"every_minutes":2}}}),
        )
        .await;
    assert_ok(&every);
    let original = &every["result"]["job"];
    let every = &original["id"];
    assert_eq!(
        window(&mut client, every, "2026-03-09T06:31:00.000Z").await["state"],
        "completed"
    );
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-03-09T06:32:30Z"}),
            )
            .await,
    );
    let renamed = client
        .rpc(
            "schedule.save",
            json!({"job":{"id":every,"name":"same anchor"}}),
        )
        .await;
    assert_ok(&renamed);
    assert_eq!(
        renamed["result"]["job"]["enabled_at"],
        original["enabled_at"]
    );
    assert_ok(
        &client
            .rpc(
                "schedule.set_enabled",
                json!({"job_id":every,"enabled":false}),
            )
            .await,
    );
    let enabled = client
        .rpc(
            "schedule.set_enabled",
            json!({"job_id":every,"enabled":true}),
        )
        .await;
    assert_ok(&enabled);
    assert_eq!(
        enabled["result"]["job"]["enabled_at"],
        "2026-03-09T06:32:30.000Z"
    );
    assert_eq!(
        window(&mut client, every, "2026-03-09T06:32:30.000Z").await["state"],
        "completed"
    );
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-03-09T06:40:00Z"}),
            )
            .await,
    );
    assert_eq!(
        window(&mut client, every, "2026-03-09T06:38:30.000Z").await["state"],
        "missed"
    );
    let history = client
        .rpc("schedule.history", json!({"job_id":every}))
        .await;
    assert_eq!(history["result"]["runs"].as_array().unwrap().len(), 3);
    assert_ok(
        &client
            .rpc(
                "schedule.set_enabled",
                json!({"job_id":every,"enabled":false}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc(
                "schedule.set_enabled",
                json!({"job_id":once,"enabled":true}),
            )
            .await,
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let history = client.rpc("schedule.history", json!({"job_id":once})).await;
    assert_eq!(history["result"]["runs"].as_array().unwrap().len(), 1);

    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-11-01T05:29:00Z"}),
            )
            .await,
    );
    let fold = client
        .rpc(
            "schedule.save",
            json!({"job":{"name":"fold","cwd":home,"prompt":"fold run",
        "schedule":{"cron":"30 1 * * *","tz":"America/New_York"}}}),
        )
        .await;
    assert_ok(&fold);
    let fold = &fold["result"]["job"]["id"];
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-11-01T05:30:00Z"}),
            )
            .await,
    );
    assert_eq!(
        window(&mut client, fold, "2026-11-01T05:30:00.000Z").await["state"],
        "completed"
    );
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-11-01T06:30:00Z"}),
            )
            .await,
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let history = client.rpc("schedule.history", json!({"job_id":fold})).await;
    assert_eq!(history["result"]["runs"].as_array().unwrap().len(), 1);
    assert_ok(
        &client
            .rpc(
                "schedulerprobe.configure",
                json!({"now":"2026-11-02T06:30:00Z"}),
            )
            .await,
    );
    assert_eq!(
        window(&mut client, fold, "2026-11-02T06:30:00.000Z").await["state"],
        "completed"
    );
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

    let generation = || {
        server
            .diagnostics()
            .plugins
            .into_iter()
            .find(|plugin| plugin.name == "@dimcode/scheduler")
            .unwrap()
            .generation
    };
    let loop_generation = || {
        executing
            .branch_diagnostics("main")
            .unwrap()
            .plugins
            .into_iter()
            .find(|plugin| plugin.name == "Loop")
            .unwrap()
            .generation
    };
    let (owner_before, loop_before) = (generation(), loop_generation());
    server.restart("Agents").await.unwrap();
    assert_eq!(
        generation(),
        owner_before,
        "the stable service does not retire Scheduler on index reload"
    );
    assert_eq!(
        loop_generation(),
        loop_before,
        "the existing native run remains loaded"
    );
    loader.restart("scheduler").await.unwrap();
    process_typescript::files_available(&mut client, "schedule.list", json!({})).await;
    let listed = client.rpc("schedule.list", json!({})).await;
    assert_ok(&listed);
    assert_eq!(
        listed["result"]["jobs"][0]["enabled_at"],
        saved["job"]["enabled_at"]
    );
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
