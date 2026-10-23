//! Admission slots (#345): Agents keeps a Session's slot while something
//! holds it, while the Session is loaded, and while it records the close of
//! a Session whose rows remain (S5c D5); every other slot goes. Clients do
//! not see slots, so these flows drive Sessions through Rpc and count the
//! index's map here instead of through a test API. The grace period is
//! longer than any test: each idle unload pass runs when the test calls it,
//! so every count is exact.
use super::{AgentIndex, builder, config};
use crate::{RunningServer, ServerConfig, agents::SessionSlot};
use dim_agent_runtime::{
    AgentBuilder, Capability, InSession, KeysOf, SessionIdentity, SessionScope, scoped,
};
use futures_util::{SinkExt, StreamExt};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};
use tokio::{net::TcpStream, sync::Notify};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

const DEADLINE: Duration = Duration::from_secs(20);
/// The idle unload pass's marks, kept by the test between passes.
type Marks = HashMap<String, (Arc<SessionSlot>, u64)>;

/// A disposable native store.
struct Store(PathBuf);
impl Store {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("dim-slots-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }
    async fn serve(&self, builder: AgentBuilder) -> RunningServer {
        crate::serve(
            builder,
            ServerConfig {
                idle_unload_after: Duration::from_secs(3600),
                ..config(&self.0)
            },
        )
        .await
        .unwrap()
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A protocol v3 client over the WebSocket transport.
struct Client(WebSocketStream<MaybeTlsStream<TcpStream>>, u64);
impl Client {
    async fn new(server: &RunningServer) -> Self {
        let address = server.websocket_address.unwrap();
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
            .await
            .unwrap();
        let mut client = Self(socket, 0);
        let hello = client
            .rpc("hello", json!({"protocolVersion":3,"clientKind":"other"}))
            .await;
        assert_eq!(hello["result"]["protocolVersion"], 3, "{hello}");
        client
    }
    /// The reply to one call; notices in between are skipped.
    async fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.1 += 1;
        let request = json!({"jsonrpc":"2.0","id":self.1,"method":method,"params":params});
        self.0
            .send(Message::Text(request.to_string().into()))
            .await
            .unwrap();
        loop {
            let message = tokio::time::timeout(DEADLINE, self.0.next())
                .await
                .unwrap_or_else(|_| panic!("deadline: reply to {method}"))
                .unwrap()
                .unwrap();
            if let Message::Text(text) = message {
                let reply: Value = serde_json::from_str(&text).unwrap();
                if reply["id"] == self.1 {
                    return reply;
                }
            }
        }
    }
    async fn ok(&mut self, method: &str, params: Value) -> Value {
        let reply = self.rpc(method, params).await;
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }
    async fn create(&mut self, store: &Store) -> String {
        let created = self
            .ok("session.create", json!({"cwd":store.0,"tools":[]}))
            .await;
        created["sessionId"].as_str().unwrap().to_owned()
    }
    /// The ids `method` (`session.list` or `session.summaries`) lists.
    async fn listed(&mut self, method: &str) -> Vec<String> {
        let mut ids: Vec<String> = self
            .ok(method, json!({}))
            .await
            .as_array()
            .unwrap()
            .iter()
            .map(|info| info["sessionId"].as_str().unwrap().to_owned())
            .collect();
        ids.sort();
        ids
    }
    async fn app_error(&mut self, method: &str, id: &str) -> Value {
        self.rpc(method, json!({"sessionId":id})).await["error"]["data"]["appError"].clone()
    }
}

/// The slots the index keeps, and how many of them are closed.
fn slots(index: &AgentIndex) -> (usize, usize) {
    let inner = index.directory.inner.lock().unwrap();
    let closed = inner
        .sessions
        .values()
        .filter(|slot| slot.closed.load(Ordering::Acquire))
        .count();
    (inner.sessions.len(), closed)
}
/// The leases on Session `id`, as the pass reads them from its slot.
fn uses(index: &AgentIndex, id: &str) -> Option<usize> {
    let inner = index.directory.inner.lock().unwrap();
    inner
        .sessions
        .get(id)
        .map(|slot| slot.uses.load(Ordering::Acquire))
}
/// Idle unload passes until `loaded` Sessions remain loaded. A Session that
/// was just loaded can refuse once while its resume is dispatched.
async fn settle(index: &AgentIndex, marks: &mut Marks, loaded: usize) {
    tokio::time::timeout(DEADLINE, async {
        loop {
            index.unload_idle(marks).await;
            if index.session_count() == loaded {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("deadline: {loaded} Sessions loaded"));
}

/// The acceptance flow of #345: many Sessions created, listed, unloaded,
/// closed and deleted. At every step the slots are those of the loaded
/// Sessions, of the closed Sessions whose rows remain, and of what a caller
/// still holds; listing creates none. The closes outlive every pass, and a
/// Session whose slot went loads again on its next use.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slots_stay_bounded_by_loaded_sessions_and_kept_closes() {
    const N: usize = 12;
    let store = Store::new();
    let server = store.serve(builder()).await;
    let index = server.root.get::<AgentIndex>().unwrap();
    let mut client = Client::new(&server).await;
    let mut marks = Marks::new();
    let mut ids = vec![];
    for _ in 0..N {
        ids.push(client.create(&store).await);
    }
    ids.sort();
    assert_eq!((index.session_count(), slots(&index)), (N, (N, 0)));
    assert_eq!(client.listed("session.list").await, ids);

    // Unloading a Session drops its slot; listing it again creates none.
    settle(&index, &mut marks, 0).await;
    assert_eq!(slots(&index), (0, 0));
    assert_eq!(client.listed("session.list").await, ids);
    assert_eq!(client.listed("session.summaries").await, ids);
    assert_eq!(slots(&index), (0, 0));

    let (closed, rest) = ids.split_at(N / 3);
    let (deleted, unloaded) = rest.split_at(N / 3);
    // A close restores its Session, and its rows remain: its slot stays,
    // closed.
    for id in closed {
        client.ok("session.close", json!({"sessionId":id})).await;
    }
    assert_eq!((index.session_count(), slots(&index)), (0, (N / 3, N / 3)));

    // A delete removes the rows and the slot. A lease taken before it keeps
    // the deleted Session's slot, closed, until the lease ends and a pass
    // runs.
    let lease = index.acquire(&deleted[0]).await.unwrap();
    for id in deleted {
        client.ok("session.delete", json!({"sessionId":id})).await;
    }
    assert_eq!(slots(&index), (N / 3 + 1, N / 3 + 1));
    assert_eq!(
        client.app_error("session.attach", &deleted[0]).await,
        "session_closed"
    );
    drop(lease);
    index.unload_idle(&mut marks).await;
    assert_eq!(slots(&index), (N / 3, N / 3));

    // The closes outlive the passes: closed Sessions are neither listed nor
    // loaded again in this process.
    assert_eq!(client.listed("session.list").await, unloaded);
    for id in closed {
        assert_eq!(
            client.app_error("session.attach", id).await,
            "session_closed"
        );
    }
    assert_eq!(index.session_count(), 0);

    // An unloaded Session whose slot went is restored by its next use, on a
    // new slot that carries its lease to the pass.
    let attach = json!({"sessionId":unloaded[0]});
    client.ok("session.attach", attach.clone()).await;
    assert_eq!(uses(&index, &unloaded[0]), Some(1));
    index.unload_idle(&mut marks).await;
    index.unload_idle(&mut marks).await;
    assert_eq!(
        (index.session_count(), slots(&index)),
        (1, (N / 3 + 1, N / 3))
    );
    client.ok("session.detach", attach).await;
    settle(&index, &mut marks, 0).await;
    assert_eq!(slots(&index), (N / 3, N / 3));

    // Deleting a closed Session removes its rows, and with them its slot.
    for id in closed {
        client.ok("session.delete", json!({"sessionId":id})).await;
    }
    assert_eq!(slots(&index), (0, 0));
    assert_eq!(client.listed("session.list").await, unloaded);
    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}

/// Idle unload §6.1 proves "no lease since the mark" by an unchanged acquire
/// count, which only holds while the marked slot stays the same: a slot
/// made anew restarts its count. A Session released outside the pass after
/// its mark, as a fault or another caller of `unload_if_idle` releases it,
/// is not loaded, yet its mark keeps the slot through a prune. Restored
/// within the grace, it is not unloaded by the next pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mark_keeps_the_slot_of_a_session_released_outside_the_pass() {
    let store = Store::new();
    let server = store.serve(builder()).await;
    let index = server.root.get::<AgentIndex>().unwrap();
    let agent = server.agent().unwrap();
    let mut client = Client::new(&server).await;
    let id = client.create(&store).await;
    let mut marks = Marks::new();
    // One lease and none since: the pass marks the Session at count 1.
    drop(index.acquire(&id).await.unwrap());
    index.unload_idle(&mut marks).await;
    assert!(
        agent
            .get_session(&id)
            .unwrap()
            .unload_if_idle()
            .await
            .unwrap()
    );
    assert!(agent.get_session(&id).is_none());

    // Any prune now, the next pass's included, keeps the marked slot.
    index.directory.inner.lock().unwrap().sweep();
    assert_eq!(slots(&index), (1, 0));
    // A restore and its lease raise the marked count to 2.
    drop(index.acquire(&id).await.unwrap());
    index.unload_idle(&mut marks).await;
    assert!(
        agent.get_session(&id).is_some(),
        "a Session restored since its mark is not unloaded"
    );
    settle(&index, &mut marks, 0).await;
    assert_eq!(slots(&index), (0, 0));
    drop(client);
    server.shutdown();
    server.join().await.unwrap();
}

/// A pass while a restore holds a Session's gate: the Session is not loaded
/// yet, and only the restoring `acquire` holds its slot. The pass keeps the
/// slot, so the lease the restore ends with is on the slot the pass reads,
/// and it holds the Session until the client detaches.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pass_during_a_restore_keeps_the_slot_the_restore_holds() {
    let store = Store::new();
    let held = HeldRestore::default();
    let mut builder = builder();
    held.install(&mut builder);
    let server = store.serve(builder).await;
    let index = server.root.get::<AgentIndex>().unwrap();
    let mut client = Client::new(&server).await;
    let id = client.create(&store).await;
    let mut marks = Marks::new();
    settle(&index, &mut marks, 0).await;
    assert_eq!(slots(&index), (0, 0));

    held.hold(&id);
    let mut attaching = Client::new(&server).await;
    let attach = json!({"sessionId":id});
    let attached = tokio::spawn({
        let attach = attach.clone();
        async move {
            attaching.ok("session.attach", attach).await;
            attaching
        }
    });
    held.entered().await;
    assert!(server.agent().unwrap().get_session(&id).is_none());
    index.unload_idle(&mut marks).await;
    assert_eq!(slots(&index), (1, 0));
    held.release();
    let mut attaching = attached.await.unwrap();
    assert_eq!(uses(&index, &id), Some(1));
    index.unload_idle(&mut marks).await;
    index.unload_idle(&mut marks).await;
    assert_eq!(index.session_count(), 1);
    attaching.ok("session.detach", attach).await;
    settle(&index, &mut marks, 0).await;
    assert_eq!(slots(&index), (0, 0));
    drop((client, attaching));
    server.shutdown();
    server.join().await.unwrap();
}

/// Holds the next restore of one Session in its Session part's apply,
/// while `acquire` holds the Session's gate and slot.
#[derive(Clone, Default)]
struct HeldRestore(Arc<HeldState>);
#[derive(Default)]
struct HeldState {
    held: Mutex<Option<String>>,
    entered: Notify,
    release: Notify,
}
impl HeldRestore {
    fn install(&self, builder: &mut AgentBuilder) {
        let held = self.clone();
        builder.session_plugin(move |scope| HeldRestorePart(held.clone(), *scope));
    }
    fn hold(&self, id: &str) {
        *self.0.held.lock().unwrap() = Some(id.to_owned());
    }
    async fn entered(&self) {
        tokio::time::timeout(DEADLINE, self.0.entered.notified())
            .await
            .expect("the held restore started");
    }
    fn release(&self) {
        self.0.release.notify_one();
    }
}
struct HeldRestorePart(HeldRestore, SessionScope);
impl TypedPlugin for HeldRestorePart {
    type Deps = InSession<SessionIdentity>;
    fn name(&self) -> &str {
        "test.held-restore"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        InSession(identity): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let state = &self.0.0;
            let held = identity.restored
                && state
                    .held
                    .lock()
                    .unwrap()
                    .take_if(|id| *id == identity.session_id)
                    .is_some();
            if held {
                state.entered.notify_one();
                state.release.notified().await;
            }
            Ok(Effect::Done)
        })
    }
}
impl Capability for HeldRestorePart {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.1)
    }
}
