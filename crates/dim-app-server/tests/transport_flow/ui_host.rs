//! The UI registry (Tui design §3.6): interfaces register the connection
//! they already have, any number per Session; a shutdown is a notification
//! on that connection; registrations leave with their connection's
//! attachment or the connection itself; the methods are local-only.
use super::*;

async fn attached_session(client: &mut Client) -> String {
    let created = client
        .rpc("session.create", json!({"cwd": std::env::temp_dir()}))
        .await;
    assert_ok(&created);
    let session_id = created["result"]["sessionId"].as_str().unwrap().to_owned();
    assert_ok(
        &client
            .rpc("session.attach", json!({"sessionId": session_id}))
            .await,
    );
    session_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interfaces_register_any_number_per_session_and_leave_on_request() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut first = Client::new_version(&server, false, 3).await;
    for method in [
        "ui.open",
        "ui.close",
        "ui.attach",
        "ui.status",
        "ui.leave",
        "ui.shutdown",
        "ui.replace",
    ] {
        assert!(
            first.features.as_array().unwrap().contains(&json!(method)),
            "{method}"
        );
    }
    // Without a leaf program the daemon registers interfaces but starts
    // none: `ui.leaf` is not offered and `ui.open` says why.
    assert!(
        !first
            .features
            .as_array()
            .unwrap()
            .contains(&json!("ui.leaf"))
    );
    let session_id = attached_session(&mut first).await;
    let cwd = std::env::temp_dir();
    assert_eq!(
        first
            .rpc(
                "ui.open",
                json!({"sessionId": session_id, "kind": "tui", "tty": "/dev/null", "cwd": cwd, "home": cwd}),
            )
            .await["error"]["data"]["appError"],
        "ui_leaf_failed"
    );

    // Registering needs the attachment; then it is idempotent.
    let unattached = first
        .rpc("ui.attach", json!({"sessionId":"missing","kind":"tui"}))
        .await;
    assert_eq!(
        unattached["error"]["data"]["appError"],
        "session_not_attached"
    );
    let attached = first
        .rpc("ui.attach", json!({"sessionId": session_id, "kind": "tui"}))
        .await;
    assert_ok(&attached);
    let ui_id = attached["result"]["uiId"].as_str().unwrap().to_owned();
    let again = first
        .rpc("ui.attach", json!({"sessionId": session_id, "kind": "tui"}))
        .await;
    assert_eq!(again["result"]["uiId"], ui_id);

    // A second viewer of the same Session is just another registration.
    let mut second = Client::new_version(&server, false, 3).await;
    assert_ok(
        &second
            .rpc("session.attach", json!({"sessionId": session_id}))
            .await,
    );
    let other = second
        .rpc(
            "ui.attach",
            json!({"sessionId": session_id, "kind": "electron"}),
        )
        .await;
    assert_ok(&other);
    let status = second.rpc("ui.status", json!({})).await;
    let instances = status["result"]["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 2);
    assert!(
        instances
            .iter()
            .all(|i| i["sessionId"] == session_id && i["sinceMs"].is_u64())
    );
    assert_eq!(status["result"]["webConnections"], 0);

    // Asking the first to leave reaches the first connection only; the
    // Session is untouched and the second viewer stays.
    assert_ok(
        &second
            .rpc(
                "ui.shutdown",
                json!({"uiId": ui_id, "reason": "another terminal took over"}),
            )
            .await,
    );
    let notice = first.notice("ui.shutdown").await;
    assert_eq!(notice["params"]["uiId"], ui_id);
    assert_eq!(notice["params"]["reason"], "another terminal took over");
    drop(first);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = second.rpc("ui.status", json!({})).await;
            if status["result"]["instances"].as_array().unwrap().len() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the registration leaves with its connection");
    assert_eq!(
        second.rpc("ui.shutdown", json!({"uiId": ui_id})).await["error"]["data"]["appError"],
        "ui_unknown"
    );

    // Replacing: the caller registers and the old interface is asked to
    // leave; a vanished old interface does not undo the new registration.
    let mut third = Client::new_version(&server, false, 3).await;
    assert_ok(
        &third
            .rpc("session.attach", json!({"sessionId": session_id}))
            .await,
    );
    let old = other["result"]["uiId"].as_str().unwrap();
    let replaced = third
        .rpc(
            "ui.replace",
            json!({"uiId": old, "sessionId": session_id, "kind": "tui"}),
        )
        .await;
    assert_ok(&replaced);
    assert_eq!(
        second.notice("ui.shutdown").await["params"]["reason"],
        "replaced"
    );
    let gone = third
        .rpc(
            "ui.replace",
            json!({"uiId": "no-such-ui", "sessionId": session_id, "kind": "tui"}),
        )
        .await;
    assert_eq!(gone["error"]["data"]["appError"], "ui_unknown");
    let status = third.rpc("ui.status", json!({})).await;
    assert!(
        status["result"]["instances"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["uiId"] == replaced["result"]["uiId"])
    );

    // The registry is managed from local connections only.
    let mut bare = Client::new_version(&server, true, 3).await;
    assert_eq!(
        bare.rpc("ui.status", json!({})).await["error"]["data"]["appError"],
        "local_transport_required"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_registration_leaves_when_its_session_is_detached_or_closed() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let first = attached_session(&mut client).await;
    let second = attached_session(&mut client).await;
    for session_id in [&first, &second] {
        assert_ok(
            &client
                .rpc("ui.attach", json!({"sessionId": session_id, "kind": "tui"}))
                .await,
        );
    }
    let registered = |status: &Value| -> Vec<String> {
        status["result"]["instances"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["sessionId"].as_str().unwrap().to_owned())
            .collect()
    };

    // Detaching ends this connection's registration on that Session only.
    assert_ok(
        &client
            .rpc("session.detach", json!({"sessionId": first}))
            .await,
    );
    let status = client.rpc("ui.status", json!({})).await;
    assert_eq!(registered(&status), std::slice::from_ref(&second));

    // A closed Session takes its viewers' registrations with it.
    assert_ok(
        &client
            .rpc("session.close", json!({"sessionId": second}))
            .await,
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = client.rpc("ui.status", json!({})).await;
            if registered(&status).is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the registration leaves with the closed Session");
}
