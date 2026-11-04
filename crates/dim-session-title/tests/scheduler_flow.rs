#![cfg(unix)]
mod support;
use dim_agent_capabilities::{adapters::MockTools, defaults};
use dim_agent_runtime::testing::branch_events;
use dim_agent_runtime::{AgentBuilder, AgentConfig};
use dim_agent_types::{EventKind, ToolContent};
use dim_app_server::{ServerConfig, serve};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduler_reports_do_not_replace_the_first_operator_input_for_titles() {
    let store = Store::new();
    std::fs::create_dir(store.0.join("scheduler")).unwrap();
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_session_title::install(&mut builder);
    dim_agent_scheduler::install(&mut builder);
    defaults::install(&mut builder);
    defaults::mock_model_hosts(&mut builder);
    defaults::tool_host(&mut builder, Arc::new(MockTools));
    dim_app_server::collaboration::install(&mut builder);
    let mut config = ServerConfig {
        product_home: Some(store.0.clone()),
        unix_path: Some(store.0.join("server.sock")),
        ..store.config()
    };
    config.session_defaults.tools = Some(vec!["echo".into(), "schedule".into()]);
    config.session_defaults.permission_policy.preset =
        dim_agent_types::PermissionPreset::FullAccess;
    let (sent, mut requests) = tokio::sync::mpsc::unbounded_channel();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let http = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/{*path}", axum::routing::post(complete))
                .with_state(sent),
        )
        .await
        .unwrap();
    });
    let server = serve(builder, config).await.unwrap();
    let agent = server.agent().unwrap();
    let mut client = client(&server).await;
    when_served(&mut client, "schedule.list", json!({})).await;
    let provider = "018ec73f-0000-7000-8000-000000000009";
    client.rpc("provider.create",json!({"providerId":provider,"displayName":"Title fixture",
        "connection":{"driverKind":"openai-compatible","baseUrl":format!("http://{address}/v1"),"auth":{"type":"anonymous"}},
        "models":[{"modelId":"main","displayName":"Main","status":"active","capabilities":dim_agent_types::CatalogCapabilities::with_limits(128_000,16_384)}],"enabled":true})).await.unwrap();
    client
        .rpc(
            "provider.select",
            json!({"providerId":provider,"modelId":"main"}),
        )
        .await
        .unwrap();
    let mut settings = json!({"enabled":false,"model":{"providerId":provider,"modelId":"main"}});
    configure(&mut client, settings.clone()).await;
    let created = client
        .rpc("session.create", json!({"cwd":store.0,"interactive":true}))
        .await
        .unwrap();
    let id = created["sessionId"].as_str().unwrap();
    let session = agent.get_session(id).unwrap();
    client
        .rpc("session.attach", json!({"sessionId":id}))
        .await
        .unwrap();
    when_served(
        &mut client,
        "session.rename",
        json!({"sessionId":id,"title":null}),
    )
    .await;
    let arguments = json!({"action":"create","job":{"name":"title provenance","schedule":{"at":"2030-01-01T00:00:00Z"},"prompt":"Review the project"}});
    let first = format!("tool:schedule {arguments}");
    client
        .rpc(
            "session.submit",
            json!({"sessionId":id,"branchId":"main","text":first}),
        )
        .await
        .unwrap();
    client
        .rpc("session.idle", json!({"sessionId":id,"branchId":"main"}))
        .await
        .unwrap();
    let history = branch_events(&session, "main", ..).await.unwrap();
    let result = history
        .iter()
        .rev()
        .find_map(|e| match &e.kind {
            EventKind::ToolResult(batch) => batch.results.last().map(|r| &r.result),
            _ => None,
        })
        .expect("the operator saves a scheduled job");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let ToolContent::Text(text) = &result.content else {
        panic!("schedule returns text")
    };
    let saved: Value = serde_json::from_str(text).unwrap();
    let original = history
        .iter()
        .find(|e| matches!(&e.kind, EventKind::InputMessage(data) if data.user.is_some()))
        .unwrap()
        .id
        .clone();
    settings["enabled"] = json!(true);
    configure(&mut client, settings).await;
    let started = client
        .rpc("schedule.run_now", json!({"job_id":saved["job"]["id"]}))
        .await
        .unwrap();
    let run = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let history = client
                .rpc(
                    "schedule.history",
                    json!({"job_id":saved["job"]["id"],"limit":50}),
                )
                .await
                .unwrap();
            if let Some(run) = history["runs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["run_id"] == started["run_id"])
                && !matches!(
                    run["state"].as_str(),
                    Some("launching" | "submitted" | "running")
                )
            {
                break run.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(run["state"], "completed", "{run}");
    let report_id = format!("scheduler-report:{}", run["run_id"].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let history = branch_events(&session, "main", ..).await.unwrap();
            if let Some(report) = history.iter().find(|event| {
                matches!(&event.kind, EventKind::InputMessage(data)
                    if data.submission_id.as_deref() == Some(&report_id))
            }) && history.iter().any(|event| {
                matches!(&event.kind, EventKind::InputApplied(data) if data.input_id == report.id)
            }) {
                assert!(history.iter().any(|event| {
                    matches!(&event.kind, EventKind::InputApplied(data) if data.input_id == original)
                        && event.sequence < report.sequence
                }));
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    client
        .rpc("session.idle", json!({"sessionId":id,"branchId":"main"}))
        .await
        .unwrap();
    let summary = client.rpc("session.summaries", json!({})).await.unwrap();
    assert!(
        summary
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["sessionId"] == id)
            .unwrap()["title"]
            .is_null()
    );
    assert!(
        requests.try_recv().is_err(),
        "an adopted raw report does not trigger a title"
    );
    client
        .rpc(
            "session.submit",
            json!({"sessionId":id,"branchId":"main","text":"Check the scheduled result"}),
        )
        .await
        .unwrap();
    let (request, release) = tokio::time::timeout(Duration::from_secs(10), requests.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request["messages"][1]["content"], first);
    release.send("Scheduled review".into()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let listed = client.rpc("session.summaries", json!({})).await.unwrap();
            if listed
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["sessionId"] == id && e["title"] == "Scheduled review")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    drop(client);
    server.shutdown();
    server.join().await.unwrap();
    http.abort();
}
