//! A detach settles the subscription it ends before its reply (#982).
use super::*;
use dim_agent_capabilities::session_broadcast::SessionNotice;

/// Creates a Session with one finished turn and leaves it idle.
async fn historical(client: &mut Client) -> String {
    let created = client.rpc("session.create", create(json!([]))).await;
    assert_ok(&created);
    let sid = created["result"]["sessionId"].as_str().unwrap().to_owned();
    let input = json!({"sessionId": sid, "branchId": "main", "text": "a historical turn"});
    assert_ok(&client.rpc("session.submit", input).await);
    assert_ok(
        &client
            .rpc(
                "session.idle",
                json!({"sessionId": sid, "branchId": "main"}),
            )
            .await,
    );
    sid
}

fn notice(data: &str) -> SessionNotice {
    SessionNotice {
        source: "test.detach".into(),
        kind: "changed".into(),
        data: json!(data),
    }
}

/// The reply says the teardown happened: no sleep, no retry. Without the
/// awaited retirement the old `follow`/`follow_mcp` tasks are still tracked
/// when the reply is written, so the counts read above their baseline.
/// `DIM_TEST_ATTACH_CYCLES` repeats the round for the optional RSS
/// measurement; the assertions hold for every round.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_detach_settles_its_subscription_before_the_reply() {
    let (server, _) = launch_with_agent(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let sid = historical(&mut client).await;
    let cycles = std::env::var("DIM_TEST_ATTACH_CYCLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(4);
    let baseline_subscriptions = server.live_subscriptions();
    let baseline_uses = server.uses(&sid);
    for round in 0..cycles {
        let attached = client
            .rpc("session.attach", json!({"sessionId": sid}))
            .await;
        assert_ok(&attached);
        assert_eq!(server.uses(&sid), baseline_uses + 1, "round {round}");
        let detached = client
            .rpc("session.detach", json!({"sessionId": sid}))
            .await;
        assert_ok(&detached);
        assert_eq!(
            server.live_subscriptions(),
            baseline_subscriptions,
            "round {round}: the detach reply waits for the subscription's tasks"
        );
        assert_eq!(server.uses(&sid), baseline_uses, "round {round}");
    }
    server.shutdown();
    server.join().await.unwrap();
}

/// A notice emitted while the connection is detached reaches no attachment
/// of it: the next notice the client reads is the one emitted after the
/// re-attach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_notice_after_a_detach_reaches_no_attachment_of_that_connection() {
    let (server, _) = launch_with_agent(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let sid = historical(&mut client).await;
    assert_ok(
        &client
            .rpc("session.attach", json!({"sessionId": sid}))
            .await,
    );
    assert_ok(
        &client
            .rpc("session.detach", json!({"sessionId": sid}))
            .await,
    );
    server.notify(&sid, notice("while detached"));
    assert_ok(
        &client
            .rpc("session.attach", json!({"sessionId": sid}))
            .await,
    );
    server.notify(&sid, notice("while attached"));
    assert_eq!(
        client.notice("session.notice").await["params"],
        json!({
            "sessionId": sid,
            "source": "test.detach",
            "kind": "changed",
            "data": "while attached",
        })
    );
    server.shutdown();
    server.join().await.unwrap();
}
