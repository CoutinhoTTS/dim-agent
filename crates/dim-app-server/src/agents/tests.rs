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

mod slots;

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

async fn launch(index: &Arc<AgentIndex>, agent: Option<String>, cwd: &Path) -> String {
    index
        .launch(LaunchRequest {
            agent,
            cwd: cwd.to_str().unwrap().into(),
            model: None,
            policy: None,
            starter: None,
        })
        .await
        .unwrap()
        .session_id
}
/// A new peer Agent. Closing it keeps its record: a stored Agent that is not
/// running, which the next generation creates again.
async fn peer(index: &AgentIndex) -> String {
    let host = index.create(uuid::Uuid::new_v4(), None).await.unwrap();
    host.agent_id.clone()
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
    let session = launch(&index, None, &directory).await;
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

/// An execution-disabled host keeps its gate: it loads neither the default
/// nor a stored Agent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn execution_disabled_hosts_load_no_stored_agent() {
    let stored = stored().await;
    let mut held = HeldCreation::new(&stored.default_agent);
    let server = crate::serve(
        builder(),
        ServerConfig {
            session_execution: false,
            ..config(&stored.directory)
        },
    )
    .await
    .unwrap();
    assert!(server.agent().is_none());
    assert!(server.root.get::<AgentIndex>().unwrap().hosts().is_empty());
    assert!(
        held.arrived.try_recv().is_err(),
        "no Agent creation started"
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(stored.directory).unwrap();
}

/// `serve` covers owner initialization with its startup deadline: a stored
/// Agent that exhausts it fails startup, whose cleanup leaves the store to
/// the next host.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stored_agent_exhausting_the_startup_deadline_fails_startup() {
    let stored = stored().await;
    let mut held = HeldCreation::new(&stored.default_agent);
    let mut starting = std::pin::pin!(crate::serve(
        builder(),
        ServerConfig {
            startup_timeout: Duration::from_secs(2),
            ..config(&stored.directory)
        },
    ));
    tokio::select! {
        _ = held.arrived() => {}
        result = &mut starting => panic!("startup ended unheld: {:?}", result.err()),
    }
    let error = match starting.await {
        Err(error) => error.to_string(),
        Ok(server) => {
            server.shutdown();
            server.join().await.unwrap();
            panic!("startup outlived its deadline");
        }
    };
    assert!(error.contains("timed out"), "{error}");
    drop(held);
    let server = crate::serve(builder(), config(&stored.directory))
        .await
        .unwrap();
    assert_eq!(
        call(&server.root, "lookup", lookup(&stored.session)).await,
        stored.accepted
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(stored.directory).unwrap();
}

/// A failure reading the stored Agents is a returned initialization error:
/// startup fails with it and cleans up, and is not recorded as an unavailable
/// Agent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_owner_read_fails_startup_and_cleans_up() {
    let stored = stored().await;
    let database = stored.directory.join("dimcode.sqlite");
    let mark_default = |default: bool| {
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute(
                "UPDATE agents SET is_default = ?1 WHERE agent_id = ?2",
                rusqlite::params![default, stored.default_agent],
            )
            .unwrap();
    };
    mark_default(false);
    let error = match crate::serve(builder(), config(&stored.directory)).await {
        Err(error) => error.to_string(),
        Ok(server) => {
            server.shutdown();
            server.join().await.unwrap();
            panic!("startup ignored an unreadable Agent directory");
        }
    };
    assert!(error.contains("no unique default Agent"), "{error}");
    mark_default(true);
    let server = crate::serve(builder(), config(&stored.directory))
        .await
        .unwrap();
    assert_eq!(
        call(&server.root, "lookup", lookup(&stored.session)).await,
        stored.accepted
    );
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(stored.directory).unwrap();
}

/// `restart("Agents")` keeps the per-Agent creation timeout and no overall
/// deadline: a stored Agent whose creation times out is recorded
/// unavailable, initialization continues, loaded Sessions stay loaded and
/// the unavailable owner's Sessions keep their real error.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agents_restart_records_a_creation_timeout_and_keeps_loaded_sessions() {
    let stored = stored().await;
    let server = crate::serve(
        builder(),
        ServerConfig {
            startup_timeout: Duration::from_secs(2),
            ..config(&stored.directory)
        },
    )
    .await
    .unwrap();
    let index = server.root.get::<AgentIndex>().unwrap();
    assert_eq!(
        call(&server.root, "lookup", lookup(&stored.session)).await,
        stored.accepted
    );
    let (_, loaded) = index.session(&stored.session).unwrap();
    let peer = peer(&index).await;
    let peer_session = launch(&index, Some(peer.clone()), &stored.directory).await;
    index.close(&peer).await.unwrap();
    let mut held = HeldCreation::new(&peer);
    let (restarted, ()) = tokio::join!(server.restart("Agents"), async {
        let root = held.arrived().await;
        let reply = call(&root, "lookup", lookup(&stored.session)).await;
        assert_eq!(reply["error"]["code"], "reloading", "{reply}");
    });
    restarted.unwrap();
    let index = server.root.get::<AgentIndex>().unwrap();
    let reason = index.unavailable_agent(&peer).unwrap();
    assert!(reason.contains("timed out"), "{reason}");
    assert!(index.host(Some(&peer)).is_none());
    assert!(index.session(&stored.session).is_some() && !loaded.is_unloading());
    assert_eq!(
        call(&server.root, "lookup", lookup(&stored.session)).await,
        stored.accepted
    );
    // The unavailable owner's Session keeps its real error.
    let reply = call(&server.root, "lookup", lookup(&peer_session)).await;
    assert_eq!(reply["error"]["code"], "failed", "{reply}");
    assert_eq!(reply["error"]["retryable"], false, "{reply}");
    drop(held);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(stored.directory).unwrap();
}

/// A dependency-driven Agents reload initializes owners as a restart does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dependency_reload_records_a_creation_timeout_and_continues() {
    let home = std::env::temp_dir().join(format!("dim-owners-{}", uuid::Uuid::new_v4()));
    let server = crate::serve(
        builder(),
        ServerConfig {
            product_home: Some(home.clone()),
            target_home: Some(home.join("native")),
            startup_timeout: Duration::from_secs(2),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let index = server.root.get::<AgentIndex>().unwrap();
    let peer = peer(&index).await;
    index.close(&peer).await.unwrap();
    let mut held = HeldCreation::new(&peer);
    // Agents injects Settings in the product profile.
    let (restarted, ()) = tokio::join!(server.processes.loader.restart("Settings"), async {
        let root = held.arrived().await;
        let reply = call(&root, "lookup", lookup("unknown")).await;
        assert_eq!(reply["error"]["code"], "reloading", "{reply}");
    });
    restarted.unwrap();
    server
        .processes
        .wait_active(&server.root, Duration::from_secs(10))
        .await
        .unwrap();
    let index = server.root.get::<AgentIndex>().unwrap();
    let reason = index.unavailable_agent(&peer).unwrap();
    assert!(reason.contains("timed out"), "{reason}");
    assert!(index.default_host().is_some());
    drop(held);
    server.shutdown();
    server.join().await.unwrap();
    std::fs::remove_dir_all(home).unwrap();
}
