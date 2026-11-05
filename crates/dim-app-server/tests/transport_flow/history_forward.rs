//! A forward `session.history` reply is bounded by encoded bytes
//! (session-history-window D11, §4.4): a page of large facts is answered as a
//! short page with `done: false`, under the transport frame, and `nextAfter`
//! reaches the rest in order.
use super::history_window::{Plan, StreamingModel, new_directory, serve_streaming, turn};
use super::*;

/// The text of each large append: two of them encode above the 16 MiB frame.
const BIG: usize = 9 * 1024 * 1024;

async fn append(client: &mut Client, id: &str, bytes: usize) {
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":id,"branchId":"main","mode":"append",
                       "text":"x".repeat(bytes)}),
            )
            .await,
    );
}

/// Every visible sequence, paging forward with `nextAfter` until `done`.
async fn sequences_via(client: &mut Client, id: &str, limit: Option<usize>) -> Vec<u64> {
    let mut sequences = Vec::new();
    let mut after = 0;
    loop {
        let mut params = json!({"sessionId":id,"branchId":"main","after":after});
        if let Some(limit) = limit {
            params["limit"] = json!(limit);
        }
        let page = client.rpc("session.history", params).await;
        assert_ok(&page);
        for event in page["result"]["events"].as_array().unwrap() {
            sequences.push(event["sequence"].as_u64().unwrap());
        }
        if page["result"]["done"].as_bool().unwrap() {
            break;
        }
        after = page["result"]["nextAfter"].as_u64().unwrap();
    }
    sequences
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forward_page_above_the_frame_is_a_short_page_the_cursor_finishes() {
    let server = launch(Arc::new(MockTools), ServerConfig::default()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let created = client.rpc("session.create", create(json!([]))).await;
    assert_ok(&created);
    let id = created["result"]["sessionId"].as_str().unwrap().to_owned();
    append(&mut client, &id, BIG).await;
    append(&mut client, &id, BIG).await;

    // The default page and an explicit larger `limit` both run into the byte
    // budget: each is answered, short, and under the transport frame.
    for limit in [None, Some(4096)] {
        let mut params = json!({"sessionId":id,"branchId":"main","after":0});
        if let Some(limit) = limit {
            params["limit"] = json!(limit);
        }
        let page = client.rpc("session.history", params).await;
        assert_ok(&page);
        assert_eq!(
            page["result"]["done"], false,
            "a page above the byte budget is short: {limit:?}"
        );
        assert!(
            serde_json::to_vec(&page).unwrap().len() < 16 * 1024 * 1024,
            "the reply stays under the transport frame: {limit:?}"
        );
    }
    assert_eq!(
        sequences_via(&mut client, &id, None).await,
        sequences_via(&mut client, &id, Some(1)).await,
        "the default page reaches the same events in the same order"
    );

    server.shutdown();
    server.join().await.unwrap();
}

/// A single batch above the transport frame cannot be carried by any page:
/// forward paging reaches it and answers `history_too_large`, as a window does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_forward_first_batch_above_the_frame_fails_history_too_large() {
    let directory = new_directory("forward-frame");
    let model = Arc::new(StreamingModel::default());
    let (server, _) = serve_streaming(&directory, model.clone()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let created = client.rpc("session.create", create(json!([]))).await;
    assert_ok(&created);
    let id = created["result"]["sessionId"].as_str().unwrap().to_owned();
    turn(
        &mut client,
        &model,
        &id,
        "frame breaker",
        Plan::Huge(17 * 1024 * 1024),
    )
    .await;

    let mut after = 0;
    loop {
        let page = client
            .rpc(
                "session.history",
                json!({"sessionId":id,"branchId":"main","after":after}),
            )
            .await;
        if page.get("error").is_some() {
            assert_eq!(
                page["error"]["data"]["appError"], "history_too_large",
                "{page}"
            );
            break;
        }
        assert_ok(&page);
        let next = page["result"]["nextAfter"].as_u64().unwrap();
        assert!(next > after, "a short page advances: {page}");
        after = next;
    }

    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
