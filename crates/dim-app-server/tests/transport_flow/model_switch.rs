//! #1038: a Session that ran a Claude model with signed thinking moves to a
//! Chat model, and back.
//!
//! The App Server runs with a development model profile, so both providers
//! point at local stubs and every turn's wire body is inspectable. Only the
//! protocol that produced the thinking receives its continuation data;
//! another protocol gets the part as it would without data. The stored block
//! stays in the Session and is replayed when the Session returns to Anthropic.
use super::*;
use axum::{Json, Router, extract::State, routing::post};
use std::{path::Path, process::Stdio, sync::Mutex as StdMutex};

const THINKING: &str = "claude thought";
const SIGNATURE: &str = "claude-signature";
const FIRST: &str = "first answer";
const SECOND: &str = "second answer";

/// A stub that records every request body it receives.
#[derive(Default)]
struct Capture {
    requests: StdMutex<Vec<Value>>,
}

/// The fake Anthropic provider: streams a signed thinking block, then text.
async fn messages(
    State(capture): State<Arc<Capture>>,
    Json(body): Json<Value>,
) -> impl axum::response::IntoResponse {
    capture.requests.lock().unwrap().push(body);
    let events = [
        (
            "message_start",
            json!({"type":"message_start","message":{"id":"msg_fixture","type":"message","role":"assistant","model":"fixture","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":11,"output_tokens":1}}}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":THINKING}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":SIGNATURE}}),
        ),
        (
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":FIRST}}),
        ),
        (
            "content_block_stop",
            json!({"type":"content_block_stop","index":1}),
        ),
        (
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":2}}),
        ),
        ("message_stop", json!({"type":"message_stop"})),
    ];
    let payload: String = events
        .iter()
        .map(|(event, data)| format!("event: {event}\ndata: {data}\n\n"))
        .collect();
    (
        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
        payload,
    )
}

/// The fake Chat provider: records the request and streams a short reply.
async fn completions(
    State(capture): State<Arc<Capture>>,
    Json(body): Json<Value>,
) -> impl axum::response::IntoResponse {
    capture.requests.lock().unwrap().push(body);
    (
        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
        format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"id":"fixture","choices":[{"index":0,"delta":{"content":SECOND},
                "finish_reason":"stop"}]})
        ),
    )
}

struct Server {
    child: tokio::process::Child,
    socket: std::path::PathBuf,
}

/// Runs the binary with `configuration` as its development model profile.
async fn launch(directory: &Path, configuration: &Path) -> Server {
    let socket = directory.join("ipc/server.sock");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_dim-app-server"))
        .args([
            "--model-config",
            configuration.to_str().unwrap(),
            "--unix",
            socket.to_str().unwrap(),
            "--no-ws",
            "--home",
            directory.join("state").to_str().unwrap(),
        ])
        .env("HOME", directory)
        .current_dir(directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).await.unwrap();
    assert!(!line.is_empty(), "server startup failed");
    Server { child, socket }
}

async fn connect(server: &Server) -> Client {
    let mut client = Client {
        transport: Transport::Unix(BufReader::new(
            UnixStream::connect(&server.socket).await.unwrap(),
        )),
        id: 0,
        notices: vec![],
        instance: Value::Null,
        features: Value::Null,
    };
    assert_ok(
        &client
            .rpc("hello", json!({"protocolVersion":3,"clientKind":"other"}))
            .await,
    );
    client
}

/// A temporary directory under `/tmp`, as the other binary flows use.
fn directory(name: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let directory = std::path::PathBuf::from("/tmp")
        .join(format!("dim-{name}-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(directory.join("home")).unwrap();
    std::fs::set_permissions(
        directory.join("home"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    directory
}

/// Submits one turn and requires that it completed.
async fn turn(client: &mut Client, session: &Value, text: &str) {
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":session,"branchId":"main","text":text}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc(
                "session.idle",
                json!({"sessionId":session,"branchId":"main"}),
            )
            .await,
    );
    let history = client
        .rpc(
            "session.history",
            json!({"sessionId":session,"branchId":"main"}),
        )
        .await;
    assert_ok(&history);
    let end = history["result"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|event| event["type"] == "turn" && event["name"] == "end")
        .expect("the turn ended");
    assert_eq!(end["data"]["status"], "completed", "{end}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_moves_from_claude_thinking_to_chat_and_back() {
    tokio::time::timeout(Duration::from_secs(120), async {
        let claude = Arc::new(Capture::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let claude_endpoint = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/v1/messages", post(messages))
            .with_state(claude.clone());
        let claude_http = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let chat = Arc::new(Capture::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let chat_endpoint = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/v1/chat/completions", post(completions))
            .with_state(chat.clone());
        let chat_http = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let directory = directory("model-switch");
        let configuration = directory.join("model.json");
        std::fs::write(
            &configuration,
            serde_json::to_vec(&json!({
                "defaultModel":{"providerId":"local","modelId":"test-model"},
                "providers":[
                    {"id":"local","kind":"anthropic","baseUrl":format!("http://{claude_endpoint}"),
                        "headers":{"x-api-key":"native-secret"},
                        "models":[{"id":"test-model","contextWindow":32_000,"maxOutputTokens":4_096,
                            "config":{"maxTokens":4096,
                                "providerOptions":{"anthropic":{"thinking":{"type":"enabled","budgetTokens":1024}}}}}]},
                    {"id":"chat","kind":"openai-compatible","baseUrl":format!("http://{chat_endpoint}/v1"),
                        "headers":{"Authorization":"Bearer native-secret"},
                        "models":[{"id":"test-model","contextWindow":32_000,"maxOutputTokens":4_096,
                            "config":{"maxTokens":4096}}]}
                ]
            }))
            .unwrap(),
        )
        .unwrap();
        let mut server = launch(&directory, &configuration).await;
        let mut client = connect(&server).await;

        let created = client
            .rpc(
                "session.create",
                json!({"cwd":directory,"tools":[],
                    "model":{"provider":"local","modelId":"test-model"}}),
            )
            .await;
        assert_ok(&created);
        let session = created["result"]["sessionId"].clone();

        // Turn 1 runs on Anthropic with thinking on and stores the signed block.
        turn(&mut client, &session, "hello").await;
        {
            let requests = claude.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0]["thinking"]["type"], "enabled", "{}", requests[0]);
            assert_eq!(requests[0]["thinking"]["budget_tokens"], 1024);
        }

        // Turn 2 runs on the Chat model: the earlier answer is kept, and
        // the signature does not reach that protocol.
        assert_ok(
            &client
                .rpc(
                    "session.model.update",
                    json!({"sessionId":session,"branchId":"main",
                        "providerId":"chat","modelId":"test-model"}),
                )
                .await,
        );
        turn(&mut client, &session, "after the switch").await;
        {
            let requests = chat.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            let sent = &requests[0];
            assert!(
                sent.to_string().contains(FIRST),
                "the earlier answer is kept: {sent}"
            );
            assert!(
                !sent.to_string().contains(SIGNATURE),
                "the signature reached the Chat model: {sent}"
            );
            // The stored thinking has no data for Chat, so it follows the
            // no-metadata path there: the text is replayed as the Chat
            // protocol's own `reasoning_content`, exactly as a thinking part
            // stored by a Chat model is (see the Chat replay test in
            // `crates/dim-agent-aimux/src/request/tests.rs`).
            let replay = sent["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|message| message["content"] == FIRST)
                .expect("the earlier answer is replayed");
            assert_eq!(replay["reasoning_content"], THINKING, "{sent}");
        }

        // Turn 3 is Anthropic again: the drop was per request, and the stored
        // block comes back with the signature it was stored with.
        assert_ok(
            &client
                .rpc(
                    "session.model.update",
                    json!({"sessionId":session,"branchId":"main",
                        "providerId":"local","modelId":"test-model"}),
                )
                .await,
        );
        turn(&mut client, &session, "back").await;
        {
            let requests = claude.requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            let replay = requests[1]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|message| message["role"] == "assistant")
                .flat_map(|message| message["content"].as_array().unwrap())
                .find(|part| part["type"] == "thinking")
                .expect("the stored thinking is replayed");
            assert_eq!(replay["thinking"], THINKING);
            assert_eq!(replay["signature"], SIGNATURE);
            assert_eq!(requests[1]["thinking"]["type"], "enabled");
        }

        server.child.kill().await.unwrap();
        claude_http.abort();
        chat_http.abort();
        std::fs::remove_dir_all(directory).unwrap();
    })
    .await
    .unwrap();
}
