#![cfg(unix)]

use dim_client::{Client, ConnectionConfig, DaemonProfile, Message, Server, callback::Capture};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UnixListener},
    sync::oneshot,
};

#[tokio::test]
async fn an_owned_connection_preserves_identity_responses_and_partial_notifications() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let endpoint = home.path().join("daemon.sock");
    let listener = UnixListener::bind(&endpoint).unwrap();
    std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (partial, written) = oneshot::channel();
    let (resume, resumed) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut read = BufReader::new(read).lines();
        let hello: Value = serde_json::from_str(&read.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(hello["params"]["clientKind"], "other");
        assert_eq!(hello["params"]["clientName"], "AcpGateway");
        let reply = json!({"jsonrpc":"2.0","id":hello["id"],"result":{
            "instanceId":"fixture","serverVersion":"fixture","protocolVersion":3,
            "daemonProfile":{"kind":"product"},"features":["session.create","session.history"]
        }});
        write
            .write_all(format!("{reply}\n").as_bytes())
            .await
            .unwrap();
        let first: Value = serde_json::from_str(&read.next_line().await.unwrap().unwrap()).unwrap();
        let second: Value =
            serde_json::from_str(&read.next_line().await.unwrap().unwrap()).unwrap();
        let replies = format!(
            "{}\n{}\n",
            json!({"jsonrpc":"2.0","id":second["id"],"result":{"order":2}}),
            json!({"jsonrpc":"2.0","id":first["id"],"result":{"order":1}})
        );
        write.write_all(replies.as_bytes()).await.unwrap();
        write
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"session.notice\",\"params\":")
            .await
            .unwrap();
        partial.send(()).unwrap();
        resumed.await.unwrap();
        write
            .write_all(b"{\"text\":\"complete\"}}\n")
            .await
            .unwrap();
    });
    let config = ConnectionConfig {
        endpoint,
        server: Server {
            program: "unused-on-unix".into(),
            embedded: false,
        },
        profile: Some(DaemonProfile::Product),
        client_kind: "other",
        client_name: "AcpGateway",
    };
    let mut client = Client::connect(&config).await.unwrap();
    assert_eq!(client.connection_config().endpoint, config.endpoint);
    let first = client.send("session.first", json!({})).await.unwrap();
    let second = client.send("session.second", json!({})).await.unwrap();
    assert_eq!(client.response(first).await.unwrap(), json!({"order":1}));
    assert!(
        matches!(client.next().await.unwrap(), Message::Response { id, result: Ok(_) } if id == second)
    );
    written.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), client.next())
            .await
            .is_err()
    );
    resume.send(()).unwrap();
    assert!(
        matches!(client.next().await.unwrap(), Message::Notice { method, params } if method == "session.notice" && params == json!({"text":"complete"}))
    );
    server.await.unwrap();
    client.wait_closed().await.unwrap();
}

#[tokio::test]
async fn loopback_capture_rejects_other_paths_without_consuming_the_callback() {
    assert!(
        Capture::bind("https://example.test/callback")
            .await
            .is_none()
    );
    let reserved = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = reserved.local_addr().unwrap();
    drop(reserved);
    let redirect = format!("http://127.0.0.1:{}/callback", address.port());
    let capture = Capture::bind(&redirect).await.unwrap();
    let expected = format!("{redirect}?code=fixture");
    let accepted = tokio::spawn(async move {
        let (url, mut stream) = capture.receive().await.unwrap();
        assert_eq!(url, expected);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    for (path, status) in [("/unrelated", "404"), ("/callback?code=fixture", "200")] {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status}")),
            "{response}"
        );
    }
    accepted.await.unwrap();
}
