use super::*;
use sha2::{Digest, Sha256};

fn begin_request(key: &str, chunks: u64, files: u64) -> Value {
    json!({"key":key,"fingerprint":"a".repeat(64),"cwd":std::env::current_dir().unwrap(),
        "model":{"providerId":"mock","modelId":"deterministic"},"title":"Imported title",
        "chunks":chunks,"files":files})
}

fn input_chunk(key: &str, index: u64, text: &str, in_context: bool) -> Value {
    json!({"key":key,"index":index,"input":{"mode":"append","blocks":[{
        "type":"messages","source":"Imported from legacy Session source-1","messages":[{
            "id":format!("old-{index}"),"createdAt":"2026-01-01T00:00:00Z",
            "message":{"role":"user","content":text},"inContext":in_context
        }]
    }]}})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conversation_import_resumes_unpublished_history_and_files_then_keeps_deletion() {
    let directory = std::env::temp_dir().join(format!("dim-import-{}", uuid::Uuid::new_v4()));
    let database = directory.join("dimcode.sqlite");
    let config = ServerConfig {
        database_path: Some(database.clone()),
        ..Default::default()
    };
    let (server, agent) = launch_with_agent(Arc::new(MockTools), config.clone()).await;
    let mut client = Client::new_version(&server, false, 3).await;
    let request = begin_request("legacy:source-1", 2, 1);
    let started = client.rpc("session.import.begin", request.clone()).await;
    assert_ok(&started);
    let id = started["result"]["sessionId"].as_str().unwrap().to_owned();
    assert_eq!(started["result"]["status"], "preparing");
    assert_eq!(
        client.rpc("session.list", json!({})).await["result"],
        json!([])
    );
    assert!(
        client
            .rpc("session.attach", json!({"sessionId":id}))
            .await
            .get("error")
            .is_some()
    );
    let first = input_chunk("legacy:source-1", 0, "compacted original", false);
    assert_ok(&client.rpc("session.import.chunk", first.clone()).await);
    assert_ok(&client.rpc("session.import.chunk", first.clone()).await);
    let mut conflict = first;
    conflict["input"]["blocks"][0]["messages"][0]["message"]["content"] = json!("different source");
    assert!(
        client
            .rpc("session.import.chunk", conflict)
            .await
            .get("error")
            .is_some()
    );
    assert!(
        client
            .rpc("session.import.finish", json!({"key":"legacy:source-1"}))
            .await
            .get("error")
            .is_some()
    );
    let bytes = b"file payload";
    let file = json!({"key":"legacy:source-1","id":"old-file","name":"payload.txt","sha256":format!("{:x}",Sha256::digest(bytes)),"bytes":bytes.len(),"offset":0,"content":&bytes[..4]});
    let part = client.rpc("session.import.file", file.clone()).await;
    assert_ok(&part);
    assert!(part["result"].is_null());
    let mut unsafe_file = file.clone();
    unsafe_file["name"] = json!("../outside");
    assert!(
        client
            .rpc("session.import.file", unsafe_file)
            .await
            .get("error")
            .is_some()
    );
    drop(client);
    drop(agent);
    server.shutdown();
    server.join().await.unwrap();

    let (server, agent) = launch_with_agent(Arc::new(MockTools), config).await;
    let mut client = Client::new_version(&server, false, 3).await;
    assert!(agent.sessions().is_empty());
    let resumed = client.rpc("session.import.begin", request.clone()).await;
    assert_ok(&resumed);
    assert_eq!(resumed["result"]["sessionId"], id);
    assert_eq!(resumed["result"]["nextChunk"], 1);
    assert_eq!(
        client.rpc("session.list", json!({})).await["result"],
        json!([])
    );
    assert_ok(&client.rpc("session.import.file", file.clone()).await);
    let mut tail = file;
    tail["offset"] = json!(4);
    tail["content"] = json!(&bytes[4..]);
    let stored = client.rpc("session.import.file", tail.clone()).await;
    assert_ok(&stored);
    assert_eq!(
        std::fs::read(stored["result"]["path"].as_str().unwrap()).unwrap(),
        bytes
    );
    assert_eq!(
        client.rpc("session.import.file", tail).await["result"],
        stored["result"]
    );
    let last = input_chunk(
        "legacy:source-1",
        1,
        "Legacy summary and continuation",
        true,
    );
    assert_ok(&client.rpc("session.import.chunk", last).await);
    let finished = client
        .rpc("session.import.finish", json!({"key":"legacy:source-1"}))
        .await;
    assert_ok(&finished);
    assert_eq!(finished["result"]["status"], "committed");
    assert_eq!(
        client
            .rpc("session.import.finish", json!({"key":"legacy:source-1"}))
            .await["result"],
        finished["result"]
    );
    let list = client.rpc("session.list", json!({})).await;
    assert_ok(&list);
    assert_eq!(list["result"].as_array().unwrap().len(), 1);
    assert_eq!(list["result"][0]["title"], "Imported title");
    let connection = rusqlite::Connection::open(&database).unwrap();
    let calls = || {
        connection.query_row("SELECT count(*) FROM timeline_events WHERE session_id=?1 AND kind IN ('model/call','tool/call')", [&id], |row| row.get::<_, i64>(0)).unwrap()
    };
    assert_eq!(calls(), 0);
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":id,"branchId":"main","text":"continue"}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc("session.idle", json!({"sessionId":id,"branchId":"main"}))
            .await,
    );
    assert_eq!(calls(), 1);
    let session = agent.get_session(&id).unwrap();
    let events = session.history("main", None).await.unwrap();
    let request = events
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(&data.request),
            _ => None,
        })
        .unwrap();
    let text = serde_json::to_string(&request.messages).unwrap();
    assert!(text.contains("Legacy summary and continuation"));
    assert!(!text.contains("compacted original"));
    assert_ok(&client.rpc("session.delete", json!({"sessionId":id})).await);
    let deleted = client
        .rpc(
            "session.import.begin",
            begin_request("legacy:source-1", 2, 1),
        )
        .await;
    assert_ok(&deleted);
    assert_eq!(deleted["result"]["status"], "deleted");
    assert_eq!(
        client.rpc("session.list", json!({})).await["result"],
        json!([])
    );
    drop(connection);
    drop(client);
    drop(agent);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conversation_import_preserves_unavailable_models_and_is_local_only() {
    let directory =
        std::env::temp_dir().join(format!("dim-import-missing-{}", uuid::Uuid::new_v4()));
    let (server, _) = launch_with_agent(
        Arc::new(MockTools),
        ServerConfig {
            database_path: Some(directory.join("store.sqlite")),
            ..Default::default()
        },
    )
    .await;
    let mut client = Client::new_version(&server, false, 3).await;
    let mut request = begin_request("legacy:missing-model", 1, 0);
    request["model"] = json!({"providerId":"removed-provider","modelId":"removed-model"});
    let result = client.rpc("session.import.begin", request.clone()).await;
    assert_ok(&result);
    assert_ok(
        &client
            .rpc(
                "session.import.chunk",
                input_chunk("legacy:missing-model", 0, "original message", true),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc(
                "session.import.finish",
                json!({"key":"legacy:missing-model"}),
            )
            .await,
    );
    let rows = client.rpc("session.list", json!({})).await;
    assert_eq!(rows["result"][0]["model"], request["model"]);
    let mut web = Client::new_version(&server, true, 3).await;
    assert_eq!(
        web.rpc("session.import.begin", request.clone()).await["error"]["data"]["appError"],
        "local_transport_required"
    );
    let mut old = Client::new_version(&server, false, 2).await;
    assert_eq!(
        old.rpc("session.import.begin", request).await["error"]["code"],
        -32601
    );
    drop(old);
    drop(web);
    drop(client);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
