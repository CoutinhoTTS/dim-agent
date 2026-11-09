//! A Goal-first Session titles from its applied objective (#1265,
//! SessionTitle.md A10). The fixtures use a scripted local model or an
//! unreachable provider; no live provider is contacted.
#![cfg(unix)]
mod support;

use axum::{Router, http::StatusCode, routing::post};
use dim_agent_capabilities::{adapters::MockTools, defaults};
use dim_agent_runtime::{AgentBuilder, AgentConfig};
use dim_app_server::{RunningServer, ServerConfig, serve};
use dim_client::{Client, Message};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use support::*;
use tokio::sync::mpsc;

const PROVIDER: &str = "018ec73f-0000-7000-8000-00000000000a";

fn builder() -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_session_title::install(&mut builder);
    dim_goal::install(&mut builder);
    defaults::install(&mut builder);
    defaults::mock_model_hosts(&mut builder);
    defaults::tool_host(&mut builder, Arc::new(MockTools));
    dim_app_server::collaboration::install(&mut builder);
    builder
}

/// A served product profile whose title requests answer through the returned
/// channel: the local fixture stands in for the provider.
async fn scripted() -> (String, mpsc::UnboundedReceiver<Request>) {
    let (sent, received) = mpsc::unbounded_channel::<Request>();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/{*path}", post(complete))
        .with_state(sent);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}/v1"), received)
}

/// A local provider endpoint that fails every request, so the title falls
/// back to the excerpt without contacting anything live.
async fn failing() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/{*path}",
        post(|| async {
            (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({"error":{"message":"no model","type":"invalid_request_error"}})),
            )
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}/v1")
}

struct Flow {
    client: Client,
    id: String,
}

impl Flow {
    /// The caller keeps the returned server alive for the flow's lifetime.
    async fn start(store: &Store, base_url: &str, interactive: bool) -> (Self, RunningServer) {
        std::fs::write(
            store.0.join("plugins.json"),
            json!({"version":1,"installed":[],"names":{"session-title":{"use":"default"}}})
                .to_string(),
        )
        .unwrap();
        let server = serve(
            builder(),
            ServerConfig {
                unix_path: Some(PathBuf::from("/tmp").join(format!(
                    "dim-goal-title-{}/server.sock",
                    uuid::Uuid::new_v4()
                ))),
                product_home: Some(store.0.clone()),
                ..store.config()
            },
        )
        .await
        .unwrap();
        let mut client = client(&server).await;
        client
            .rpc("provider.create", json!({"providerId":PROVIDER,"displayName":"Goal title fixture","connection":{"driverKind":"openai-compatible","baseUrl":base_url,"auth":{"type":"anonymous"}},"models":[{"modelId":"main","displayName":"Main","status":"active",
                "capabilities":dim_agent_types::CatalogCapabilities::with_limits(128_000, 16_384)}],"enabled":true}))
            .await
            .unwrap();
        client
            .rpc(
                "provider.select",
                json!({"providerId":PROVIDER,"modelId":"main"}),
            )
            .await
            .unwrap();
        // The explicit title model keeps the scripted provider in the request
        // path: the adopted-model lookup may run before the goal's first turn
        // writes `context/configured`.
        configure(
            &mut client,
            json!({"enabled":true,"maxLength":80,
                "model":{"providerId":PROVIDER,"modelId":"main"}}),
        )
        .await;
        // One automatic continuation per turn end, then Goal pauses: the flows
        // need a bounded run, not Goal's default no-limit policy.
        let read = client
            .rpc("settings.read", json!({"source":"goal","scope":"user"}))
            .await
            .unwrap();
        client
            .rpc(
                "settings.update",
                json!({"source":"goal","scope":"user","expectedRevision":read["revision"],
                "document":{"maxContinuations":1}}),
            )
            .await
            .unwrap();
        let created = client
            .rpc(
                "session.create",
                json!({"cwd":store.0,"tools":["wait"],"interactive":interactive,
            "permissionPolicy":{"preset":"full-access","allowedTools":[],"disallowedTools":[]}}),
            )
            .await
            .unwrap();
        let id = created["sessionId"].as_str().unwrap().to_owned();
        client
            .rpc("session.attach", json!({"sessionId":id}))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let reply = client
                    .rpc("session.rename", json!({"sessionId":id,"title":0}))
                    .await
                    .unwrap_err();
                if dim_client::app_error(&reply) == Some("invalid_params") {
                    break;
                }
                assert!(waiting(&reply), "{reply}");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the built-in SessionTitle part activates");
        (Self { client, id }, server)
    }

    async fn entry(&mut self) -> Value {
        self.client
            .rpc("session.summaries", json!({}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["sessionId"] == self.id)
            .unwrap()
            .clone()
    }

    async fn titled(&mut self, text: &str, source: &str) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let current = self.entry().await;
                if current["title"] == text
                    && current["titleSource"] == source
                    && self.client.notices.iter().any(|notice| {
                        matches!(notice,
                        Message::Notice { method, params }
                        if method == "session.notice" && params["sessionId"] == self.id.clone()
                            && params["kind"] == "title"
                            && params["data"]["titleRevision"] == current["titleRevision"])
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the committed title and notice arrive");
    }

    async fn goal(&mut self, objective: &str) {
        when_served(
            &mut self.client,
            "session.goal.set",
            json!({"sessionId":self.id,"branchId":"main","objective":objective}),
        )
        .await;
    }

    async fn goal_paused(&mut self) {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let state = self
                    .client
                    .rpc(
                        "session.goal.get",
                        json!({"sessionId":self.id,"branchId":"main"}),
                    )
                    .await
                    .unwrap();
                if state["goal"]["status"]["kind"] == "paused" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the goal's continuations settle on its limit");
    }

    async fn rename(&mut self, title: &str) {
        when_served(
            &mut self.client,
            "session.rename",
            json!({"sessionId":self.id,"title":title}),
        )
        .await;
    }

    async fn submit(&mut self, text: &str) {
        self.client
            .rpc(
                "session.submit",
                json!({"sessionId":self.id,"branchId":"main","text":text}),
            )
            .await
            .unwrap();
    }

    async fn idle(&mut self) {
        self.client
            .rpc(
                "session.idle",
                json!({"sessionId":self.id,"branchId":"main"}),
            )
            .await
            .unwrap();
    }
}

async fn held(
    requests: &mut mpsc::UnboundedReceiver<Request>,
) -> (Value, tokio::sync::oneshot::Sender<String>) {
    tokio::time::timeout(Duration::from_secs(10), requests.recv())
        .await
        .expect("the adopted model receives the title request")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn goal_first_session_titles_from_the_objective() {
    let (base, mut requests) = scripted().await;
    let store = Store::new();
    let (mut flow, _server) = Flow::start(&store, &base, true).await;

    flow.goal("Ship the probe").await;
    let (request, release) = held(&mut requests).await;
    // The source is the objective alone: not the record's policy text, not the
    // trigger's instruction line.
    assert_eq!(request["messages"][1]["content"], "Ship the probe");
    release.send("Objective title".into()).unwrap();

    flow.titled("Objective title", "auto").await;
    flow.goal_paused().await;
    flow.idle().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_later_user_turn_does_not_retitle_a_goal_titled_session() {
    let (base, mut requests) = scripted().await;
    let store = Store::new();
    let (mut flow, _server) = Flow::start(&store, &base, true).await;

    flow.goal("Ship the probe").await;
    let (_, release) = held(&mut requests).await;
    release.send("Objective title".into()).unwrap();
    flow.titled("Objective title", "auto").await;
    let revision = flow.entry().await["titleRevision"].clone();

    flow.submit("a later user turn").await;
    flow.idle().await;
    assert!(
        requests.try_recv().is_err(),
        "a later user turn does not request another title"
    );
    let entry = flow.entry().await;
    assert_eq!(entry["title"], "Objective title");
    assert_eq!(entry["titleRevision"], revision);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_user_first_session_keeps_its_turn_title() {
    let (base, mut requests) = scripted().await;
    let store = Store::new();
    let (mut flow, _server) = Flow::start(&store, &base, true).await;

    flow.submit("title from the turn").await;
    let (request, release) = held(&mut requests).await;
    assert_eq!(request["messages"][1]["content"], "title from the turn");
    release.send("Turn title".into()).unwrap();
    flow.titled("Turn title", "auto").await;

    flow.goal("A goal set after the title").await;
    // Let the goal's own inputs apply and its turn settle, so a title attempt
    // they triggered would already have reached the provider.
    flow.goal_paused().await;
    flow.idle().await;
    assert!(
        requests.try_recv().is_err(),
        "a later goal does not request another title"
    );
    assert_eq!(flow.entry().await["title"], "Turn title");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn goal_title_requires_an_interactive_leader() {
    let (base, mut requests) = scripted().await;
    let store = Store::new();
    let (mut flow, _server) = Flow::start(&store, &base, false).await;

    flow.goal("No background title").await;
    flow.goal_paused().await;
    flow.idle().await;
    assert!(
        requests.try_recv().is_err(),
        "a non-interactive Session never requests a title"
    );
    assert_eq!(flow.entry().await["title"], Value::Null);

    flow.rename("Manual background title").await;
    flow.titled("Manual background title", "manual").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manual_rename_wins_over_a_goal_title() {
    let (base, mut requests) = scripted().await;
    let store = Store::new();
    let (mut flow, _server) = Flow::start(&store, &base, true).await;

    flow.goal("Ship the probe").await;
    let (_, release) = held(&mut requests).await;
    flow.rename("Manual winner").await;
    release.send("Automatic loser".into()).unwrap();

    flow.titled("Manual winner", "manual").await;
    flow.idle().await;
    assert_eq!(flow.entry().await["title"], "Manual winner");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn goal_objective_excerpt_falls_back_without_a_model() {
    let base = failing().await;
    let store = Store::new();
    let (mut flow, _server) = Flow::start(&store, &base, true).await;

    flow.goal("Fallback objective").await;
    flow.titled("Fallback objective", "auto").await;
    flow.idle().await;
}
