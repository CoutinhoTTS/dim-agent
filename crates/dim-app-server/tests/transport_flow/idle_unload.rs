//! Idle Session unload (idle unload design §13) on disposable native stores
//! with a grace period of a few hundred milliseconds. Every wait ends on an
//! observed transition: the release of a loaded instance (`Session::closed`),
//! the loaded count in `agent.list`, or a call's reply.
use super::super::deletion::eventually;
use super::*;
use dim_agent_runtime::{SessionIdentity, TimelineTreeControl};
use std::sync::Weak;
use tokio::sync::Notify;

const GRACE: Duration = Duration::from_millis(200);
/// Deadline of one transition; a passing run stays far below it.
const DEADLINE: Duration = Duration::from_secs(20);

/// Every load of a Session, creation or restore, with its Timeline tree held
/// weakly: once a released instance is no longer referenced anywhere, its
/// tree is gone. It can also hold one Session's next unload in its cleanup,
/// keeping that release under way until the test lets it finish.
#[derive(Clone, Default)]
pub(super) struct Loads(Arc<LoadsState>);
type Load = (String, bool, Weak<dyn TimelineTreeControl>);
#[derive(Default)]
struct LoadsState {
    loads: std::sync::Mutex<Vec<Load>>,
    held: std::sync::Mutex<Option<String>>,
    entered: Notify,
    release: Notify,
}
impl Loads {
    pub(super) fn install(&self, builder: &mut AgentBuilder) {
        let loads = self.clone();
        builder.session_plugin(move |scope| LoadsPart(loads.clone(), *scope));
    }
    /// How often `id` has loaded, and whether its last load restored it.
    fn of(&self, id: &str) -> (usize, bool) {
        let loads = self.0.loads.lock().unwrap();
        let mut of = loads.iter().filter(|(session, ..)| session == id);
        let count = of.clone().count();
        (
            count,
            of.next_back().is_some_and(|(_, restored, _)| *restored),
        )
    }
    /// Waits until no instance of `id` is referenced any more: nothing keeps
    /// a released Session alive.
    async fn released(&self, id: &str) {
        eventually(&format!("instances of {id} released"), || {
            self.0
                .loads
                .lock()
                .unwrap()
                .iter()
                .filter(|(session, ..)| session == id)
                .all(|(.., tree)| tree.upgrade().is_none())
        })
        .await;
    }
    /// Holds the next unload of `id` in its cleanup until [`Self::release`].
    fn hold(&self, id: &str) {
        *self.0.held.lock().unwrap() = Some(id.to_owned());
    }
    /// Waits until the held unload has started: the pass holds the
    /// Session's admission gate until it finishes.
    async fn entered(&self) {
        tokio::time::timeout(DEADLINE, self.0.entered.notified())
            .await
            .expect("the held unload started");
    }
    fn release(&self) {
        self.0.release.notify_one();
    }
}
struct LoadsPart(Loads, dim_agent_runtime::SessionScope);
impl TypedPlugin for LoadsPart {
    type Deps = (
        InSession<SessionIdentity>,
        InSession<dyn TimelineTreeControl>,
    );
    fn name(&self) -> &str {
        "test.loads"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a rutis::Ctx,
        (InSession(identity), InSession(tree)): Self::Deps,
    ) -> rutis::BoxFuture<'a, Result<rutis::Effect, rutis::CordisError>> {
        Box::pin(async move {
            let state = self.0.0.clone();
            state.loads.lock().unwrap().push((
                identity.session_id.clone(),
                identity.restored,
                Arc::downgrade(&tree),
            ));
            ctx.effect(move || {
                rutis::Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        let held = identity.unloading.is_cancelled()
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
                        Ok(())
                    })
                }))
            })?;
            Ok(rutis::Effect::Done)
        })
    }
}
impl Capability for LoadsPart {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.1)
    }
}

async fn serve_unloading(store: &Store, loads: &Loads, grace: Duration) -> Served {
    serve_with(Setup {
        database: Some(store.database()),
        grace: Some(grace),
        loads: Some(loads.clone()),
        ..Default::default()
    })
    .await
}

/// Waits for the release of this loaded instance.
async fn unloaded(session: Session) {
    tokio::time::timeout(DEADLINE, session.closed())
        .await
        .unwrap_or_else(|_| panic!("deadline: unload of {}", session.id()))
        .unwrap();
    assert!(session.is_unloading(), "{} was closed", session.id());
}
/// The default Agent's loaded Sessions, as `agent.list` reports them.
async fn loaded(client: &mut Client) -> u64 {
    let listed = client.rpc("agent.list", json!({})).await;
    listed["result"][0]["sessions"].as_u64().unwrap()
}
async fn launch(sessions: &Sessions) -> String {
    let reply = sessions
        .ok("launch", json!({"cwd":std::env::temp_dir()}))
        .await;
    reply["sessionId"].as_str().unwrap().to_owned()
}
fn at(id: &str) -> Value {
    json!({"sessionId":id,"branchId":"main"})
}
fn text_input(id: &str, submission: &str, text: &str) -> Value {
    json!({"sessionId":id,"branchId":"main","mode":"queue","submissionId":submission,
        "rawBlocks":[],"content":[{"type":"text","text":text}]})
}
/// `main` through `through`, in pages of `limit`.
async fn pages(sessions: &Sessions, id: &str, through: u64, limit: usize) -> Vec<Value> {
    let mut events = vec![];
    let mut after = 0;
    while after < through {
        let page = sessions
            .ok(
                "history",
                json!({"sessionId":id,"branchId":"main","after":after,"limit":limit,"through":through}),
            )
            .await;
        let page = page["events"].as_array().unwrap().clone();
        after = page.last().unwrap()["sequence"].as_u64().unwrap();
        events.extend(page);
    }
    events
}

/// §13 item 1: 50 runs through `sessions`, each one input, `idle` and the
/// `history` pages. Once dropped, all are unloaded after the grace and every
/// released instance is gone; `session.list` still lists each at its stored
/// head and its history reads in full. A later input restores one, and its
/// turn runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fifty_dropped_runs_unload_and_the_next_input_restores_one() {
    let store = Store::new();
    let loads = Loads::default();
    let Served {
        server,
        agent,
        sessions,
        mut client,
        ..
    } = serve_unloading(&store, &loads, GRACE).await;
    client_session(&mut client, &agent, host_policy()).await;
    let mut runs = vec![];
    for run in 0..50 {
        let id = launch(&sessions).await;
        sessions
            .ok("input", text_input(&id, &format!("test:{run}"), "hello"))
            .await;
        let through = sessions.ok("idle", at(&id)).await["sequence"]
            .as_u64()
            .unwrap();
        let events = pages(&sessions, &id, through, 4).await;
        runs.push((id, through, events));
    }
    eventually("every Session unloaded", || agent.sessions().is_empty()).await;
    assert_eq!(loaded(&mut client).await, 0);
    for (id, ..) in &runs {
        loads.released(id).await;
        assert_eq!(loads.of(id), (1, false), "{id} was not restored");
    }
    let listed = client.rpc("session.list", json!({})).await;
    for (id, through, _) in &runs {
        let info = listed["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|info| info["sessionId"] == *id)
            .unwrap_or_else(|| panic!("{id} listed: {listed}"));
        assert_eq!(info["sequence"], *through, "{info}");
    }

    let (first, through, _) = &runs[0];
    let receipt = sessions
        .ok("input", text_input(first, "test:again", "again"))
        .await;
    assert_eq!(loads.of(first), (2, true));
    let idle = sessions.ok("idle", at(first)).await["sequence"]
        .as_u64()
        .unwrap();
    let after = pages(&sessions, first, idle, 256).await;
    let after: Vec<_> = after
        .iter()
        .skip_while(|event| event["sequence"].as_u64().unwrap() <= *through)
        .collect();
    assert_eq!(after[0]["type"], "branch", "{after:?}");
    assert_eq!(after[0]["name"], "resume", "{after:?}");
    assert!(
        after
            .iter()
            .skip_while(|event| event["id"] != receipt["inputId"])
            .any(|event| event["type"] == "turn" && event["name"] == "end"),
        "the restored Session ran its turn: {after:?}"
    );
    for (id, through, events) in &runs {
        assert_eq!(&pages(&sessions, id, *through, 4).await, events, "{id}");
    }
    server.shutdown();
    server.join().await.unwrap();
}

/// §13 item 2: a run's calls (`input`, `idle`, three `history` pages) come
/// closer together than the grace period, so the pass never releases the
/// Session between them: it loads once and `main` gets no `branch/resume`.
/// Once the run is over the Session unloads.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_is_not_restored_between_its_calls() {
    const GRACE: Duration = Duration::from_millis(800);
    let store = Store::new();
    let loads = Loads::default();
    let Served {
        server,
        agent,
        sessions,
        mut client,
        ..
    } = serve_unloading(&store, &loads, GRACE).await;
    client_session(&mut client, &agent, host_policy()).await;
    let id = launch(&sessions).await;
    let session = agent.get_session(&id).unwrap();
    // Spaced so that passes run between the calls.
    let gap = || tokio::time::sleep(GRACE / 4);
    gap().await;
    sessions
        .ok("input", text_input(&id, "test:run", "hello"))
        .await;
    gap().await;
    let through = sessions.ok("idle", at(&id)).await["sequence"]
        .as_u64()
        .unwrap();
    let mut events = vec![];
    let mut after = 0;
    while after < through {
        gap().await;
        let page = sessions
            .ok(
                "history",
                json!({"sessionId":id,"branchId":"main","after":after,"limit":3,"through":through}),
            )
            .await;
        let page = page["events"].as_array().unwrap().clone();
        after = page.last().unwrap()["sequence"].as_u64().unwrap();
        events.extend(page);
    }
    assert!(events.len() > 6, "three pages or more: {events:?}");
    assert!(
        !events
            .iter()
            .any(|event| event["type"] == "branch" && event["name"] == "resume"),
        "{events:?}"
    );
    assert_eq!(loads.of(&id), (1, false));
    unloaded(session).await;
    loads.released(&id).await;
    server.shutdown();
    server.join().await.unwrap();
}

/// §13 item 11: without a native store no pass runs, so nothing is
/// unloaded. Nothing happens to wait for, so the test waits several grace
/// periods.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_is_unloaded_without_a_native_store() {
    let loads = Loads::default();
    let Served {
        server,
        agent,
        sessions,
        mut client,
        ..
    } = serve_with(Setup {
        grace: Some(GRACE),
        loads: Some(loads.clone()),
        ..Default::default()
    })
    .await;
    client_session(&mut client, &agent, host_policy()).await;
    let id = launch(&sessions).await;
    sessions
        .ok("input", text_input(&id, "test:run", "hello"))
        .await;
    sessions.ok("idle", at(&id)).await;
    tokio::time::sleep(GRACE * 5).await;
    assert_eq!(loaded(&mut client).await, 2);
    assert!(agent.get_session(&id).is_some());
    assert_eq!(loads.of(&id), (1, false));
    server.shutdown();
    server.join().await.unwrap();
}

/// A launched Session, idle after one turn: the loaded instance.
async fn idle_run(sessions: &Sessions, agent: &Agent) -> Session {
    let id = launch(sessions).await;
    sessions
        .ok("input", text_input(&id, "test:run", "hello"))
        .await;
    sessions.ok("idle", at(&id)).await;
    agent.get_session(&id).unwrap()
}

/// §13 item 3: a Session part holds the Session's unload in its cleanup, so
/// a release is under way when `sessions.history` and an Rpc
/// `session.history` arrive. Both wait for the admission gate the pass
/// holds, then read the restored Session; neither replies `session_closed`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn calls_during_an_unload_wait_and_read_the_restored_session() {
    let store = Store::new();
    let loads = Loads::default();
    let Served {
        server,
        agent,
        sessions,
        mut client,
        ..
    } = serve_unloading(&store, &loads, GRACE).await;
    client_session(&mut client, &agent, host_policy()).await;
    let session = idle_run(&sessions, &agent).await;
    let id = session.id().to_owned();
    let through = session.sequence("main").unwrap();
    loads.hold(&id);
    loads.entered().await;
    let native = tokio::spawn({
        let sessions = sessions.clone();
        let request = json!({"sessionId":id,"branchId":"main","through":through});
        async move { sessions.call("history", request).await }
    });
    let rpc = client
        .send(
            "session.history",
            json!({"sessionId":id,"branchId":"main","after":0,"limit":1000}),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!native.is_finished(), "the call waits for the release");
    loads.release();
    unloaded(session).await;
    let native = native.await.unwrap();
    assert_eq!(native["ok"], true, "{native}");
    assert_eq!(native["value"]["through"], through);
    let rpc = client.response(rpc).await;
    assert_ok(&rpc);
    // Read from the restored Session: its `branch/resume`, a control event
    // the wire filters, is in the range.
    assert!(
        rpc["result"]["through"].as_u64().unwrap() > through,
        "{rpc}"
    );
    assert_eq!(loads.of(&id), (2, true));
    server.shutdown();
    server.join().await.unwrap();
}

/// §13 item 9: an Agents index reload while a release is held under way, as
/// in item 3. The reload waits for it; no Session is lost; the next
/// generation unloads the lease-free Session the old pass never reached; and
/// a call after the reload restores the released one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_index_reload_waits_for_a_release_and_the_next_generation_unloads() {
    let store = Store::new();
    let loads = Loads::default();
    let Served {
        server,
        agent,
        sessions,
        mut client,
        ..
    } = serve_unloading(&store, &loads, GRACE).await;
    client_session(&mut client, &agent, host_policy()).await;
    let first = idle_run(&sessions, &agent).await;
    let first_id = first.id().to_owned();
    loads.hold(&first_id);
    loads.entered().await;
    // Created while the old pass is held in the first release.
    let second = idle_run(&sessions, &agent).await;
    let second_id = second.id().to_owned();
    {
        let restart = server.restart("Agents");
        tokio::pin!(restart);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut restart)
                .await
                .is_err(),
            "the old generation ends after its release"
        );
        loads.release();
        restart.await.unwrap();
    }
    unloaded(first).await;
    unloaded(second).await;
    let listed = client.rpc("session.list", json!({})).await;
    for id in [&first_id, &second_id] {
        assert!(
            listed["result"]
                .as_array()
                .unwrap()
                .iter()
                .any(|info| info["sessionId"] == **id),
            "{id}: {listed}"
        );
    }
    assert_eq!(loads.of(&second_id), (1, false));
    let read = sessions.ok("history", at(&first_id)).await;
    assert!(read["through"].as_u64().unwrap() > 0, "{read}");
    assert_eq!(loads.of(&first_id), (2, true));
    server.shutdown();
    server.join().await.unwrap();
}
