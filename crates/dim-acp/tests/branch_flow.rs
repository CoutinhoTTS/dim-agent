#![cfg(unix)]

use dim_acp::{AcpGateway, Io};
use dim_client::{Client, ConnectionConfig, DaemonProfile, Server};
use rutis::host::CtxHost;
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixListener,
    sync::oneshot,
};

async fn line(reader: &mut (impl AsyncBufReadExt + Unpin)) -> Option<Value> {
    let mut line = String::new();
    let read = tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    (read > 0).then(|| serde_json::from_str(&line).unwrap())
}

async fn frame(reader: &mut (impl AsyncBufReadExt + Unpin)) -> Value {
    line(reader).await.expect("the peer stays open")
}

/// The reply to `id`, skipping the notifications that may come first.
async fn reply(reader: &mut (impl AsyncBufReadExt + Unpin), id: u64) -> Value {
    loop {
        let frame = frame(reader).await;
        if frame["id"] == json!(id) {
            return frame;
        }
    }
}

async fn send(writer: &mut (impl AsyncWriteExt + Unpin), value: Value) {
    writer
        .write_all(format!("{value}\n").as_bytes())
        .await
        .unwrap();
    writer.flush().await.unwrap();
}

/// An ACP call names no line: the gateway reads the Session's default branch
/// once and every call it makes reaches it (design §4, §9, §14).
#[tokio::test]
async fn a_gateway_call_without_a_line_targets_the_default_branch() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let endpoint = home.path().join("daemon.sock");
    let cwd = home.path().to_str().unwrap().to_owned();
    let listener = UnixListener::bind(&endpoint).unwrap();
    std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (submitted, submit_seen) = oneshot::channel();
    let daemon_cwd = cwd.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut read = BufReader::new(read);
        let hello = frame(&mut read).await;
        send(&mut write,json!({"jsonrpc":"2.0","id":hello["id"],"result":{"instanceId":"fixture","serverVersion":"fixture","protocolVersion":3,"daemonProfile":{"kind":"product"},"features":["session.create","session.attach","session.attach.expectedCwd","session.detach","session.history","session.history.latestTurn","session.input_lookup","session.submit","session.stop","session.branches","session.methods","session.input.pending","session.question.pending","session.turn.get","session.exec.pending"]}})).await;
        let mut submitted = Some(submitted);
        while let Some(request) = line(&mut read).await {
            let method = request["method"].as_str().unwrap_or_default().to_owned();
            let branch = request["params"]["branchId"]
                .as_str()
                .unwrap_or("main")
                .to_owned();
            let through = request["params"]["through"].as_u64().unwrap_or(2);
            let id = request["id"].clone();
            match method.as_str() {
                "session.attach" => {
                    assert_eq!(request["params"]["expectedCwd"], json!(daemon_cwd));
                    send(
                        &mut write,
                        json!({"jsonrpc":"2.0","id":id,"result":{
                        "sequence":2,"cwd":daemon_cwd,
                        "model":{"providerId":"fixture","modelId":"fixture-model"}}}),
                    )
                    .await;
                }
                "session.branches" => {
                    send(&mut write, json!({"jsonrpc":"2.0","id":id,"result":{
                        "defaultBranchId":"line-1",
                        "branches":[
                            {"branchId":"main","state":"open","head":2},
                            {"branchId":"line-1","parentBranchId":"main","branchPoint":1,"state":"open","head":2}]}})).await;
                }
                "session.history" => {
                    assert_eq!(branch, "line-1", "the recovery reads the default branch");
                    send(
                        &mut write,
                        json!({"jsonrpc":"2.0","id":id,"result":{
                        "events":[],"through":2,"nextBefore":null,"nextAfter":2}}),
                    )
                    .await;
                }
                "session.methods" => {
                    let methods: Vec<_> = [
                        "session.input.pending",
                        "session.question.pending",
                        "session.exec.pending",
                        "session.turn.get",
                    ]
                    .into_iter()
                    .map(|name| json!({"name":name,"mutating":false}))
                    .collect();
                    send(
                        &mut write,
                        json!({"jsonrpc":"2.0","id":id,"result":{"methods":methods}}),
                    )
                    .await;
                }
                "session.question.pending" => {
                    send(
                        &mut write,
                        json!({"jsonrpc":"2.0","id":id,"result":{
                        "branchId":branch,"through":through,"batches":[],"next":null}}),
                    )
                    .await;
                }
                "session.exec.pending" => {
                    send(
                        &mut write,
                        json!({"jsonrpc":"2.0","id":id,"result":{
                        "branchId":branch,"through":through,"tasks":[],"next":null}}),
                    )
                    .await;
                }
                "session.input.pending" => {
                    send(
                        &mut write,
                        json!({"jsonrpc":"2.0","id":id,"result":{
                        "branchId":branch,"through":through,"inputs":[],"next":null}}),
                    )
                    .await;
                }
                "session.turn.get" => {
                    assert_eq!(branch, "line-1", "the turn read is the default branch's");
                    send(
                        &mut write,
                        json!({"jsonrpc":"2.0","id":id,"result":{
                        "branchId":branch,"through":through,"turn":null,"output":null}}),
                    )
                    .await;
                }
                "session.submit" => {
                    assert_eq!(
                        request["params"]["branchId"],
                        json!("line-1"),
                        "a prompt with no line reaches the Session's default branch"
                    );
                    send(&mut write, json!({"jsonrpc":"2.0","id":id,"result":{"event":{
                        "id":"in-1","sequence":3,"create_at":1,"type":"input","name":"message",
                        "data":{"user":{"content":[{"type":"text","text":"hello"}]},"mode":"queue"}}}})).await;
                    submitted.take().unwrap().send(()).unwrap();
                }
                "session.detach" => {
                    send(&mut write, json!({"jsonrpc":"2.0","id":id,"result":{}})).await;
                    break;
                }
                other => panic!("unexpected {other}: {request}"),
            }
        }
    });
    let client = Client::connect(&ConnectionConfig {
        endpoint,
        server: Server {
            program: "unused".into(),
            embedded: false,
        },
        profile: Some(DaemonProfile::Product),
        client_kind: "other",
        client_name: "AcpGateway",
        protocol_version: 3,
    })
    .await
    .unwrap();
    let (peer, transport) = tokio::io::duplex(64 * 1024);
    let (reader, writer) = tokio::io::split(transport);
    let (gateway, finished) = AcpGateway::new(
        client,
        Io {
            reader: Box::new(reader),
            writer: Box::new(writer),
        },
        None,
        env!("CARGO_PKG_VERSION").to_owned(),
    );
    let root = rutis::Ctx::root().unwrap();
    let mounted = root.plugin(gateway);
    (&mounted).await.unwrap();
    let (read, mut write) = tokio::io::split(peer);
    let mut read = BufReader::new(read);
    send(&mut write,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}})).await;
    let initialize = reply(&mut read, 1).await;
    assert!(initialize["error"].is_null(), "{initialize}");
    send(&mut write,json!({"jsonrpc":"2.0","id":2,"method":"session/load","params":{"sessionId":"session","cwd":cwd,"mcpServers":[]}})).await;
    let loaded = reply(&mut read, 2).await;
    assert!(loaded["error"].is_null(), "{loaded}");
    send(&mut write,json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"session","prompt":[{"type":"text","text":"hello"}]}})).await;
    tokio::time::timeout(Duration::from_secs(5), submit_seen)
        .await
        .unwrap()
        .unwrap();
    // The prompt is answered on the line the default branch names; the reply
    // itself is not this test's subject.
    drop(read);
    let _ = write.shutdown().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), finished).await;
    mounted.dispose().await.unwrap();
    root.shutdown().await.unwrap();
    server.await.unwrap();
}
