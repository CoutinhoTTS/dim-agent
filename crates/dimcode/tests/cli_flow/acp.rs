use super::*;

const METHODS: &[&str] = &[
    "session.attach",
    "session.detach",
    "session.submit",
    "session.input_lookup",
    "session.stop",
    "session.list",
];

fn event(sequence: u64, id: &str, kind: &str, name: &str, data: Value) -> Value {
    json!({"id":id,"sequence":sequence,"create_at":1,"type":kind,"name":name,"data":data})
}

struct Peer {
    child: Child,
    input: std::process::ChildStdin,
    output: BufReader<std::process::ChildStdout>,
    updates: Vec<Value>,
}

impl Peer {
    fn start(home: &Home) -> Self {
        let mut child = home
            .command()
            .arg("acp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input,
            output,
            updates: Vec::new(),
        }
    }

    fn send(&mut self, id: i64, method: &str, params: Value) {
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
    }

    fn response(&mut self, id: i64) -> Value {
        loop {
            if self.output.buffer().is_empty() {
                let mut fd = libc::pollfd {
                    fd: self.output.get_ref().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                assert!(
                    unsafe { libc::poll(&mut fd, 1, PATIENCE.as_millis() as i32) } > 0,
                    "ACP response {id} timed out"
                );
            }
            let mut line = String::new();
            assert_ne!(
                self.output.read_line(&mut line).unwrap(),
                0,
                "ACP closed before response {id}"
            );
            let frame: Value = serde_json::from_str(&line).expect("stdout must contain only ACP");
            if frame.get("id") == Some(&json!(id)) {
                return frame;
            }
            assert_eq!(frame["method"], "session/update", "{frame}");
            self.updates.push(frame);
        }
    }

    fn initialize(&mut self) {
        self.send(
            1,
            "initialize",
            json!({"protocolVersion":1,"clientCapabilities":{}}),
        );
        let response = self.response(1);
        assert_eq!(response["result"]["protocolVersion"], 1, "{response}");
        assert!(
            response["result"]["agentCapabilities"]
                .get("providers")
                .is_none()
        );
    }

    fn close(self) {
        drop(self.input);
        let output = wait(self.child);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn acp_stdio_runs_against_the_production_daemon_and_a_local_model() {
    let home = Home::new();
    let _daemon = OwnedDaemon(home.0.join("rust/daemon.sock"));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let model = thread::spawn(move || {
        let deadline = Instant::now() + PATIENCE;
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "The production daemon did not request the model"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream.set_read_timeout(Some(PATIENCE)).unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = String::new();
        stream.read_line(&mut line).unwrap();
        assert!(line.starts_with("POST /v1/chat/completions "), "{line}");
        let mut length = 0;
        loop {
            line.clear();
            stream.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap();
            }
        }
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).unwrap();
        let request: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(request["model"], "fixture-model");
        assert!(
            request["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["role"] == "user"
                    && message["content"].to_string().contains("ACP_REAL_REQUEST"))
        );
        let chunk = |delta: Value, finish: Value| {
            json!({"id":"fixture-response","object":"chat.completion.chunk","model":"fixture-model","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}).to_string()
        };
        let data = format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            chunk(
                json!({"role":"assistant","content":"ACP_REAL_OK"}),
                Value::Null
            ),
            chunk(json!({}), json!("stop"))
        );
        write!(stream.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}", data.len()).unwrap();
        stream.get_mut().flush().unwrap();
    });
    let config = home.0.join("model.json");
    fs::write(&config, json!({"defaultModel":{"providerId":"local","modelId":"fixture-model"},"providers":[{"id":"local","kind":"openai-compatible","baseUrl":format!("http://{address}/v1"),"models":[{"id":"fixture-model","config":{"maxTokens":256}}]}]}).to_string()).unwrap();
    let mut child = home
        .command()
        .args(["--model-config", config.to_str().unwrap(), "acp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut peer = Peer {
        input: child.stdin.take().unwrap(),
        output: BufReader::new(child.stdout.take().unwrap()),
        child,
        updates: Vec::new(),
    };
    peer.initialize();
    peer.send(2, "session/new", json!({"cwd":home.0,"mcpServers":[]}));
    let created = peer.response(2);
    assert!(created.get("error").is_none(), "{created}");
    let id = created["result"]["sessionId"].as_str().unwrap().to_owned();
    peer.send(
        3,
        "session/prompt",
        json!({"sessionId":id,"prompt":[{"type":"text","text":"ACP_REAL_REQUEST"}]}),
    );
    let response = peer.response(3);
    assert_eq!(response["result"]["stopReason"], "end_turn", "{response}");
    let text: String = peer
        .updates
        .iter()
        .filter(|frame| frame["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .map(|frame| {
            frame["params"]["update"]["content"]["text"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(text, "ACP_REAL_OK");
    peer.updates.clear();
    peer.send(4, "session/resume", json!({"sessionId":id,"cwd":home.0}));
    assert!(peer.response(4).get("error").is_none());
    assert!(
        peer.updates.is_empty(),
        "resume must rebuild without replaying output"
    );
    peer.close();
    model.join().unwrap();
    let shutdown = home.command().args(["daemon", "stop"]).output().unwrap();
    assert!(
        shutdown.status.success(),
        "{}",
        String::from_utf8_lossy(&shutdown.stderr)
    );
}

#[test]
fn acp_stdio_correlates_the_consumed_input_and_deduplicates_unicode_progress() {
    let home = Home::new();
    let daemon = home.daemon(|wire| {
        let hello = wire.read("hello");
        assert_eq!(hello["params"]["clientKind"], "other");
        assert_eq!(hello["params"]["clientName"], "AcpGateway");
        let mut methods = vec!["session.create", "session.history"];
        methods.extend_from_slice(METHODS);
        wire.reply(&hello, json!({"instanceId":"fixture","serverVersion":"fixture","protocolVersion":3,"daemonProfile":{"kind":"product"},"features":methods}));
        let create = wire.read("session.create");
        wire.reply(&create, json!({"sessionId":"session","sequence":7,"model":{"providerId":"fixture","modelId":"fixture"}}));
        let attach = wire.read("session.attach");
        wire.reply(&attach, json!({}));
        let history = wire.read("session.history");
        wire.reply(&history, json!({"events":[],"through":7,"nextAfter":7,"done":true}));
        let submit = wire.read("session.submit");
        assert_eq!(submit["params"]["mode"], "queue");
        let input = event(9, "owned-input", "input", "message", json!({"submission_id":submit["params"]["submissionId"],"user":{"content":[{"type":"text","text":"Hello"}]},"blocks":[],"mode":"queue"}));
        let notice = |wire: &mut Wire, value: Value| wire.notice("session.event", json!({"sessionId":"session","branchId":"main","event":value}));
        notice(wire, event(8,"other-start","turn","start",json!({"turn_id":"other"})));
        notice(wire, input.clone());
        notice(wire, event(10,"other-end","turn","end",json!({"turn_id":"other","status":"completed","reason":"end_turn"})));
        wire.reply(&submit, json!({"session_id":"session","event":input}));
        notice(wire, event(11,"owned-start","turn","start",json!({"turn_id":"owned"})));
        notice(wire, event(12,"consumed","input","consumed",json!({"input_id":"owned-input"})));
        notice(wire, event(13,"model-start","step","model-start",json!({"turn_id":"owned","request_id":"model"})));
        wire.notice("session.model.progress", json!({"sessionId":"session","branchId":"main","requestId":"model","text":"A🦀","textOffset":0,"thinking":"","thinkingOffset":0}));
        notice(wire, event(14,"text-a","step","text",json!({"turn_id":"owned","request_id":"model","delta":"A"})));
        wire.notice("session.model.progress", json!({"sessionId":"session","branchId":"main","requestId":"model","text":"🦀B","textOffset":1,"thinking":"","thinkingOffset":0}));
        notice(wire, event(15,"text-b","step","text",json!({"turn_id":"owned","request_id":"model","delta":"🦀B"})));
        notice(wire, event(16,"owned-end","turn","end",json!({"turn_id":"owned","status":"completed","reason":"end_turn"})));
        let mut extra = String::new();
        assert_eq!(wire.0.read_line(&mut extra).unwrap(), 0, "EOF must detach, not stop a Session or daemon: {extra}");
    });
    let mut peer = Peer::start(&home);
    peer.initialize();
    peer.send(2, "session/new", json!({"cwd":home.0,"mcpServers":[]}));
    assert_eq!(peer.response(2)["result"]["sessionId"], "session");
    peer.send(
        3,
        "session/prompt",
        json!({"sessionId":"session","prompt":[{"type":"text","text":"Hello"}]}),
    );
    let response = peer.response(3);
    assert_eq!(response["result"]["stopReason"], "end_turn", "{response}");
    let text: String = peer
        .updates
        .iter()
        .filter(|frame| frame["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .map(|frame| {
            frame["params"]["update"]["content"]["text"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(text, "A🦀B");
    assert!(
        !peer
            .updates
            .iter()
            .any(|frame| frame["params"]["update"]["sessionUpdate"] == "user_message_chunk")
    );
    peer.close();
    daemon.join().unwrap();
}

#[test]
fn acp_stdio_load_coalesces_fixed_head_replay_and_rejects_unsupported_inputs() {
    let home = Home::new();
    let cwd = fs::canonicalize(&home.0).unwrap();
    let daemon = home.daemon(move |wire| {
        wire.hello(METHODS);
        let list = wire.read("session.list");
        wire.reply(&list, json!([{"sessionId":"saved","cwd":cwd}]));
        let attach = wire.read("session.attach");
        wire.reply(&attach, json!({}));
        let first = wire.read("session.history");
        assert!(first["params"].get("through").is_none());
        wire.reply(
            &first,
            json!({"events":[],"through":8,"nextAfter":3,"done":false}),
        );
        let second = wire.read("session.history");
        assert_eq!(second["params"]["through"], 8);
        assert_eq!(second["params"]["after"], 3);
        let text = event(
            8,
            "answer",
            "step",
            "text",
            json!({"turn_id":"turn","request_id":"request","delta":"saved answer"}),
        );
        wire.notice(
            "session.event",
            json!({"sessionId":"saved","branchId":"main","event":text}),
        );
        wire.reply(
            &second,
            json!({"events":[text],"through":8,"nextAfter":8,"done":true}),
        );
        let mut extra = String::new();
        assert_eq!(
            wire.0.read_line(&mut extra).unwrap(),
            0,
            "unexpected native mutation: {extra}"
        );
    });
    let mut peer = Peer::start(&home);
    peer.initialize();
    peer.send(2, "session/new", json!({"cwd":home.0,"mcpServers":[{"type":"http","name":"client","url":"http://localhost","headers":[]}]}));
    assert!(peer.response(2).get("error").is_some());
    peer.send(
        3,
        "session/load",
        json!({"sessionId":"saved","cwd":home.0,"mcpServers":[]}),
    );
    peer.send(
        4,
        "session/resume",
        json!({"sessionId":"saved","cwd":home.0}),
    );
    assert!(peer.response(3).get("error").is_none());
    assert!(peer.response(4).get("error").is_none());
    assert_eq!(
        peer.updates.len(),
        1,
        "live copy of replayed fact must be suppressed"
    );
    assert_eq!(
        peer.updates[0]["params"]["update"]["content"]["text"],
        "saved answer"
    );
    peer.send(5, "session/prompt", json!({"sessionId":"saved","prompt":[{"type":"audio","data":"AA==","mimeType":"audio/wav"}]}));
    assert!(peer.response(5).get("error").is_some());
    peer.close();
    daemon.join().unwrap();
}
