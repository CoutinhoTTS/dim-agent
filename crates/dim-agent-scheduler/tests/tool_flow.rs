//! The `schedule` tool through a real App Server: a scripted model calls the
//! tool from a Session, the process owner saves the job with that branch as
//! its origin, and a non-interactive Session is refused.
use dim_agent_capabilities::{defaults, history::HistoryRenderer, host::*};
use dim_agent_runtime::{AgentBuilder, AgentConfig};
use dim_agent_scheduler::install;
use dim_agent_types::*;
use dim_app_server::{ServerConfig, serve};
use dim_client::{Client, ConnectionConfig, Server};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

/// Replies with queued tool batches, then with text; keeps every request.
#[derive(Default)]
struct Script {
    batches: Mutex<VecDeque<Vec<ToolCall>>>,
    requests: Mutex<Vec<Arc<ModelRequest>>>,
}

impl Script {
    fn push(&self, calls: Vec<ToolCall>) {
        self.batches.lock().unwrap().push_back(calls);
    }
    fn texts(&self) -> String {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| format!("{request:?}"))
            .collect()
    }
}

impl ModelDriver for Script {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        _: ModelConnection,
        _: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            let id = format!("m{}", self.requests.lock().unwrap().len());
            let mut chunks = vec![Ok(ModelChunk::MessageStart {
                message_id: id.clone(),
            })];
            let reason = match self.batches.lock().unwrap().pop_front() {
                Some(calls) => {
                    for call in calls {
                        chunks.push(Ok(ModelChunk::ToolStart {
                            id: call.id.clone(),
                            name: call.name.clone(),
                        }));
                        chunks.push(Ok(ModelChunk::ToolUse(call)));
                    }
                    ModelStopReason::ToolUse
                }
                None => {
                    chunks.push(Ok(ModelChunk::Text("done".into())));
                    chunks.push(Ok(ModelChunk::TextEnd));
                    ModelStopReason::EndTurn
                }
            };
            chunks.push(Ok(ModelChunk::MessageStop {
                message_id: id,
                reason,
            }));
            Ok(Box::pin(futures_util::stream::iter(chunks)) as ModelStream)
        })
    }
}

fn call(id: &str, input: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "schedule".into(),
        input: input.as_object().unwrap().clone(),
    }
}

async fn wait_for(description: &str, mut check: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {description}"));
}

/// A selected model, as the App Server's profile needs before a Session runs.
/// The scripted model answers every request, so the endpoint is never called.
async fn select_model(client: &mut Client) {
    client
        .rpc(
            "provider.create",
            json!({"providerId": "018ec73f-0000-7000-8000-000000000007", "displayName": "Fixture",
                "connection": {"driverKind": "openai-compatible", "baseUrl": "http://localhost/v1",
                    "auth": {"type": "anonymous"}},
                "models": [{"modelId": "fixture", "displayName": "Fixture", "status": "active",
                    "capabilities": dim_agent_types::CatalogCapabilities::with_limits(128_000, 16_384)}],
                "enabled": true}),
        )
        .await
        .unwrap();
    client
        .rpc(
            "provider.select",
            json!({"providerId": "018ec73f-0000-7000-8000-000000000007", "modelId": "fixture"}),
        )
        .await
        .unwrap();
}

async fn create_session(client: &mut Client, home: &Path, interactive: bool) -> String {
    let created = client
        .rpc(
            "session.create",
            json!({
                "cwd": home,
                "tools": ["schedule"],
                "interactive": interactive,
                "permissionPolicy": {"preset": "full-access", "allowedTools": [], "disallowedTools": []}
            }),
        )
        .await
        .unwrap();
    created["sessionId"].as_str().unwrap().to_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_schedule_tool_saves_jobs_for_interactive_sessions_only() {
    let home = tempfile::tempdir().unwrap();
    std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::create_dir_all(home.path().join("scheduler")).unwrap();

    let model = Arc::new(Script::default());
    let mut builder = AgentBuilder::new(AgentConfig::default());
    defaults::install(&mut builder);
    defaults::mock_hosts_with_model(&mut builder, HistoryRenderer::default(), model.clone());
    install(&mut builder);
    let server = serve(
        builder,
        ServerConfig {
            startup_timeout: Duration::from_secs(60),
            database_path: Some(home.path().join("dimcode.sqlite")),
            product_home: Some(home.path().to_owned()),
            unix_path: Some(home.path().join("native.sock")),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let config = ConnectionConfig {
        endpoint: server.unix_path.clone().unwrap(),
        server: Server {
            program: std::env::current_exe().unwrap(),
            embedded: false,
        },
        profile: None,
        client_kind: "cli",
        client_name: "scheduler-tool-test",
    };
    let mut client = Client::connect(&config).await.unwrap();
    select_model(&mut client).await;

    let interactive = create_session(&mut client, home.path(), true).await;
    model.push(vec![call(
        "c1",
        json!({"action": "create", "job": {
            "name": "nightly", "schedule": {"at": "2030-01-01T00:00:00Z"},
            "prompt": "Summarize the day.", "cwd": home.path()}}),
    )]);
    client
        .rpc(
            "session.submit",
            json!({"sessionId": interactive, "branchId": "main", "text": "schedule it"}),
        )
        .await
        .unwrap();
    wait_for("the saved job", || {
        model.texts().contains("schedule: saved job")
    })
    .await;

    let listed: Value = client.typed("schedule.list", json!({})).await.unwrap();
    let saved = &listed["jobs"][0];
    assert_eq!(saved["name"], "nightly", "{listed}");
    assert_eq!(
        saved["created_by"],
        json!({"tool": {"session_id": interactive, "branch_id": "main"}}),
        "{listed}"
    );
    assert_eq!(saved["policy_cap"]["preset"], "full-access", "{listed}");

    let member = create_session(&mut client, home.path(), false).await;
    model.push(vec![call("c2", json!({"action": "list"}))]);
    client
        .rpc(
            "session.submit",
            json!({"sessionId": member, "branchId": "main", "text": "list"}),
        )
        .await
        .unwrap();
    wait_for("the refusal", || {
        model
            .texts()
            .contains("available only in interactive leader Sessions")
    })
    .await;

    let job_id = listed["jobs"][0]["id"].as_str().unwrap().to_owned();
    model.push(vec![call(
        "c3",
        json!({"action": "run_now", "job_id": job_id}),
    )]);
    client
        .rpc(
            "session.submit",
            json!({"sessionId": interactive, "branchId": "main", "text": "run it now"}),
        )
        .await
        .unwrap();
    wait_for("the report steered to the saving branch", || {
        model
            .texts()
            .contains("Inspect schedule.history for this job.")
    })
    .await;
    let history: Value = client
        .typed("schedule.history", json!({"job_id": job_id}))
        .await
        .unwrap();
    let run = &history["runs"][0];
    assert_eq!(run["state"], "completed", "{history}");
    assert_eq!(
        run["report_to"]["session_id"],
        json!(interactive),
        "{history}"
    );
    assert!(
        !run["detail"].as_str().unwrap().contains("report_failed"),
        "the report was delivered: {history}"
    );

    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}
