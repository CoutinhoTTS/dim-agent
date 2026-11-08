//! The opening handover of a rebuilding part ([ADR 020 D3](../../../docs/adr/020-timeline-read-convergence.md#d3-replay-is-removed)):
//! subscribe and hold live events, read the branch's facts with the query plus
//! its unpublished tail, then release only what lies above the cutoff, so
//! every fact is folded exactly once both when the tail is empty and when a
//! fact is written but not yet published.

use dim_agent_runtime::testing::{MinimalHost, timeline_events};
use dim_agent_runtime::*;
use dim_agent_types::*;
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use std::{
    collections::VecDeque,
    ops::RangeInclusive,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Publish callbacks a test holds back until it releases them.
type HeldPublishes = Arc<Mutex<VecDeque<(PublishCallback, Result<()>)>>>;

/// Memory storage whose publications wait for [`Self::release`], so a test can
/// write a fact and keep it unpublished.
struct Held {
    memory: MemoryTimelineStore,
    held: HeldPublishes,
    released: Arc<AtomicBool>,
}
impl Default for Held {
    fn default() -> Self {
        Self {
            memory: MemoryTimelineStore::default(),
            held: Arc::default(),
            released: Arc::new(AtomicBool::new(true)),
        }
    }
}
impl Held {
    /// Holds the publications of the batches appended from now on.
    fn hold(&self) {
        self.released.store(false, Ordering::SeqCst);
    }
    /// Publishes the held batches in commit order; later batches publish at
    /// once.
    fn release(&self) {
        let mut held = self.held.lock().unwrap();
        self.released.store(true, Ordering::SeqCst);
        while let Some((publish, result)) = held.pop_front() {
            publish(result);
        }
    }
}
impl TimelineStore for Held {
    fn query<'a>(
        &'a self,
        scope: &'a dim_agent_runtime::TimelineReadScope,
        query: &'a dim_agent_runtime::QuerySpec,
        page: Option<&'a dim_agent_runtime::QueryPage>,
    ) -> dim_agent_runtime::BoxFuture<'a, dim_agent_runtime::Result<dim_agent_runtime::QueryResult>>
    {
        self.memory.query(scope, query, page)
    }

    fn create_session<'a>(
        &'a self,
        id: &'a str,
        config: &'a SessionConfig,
    ) -> BoxFuture<'a, Result<()>> {
        self.memory.create_session(id, config)
    }
    fn rollback_session<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<()>> {
        self.memory.rollback_session(id)
    }
    fn release_session<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<()>> {
        self.memory.release_session(id)
    }
    fn session_config<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Option<SessionConfig>>> {
        self.memory.session_config(id)
    }
    fn branches<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Vec<StoredBranch>>> {
        self.memory.branches(id)
    }
    fn head<'a>(&'a self, key: &'a TimelineKey) -> BoxFuture<'a, Result<u64>> {
        self.memory.head(key)
    }
    fn batch_end<'a>(&'a self, key: &'a TimelineKey, sequence: u64) -> BoxFuture<'a, Result<()>> {
        self.memory.batch_end(key, sequence)
    }
    fn append(
        &self,
        key: TimelineKey,
        events: Vec<StoredEvent>,
        publish: PublishCallback,
    ) -> Result<()> {
        if self.released.load(Ordering::SeqCst) {
            return self.memory.append(key, events, publish);
        }
        let released = self.released.clone();
        let held = self.held.clone();
        self.memory.append(
            key,
            events,
            Box::new(move |result| {
                let mut queue = held.lock().unwrap();
                if released.load(Ordering::SeqCst) {
                    drop(queue);
                    publish(result);
                } else {
                    queue.push_back((publish, result));
                }
            }),
        )
    }
    fn read_segments<'a>(
        &'a self,
        id: &'a str,
        segments: Vec<(String, RangeInclusive<u64>)>,
    ) -> BoxFuture<'a, Result<Vec<Vec<StoredEvent>>>> {
        self.memory.read_segments(id, segments)
    }
    fn trace<'a>(
        &'a self,
        key: &'a TimelineKey,
        sequence: u64,
    ) -> BoxFuture<'a, Result<Option<TimelineTrace>>> {
        self.memory.trace(key, sequence)
    }
    fn save_snapshot<'a>(
        &'a self,
        key: &'a TimelineKey,
        snapshot: RecoverySnapshot,
    ) -> BoxFuture<'a, Result<()>> {
        self.memory.save_snapshot(key, snapshot)
    }
    fn latest_snapshot<'a>(
        &'a self,
        key: &'a TimelineKey,
        through: u64,
        interpreter: &'a str,
    ) -> BoxFuture<'a, Result<Option<RecoverySnapshot>>> {
        self.memory.latest_snapshot(key, through, interpreter)
    }
}

/// What the rebuilding part did: the facts it folded in order, and the read
/// that started it.
#[derive(Default)]
struct Rebuild {
    staged: Vec<Arc<AgentEvent>>,
    processed: Vec<u64>,
    read: bool,
    covered: u64,
    published: u64,
}

struct TailPart {
    timeline: Key<Timeline>,
    dispatcher: Key<Dispatcher>,
    lifecycle: Key<BranchLifecycle>,
    shared: Arc<Mutex<Rebuild>>,
    read_now: Arc<Mutex<Option<UnboundedReceiver<()>>>>,
    read_done: UnboundedSender<()>,
}
impl TailPart {
    fn new(
        names: &Names<'_>,
        shared: Arc<Mutex<Rebuild>>,
        read_now: Arc<Mutex<Option<UnboundedReceiver<()>>>>,
        read_done: UnboundedSender<()>,
    ) -> rutis::Result<Self> {
        Ok(Self {
            timeline: names.key()?,
            dispatcher: names.key()?,
            lifecycle: names.key()?,
            shared,
            read_now,
            read_done,
        })
    }
}
impl DynPlugin for TailPart {
    fn name(&self) -> &str {
        "test.tail-handover"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.timeline.erase(),
            self.dispatcher.erase(),
            self.lifecycle.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let timeline = ctx.require_at(&self.timeline)?;
            let dispatcher = ctx.require_at(&self.dispatcher)?;
            let lifecycle = ctx.require_at(&self.lifecycle)?;
            let shared = self.shared.clone();
            let staging = shared.clone();
            dispatcher.subscribe(ctx, "TailHandover", &["*"], move |event| {
                let mut rebuild = staging.lock().unwrap();
                if !rebuild.read {
                    rebuild.staged.push(event);
                } else if event.sequence > rebuild.covered {
                    rebuild.processed.push(event.sequence);
                }
                Box::pin(async { Ok(()) })
            })?;
            let read_now = self.read_now.lock().unwrap().take();
            let done = self.read_done.clone();
            let task = tokio::spawn(async move {
                let Some(mut read_now) = read_now else {
                    return;
                };
                if !lifecycle.activated().await {
                    return;
                }
                let Some(()) = read_now.recv().await else {
                    return;
                };
                let published = timeline.sequence();
                let reply = timeline
                    .query(page(4096, true))
                    .await
                    .expect("the read that adds the tail");
                let QueryOutput::Events { events } = reply.result else {
                    panic!("handover selects events");
                };
                let mut rebuild = shared.lock().unwrap();
                rebuild.covered = reply.through;
                rebuild.published = published;
                rebuild.read = true;
                rebuild
                    .processed
                    .extend(events.iter().map(|row| row.event.sequence));
                let staged = std::mem::take(&mut rebuild.staged);
                rebuild.processed.extend(
                    staged
                        .into_iter()
                        .map(|event| event.sequence)
                        .filter(|sequence| *sequence > reply.through),
                );
                drop(rebuild);
                let _ = done.send(());
            });
            ctx.effect(move || async move {
                task.abort();
                let _ = task.await;
                Ok(())
            })?;
            Ok(Effect::Done)
        })
    }
}

/// Runs one handover against a live branch: `tail` writes a fact and holds its
/// publication until after the rebuild read; otherwise everything is
/// published before it. Returns the read's cutoff and the published head it
/// saw, after folding the rest of the run's facts live.
async fn handover(tail: bool) -> (u64, u64) {
    let store = Arc::new(Held::default());
    let shared = Arc::new(Mutex::new(Rebuild::default()));
    let (now_tx, now_rx) = unbounded_channel();
    let (done_tx, mut done_rx) = unbounded_channel();
    let read_now = Arc::new(Mutex::new(Some(now_rx)));
    let mut builder = AgentBuilder::new(AgentConfig::default());
    dim_agent_capabilities::defaults::mock_hosts(&mut builder);
    dim_agent_capabilities::defaults::install(&mut builder);
    let part = shared.clone();
    builder.timeline_store(store.clone()).require_feature(
        Feature::new("test.tail-handover").branch("test.tail-handover", move |names| {
            Ok(Box::new(TailPart::new(
                names,
                part.clone(),
                read_now.clone(),
                done_tx.clone(),
            )?))
        }),
    );
    let agent = MinimalHost::start(&builder).await.unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    store.hold();
    let pending = session
        .submit("first turn".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    if !tail {
        store.release();
        session.idle("main").await.unwrap();
    }
    now_tx.send(()).unwrap();
    done_rx.recv().await.unwrap();
    let (published, covered) = {
        let rebuild = shared.lock().unwrap();
        (rebuild.published, rebuild.covered)
    };
    if tail {
        assert!(
            published < pending.sequence,
            "the input at {} is written and not published when the read runs; published {published}",
            pending.sequence
        );
        store.release();
    }
    session
        .submit("second turn".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    let head = session.idle("main").await.unwrap();
    let processed = shared.lock().unwrap().processed.clone();
    assert!(
        processed.iter().copied().eq(1..=head),
        "every fact folded once in sequence order, through {head}: {processed:?}"
    );
    agent.close().await.unwrap();
    (published, covered)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_handover_folds_every_fact_once_when_the_tail_is_unpublished() {
    let (published, covered) = handover(true).await;
    assert!(
        published < covered,
        "the read covers the unpublished tail: published {published}, cutoff {covered}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_handover_folds_every_fact_once_when_nothing_is_unpublished() {
    let (published, covered) = handover(false).await;
    assert_eq!(
        published, covered,
        "a published branch leaves the read's cutoff at its published head"
    );
}

fn input(text: &str) -> EventDraft {
    EventKind::InputMessage(InputMessageData::text(text, None)).into()
}

fn draft(kind: EventKind, id: &str) -> EventDraft {
    EventDraft::from(kind).with_id(id)
}

fn page(limit: u64, include_publishing: bool) -> TimelineQuery {
    TimelineQuery {
        scope: TimelineQueryScope::Branch,
        include_publishing,
        from: None,
        through: None,
        query: QuerySpec {
            filter: None,
            select: QuerySelect::Events,
            group_by: vec![],
            pick: None,
            order_by: vec![],
        },
        page: Some(QueryPage {
            limit: std::num::NonZeroU64::new(limit).unwrap(),
            after: None,
        }),
    }
}

fn ids(reply: &TimelineQueryReply) -> Vec<&str> {
    let QueryOutput::Events { events } = &reply.result else {
        panic!("the selection returns events");
    };
    events.iter().map(|row| row.event.id.as_str()).collect()
}

#[tokio::test]
async fn a_tail_cursor_continues_the_published_page_until_exhaustion() {
    let store = Arc::new(Held::default());
    store
        .create_session("tail", &SessionConfig::default())
        .await
        .unwrap();
    let timeline = Timeline::new(
        Ctx::root().unwrap(),
        TimelineKey {
            session_id: "tail".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    for id in ["1", "2", "3"] {
        timeline.append(input(id).with_id(id)).unwrap();
    }
    store.hold();
    for id in ["4", "5"] {
        timeline.append(input(id).with_id(id)).unwrap();
    }
    let mut pages = Vec::new();
    let mut query = page(2, true);
    loop {
        let reply = timeline.query(query.clone()).await.unwrap();
        assert_eq!(reply.through, 5);
        pages.push(
            ids(&reply)
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        );
        query.through = Some(reply.through);
        query.page.as_mut().unwrap().after = reply.next;
        if query.page.as_ref().unwrap().after.is_none() {
            break;
        }
    }
    assert_eq!(pages, vec![vec!["1", "2"], vec!["3", "4"], vec!["5"]]);
    let mut query = page(2, true);
    query.query.order_by = vec![QueryOrder {
        key: QueryOrderKey::Position,
        direction: TimelineQueryDirection::Descending,
    }];
    let newest = timeline.query(query.clone()).await.unwrap();
    assert_eq!(ids(&newest), ["5", "4"]);
    query.through = Some(newest.through);
    query.page.as_mut().unwrap().after = newest.next;
    let descending = timeline.query(query).await.unwrap();
    assert_eq!(
        ids(&descending),
        ["3", "2"],
        "a descending cursor moves from the tail into published facts"
    );
    let started = timeline
        .query(TimelineQuery {
            from: Some(TimelinePosition {
                branch_id: "main".into(),
                sequence: 4,
            }),
            ..page(2, true)
        })
        .await
        .unwrap();
    assert_eq!(ids(&started), ["4", "5"]);
    let mut lookup = page(1, true);
    lookup.query.filter = Some(QueryFilter::Compare {
        left: QueryOperand::Field { path: "id".into() },
        comparison: QueryComparison::Eq,
        right: QueryOperand::Value { value: "5".into() },
    });
    let found = timeline.query(lookup).await.unwrap();
    let QueryOutput::Events { events } = found.result else {
        panic!("the identity query selects events");
    };
    assert_eq!(
        events.len(),
        1,
        "the unpublished identity is found in the tail"
    );
    let from_tail_event = timeline
        .query(TimelineQuery {
            from: Some(events[0].position.clone()),
            through: Some(found.through),
            ..page(2, true)
        })
        .await
        .unwrap();
    assert_eq!(ids(&from_tail_event), ["5"]);
    store.release();
}

#[tokio::test]
async fn one_selection_reads_to_either_bound() {
    let store = Arc::new(Held::default());
    store
        .create_session("bounds", &SessionConfig::default())
        .await
        .unwrap();
    let timeline = Timeline::new(
        Ctx::root().unwrap(),
        TimelineKey {
            session_id: "bounds".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    for id in ["1", "2", "3"] {
        timeline.append(input(id).with_id(id)).unwrap();
    }
    store.hold();
    for id in ["4", "5"] {
        timeline.append(input(id).with_id(id)).unwrap();
    }
    let published = timeline.sequence();
    let committed = timeline.committed();
    let selection = page(16, false);
    let at_published = timeline.query(selection.clone()).await.unwrap();
    assert_eq!(ids(&at_published), vec!["1", "2", "3"]);
    assert_eq!(
        at_published.through, published,
        "a published read stops at what is published and reports that cutoff"
    );
    let at_committed = timeline
        .query(TimelineQuery {
            include_publishing: true,
            ..selection
        })
        .await
        .unwrap();
    assert_eq!(
        ids(&at_committed),
        vec!["1", "2", "3", "4", "5"],
        "the committed read joins the unpublished tail to the published page"
    );
    assert_eq!(
        at_committed.through, committed,
        "a committed read reports the write head it started at"
    );
    store.release();
}

#[tokio::test]
async fn property_pages_include_matching_unpublished_facts_at_the_same_cut() {
    let store = Arc::new(Held::default());
    store
        .create_session("fold", &SessionConfig::default())
        .await
        .unwrap();
    let timeline = Timeline::new(
        Ctx::root().unwrap(),
        TimelineKey {
            session_id: "fold".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    let drafts: Vec<EventDraft> = vec![
        input("one").with_id("1"),
        draft(
            EventKind::TurnStart(TurnStartData {
                turn_id: "t1".into(),
            }),
            "2",
        ),
        draft(
            EventKind::InputConsumed(InputConsumedData {
                input_id: "1".into(),
            }),
            "3",
        ),
        draft(
            EventKind::TurnEnd(TurnEndData {
                turn_id: "t1".into(),
                outcome: TurnOutcome::Completed {
                    reason: "end_turn".into(),
                },
            }),
            "4",
        ),
    ];
    for draft in drafts {
        timeline.append(draft).unwrap();
    }
    store.hold();
    let tail: Vec<EventDraft> = vec![
        input("two").with_id("5"),
        draft(
            EventKind::TurnStart(TurnStartData {
                turn_id: "t2".into(),
            }),
            "6",
        ),
        draft(
            EventKind::StopInterrupt(StopInterruptData {
                reason: "stop".into(),
                scope: InterruptScope::Turn,
            }),
            "7",
        ),
    ];
    for draft in tail {
        timeline.append(draft).unwrap();
    }
    let published = timeline.sequence();
    let through = timeline.committed();
    assert!(
        published < through,
        "the tail is written and not published: {published} < {through}"
    );
    let replay = timeline_events(
        &timeline,
        TimelineQueryScope::Branch,
        ..=timeline.committed(),
    )
    .await
    .unwrap();
    assert_eq!(
        replay.len(),
        7,
        "the fixture contains published and unpublished facts"
    );
    for kinds in [
        &["input/message"][..],
        &["turn/end"],
        &["input/message", "turn/start", "stop/interrupt"],
        &["approval/request"],
    ] {
        let expected: Vec<_> = replay
            .iter()
            .filter(|event| kinds.contains(&event.kind.key()))
            .map(|event| event.id.clone())
            .collect();
        let mut query = TimelineQuery {
            scope: TimelineQueryScope::Branch,
            from: None,
            through: Some(through),
            include_publishing: true,
            query: QuerySpec {
                filter: Some(QueryFilter::In {
                    value: QueryOperand::Field {
                        path: "kind".into(),
                    },
                    values: kinds.iter().map(|kind| (*kind).into()).collect(),
                }),
                select: QuerySelect::Fields {
                    fields: vec!["id".into()],
                },
                group_by: vec![],
                pick: None,
                order_by: vec![],
            },
            page: Some(QueryPage {
                limit: std::num::NonZeroU64::new(2).unwrap(),
                after: None,
            }),
        };
        let mut folded = Vec::new();
        loop {
            let reply = timeline.query(query.clone()).await.unwrap();
            assert_eq!(reply.through, through, "{kinds:?}");
            let QueryOutput::Rows { rows } = reply.result else {
                panic!("selected id fields")
            };
            for row in rows {
                let [QueryValue::Value { value }] = row.values.as_slice() else {
                    panic!("an event id")
                };
                folded.push(value.as_str().unwrap().to_owned());
            }
            let Some(next) = reply.next else { break };
            query.page.as_mut().unwrap().after = Some(next);
        }
        assert_eq!(folded, expected, "{kinds:?}");
        for unpublished in ["5", "6", "7"] {
            let kind = replay
                .iter()
                .find(|event| event.id == unpublished)
                .map(|event| event.kind.key().to_owned())
                .expect("the tail event");
            if kinds.contains(&kind.as_str()) {
                assert!(
                    folded.iter().any(|id| id == unpublished),
                    "{kinds:?}: the read adds the unpublished tail"
                );
            }
        }
    }
    store.release();
}
