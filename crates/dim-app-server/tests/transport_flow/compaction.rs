//! ContextCompaction acceptance against the product profile: the App Server
//! binary with a product home and a mock HTTP provider with a small window.
use super::*;
use axum::{Json, Router, extract::State, routing::post};
use std::{process::Stdio, sync::atomic::AtomicBool};

const SUMMARIZER: &str = "You summarize a coding session";

/// An OpenAI-compatible provider: the summarizer gets a fixed summary and
/// can be held open; every other request gets a short answer.
#[derive(Default)]
struct Provider {
    requests: std::sync::Mutex<Vec<Value>>,
    hold: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Provider {
    fn summaries(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|body| body.to_string().contains(SUMMARIZER))
            .count()
    }
}
async fn completion(State(provider): State<Arc<Provider>>, Json(body): Json<Value>) -> String {
    let summarizer = body.to_string().contains(SUMMARIZER);
    provider.requests.lock().unwrap().push(body);
    if summarizer && provider.hold.swap(false, Ordering::SeqCst) {
        provider.entered.notify_one();
        // Held until released or the process is killed.
        provider.release.notified().await;
    }
    let text = if summarizer {
        "SUMMARY OF EARLIER TURNS"
    } else {
        "ok"
    };
    format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"id":"fixture","choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]})
    )
}

struct Server {
    child: tokio::process::Child,
    socket: std::path::PathBuf,
}
async fn launch_binary(directory: &std::path::Path) -> Server {
    let home = directory.join("home");
    let socket = directory.join("ipc/server.sock");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_dim-app-server"))
        .args([
            "--product-home",
            home.to_str().unwrap(),
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

async fn history(client: &mut Client, sid: &Value) -> Vec<Value> {
    let mut events = Vec::new();
    let mut after = 0;
    loop {
        let page = client
            .rpc(
                "session.history",
                json!({"sessionId":sid,"branchId":"main","after":after,"limit":1000}),
            )
            .await;
        assert_ok(&page);
        events.extend(page["result"]["events"].as_array().unwrap().iter().cloned());
        after = page["result"]["nextAfter"].as_u64().unwrap();
        if page["result"]["done"] == true {
            return events;
        }
    }
}
fn is(event: &Value, kind: &str) -> bool {
    format!(
        "{}/{}",
        event["type"].as_str().unwrap(),
        event["name"].as_str().unwrap()
    ) == kind
}
async fn turn(client: &mut Client, sid: &Value, text: &str) {
    assert_ok(
        &client
            .rpc(
                "session.submit",
                json!({"sessionId":sid,"branchId":"main","text":text}),
            )
            .await,
    );
    assert_ok(
        &client
            .rpc("session.idle", json!({"sessionId":sid,"branchId":"main"}))
            .await,
    );
}
/// A turn of `tokens` estimated tokens (four bytes each).
fn big(index: usize, tokens: f64) -> String {
    format!("turn-{index} {}", "x".repeat((tokens * 4.) as usize))
}

/// Each start has exactly one result with its `start` and ids; no result
/// names a start that does not exist (acceptance 8).
fn assert_paired(events: &[Value]) {
    for start in events
        .iter()
        .filter(|event| is(event, "context/compaction-start"))
    {
        let results: Vec<_> = events
            .iter()
            .filter(|event| {
                is(event, "context/compaction") && event["data"]["start"] == start["sequence"]
            })
            .collect();
        assert_eq!(results.len(), 1, "{start}");
        for field in ["turn_id", "request_id", "command", "through"] {
            assert_eq!(results[0]["data"][field], start["data"][field], "{field}");
        }
    }
    for result in events
        .iter()
        .filter(|event| is(event, "context/compaction"))
    {
        if let Some(start) = result["data"]["start"].as_u64() {
            assert!(
                events.iter().any(
                    |event| event["sequence"] == start && is(event, "context/compaction-start")
                )
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn product_compaction_runs_automatically_on_request_and_settles_after_a_crash() {
    tokio::time::timeout(Duration::from_secs(120), async {
        let provider = Arc::new(Provider::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/v1/chat/completions", post(completion))
            .with_state(provider.clone());
        let http = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let directory = std::path::PathBuf::from("/tmp")
            .join(format!("dim-compaction-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(directory.join("home")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            directory.join("home"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let mut server = launch_binary(&directory).await;
        let mut client = connect(&server).await;
        let capabilities = dim_agent_types::ModelCapabilities {
            context_window: 4_000,
            max_output_tokens: 200,
            ..Default::default()
        };
        assert_ok(&client.rpc("provider.save", json!({"provider":{
            "providerId":"window","displayName":"Window","driverKind":"openai-compatible",
            "baseUrl":format!("http://{endpoint}/v1"),"auth":{"type":"anonymous"},
            "models":[{"modelId":"small","displayName":"small","status":"active","capabilities":capabilities}],
            "enabled":true
        }})).await);
        assert_ok(
            &client
                .rpc("provider.select", json!({"providerId":"window","modelId":"small"}))
                .await,
        );
        let created = client
            .rpc("session.create", json!({"cwd":directory,"tools":[]}))
            .await;
        assert_ok(&created);
        let sid = created["result"]["sessionId"].clone();
        assert_ok(&client.rpc("session.attach", json!({"sessionId":sid})).await);
        // The usage query reads the adopted window facts and writes nothing.
        let usage = client
            .rpc("session.context.usage", json!({"sessionId":sid,"branchId":"main"}))
            .await;
        assert_ok(&usage);
        assert_eq!(usage["result"]["contextWindow"], 4000.0);
        assert_eq!(usage["result"]["outputTokens"], 200.0);
        assert_eq!(usage["result"]["thresholdTokens"], 0.8 * 3800.0);
        assert_eq!(usage["result"]["baselineSource"], "full_estimate");
        let overhead = usage["result"]["estimatedInputTokens"].as_f64().unwrap();
        assert!(overhead > 0.);
        // With the state blocks of the first turn, two turns stay below the
        // threshold, the third crosses it, and two with a summary are below
        // it again.
        let size = (0.8 * 3800.0 - overhead) / 3.;
        let missing = client
            .rpc("session.context.compact", json!({"sessionId":sid}))
            .await;
        assert_eq!(missing["error"]["code"], -32602);

        // Acceptance 1: past the threshold, a preparation compacts the turns
        // before the last two with one summarizer request.
        let mut index = 0;
        let events = loop {
            turn(&mut client, &sid, &big(index, size)).await;
            index += 1;
            let events = history(&mut client, &sid).await;
            if events.iter().any(|event| is(event, "context/compaction")
                && event["data"]["status"] == "completed")
            {
                break events;
            }
            assert!(index < 10, "no automatic compaction");
        };
        assert_eq!(provider.summaries(), 1);
        let result = events
            .iter()
            .find(|event| is(event, "context/compaction") && event["data"]["status"] == "completed")
            .unwrap();
        let data = &result["data"];
        assert!(data["turn_id"].is_string() && data["request_id"].is_string());
        assert!(data.get("command").is_none());
        assert_eq!(data["strategy"], "summary");
        assert_eq!(data["replacement"].as_array().unwrap().len(), 1);
        let start = events
            .iter()
            .find(|event| event["sequence"] == data["start"])
            .unwrap();
        assert_eq!(start["data"]["trigger"], "threshold");
        assert_eq!(start["data"]["request_id"], data["request_id"]);
        let budget = events
            .iter()
            .position(|event| is(event, "context/budget"))
            .map(|_| ());
        assert!(budget.is_none(), "context/budget stays hidden from clients");
        let starts: Vec<_> = events
            .iter()
            .filter(|event| is(event, "turn/start"))
            .collect();
        assert_eq!(data["retain_from"], starts[starts.len() - 2]["id"]);
        // The request starts with the replacement and keeps the last two turns.
        let request = provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|body| !body.to_string().contains(SUMMARIZER))
            .cloned()
            .unwrap();
        let sent = request["messages"].to_string();
        assert!(sent.contains("SUMMARY OF EARLIER TURNS"));
        assert!(sent.contains(&format!("turn-{} ", index - 1)));
        assert!(sent.contains(&format!("turn-{} ", index - 2)));
        assert!(!sent.contains("turn-0 "));

        // Acceptance 3 and 8: with no turn, the method writes the command,
        // the start and the result, and v3 clients receive them live.
        turn(&mut client, &sid, "small one").await;
        let before = history(&mut client, &sid).await.len();
        let reply = client
            .rpc("session.context.compact", json!({"sessionId":sid,"branchId":"main"}))
            .await;
        assert_ok(&reply);
        assert_eq!(reply["result"]["status"], "completed");
        let events = history(&mut client, &sid).await;
        let manual: Vec<_> = events[before..].iter().collect();
        assert_eq!(manual.len(), 3, "{manual:?}");
        assert!(is(manual[0], "context/compact"));
        assert!(is(manual[1], "context/compaction-start"));
        assert_eq!(manual[1]["data"]["trigger"], "manual");
        assert_eq!(manual[1]["data"]["command"], manual[0]["sequence"]);
        assert!(manual[1]["data"].get("turn_id").is_none());
        assert!(is(manual[2], "context/compaction"));
        assert_eq!(manual[2]["sequence"], reply["result"]["sequence"]);
        assert_eq!(manual[2]["data"]["start"], manual[1]["sequence"]);
        for kind in ["context/compact", "context/compaction-start", "context/compaction"] {
            let live = loop {
                let notice = client.notice("session.event").await;
                if is(&notice["params"]["event"], kind)
                    && notice["params"]["event"]["sequence"].as_u64() >= manual[0]["sequence"].as_u64()
                {
                    break notice;
                }
            };
            assert_eq!(live["params"]["branchId"], "main");
        }

        // Two commands interleaved with a running step: the second waits as
        // the next step, and each reply comes from the step that served it.
        turn(&mut client, &sid, "small extra").await;
        provider.hold.store(true, Ordering::SeqCst);
        let first = client
            .send("session.context.compact", json!({"sessionId":sid,"branchId":"main"}))
            .await;
        provider.entered.notified().await;
        let second = client
            .send("session.context.compact", json!({"sessionId":sid,"branchId":"main"}))
            .await;
        let mut observer = connect(&server).await;
        let (c1, c2) = loop {
            let commands: Vec<_> = history(&mut observer, &sid)
                .await
                .into_iter()
                .filter(|event| is(event, "context/compact"))
                .map(|event| event["sequence"].as_u64().unwrap())
                .collect();
            if let [.., c1, c2] = commands[..]
                && commands.len() >= 3
            {
                break (c1, c2);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        drop(observer);
        provider.release.notify_one();
        let first = client.response(first).await;
        let second = client.response(second).await;
        assert_ok(&first);
        assert_ok(&second);
        assert_eq!(first["result"]["status"], "completed");
        let events = history(&mut client, &sid).await;
        let result_of = |command: u64| {
            events
                .iter()
                .find(|event| is(event, "context/compaction") && event["data"]["command"] == command)
                .unwrap()
        };
        assert_eq!(first["result"]["sequence"], result_of(c1)["sequence"]);
        let served = result_of(c2);
        assert!(served["data"]["start"].is_u64(), "{served}");
        assert!(served["sequence"].as_u64() > result_of(c1)["sequence"].as_u64());
        match second["result"]["status"].as_str().unwrap() {
            "completed" => assert_eq!(second["result"]["sequence"], served["sequence"]),
            status => assert_eq!((status, &second["result"]["reason"]), ("skipped", &served["data"]["reason"])),
        }

        // Acceptance 6 and 8: a client that connects while the summarizer is
        // held sees a running compaction from history; a crash leaves it
        // unmatched and the next load settles it.
        turn(&mut client, &sid, "small two").await;
        provider.hold.store(true, Ordering::SeqCst);
        client
            .send("session.context.compact", json!({"sessionId":sid,"branchId":"main"}))
            .await;
        provider.entered.notified().await;
        let mut observer = connect(&server).await;
        let events = loop {
            let events = history(&mut observer, &sid).await;
            if is(events.last().unwrap(), "context/compaction-start") {
                break events;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let running = events.last().unwrap();
        assert!(is(running, "context/compaction-start"), "{running}");
        assert_eq!(running["data"]["trigger"], "manual");
        let held = running["sequence"].clone();
        let completed_before = events
            .iter()
            .filter(|event| is(event, "context/compaction") && event["data"]["status"] == "completed")
            .count();
        server.child.kill().await.unwrap();
        drop((client, observer));
        server = launch_binary(&directory).await;
        let mut client = connect(&server).await;
        assert_ok(&client.rpc("session.attach", json!({"sessionId":sid})).await);
        let events = history(&mut client, &sid).await;
        let settled: Vec<_> = events
            .iter()
            .filter(|event| is(event, "context/compaction") && event["data"]["start"] == held)
            .collect();
        assert_eq!(settled.len(), 1);
        assert_eq!(settled[0]["data"]["status"], "cancelled");
        assert_eq!(settled[0]["data"]["reason"], "capability reloading");
        assert_eq!(
            events
                .iter()
                .filter(|event| is(event, "context/compaction")
                    && event["data"]["status"] == "completed")
                .count(),
            completed_before
        );
        // A new compaction runs normally.
        let reply = client
            .rpc("session.context.compact", json!({"sessionId":sid,"branchId":"main"}))
            .await;
        assert_ok(&reply);
        assert_eq!(reply["result"]["status"], "completed");

        // The same holds for an automatic compaction.
        provider.hold.store(true, Ordering::SeqCst);
        let mut held = None;
        for _ in 0..10 {
            assert_ok(&client.rpc("session.submit", json!({"sessionId":sid,"branchId":"main","text":big(index, size)})).await);
            index += 1;
            let idle = client.send("session.idle", json!({"sessionId":sid,"branchId":"main"})).await;
            tokio::select! {
                _ = provider.entered.notified() => { held = Some(()); break; }
                reply = client.response(idle) => assert_ok(&reply),
            }
        }
        assert!(held.is_some(), "no automatic compaction was held");
        // The start commits before the summarizer request; its publication
        // may follow the request.
        let mut observer = connect(&server).await;
        let running = loop {
            let start = history(&mut observer, &sid)
                .await
                .into_iter()
                .rev()
                .find(|event| is(event, "context/compaction-start"))
                .unwrap();
            if start["data"]["trigger"] == "threshold" {
                break start;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        server.child.kill().await.unwrap();
        drop((client, observer));
        server = launch_binary(&directory).await;
        let mut client = connect(&server).await;
        assert_ok(&client.rpc("session.attach", json!({"sessionId":sid})).await);
        let events = history(&mut client, &sid).await;
        let settled = events
            .iter()
            .find(|event| is(event, "context/compaction")
                && event["data"]["start"] == running["sequence"])
            .unwrap();
        assert_eq!(settled["data"]["status"], "cancelled");
        assert_eq!(settled["data"]["reason"], "capability reloading");
        assert_eq!(settled["data"]["request_id"], running["data"]["request_id"]);
        assert_paired(&events);
        server.child.kill().await.unwrap();
        http.abort();
        std::fs::remove_dir_all(directory).unwrap();
    })
    .await
    .unwrap();
}
