//! Owner initialization of an Agents generation (#318) through `serve`, as
//! process owners see it through `sessions`. No public flow can call
//! `sessions` at a chosen point of that initialization, so these tests hold a
//! stored Agent's instance creation inside its creation timeout, as a slow
//! Agent would. The hold exists only in this crate's unit tests, like the
//! title-write hold of the storage flow tests.
use super::{
    AgentIndex,
    sessions::{InputWrite, LaunchRequest, RawBlock},
};
use crate::ServerConfig;
use dim_agent_runtime::{AgentBuilder, AgentConfig};
use dim_agent_types::InputMode;
use rutis::Ctx;
use rutis_bridge::session::{HostDispatch, host_key};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;

type Hold = (String, oneshot::Sender<Ctx>, oneshot::Receiver<()>);
static HOLDS: Mutex<Vec<Hold>> = Mutex::new(Vec::new());

/// Holds the next instance creation of one Agent until it is dropped.
struct HeldCreation {
    arrived: oneshot::Receiver<Ctx>,
    _release: oneshot::Sender<()>,
}
impl HeldCreation {
    fn new(agent_id: &str) -> Self {
        let (arrive, arrived) = oneshot::channel();
        let (release, released) = oneshot::channel();
        HOLDS
            .lock()
            .unwrap()
            .push((agent_id.to_owned(), arrive, released));
        Self {
            arrived,
            _release: release,
        }
    }
    /// The process root, once the held creation has started.
    async fn arrived(&mut self) -> Ctx {
        tokio::time::timeout(Duration::from_secs(10), &mut self.arrived)
            .await
            .expect("the held Agent creation starts")
            .unwrap()
    }
}
/// Called by `AgentIndex::load` inside the creation timeout.
pub(super) async fn creation_held(agent_id: &str, root: &Ctx) {
    let hold = {
        let mut holds = HOLDS.lock().unwrap();
        let index = holds.iter().position(|(held, ..)| held == agent_id);
        index.map(|index| holds.swap_remove(index))
    };
    if let Some((_, arrive, released)) = hold {
        let _ = arrive.send(root.clone());
        let _ = released.await;
    }
}

const SUBMISSION: &str = "test:stored-owner";

fn builder() -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::install(&mut builder);
    dim_agent_capabilities::defaults::mock_model_hosts(&mut builder);
    dim_agent_capabilities::defaults::tool_host(
        &mut builder,
        Arc::new(dim_agent_capabilities::adapters::MockTools),
    );
    builder
}
fn config(directory: &Path) -> ServerConfig {
    ServerConfig {
        database_path: Some(directory.join("dimcode.sqlite")),
        ..Default::default()
    }
}

/// One `sessions` call as a process owner makes it: the reply envelope.
async fn call(root: &Ctx, method: &str, request: Value) -> Value {
    let service = root
        .get_as::<dyn HostDispatch>(host_key("sessions"))
        .expect("the sessions service");
    match service.invoke(method, json!([request]).into()).unwrap() {
        rutis_bridge::session::Value::Reference(reply) if reply.is_future() => {
            reply.wait_async().await.unwrap().json().unwrap()
        }
        reply => panic!("sessions replies asynchronously: {:?}", reply.json()),
    }
}
fn lookup(session: &str) -> Value {
    json!({"sessionId": session, "branchId": "main", "submissionId": SUBMISSION})
}

/// A native store whose default Agent owns a Session with one input.
struct Stored {
    directory: PathBuf,
    default_agent: String,
    session: String,
    /// The `lookup` reply for the input.
    accepted: Value,
}
async fn stored() -> Stored {
    let directory = std::env::temp_dir().join(format!("dim-owners-{}", uuid::Uuid::new_v4()));
    let server = crate::serve(builder(), config(&directory)).await.unwrap();
    let index = server.root.get::<AgentIndex>().unwrap();
    let session = index
        .launch(LaunchRequest {
            agent: None,
            cwd: directory.to_str().unwrap().into(),
            model: None,
            policy: None,
            starter: None,
        })
        .await
        .unwrap()
        .session_id;
    index
        .input(InputWrite {
            session_id: session.clone(),
            branch_id: "main".into(),
            mode: InputMode::Append,
            submission_id: SUBMISSION.into(),
            raw_blocks: vec![RawBlock {
                text: "stored input".into(),
                key: None,
            }],
            content: vec![],
        })
        .await
        .unwrap();
    let accepted = call(&server.root, "lookup", lookup(&session)).await;
    assert_eq!(accepted["value"]["status"], "accepted", "{accepted}");
    let default_agent = index.default_agent_id().to_owned();
    server.shutdown();
    server.join().await.unwrap();
    Stored {
        directory,
        default_agent,
        session,
        accepted,
    }
}

/// A process owner that looks up a committed input while the host starts
/// gets `reloading` until the stored owners are installed, then the input;
/// never a permanent `failed` for a healthy stored Session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_never_fails_a_stored_session_while_its_owner_installs() {
    let stored = stored().await;
    let mut held = HeldCreation::new(&stored.default_agent);
    let mut starting = std::pin::pin!(crate::serve(builder(), config(&stored.directory)));
    let root = tokio::select! {
        root = held.arrived() => root,
        result = &mut starting => panic!("startup ended unheld: {:?}", result.err()),
    };
    let reply = call(&root, "lookup", lookup(&stored.session)).await;
    assert_eq!(reply["error"]["code"], "reloading", "{reply}");
    assert_eq!(reply["error"]["retryable"], true, "{reply}");
    drop(held);
    let server = starting.await.unwrap();
    assert_eq!(
        call(&server.root, "lookup", lookup(&stored.session)).await,
        stored.accepted
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(stored.directory).unwrap();
}
