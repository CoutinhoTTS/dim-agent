//! Run lifecycle details through a real App Server: a tool-saved run that is
//! skipped sends no report, a run reads past one history page, a consumed
//! input is `running`, a past `at` job records `missed`, and an unload leaves
//! an active run for the next generation without settling it.
mod support;
use serde_json::{Value, json};
use std::sync::Arc;
use support::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_skipped_tool_run_keeps_history_and_sends_no_report() {
    let home = home();
    let model = Arc::new(Script::default());
    let server = start(home.path(), model.clone()).await;
    let mut client = connect(&server).await;

    let saver = create_session(&mut client, home.path(), true).await;
    model.push(vec![call(
        "c1",
        json!({"action": "create", "job": {"name": "inbox", "mode": "reuse",
            "schedule": {"at": "2030-01-01T00:00:00Z"}, "prompt": "Check the inbox.",
            "policy": "read-only"}}),
    )]);
    client
        .rpc(
            "session.submit",
            json!({"sessionId": saver, "branchId": "main", "text": "schedule it"}),
        )
        .await
        .unwrap();
    client
        .rpc(
            "session.idle",
            json!({"sessionId": saver, "branchId": "main"}),
        )
        .await
        .unwrap();
    let listed: Value = client.typed("schedule.list", json!({})).await.unwrap();
    let job_id = listed["jobs"][0]["id"].as_str().unwrap().to_owned();

    let first: Value = client
        .typed("schedule.run_now", json!({"job_id": job_id}))
        .await
        .unwrap();
    let first = settled(&mut client, &job_id, first["run_id"].as_str().unwrap()).await;
    assert_eq!(first["state"], "completed", "{first}");
    let bound = first["session_id"].as_str().unwrap().to_owned();
    wait_for("the completed run's report", || {
        model.texts().matches("kind=\\\"report\\\"").count() == 1
    })
    .await;

    let current = client
        .rpc(
            "session.permission.get",
            json!({"sessionId": bound, "branchId": "main"}),
        )
        .await
        .unwrap();
    client
        .rpc(
            "session.permission.update",
            json!({"sessionId": bound, "branchId": "main", "expectedVersion": current["version"],
                "policy": {"preset": "full-access", "allowedTools": [], "disallowedTools": []}}),
        )
        .await
        .unwrap();

    let second: Value = client
        .typed("schedule.run_now", json!({"job_id": job_id}))
        .await
        .unwrap();
    let second = settled(&mut client, &job_id, second["run_id"].as_str().unwrap()).await;
    assert_eq!(second["state"], "skipped", "{second}");
    assert_eq!(second["detail"], "policy", "{second}");
    assert_eq!(second["report_to"]["session_id"], json!(saver), "{second}");

    let third: Value = client
        .typed("schedule.run_now", json!({"job_id": job_id}))
        .await
        .unwrap();
    settled(&mut client, &job_id, third["run_id"].as_str().unwrap()).await;
    client
        .rpc(
            "session.idle",
            json!({"sessionId": saver, "branchId": "main"}),
        )
        .await
        .unwrap();
    let history = client
        .rpc(
            "session.history",
            json!({"sessionId": saver, "branchId": "main"}),
        )
        .await
        .unwrap();
    let reports = history.to_string().matches("scheduler-report:").count();
    assert_eq!(
        reports, 1,
        "only the completed run reports to the saving branch: {history}"
    );

    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reused_session_with_a_long_history_still_settles() {
    let home = home();
    let model = Arc::new(Script::default());
    let server = start(home.path(), model.clone()).await;
    let mut client = connect(&server).await;

    let saved: Value = client
        .typed(
            "schedule.save",
            json!({"job": {"name": "inbox", "schedule": {"at": "2030-01-01T00:00:00Z"},
                "prompt": "Check the inbox.", "cwd": home.path(), "mode": "reuse"}}),
        )
        .await
        .unwrap();
    let job_id = saved["job"]["id"].as_str().unwrap().to_owned();
    let first: Value = client
        .typed("schedule.run_now", json!({"job_id": job_id}))
        .await
        .unwrap();
    let first = settled(&mut client, &job_id, first["run_id"].as_str().unwrap()).await;
    let bound = first["session_id"].as_str().unwrap().to_owned();

    for turn in 0..60 {
        client
            .rpc(
                "session.submit",
                json!({"sessionId": bound, "branchId": "main", "text": format!("turn {turn}")}),
            )
            .await
            .unwrap();
        client
            .rpc(
                "session.idle",
                json!({"sessionId": bound, "branchId": "main"}),
            )
            .await
            .unwrap();
    }
    let head = client
        .rpc(
            "session.history",
            json!({"sessionId": bound, "branchId": "main"}),
        )
        .await
        .unwrap();
    let last = head["events"]
        .as_array()
        .and_then(|events| events.last())
        .and_then(|event| event["sequence"].as_u64())
        .unwrap_or_else(|| panic!("history has events: {head}"));
    assert!(
        last >= 256,
        "the reused Session has more than one history page: first page ends at {last}"
    );

    let second: Value = client
        .typed("schedule.run_now", json!({"job_id": job_id}))
        .await
        .unwrap();
    let second = settled(&mut client, &job_id, second["run_id"].as_str().unwrap()).await;
    assert_eq!(second["state"], "completed", "{second}");
    assert_eq!(second["session_id"], json!(bound), "{second}");

    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_consumed_input_is_running_and_an_unload_leaves_it_for_recovery() {
    let home = home();
    let model = Arc::new(Script::default());
    model.hold("Hold this run.");
    let server = start(home.path(), model.clone()).await;
    let mut client = connect(&server).await;

    let saved: Value = client
        .typed(
            "schedule.save",
            json!({"job": {"name": "slow", "schedule": {"at": "2030-01-01T00:00:00Z"},
                "prompt": "Hold this run.", "cwd": home.path()}}),
        )
        .await
        .unwrap();
    let job_id = saved["job"]["id"].as_str().unwrap().to_owned();
    let accepted: Value = client
        .typed("schedule.run_now", json!({"job_id": job_id}))
        .await
        .unwrap();
    let run_id = accepted["run_id"].as_str().unwrap().to_owned();
    let running = run_where(&mut client, &job_id, &run_id, |run| {
        run["state"] == "running"
    })
    .await;
    assert!(running["input_event_id"].is_string(), "{running}");

    drop(client);
    server.shutdown();
    server.join().await.unwrap();
    let stored = stored_runs(home.path());
    let run = stored
        .iter()
        .find(|run| run["run_id"] == run_id.as_str())
        .unwrap();
    assert_eq!(
        run["state"], "running",
        "unload stops the tracker without settling the run: {run}"
    );

    model.release();
    let server = start(home.path(), Arc::new(Script::default())).await;
    let mut client = connect(&server).await;
    let resumed = settled(&mut client, &job_id, &run_id).await;
    assert_ne!(resumed["state"], "lost", "{resumed}");
    assert_eq!(
        resumed["input_event_id"], running["input_event_id"],
        "{resumed}"
    );

    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_at_job_saved_after_its_instant_records_missed_and_disables() {
    let home = home();
    let server = start(home.path(), Arc::new(Script::default())).await;
    let mut client = connect(&server).await;

    let saved: Value = client
        .typed(
            "schedule.save",
            json!({"job": {"name": "late", "schedule": {"at": "2020-01-01T00:00:00Z"},
                "prompt": "Too late.", "cwd": home.path()}}),
        )
        .await
        .unwrap();
    let job_id = saved["job"]["id"].as_str().unwrap().to_owned();

    let disabled = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let listed: Value = client.typed("schedule.list", json!({})).await.unwrap();
            if listed["jobs"][0]["enabled"] == false {
                return listed;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the past at job disables itself");
    let history: Value = client
        .typed("schedule.history", json!({"job_id": job_id}))
        .await
        .unwrap();
    let runs = history["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1, "{history}");
    assert_eq!(runs[0]["state"], "missed", "{history}");
    assert_eq!(runs[0]["window"], "2020-01-01T00:00:00.000Z", "{history}");
    assert!(runs[0]["session_id"].is_null(), "{history} {disabled}");

    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}
