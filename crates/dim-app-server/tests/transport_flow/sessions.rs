//! The Agents `sessions` service (sessions service design §16), called by
//! name through `Keyed<dyn HostDispatch>` as a process plugin calls it.
use super::*;
use dim_agent_capabilities::permission::{Permission, PermissionState};
use dim_agent_runtime::{Agent, Session};
use dim_agent_types::PermissionPolicy;
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
async fn serve_sessions() -> (RunningServer, Agent, Sessions, Client) {
    let sessions = Sessions::default();
    let consumer = sessions.clone();
    let mut config = ServerConfig::default();
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
    let (server, agent, sessions, mut client) = serve_sessions().await;
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
