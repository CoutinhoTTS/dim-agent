//! The Agents `sessions` service (sessions service design §16), called by
//! name through `Keyed<dyn HostDispatch>` as a process plugin calls it.
use super::*;
use dim_agent_capabilities::permission::{Permission, PermissionState};
use dim_agent_runtime::{Agent, Session};
use dim_agent_types::{InputBlock, InputMessageData, PermissionPolicy, UserInput};
use rutis::{DepKey, Keyed};
use rutis_bridge::session::HostDispatch;
use std::sync::OnceLock;

/// The `sessions` service as a consumer injects it. `serve` takes no extra
/// process rows, so a Session part injects it by name for the test.
#[derive(Clone, Default)]
struct Sessions {
    service: Arc<OnceLock<Arc<dyn HostDispatch>>>,
    /// How often the consumer part applied; an index reload must not reload it.
    applied: Arc<AtomicUsize>,
}
struct SessionsConsumer(Sessions);
impl TypedPlugin for SessionsConsumer {
    type Deps = Keyed<dyn HostDispatch>;
    fn name(&self) -> &str {
        "sessions-consumer"
    }
    fn apply<'a>(
        &'a self,
        _: &'a rutis::Ctx,
        Keyed(service): Self::Deps,
    ) -> rutis::BoxFuture<'a, Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            self.0.applied.fetch_add(1, Ordering::SeqCst);
            let _ = self.0.service.set(service);
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for SessionsConsumer {
    fn keys(&self) -> KeysOf<Self> {
        DepKey::dynamic("sessions")
    }
}
impl Sessions {
    /// One call with one request object; the reply envelope.
    async fn call(&self, method: &str, request: Value) -> Value {
        self.raw(method, json!([request])).await
    }
    async fn raw(&self, method: &str, args: Value) -> Value {
        let service = self.service.get().expect("a Session injected sessions");
        match service.invoke(method, args.into()).unwrap() {
            rutis_bridge::session::Value::Reference(reply) if reply.is_future() => {
                reply.wait_async().await.unwrap().json().unwrap()
            }
            reply => panic!("sessions replies asynchronously: {:?}", reply.json()),
        }
    }
    async fn ok(&self, method: &str, request: Value) -> Value {
        let reply = self.call(method, request).await;
        assert_eq!(reply["ok"], true, "{reply}");
        reply["value"].clone()
    }
    /// The error code; asserts the envelope's shape.
    async fn error(&self, method: &str, request: Value) -> String {
        let reply = self.call(method, request).await;
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply["error"]["message"].is_string(), "{reply}");
        let code = reply["error"]["code"].as_str().unwrap().to_owned();
        assert_eq!(
            reply["error"]["retryable"],
            matches!(code.as_str(), "reloading" | "publication_unknown"),
            "{reply}"
        );
        code
    }
}

/// Host Session defaults of these tests: the bound of a launch without a
/// starter.
fn host_policy() -> Value {
    json!({"preset":"workspace-write","allowedTools":["confirm"],"disallowedTools":["blocked"]})
}
async fn serve_sessions(
    database: Option<std::path::PathBuf>,
) -> (RunningServer, Agent, Sessions, Client) {
    let sessions = Sessions::default();
    let consumer = sessions.clone();
    let mut config = ServerConfig {
        database_path: database,
        ..Default::default()
    };
    config.session_defaults.tools = vec!["confirm".into()];
    config.session_defaults.permission_policy = serde_json::from_value(host_policy()).unwrap();
    let (server, agent) = launch_with(
        Arc::new(ConfirmTool {
            calls: Arc::new(AtomicUsize::new(0)),
            seen: Arc::new(Mutex::new(vec![])),
        }),
        config,
        move |builder| {
            builder.session_plugin(move |_| SessionsConsumer(consumer.clone()));
        },
    )
    .await;
    let client = Client::new_version(&server, false, 3).await;
    (server, agent, sessions, client)
}
/// A client Session; creating it also gives the test its consumer part.
async fn client_session(client: &mut Client, agent: &Agent, policy: Value) -> Session {
    let created = client
        .rpc(
            "session.create",
            json!({"cwd":std::env::temp_dir(),"tools":["confirm"],"permissionPolicy":policy}),
        )
        .await;
    assert_ok(&created);
    agent
        .get_session(created["result"]["sessionId"].as_str().unwrap())
        .unwrap()
}
fn policy(value: Value) -> PermissionPolicy {
    serde_json::from_value(value).unwrap()
}
/// The launched Session is non-interactive, starts with the effective policy
/// and is served to current clients.
async fn assert_launched(client: &mut Client, agent: &Agent, reply: &Value, expected: Value) {
    assert_eq!(reply["policy"], expected);
    let session = agent
        .get_session(reply["sessionId"].as_str().unwrap())
        .unwrap();
    assert!(!session.config().interactive);
    assert_eq!(session.config().permission_policy, policy(expected.clone()));
    assert_eq!(
        session.get::<PermissionState>("main").unwrap().unwrap(),
        PermissionState {
            version: 0,
            policy: policy(expected),
        }
    );
    let listed = client.rpc("session.list", json!({})).await;
    assert!(
        listed["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|info| info["sessionId"] == reply["sessionId"]),
        "{listed}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn launch_caps_the_policy_by_the_starters_current_policy_or_the_host_defaults() {
    let (server, agent, sessions, mut client) = serve_sessions(None).await;
    let cwd = std::env::temp_dir();
    // Created with full access, then narrowed: the bound is the dispatched
    // policy, not the creation-time config.
    let starter = client_session(&mut client, &agent, json!({"preset":"full-access"})).await;
    let narrowed = json!({"preset":"workspace-write","allowedTools":["confirm","echo"],"disallowedTools":["wait"]});
    starter
        .service::<Permission>("main")
        .unwrap()
        .update(0, policy(narrowed.clone()))
        .await
        .unwrap();
    let at = json!({"sessionId":starter.id(),"branchId":"main"});

    let reply = sessions
        .ok(
            "launch",
            json!({"cwd":cwd,"starter":at,"policy":{"preset":"full-access","allowedTools":["confirm","other"],"disallowedTools":["x"]}}),
        )
        .await;
    assert_launched(
        &mut client,
        &agent,
        &reply,
        json!({"preset":"workspace-write","allowedTools":["confirm","echo"],"disallowedTools":["x","wait"]}),
    )
    .await;
    // No requested policy: the starter's own.
    let reply = sessions.ok("launch", json!({"cwd":cwd,"starter":at})).await;
    assert_launched(&mut client, &agent, &reply, narrowed).await;
    // No starter: capped by the host defaults; no policy means read-only.
    let reply = sessions
        .ok(
            "launch",
            json!({"cwd":cwd,"policy":{"preset":"full-access"}}),
        )
        .await;
    assert_launched(&mut client, &agent, &reply, host_policy()).await;
    let reply = sessions.ok("launch", json!({"cwd":cwd})).await;
    assert_launched(
        &mut client,
        &agent,
        &reply,
        json!({"preset":"read-only","allowedTools":[],"disallowedTools":["blocked"]}),
    )
    .await;

    for (request, code) in [
        (json!({"cwd":cwd,"agent":"missing"}), "agent_unknown"),
        (json!({"cwd":"relative"}), "rejected"),
        (
            json!({"cwd":cwd,"policy":{"allowedTools":[" "]}}),
            "rejected",
        ),
        (json!({"cwd":cwd,"source":"dim-scheduler"}), "rejected"),
        (
            json!({"cwd":cwd,"starter":{"sessionId":"missing","branchId":"main"}}),
            "session_closed",
        ),
        (
            json!({"cwd":cwd,"starter":{"sessionId":starter.id(),"branchId":"missing"}}),
            "branch_missing",
        ),
    ] {
        assert_eq!(
            sessions.error("launch", request.clone()).await,
            code,
            "{request}"
        );
    }
    assert_ok(
        &client
            .rpc("session.close", json!({"sessionId":starter.id()}))
            .await,
    );
    assert_eq!(
        sessions
            .error("launch", json!({"cwd":cwd,"starter":at}))
            .await,
        "session_closed"
    );
    // Shape errors are data too.
    for (method, args) in [
        ("launch", json!([])),
        ("launch", json!([{"cwd":cwd}, {}])),
        ("close", json!([{}])),
    ] {
        let reply = sessions.raw(method, args).await;
        assert_eq!(reply["error"]["code"], "rejected", "{reply}");
    }
    assert_eq!(agent.sessions().len(), 4, "failed launches create nothing");
    server.shutdown();
    server.join().await.unwrap();
}

/// A private directory for a native store, removed on drop.
struct Store(std::path::PathBuf);
impl Store {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let path = std::path::PathBuf::from("/tmp").join(format!("dss-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
    fn database(&self) -> std::path::PathBuf {
        self.0.join("dimcode.sqlite")
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn launched(sessions: &Sessions, agent: &Agent) -> Session {
    let reply = sessions
        .ok("launch", json!({"cwd":std::env::temp_dir()}))
        .await;
    agent
        .get_session(reply["sessionId"].as_str().unwrap())
        .unwrap()
}
fn input(session: &Session, submission: &str, content: Value) -> Value {
    json!({"sessionId":session.id(),"branchId":"main","mode":"queue","submissionId":submission,
        "rawBlocks":[{"text":"<dim-test kind=\"run\">marker</dim-test>"}],"content":content})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_replies_after_publication_and_lookup_waits_for_committed_inputs() {
    let store = Store::new();
    let (server, agent, sessions, mut client) = serve_sessions(Some(store.database())).await;
    client_session(&mut client, &agent, host_policy()).await;
    let session = launched(&sessions, &agent).await;
    let submission = format!("test:{}", uuid::Uuid::new_v4());
    let mut request = input(
        &session,
        &submission,
        json!([{"type":"text","text":"hello"}]),
    );
    request["rawBlocks"].as_array_mut().unwrap().extend([
        json!({"text":"live","key":{"use":"live","name":"test/live"}}),
        json!({"text":"retire","key":{"use":"retire","name":"test/old"}}),
    ]);
    let receipt = sessions.ok("input", request).await;
    let sequence = receipt["sequence"].as_u64().unwrap();
    // Replied after publication: readers see it at once.
    assert!(session.sequence("main").unwrap() >= sequence);
    let event = session
        .local("main", sequence..=sequence)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(event.id, receipt["inputId"].as_str().unwrap());
    let EventKind::InputMessage(data) = &event.kind else {
        panic!("{event:?}")
    };
    assert_eq!(
        *data,
        InputMessageData {
            submission_id: Some(submission.clone()),
            user: Some(UserInput {
                content: vec![dim_agent_types::ContentBlock::Text {
                    text: "hello".into()
                }],
            }),
            blocks: vec![
                InputBlock::Raw {
                    text: "<dim-test kind=\"run\">marker</dim-test>".into(),
                    key: None,
                    retire: false,
                },
                InputBlock::Raw {
                    text: "live".into(),
                    key: Some("test/live".into()),
                    retire: false,
                },
                InputBlock::Raw {
                    text: "retire".into(),
                    key: Some("test/old".into()),
                    retire: true,
                },
            ],
            mode: Some(dim_agent_types::InputMode::Queue),
        }
    );
    let at = |submission: &str| json!({"sessionId":session.id(),"branchId":"main","submissionId":submission});
    assert_eq!(
        sessions.ok("lookup", at(&submission)).await,
        json!({"status":"accepted","inputId":event.id,"sequence":sequence})
    );
    assert_eq!(
        sessions.ok("lookup", at("test:other")).await,
        json!({"status":"unknown"})
    );
    for _ in 0..2 {
        sessions
            .ok("input", input(&session, "test:twice", json!([])))
            .await;
    }
    assert_eq!(
        sessions.error("lookup", at("test:twice")).await,
        "ambiguous"
    );
    let mut missing_mode = input(&session, "test:invalid", json!([]));
    missing_mode.as_object_mut().unwrap().remove("mode");
    let mut empty = input(&session, "test:invalid", json!([{"type":"text","text":""}]));
    empty["rawBlocks"] = json!([]);
    let mut bad_key = input(&session, "test:invalid", json!([]));
    bad_key["rawBlocks"][0]["key"] = json!({"use":"forever","name":"test/key"});
    let mut no_branch = input(&session, "test:invalid", json!([]));
    no_branch["branchId"] = json!("missing");
    let mut no_session = input(&session, "test:invalid", json!([]));
    no_session["sessionId"] = json!("missing");
    for (request, code) in [
        (input(&session, "", json!([])), "rejected"),
        (missing_mode, "rejected"),
        (empty, "rejected"),
        (bad_key, "rejected"),
        (no_branch, "branch_missing"),
        (no_session, "session_closed"),
    ] {
        assert_eq!(
            sessions.error("input", request.clone()).await,
            code,
            "{request}"
        );
    }
    assert_eq!(sessions.error("lookup", at("")).await, "rejected");
    assert_eq!(
        sessions.ok("lookup", at("test:invalid")).await,
        json!({"status":"unknown"}),
        "rejected inputs write nothing"
    );

    // Hold publication: the input commits, its reply waits, and a lookup
    // that starts after the commit waits too instead of reading `unknown`.
    session.idle("main").await.unwrap();
    let publication = session.publication("main").unwrap();
    let lock = rusqlite::Connection::open(store.database()).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let pending = format!("test:{}", uuid::Uuid::new_v4());
    let write = tokio::spawn({
        let sessions = sessions.clone();
        let request = input(&session, &pending, json!([]));
        async move { sessions.call("input", request).await }
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !publication.has_pending() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let found = tokio::spawn({
        let sessions = sessions.clone();
        let request = at(&pending);
        async move { sessions.call("lookup", request).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!write.is_finished() && !found.is_finished());
    lock.execute_batch("COMMIT").unwrap();
    let receipt = write.await.unwrap();
    assert_eq!(receipt["ok"], true, "{receipt}");
    assert_eq!(
        found.await.unwrap()["value"],
        json!({"status":"accepted","inputId":receipt["value"]["inputId"],"sequence":receipt["value"]["sequence"]})
    );
    server.shutdown();
    server.join().await.unwrap();
}
