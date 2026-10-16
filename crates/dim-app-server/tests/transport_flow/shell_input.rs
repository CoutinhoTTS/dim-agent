//! ShellInput through the public RPC interface (design Appendix D).
use super::*;
use dim_agent_capabilities::{
    model_provider::ModelProviderPlugin, workspace_tools::WorkspaceToolsConfig,
};
use std::{
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

/// A local-execution server, ShellInput included, with an isolated home whose
/// exec shell is `/bin/sh`, and one Session working in `work`.
struct Shell {
    server: RunningServer,
    client: Client,
    sid: String,
    home: PathBuf,
    work: PathBuf,
}
impl Shell {
    async fn launch() -> Self {
        let home = PathBuf::from("/tmp").join(format!("dsh-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&home)
            .unwrap();
        let work = home.join("work");
        std::fs::create_dir_all(&work).unwrap();
        set_shell(&home, "/bin/sh");
        let mut builder = AgentBuilder::new(AgentConfig::default());
        defaults::install_local_execution(
            &mut builder,
            ModelProviderPlugin::new,
            WorkspaceToolsConfig {
                output_root: home.join("rust/outputs"),
            },
        );
        defaults::mock_model_hosts(&mut builder);
        let server = serve(
            builder,
            ServerConfig {
                product_home: Some(home.clone()),
                target_home: Some(home.join("rust")),
                unix_path: Some(home.join("rust/daemon.sock")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut client = Client::new_version(&server, false, 3).await;
        // A Session needs a model selection; the mock model host answers it.
        let capabilities = dim_agent_types::ModelCapabilities {
            context_window: 32_000,
            max_output_tokens: 2048,
            ..Default::default()
        };
        let model = json!({"modelId": "fixture", "displayName": "Fixture", "status": "active", "capabilities": capabilities});
        assert_ok(&client.rpc("provider.save", json!({"provider": {
            "providerId": "fixture", "displayName": "Fixture", "driverKind": "openai-compatible",
            "baseUrl": "https://model.example/v1", "auth": {"type": "bearer"},
            "credential": {"action": "set", "value": "fixture-access"},
            "models": [model], "enabled": true,
        }})).await);
        let created = client
            .rpc("session.create", json!({"cwd": work, "tools": []}))
            .await;
        let sid = created["result"]["sessionId"]
            .as_str()
            .unwrap_or_else(|| panic!("{created}"))
            .to_owned();
        Self {
            server,
            client,
            sid,
            home,
            work,
        }
    }
    fn run_params(&self, run_id: &str, command: &str, mode: &str) -> Value {
        json!({"sessionId": self.sid, "runId": run_id, "command": command, "mode": mode})
    }
    async fn run(&mut self, run_id: &str, command: &str, mode: &str) -> Value {
        let params = self.run_params(run_id, command, mode);
        self.client.rpc("shell.run", params).await
    }
    async fn send_run(&mut self, run_id: &str, command: &str, mode: &str) -> u64 {
        let params = self.run_params(run_id, command, mode);
        self.client.send("shell.run", params).await
    }
    async fn list(&mut self) -> Value {
        let sid = self.sid.clone();
        let reply = self
            .client
            .rpc("shell.list", json!({"sessionId": sid}))
            .await;
        reply["result"]["runs"].clone()
    }
    async fn stop(&mut self, run_id: &str) -> Value {
        let sid = self.sid.clone();
        self.client
            .rpc("shell.stop", json!({"sessionId": sid, "runId": run_id}))
            .await
    }
    /// Main-branch events after `after`, once the branch has dispatched them.
    async fn events(&mut self, after: u64) -> Vec<Value> {
        let sid = self.sid.clone();
        assert_ok(
            &self
                .client
                .rpc(
                    "session.idle",
                    json!({"sessionId": sid, "branchId": "main"}),
                )
                .await,
        );
        let page = self
            .client
            .rpc(
                "session.history",
                json!({"sessionId": sid, "branchId": "main", "after": after, "limit": 1000}),
            )
            .await;
        page["result"]["events"].as_array().unwrap().clone()
    }
    /// The recorded inputs among main-branch events after `after`.
    async fn recorded(&mut self, after: u64) -> Vec<Value> {
        self.events(after)
            .await
            .into_iter()
            .filter(|event| {
                event["type"] == "input"
                    && event["name"] == "message"
                    && event["data"]["blocks"][0]["text"]
                        .as_str()
                        .is_some_and(|text| text.starts_with("<dim-shell-input "))
            })
            .collect()
    }
    /// The last sequence of the main branch, read through `session.history`.
    async fn through(&mut self) -> u64 {
        let sid = self.sid.clone();
        let page = self
            .client
            .rpc(
                "session.history",
                json!({"sessionId": sid, "branchId": "main", "after": 0, "limit": 1000}),
            )
            .await;
        page["result"]["through"].as_u64().unwrap()
    }
    async fn shutdown(self) {
        drop(self.client);
        self.server.shutdown();
        self.server.join().await.unwrap();
        std::fs::remove_dir_all(self.home).unwrap();
    }
}

fn set_shell(home: &Path, executable: &str) {
    std::fs::write(
        home.join("exec-shell.json"),
        json!({"version": 1, "executablePath": executable}).to_string(),
    )
    .unwrap();
}

fn app_error(reply: &Value) -> &str {
    reply["error"]["data"]["appError"]
        .as_str()
        .unwrap_or_default()
}

/// Waits until a command has written `name` (through a rename, so whole).
async fn written(work: &Path, name: &str) -> String {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(work.join(name)) {
                return text.trim().to_owned();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{name} was not written"))
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// Waits until no process of `pids` is left.
async fn gone(pids: &[&str]) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while pids.iter().any(|pid| alive(pid)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("processes {pids:?} are still running"));
}

/// A command that records its shell and a background child, then runs until
/// it finds `go` in its working directory.
const HELD: &str = "sleep 600 & echo $! > child.tmp && mv child.tmp child; \
                    echo $$ > pid.tmp && mv pid.tmp pid; echo started; \
                    while [ ! -f go ]; do sleep 0.02; done; echo finished";
/// A command that records its shell and runs until it finds `go`.
const WAITING: &str = "echo $$ > pid.tmp && mv pid.tmp pid; echo started; \
                       while [ ! -f go ]; do sleep 0.02; done; echo finished";

/// Design D11 (first half): without ShellInput the names are unknown. With
/// it, each method checks its own params, and a shell the builder refuses or
/// an unknown branch is reported before anything runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_methods_need_the_assembly_and_check_their_requests() {
    let (server, _) = launch_with(Arc::new(MockTools), ServerConfig::default(), |_| {}).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let created = client.rpc("session.create", create(json!([]))).await;
    let sid = created["result"]["sessionId"].as_str().unwrap().to_owned();
    for method in ["shell.run", "shell.stop", "shell.list"] {
        let reply = client.rpc(method, json!({"sessionId": sid})).await;
        assert_eq!(app_error(&reply), "unknown_method", "{method}");
    }
    drop(client);
    server.shutdown();
    server.join().await.unwrap();

    let mut shell = Shell::launch().await;
    let sid = shell.sid.clone();
    let methods = shell
        .client
        .rpc("session.methods", json!({"sessionId": sid}))
        .await;
    for method in [
        json!({"name": "shell.list", "mutating": false}),
        json!({"name": "shell.run", "mutating": true}),
        json!({"name": "shell.stop", "mutating": true}),
    ] {
        assert!(
            methods["result"]["methods"]
                .as_array()
                .unwrap()
                .contains(&method),
            "{methods}"
        );
    }
    let limit = 64 * 1024;
    let valid = shell.run_params("r1", "true", "private");
    for change in [
        json!({"command": ""}),
        json!({"command": "x".repeat(limit + 1)}),
        json!({"runId": ""}),
        json!({"runId": "a_b"}),
        json!({"runId": "r".repeat(65)}),
        json!({"runId": 7}),
        json!({"mode": "loud"}),
        json!({"extra": true}),
    ] {
        let mut params = valid.clone();
        params
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        let reply = shell.client.rpc("shell.run", params).await;
        assert_eq!(reply["error"]["code"], -32602, "{change}: {reply}");
        assert_eq!(app_error(&reply), "invalid_params", "{change}");
    }
    for (method, params) in [
        (
            "shell.run",
            json!({"sessionId": sid, "command": "true", "mode": "private"}),
        ),
        ("shell.stop", json!({"sessionId": sid})),
        ("shell.list", json!({"sessionId": sid, "runId": "r1"})),
    ] {
        let reply = shell.client.rpc(method, params).await;
        assert_eq!(app_error(&reply), "invalid_params", "{method}: {reply}");
    }
    // The longest command and runId run.
    let longest = format!("true{}", " ".repeat(limit - 4));
    let reply = shell.run(&"r".repeat(64), &longest, "private").await;
    assert_eq!(reply["result"]["status"], "completed", "{reply}");

    let mut params = shell.run_params("r2", "true", "private");
    params["branchId"] = json!("missing");
    let reply = shell.client.rpc("shell.run", params).await;
    assert_eq!(app_error(&reply), "session_invalid_input", "{reply}");

    set_shell(&shell.home, "/missing/shell");
    let reply = shell.run("r3", "true", "private").await;
    assert_eq!(app_error(&reply), "shell_unavailable", "{reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("EXEC_SHELL_UNAVAILABLE: "),
        "{reply}"
    );
    // The builder reads the configuration for every run; nothing ran, so the
    // runId is still free.
    set_shell(&shell.home, "/bin/sh");
    assert_eq!(
        shell.run("r3", "true", "private").await["result"]["status"],
        "completed"
    );
    shell.shutdown().await;
}

/// Design D3 and A.2: a private run replies the output as exec bounds it
/// (decoded, control characters removed, the last 2000 lines) with its exit,
/// and the Timeline does not change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_private_run_replies_exec_bounded_output_and_writes_nothing() {
    let mut shell = Shell::launch().await;
    let before = shell.through().await;

    let reply = shell
        .run(
            "exit",
            "printf 'out\\n'; printf 'err\\n' >&2; exit 3",
            "private",
        )
        .await;
    let result = &reply["result"];
    assert_eq!(result["runId"], "exit");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["exitCode"], 3);
    assert!(result.get("signal").is_none() && result.get("recorded").is_none());
    let output = result["output"].as_str().unwrap();
    assert!(
        output.contains("out\n") && output.contains("err\n"),
        "{output}"
    );
    assert_eq!(result["truncated"], false);
    assert!(result["durationMs"].is_u64());

    let reply = shell.run("signal", "kill -TERM $$", "private").await;
    assert_eq!(reply["result"]["signal"], "SIGTERM", "{reply}");
    assert!(reply["result"].get("exitCode").is_none());

    let reply = shell
        .run("bytes", "printf 'a\\033[31mb\\377\\n'", "private")
        .await;
    assert_eq!(reply["result"]["output"], "a[31mb<0xFF>\n");

    let lines = "i=0; while [ $i -lt 3000 ]; do echo line$i; i=$((i+1)); done";
    let reply = shell.run("lines", lines, "private").await;
    let output = reply["result"]["output"].as_str().unwrap();
    // exec keeps the last 2000 segments split at newlines, the empty one
    // after the final newline included.
    assert_eq!(output.lines().count(), 1999);
    assert!(output.starts_with("line1001\n") && output.ends_with("line2999\n"));
    assert_eq!(reply["result"]["truncated"], true);

    assert_eq!(shell.through().await, before);
    shell.shutdown().await;
}

/// Design D4 and D5: one run per Session; a turn stop does not reach it;
/// `shell.stop` terminates the whole tree and the run records `stopped` with
/// the output read so far. A used runId is refused for this generation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_run_per_session_until_it_ends_or_is_stopped() {
    let mut shell = Shell::launch().await;
    let sid = shell.sid.clone();
    let before = shell.through().await;
    assert_eq!(shell.list().await, json!([]));
    let held = shell.send_run("held", HELD, "record").await;
    let (pid, child) = (
        written(&shell.work, "pid").await,
        written(&shell.work, "child").await,
    );
    let runs = shell.list().await;
    assert_eq!(runs.as_array().unwrap().len(), 1, "{runs}");
    assert_eq!(runs[0]["runId"], "held");
    assert_eq!(runs[0]["branchId"], "main");
    assert_eq!(runs[0]["mode"], "record");
    assert_eq!(runs[0]["command"], HELD);
    assert!(runs[0]["startedAt"].as_u64().unwrap() > 0);

    for run_id in ["other", "held"] {
        let reply = shell.run(run_id, "true", "private").await;
        assert_eq!(app_error(&reply), "shell_busy", "{run_id}: {reply}");
    }
    assert_ok(
        &shell
            .client
            .rpc(
                "session.stop",
                json!({"sessionId": sid, "branchId": "main"}),
            )
            .await,
    );
    assert_eq!(shell.list().await[0]["runId"], "held");
    assert!(alive(&pid) && alive(&child));

    assert_eq!(shell.stop("held").await["result"], json!({"stopped": true}));
    let reply = shell.client.response(held).await;
    let result = &reply["result"];
    assert_eq!(result["status"], "stopped", "{reply}");
    assert_eq!(result["output"], "started\n");
    assert!(result.get("exitCode").is_none() && result.get("signal").is_none());
    gone(&[&pid, &child]).await;
    let recorded = shell.recorded(before).await;
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0]["sequence"], result["recorded"]["sequence"]);
    let text = recorded[0]["data"]["blocks"][0]["text"].as_str().unwrap();
    assert!(
        text.starts_with(
            "<dim-shell-input kind=\"command\" status=\"stopped\" truncated=\"false\">\n$ sleep 600 &amp; "
        ) && text.ends_with("\nstarted\n\n</dim-shell-input>"),
        "{text}"
    );

    assert_eq!(
        shell.stop("held").await["result"],
        json!({"stopped": false})
    );
    assert_eq!(shell.list().await, json!([]));
    let reply = shell.run("held", "true", "private").await;
    assert_eq!(app_error(&reply), "duplicate_run", "{reply}");
    let reply = shell.run("next", "echo next", "private").await;
    assert_eq!(reply["result"]["output"], "next\n");
    shell.shutdown().await;
}

/// Design D7 (second half): closing the Session terminates the tree and the
/// call replies `operation_interrupted`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_the_session_terminates_the_run() {
    let mut shell = Shell::launch().await;
    let sid = shell.sid.clone();
    let held = shell.send_run("held", HELD, "record").await;
    let (pid, child) = (
        written(&shell.work, "pid").await,
        written(&shell.work, "child").await,
    );
    let mut other = Client::new_version(&shell.server, false, 3).await;
    assert_ok(&other.rpc("session.close", json!({"sessionId": sid})).await);
    let reply = shell.client.response(held).await;
    assert_eq!(app_error(&reply), "operation_interrupted", "{reply}");
    gone(&[&pid, &child]).await;
    drop(other);
    shell.shutdown().await;
}

/// Design D10: an Rpc reload during a run completes at once and the call
/// replies `operation_interrupted`; the run stays with ShellInput, ends on
/// its own and is recorded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_rpc_reload_ends_the_wait_but_not_the_run() {
    let mut shell = Shell::launch().await;
    let before = shell.through().await;
    let held = shell.send_run("held", WAITING, "record").await;
    let pid = written(&shell.work, "pid").await;
    tokio::time::timeout(Duration::from_secs(5), shell.server.restart("Rpc"))
        .await
        .expect("the reload does not wait for the command")
        .unwrap();
    let reply = shell.client.response(held).await;
    assert_eq!(app_error(&reply), "operation_interrupted", "{reply}");
    assert!(alive(&pid));
    assert_eq!(shell.list().await[0]["runId"], "held");

    std::fs::write(shell.work.join("go"), "").unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while shell.list().await != json!([]) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the run ends on its own");
    gone(&[&pid]).await;
    let recorded = shell.recorded(before).await;
    assert_eq!(recorded.len(), 1);
    let text = recorded[0]["data"]["blocks"][0]["text"].as_str().unwrap();
    assert!(
        text.starts_with("<dim-shell-input kind=\"command\" status=\"completed\" exit=\"0\" truncated=\"false\">\n$ ")
            && text.ends_with("\nstarted\nfinished\n\n</dim-shell-input>"),
        "{text}"
    );
    shell.shutdown().await;
}

/// Design D1: `!command` while idle writes one Append input with the marker
/// once the command has ended; it starts no turn and is consumed with the
/// first model request of the next prompt, before the prompt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_run_is_one_append_input_for_the_next_request() {
    let mut shell = Shell::launch().await;
    let sid = shell.sid.clone();
    let before = shell.through().await;
    let reply = shell.run("echo", "echo hi", "record").await;
    let sequence = reply["result"]["recorded"]["sequence"].as_u64().unwrap();
    // Other capabilities may append their own inputs meanwhile; only the
    // recorded one and its application are this run's, and no turn starts.
    let events = shell.events(before).await;
    let input = events
        .iter()
        .find(|event| event["sequence"] == sequence)
        .unwrap()
        .clone();
    assert_eq!(input["name"], "message");
    assert_eq!(
        input["data"],
        json!({"mode": "append", "blocks": [{"type": "raw", "text":
            "<dim-shell-input kind=\"command\" status=\"completed\" exit=\"0\" truncated=\"false\">\n$ echo hi\nhi\n\n</dim-shell-input>"}]})
    );
    assert!(events.iter().any(|event| event["name"] == "applied"
        && event["data"] == json!({"input_id": input["id"], "mode": "append"})));
    assert!(
        events.iter().all(|event| event["type"] != "turn"),
        "{events:?}"
    );

    let prompt = shell
        .client
        .rpc(
            "session.submit",
            json!({"sessionId": sid, "branchId": "main", "text": "next prompt"}),
        )
        .await;
    let prompt = &prompt["result"]["event"]["id"];
    let events = shell.events(sequence).await;
    let position = |name: &str, input: &Value| {
        events
            .iter()
            .position(|event| {
                event["name"] == name && (input.is_null() || event["data"]["input_id"] == *input)
            })
            .unwrap_or_else(|| panic!("{name} {input}: {events:?}"))
    };
    let consumed = position("consumed", &input["id"]);
    assert!(consumed < position("consumed", prompt));
    assert!(position("consumed", prompt) < position("model-start", &Value::Null));
    shell.shutdown().await;
}

/// Design D9: a caller that disconnects does not stop its run, which is
/// recorded when it ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disconnected_caller_does_not_stop_its_run() {
    let mut shell = Shell::launch().await;
    let before = shell.through().await;
    let mut leaving = Client::new_version(&shell.server, false, 3).await;
    let params = shell.run_params("held", WAITING, "record");
    leaving.send("shell.run", params).await;
    let pid = written(&shell.work, "pid").await;
    drop(leaving);
    std::fs::write(shell.work.join("go"), "").unwrap();
    gone(&[&pid]).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while shell.recorded(before).await.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the run is recorded");
    assert_eq!(shell.list().await, json!([]));
    shell.shutdown().await;
}

/// Design D8: unloading the Session during a run (a host's idle release, then
/// a restore) terminates the tree and writes nothing; the command does not
/// run again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_reload_terminates_the_run_without_writing() {
    let mut shell = Shell::launch().await;
    let before = shell.through().await;
    let held = shell.send_run("held", HELD, "record").await;
    let (pid, child) = (
        written(&shell.work, "pid").await,
        written(&shell.work, "child").await,
    );
    std::fs::remove_file(shell.work.join("pid")).unwrap();
    let session = shell
        .server
        .agent()
        .unwrap()
        .get_session(&shell.sid)
        .unwrap();
    assert!(session.unload_if_idle().await.unwrap());
    let reply = shell.client.response(held).await;
    assert_eq!(app_error(&reply), "operation_interrupted", "{reply}");
    gone(&[&pid, &child]).await;
    // The Session restores without the run.
    assert_eq!(shell.list().await, json!([]));
    assert!(shell.recorded(before).await.is_empty());
    assert!(!shell.work.join("pid").exists());
    shell.shutdown().await;
}

/// Reads frames until the reply to `id`, returning the progress data of `id`
/// received before it.
async fn progress_until_reply(client: &mut Client, id: u64) -> (Vec<Value>, Value) {
    let mut progress = vec![];
    loop {
        let frame = client.read().await;
        if frame["id"] == id {
            return (progress, frame);
        }
        if frame["method"] == "management.progress" && frame["params"]["id"] == id {
            progress.push(frame["params"]["data"].clone());
        }
    }
}

/// Design D3 and A.1: the caller receives the output as progress while the
/// command runs and before the reply, decoded and with control characters
/// removed as in exec.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_caller_receives_progress_before_the_reply() {
    let mut shell = Shell::launch().await;
    let id = shell
        .send_run(
            "streamed",
            "printf 'one\\033\\n'; while [ ! -f go ]; do sleep 0.02; done; printf 'two\\n'",
            "private",
        )
        .await;
    let first = loop {
        let frame = shell.client.read().await;
        if frame["method"] == "management.progress" {
            break frame["params"].clone();
        }
    };
    assert_eq!(
        first,
        json!({"id": id, "method": "shell.run", "data": {"text": "one\n"}})
    );
    std::fs::write(shell.work.join("go"), "").unwrap();
    let (progress, reply) = progress_until_reply(&mut shell.client, id).await;
    assert_eq!(progress, [json!({"text": "two\n"})]);
    assert_eq!(reply["result"]["output"], "one\ntwo\n");
    shell.shutdown().await;
}

/// Design D6: a command writing 10 MiB at once. Each progress item holds at
/// most 64 KiB of text and counts the bytes it left out; the items precede
/// the reply and the connection stays open; the reply and the recorded input
/// keep exec's bounds and are marked truncated.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn progress_items_are_capped_and_count_what_they_leave_out() {
    let mut shell = Shell::launch().await;
    let before = shell.through().await;
    let total = 10 * 1024 * 1024;
    // 64-byte lines.
    let line = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ-";
    let command = format!("yes {line} | head -c {total}");
    let id = shell.send_run("flood", &command, "record").await;
    let (progress, reply) = progress_until_reply(&mut shell.client, id).await;
    let (mut text, mut skipped) = (0, 0);
    for item in &progress {
        let length = item["text"].as_str().unwrap().len();
        assert!(length <= 64 * 1024, "{length}");
        text += length;
        skipped += item["skipped"].as_u64().unwrap_or_default() as usize;
    }
    assert!(skipped > 0, "{progress:?}");
    assert_eq!(text + skipped, total);
    assert_ok(&shell.client.rpc("daemon.ping", json!({})).await);

    let result = &reply["result"];
    assert_eq!(result["truncated"], true);
    let output = result["output"].as_str().unwrap();
    assert!(output.len() <= 50 * 1024 && output.ends_with(&format!("{line}\n")));
    let recorded = shell.recorded(before).await;
    let marker = recorded[0]["data"]["blocks"][0]["text"].as_str().unwrap();
    assert!(marker.contains("truncated=\"true\""), "{marker:.200}");
    assert!(marker.ends_with(&format!("\n{output}\n</dim-shell-input>")));
    shell.shutdown().await;
}
