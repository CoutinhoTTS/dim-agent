use dim_agent_runtime::{
    AgentError, BoxFuture, MemoryTimelineStore, PublishCallback, Result, SessionConfig,
    StoredBranch, StoredEvent, Timeline, TimelineKey, TimelineStore, TimelineTrace,
};
use dim_agent_types::*;
use rutis::Ctx;
use rutis::host::CtxHost;
use std::{
    ops::RangeInclusive,
    sync::{Arc, Mutex},
};
fn input(text: &str) -> EventDraft {
    EventKind::InputMessage(InputMessageData::text(text, None)).into()
}

/// Counts every segment read from the store, in call order.
#[derive(Default)]
struct Reads {
    memory: MemoryTimelineStore,
    segments: Mutex<Vec<(String, RangeInclusive<u64>)>>,
}
impl TimelineStore for Reads {
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
    fn append(
        &self,
        key: TimelineKey,
        events: Vec<StoredEvent>,
        publish: PublishCallback,
    ) -> Result<()> {
        self.memory.append(key, events, publish)
    }
    fn read_segments<'a>(
        &'a self,
        session_id: &'a str,
        segments: Vec<(String, RangeInclusive<u64>)>,
    ) -> BoxFuture<'a, Result<Vec<Vec<StoredEvent>>>> {
        Box::pin(async move {
            for (branch, range) in &segments {
                self.segments
                    .lock()
                    .unwrap()
                    .push((branch.clone(), range.clone()));
            }
            self.memory.read_segments(session_id, segments).await
        })
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
    fn query<'a>(
        &'a self,
        key: &'a TimelineKey,
        query: &'a dim_agent_runtime::TimelineQuery,
    ) -> BoxFuture<'a, Result<dim_agent_runtime::TimelineQueryReply>> {
        self.memory.query(key, query)
    }
}

/// Decodes a checkpoint whose resume is the sequence held in the snapshot
/// state, on `main`.
fn checkpoint_resume(
    snapshot: &RecoverySnapshot,
    _anchor: &AgentEvent,
) -> Option<ProjectionCheckpoint> {
    Some(ProjectionCheckpoint {
        resume: TimelinePosition {
            branch_id: "main".into(),
            sequence: snapshot.state.parse().ok()?,
        },
        state: ProjectionCheckpointState::default(),
    })
}
#[tokio::test]
async fn producer_preassigned_event_id_is_the_first_delta_part_id() {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let timeline = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store,
    );
    let _rewrite = timeline
        .before(&ctx, "rewrite", &["input/message"], |draft| {
            Ok(EventDraft::new(draft.kind))
        })
        .unwrap();
    let event = timeline
        .append(input("first part").with_id("part-1"))
        .unwrap();
    assert_eq!(event.id, "part-1");
}

#[tokio::test]
async fn before_pipeline_is_atomic_and_branch_snapshots_share_only_fixed_prefix() {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let timeline = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    let _first = timeline
        .before(&ctx, "first", &["input/message"], |mut draft| {
            if let EventKind::InputMessage(data) = &mut draft.kind {
                let ContentBlock::Text { text } =
                    &mut data.user.as_mut().expect("text input").content[0]
                else {
                    panic!("expected text input");
                };
                text.push_str(" changed");
            }
            Ok(draft)
        })
        .unwrap();
    let failing = timeline
        .before(&ctx, "second", &["input/message"], |_| {
            Err(AgentError::fault("extension", "failed"))
        })
        .unwrap();
    let original = timeline.append(input("one")).unwrap();
    assert_eq!(original.kind, input("one").kind);
    assert!(timeline.trace(1).await.unwrap().is_none());
    failing.dispose().await.unwrap();
    let changed = timeline.append(input("two")).unwrap();
    assert_eq!(changed.kind, input("two changed").kind);
    assert_eq!(timeline.trace(2).await.unwrap().unwrap().steps.len(), 1);
    let unchanged = timeline
        .before(&ctx, "identity", &["input/message"], Ok)
        .unwrap();
    let traced = timeline.append(input("trace")).unwrap();
    let trace = timeline.trace(traced.sequence).await.unwrap().unwrap();
    let handlers: Vec<_> = trace
        .steps
        .iter()
        .map(|step| step.handler.as_str())
        .collect();
    assert_eq!(handlers, ["identity", "first"]);
    assert!(trace.steps[0].result.is_none());
    unchanged.dispose().await.unwrap();
    timeline
        .append(
            EventKind::BranchCreate(BranchCreateData {
                at: BranchCreateAt::Sequence { sequence: 1 },
                source: BranchSource::Host {
                    operation_id: "fork".into(),
                },
                branch: "branch".into(),
                prompt: "branch prompt".into(),
            })
            .into(),
        )
        .unwrap();
    let branch = Timeline::new(
        rutis::Ctx::root().unwrap(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "branch".into(),
        },
        vec![("main".into(), 1)],
        store,
    );
    timeline.append(input("later")).unwrap();
    let local = branch.append(input("branch")).unwrap();
    let snapshot = branch.history(None).await.unwrap();
    assert_eq!(snapshot.len(), 2);
    assert!(Arc::ptr_eq(&snapshot[0], &original));
    assert_eq!(local.sequence, 1);
}
#[tokio::test]
async fn history_has_no_byte_quota_and_atomic_cuts_stay_fixed() {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let timeline = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store,
    );
    let large = "x".repeat(2 * 1024 * 1024);
    timeline.append(input(&large)).unwrap();
    let batch = timeline
        .append_batch_registered(vec![input("first"), input("second")], |_| Ok(()))
        .unwrap();
    assert_eq!(timeline.sequence(), 3);
    assert!(matches!(
        timeline.history(Some(batch[0].sequence)).await,
        Err(AgentError::InvalidInput(_))
    ));
    assert!(matches!(
        timeline.local(..=batch[0].sequence).await,
        Err(AgentError::InvalidInput(_))
    ));
    assert!(matches!(
        timeline.context(batch[0].sequence).await,
        Err(AgentError::InvalidInput(_))
    ));
    assert_eq!(
        timeline
            .history(Some(batch[1].sequence))
            .await
            .unwrap()
            .len(),
        3
    );
    assert_eq!(timeline.local(2..=3).await.unwrap().len(), 2);
    assert!(matches!(
        timeline.history(Some(4)).await,
        Err(AgentError::InvalidInput(_))
    ));
    dim_agent_runtime::core::close_timeline(&timeline)
        .await
        .unwrap();
    assert!(matches!(
        timeline.append(input("closed")),
        Err(AgentError::Closed(_))
    ));
}
#[tokio::test]
async fn before_can_read_history_and_rejects_nested_append() {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let timeline = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    timeline.append(input("existing")).unwrap();
    let reader = timeline.clone();
    let _hook = timeline
        .before(&ctx, "history-reader", &["input/message"], move |draft| {
            assert_eq!(reader.sequence(), 1);
            assert!(reader.append(input("recursive")).is_err());
            Ok(draft)
        })
        .unwrap();
    let event = timeline.append(input("second")).unwrap();
    assert_eq!(event.sequence, 2);
    assert_eq!(timeline.sequence(), 2);
}

#[tokio::test]
async fn failed_registration_commits_nothing() {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let timeline = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    timeline.append(input("existing")).unwrap();
    let registered = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = registered.clone();
    let error = timeline
        .append_registered(input("rejected"), |event| {
            seen.store(
                event.sequence.try_into().unwrap(),
                std::sync::atomic::Ordering::SeqCst,
            );
            Err(AgentError::InvalidInput("registration rejected".into()))
        })
        .unwrap_err();
    assert!(matches!(error, AgentError::InvalidInput(_)));
    assert_eq!(registered.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(timeline.sequence(), 1);
    let event = timeline
        .append_registered(input("accepted"), |_| Ok(()))
        .unwrap();
    assert_eq!(event.sequence, 2);
    assert_eq!(timeline.sequence(), 2);
}

#[tokio::test]
async fn panicking_before_keeps_the_original_fact_and_the_branch_usable() {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let timeline = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store,
    );
    let _hook = timeline
        .before(&ctx, "panicking-before", &["input/message"], |_| {
            panic!("injected before panic")
        })
        .unwrap();
    let original = input("original");
    let first = timeline.append(original.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&first.kind).unwrap(),
        serde_json::to_value(original.kind).unwrap()
    );
    assert_eq!(first.sequence, 1);
    let next = input("after panic");
    let second = timeline.append(next.clone()).unwrap();
    assert_eq!(second.sequence, 2);
    assert_eq!(
        serde_json::to_value(second.kind.clone()).unwrap(),
        serde_json::to_value(next.kind).unwrap()
    );
    assert_eq!(timeline.local(..).await.unwrap().len(), 2);
    dim_agent_runtime::core::close_timeline(&timeline)
        .await
        .unwrap();
}

#[tokio::test]
async fn unknown_storage_keys_cannot_create_sessions_or_branches() {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let main = TimelineKey {
        session_id: "session".into(),
        branch_id: "main".into(),
    };
    let empty_head = store.head(&main).await.unwrap();
    assert_eq!(empty_head, 0);
    let empty_range = 1..=empty_head;
    assert!(
        store
            .read(&main, empty_range.clone())
            .await
            .unwrap()
            .is_empty()
    );

    let ctx = Ctx::root().unwrap();
    let timeline = Timeline::new(ctx.clone(), main.clone(), vec![], store.clone());
    timeline.append(input("known")).unwrap();
    let record = store.read(&main, 1..=1).await.unwrap();
    for key in [
        TimelineKey {
            session_id: "missing".into(),
            branch_id: "main".into(),
        },
        TimelineKey {
            session_id: "session".into(),
            branch_id: "missing".into(),
        },
    ] {
        assert!(matches!(
            store.head(&key).await,
            Err(AgentError::Storage(_))
        ));
        assert!(matches!(
            store.read(&key, 1..=1).await,
            Err(AgentError::Storage(_))
        ));
        assert!(matches!(
            store.read(&key, empty_range.clone()).await,
            Err(AgentError::Storage(_))
        ));
        assert!(matches!(
            store.append(key.clone(), record.clone(), Box::new(|_| {})),
            Err(AgentError::Storage(_))
        ));
        assert!(matches!(
            store.head(&key).await,
            Err(AgentError::Storage(_))
        ));
    }
    assert!(store.session_config("missing").await.unwrap().is_none());
    assert_eq!(store.branches("session").await.unwrap().len(), 1);
    assert_eq!(store.head(&main).await.unwrap(), 1);
}

/// Finalizers of a closing scope still commit their terminal events; the
/// unloading `before` hooks no longer run, so the original is committed.
#[tokio::test]
async fn closing_scope_commits_the_original_event_without_before() {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let timeline = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store,
    );
    let _hook = timeline
        .before(&ctx, "change", &["input/message"], |mut draft| {
            if let EventKind::InputMessage(data) = &mut draft.kind {
                let ContentBlock::Text { text } =
                    &mut data.user.as_mut().expect("text input").content[0]
                else {
                    panic!("expected text input");
                };
                text.push_str(" changed");
            }
            Ok(draft)
        })
        .unwrap();
    let closing = ctx.shutdown();
    let event = timeline.append(input("late")).unwrap();
    assert_eq!(event.kind, input("late").kind);
    assert!(timeline.trace(event.sequence).await.unwrap().is_none());
    closing.await.unwrap();
}

#[tokio::test]
async fn before_keys_accept_a_domain_wildcard() {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let timeline = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store,
    );
    assert!(matches!(
        timeline.before(&ctx, "bad", &["input/*/x"], Ok),
        Err(AgentError::InvalidInput(_))
    ));
    let _hook = timeline
        .before(&ctx, "domain", &["input/*"], |mut draft| {
            draft.tags = Some(vec!["seen".into()]);
            Ok(draft)
        })
        .unwrap();
    let input = timeline.append(input("tagged")).unwrap();
    assert_eq!(input.tags, Some(vec!["seen".into()]));
    let other = timeline
        .append(
            EventKind::StopInterrupt(StopInterruptData {
                reason: "other domain".into(),
                scope: InterruptScope::Turn,
            })
            .into(),
        )
        .unwrap();
    assert_eq!(other.tags, None);
}

/// A checkpointed read reads the anchor as one row and never a range that
/// spans it to the cut, so the tail is not decoded twice.
#[tokio::test]
async fn a_checkpointed_read_decodes_one_anchor_event() {
    let store = Arc::new(Reads::default());
    store
        .memory
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let key = TimelineKey {
        session_id: "session".into(),
        branch_id: "main".into(),
    };
    let writer = Timeline::new(Ctx::root().unwrap(), key.clone(), vec![], store.clone());
    writer
        .append_batch_registered(
            (1..=50)
                .map(|index| input(&format!("prefix {index}")))
                .collect(),
            |_| Ok(()),
        )
        .unwrap();
    let anchor = writer.append(input("anchor")).unwrap();
    writer
        .save_snapshot(RecoverySnapshot {
            sequence: anchor.sequence,
            event_id: anchor.id.clone(),
            interpreter: "test/anchor".into(),
            state: "40".into(),
        })
        .await
        .unwrap();
    writer
        .append_batch_registered(
            (1..=1000)
                .map(|index| input(&format!("tail {index}")))
                .collect(),
            |_| Ok(()),
        )
        .unwrap();
    let reader = Timeline::restore(Ctx::root().unwrap(), key, vec![], store.clone())
        .await
        .unwrap();
    let through = reader.committed();
    assert_eq!(through, 1051);
    store.segments.lock().unwrap().clear();
    let context = reader
        .context_with_checkpoint(through, "test/anchor", checkpoint_resume)
        .await
        .unwrap();
    assert!(context.checkpoint.is_some());
    assert_eq!(context.events.len(), (through - 39) as usize);
    let segments = store.segments.lock().unwrap();
    assert_eq!(
        segments
            .first()
            .map(|(branch, range)| (branch.as_str(), range.clone())),
        Some(("main", 51..=51)),
        "the anchor check reads the anchor row"
    );
    assert!(
        !segments
            .iter()
            .any(|(_, range)| *range.start() >= 51 && *range.end() >= through),
        "no read runs from the anchor row to the cut"
    );
}

/// A live branch serves the anchor from its hot window: the anchor check makes
/// no store read, and the checkpoint is used.
#[tokio::test]
async fn a_checkpoint_anchored_in_the_hot_window_is_used() {
    let store = Arc::new(Reads::default());
    store
        .memory
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let key = TimelineKey {
        session_id: "session".into(),
        branch_id: "main".into(),
    };
    let writer = Timeline::new(Ctx::root().unwrap(), key.clone(), vec![], store.clone());
    writer
        .append_batch_registered(
            (1..=50)
                .map(|index| input(&format!("prefix {index}")))
                .collect(),
            |_| Ok(()),
        )
        .unwrap();
    let branch = Timeline::restore(Ctx::root().unwrap(), key, vec![], store.clone())
        .await
        .unwrap();
    let anchor = branch.append(input("anchor")).unwrap();
    branch
        .save_snapshot(RecoverySnapshot {
            sequence: anchor.sequence,
            event_id: anchor.id.clone(),
            interpreter: "test/anchor".into(),
            state: "40".into(),
        })
        .await
        .unwrap();
    branch
        .append_batch_registered(
            (1..=1000)
                .map(|index| input(&format!("tail {index}")))
                .collect(),
            |_| Ok(()),
        )
        .unwrap();
    let through = branch.committed();
    assert_eq!(through, 1051);
    store.segments.lock().unwrap().clear();
    let context = branch
        .context_with_checkpoint(through, "test/anchor", checkpoint_resume)
        .await
        .unwrap();
    assert!(context.checkpoint.is_some(), "the checkpoint is used");
    assert_eq!(context.events.len(), (through - 39) as usize);
    assert_eq!(
        *store.segments.lock().unwrap(),
        [("main".to_string(), 40..=through)],
        "the anchor comes from the hot window; only the context read reaches the store"
    );
}

/// An anchor row inside an atomic batch keeps its checkpoint: the point read
/// cannot end there, so the segment read finds the same anchor as before.
#[tokio::test]
async fn a_checkpoint_anchored_inside_an_atomic_batch_is_still_used() {
    let store = Arc::new(Reads::default());
    store
        .memory
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let key = TimelineKey {
        session_id: "session".into(),
        branch_id: "main".into(),
    };
    let writer = Timeline::new(Ctx::root().unwrap(), key.clone(), vec![], store.clone());
    writer
        .append_batch_registered(
            (1..=50)
                .map(|index| input(&format!("prefix {index}")))
                .collect(),
            |_| Ok(()),
        )
        .unwrap();
    let batch = writer
        .append_batch_registered(vec![input("anchor"), input("after")], |_| Ok(()))
        .unwrap();
    writer
        .save_snapshot(RecoverySnapshot {
            sequence: batch[0].sequence,
            event_id: batch[0].id.clone(),
            interpreter: "test/anchor".into(),
            state: "40".into(),
        })
        .await
        .unwrap();
    let reader = Timeline::restore(Ctx::root().unwrap(), key, vec![], store.clone())
        .await
        .unwrap();
    let context = reader
        .context_with_checkpoint(52, "test/anchor", checkpoint_resume)
        .await
        .unwrap();
    assert!(
        context.checkpoint.is_some(),
        "a mid-batch anchor still selects its checkpoint"
    );
    assert_eq!(context.events.len(), 13);
}

#[tokio::test]
async fn last_event_reads_a_cut_inside_an_atomic_batch() {
    use dim_agent_runtime::{StoredEvent, TimelineStore};
    use dim_agent_types::{AgentEvent, EventKind, InputMessageData};

    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let key = TimelineKey {
        session_id: "session".into(),
        branch_id: "main".into(),
    };
    let event = |id: &str, sequence: u64| AgentEvent {
        id: id.into(),
        sequence,
        create_at: 0,
        tags: None,
        kind: EventKind::InputMessage(InputMessageData::text(id, None)),
    };
    store
        .append(
            key.clone(),
            vec![
                StoredEvent {
                    event: Arc::new(event("a", 1)),
                    batch_end: 2,
                    trace: None,
                },
                StoredEvent {
                    event: Arc::new(event("b", 2)),
                    batch_end: 2,
                    trace: None,
                },
            ],
            Box::new(|_| {}),
        )
        .unwrap();
    let last = store.last_event(&key, 1).await.unwrap().unwrap();
    assert_eq!(last.sequence, 1);
    let last = store.last_event(&key, 2).await.unwrap().unwrap();
    assert_eq!(last.sequence, 2);
}

/// A query the tests narrow.
fn branch_query(
    scope: dim_agent_runtime::TimelineQueryScope,
    position: dim_agent_runtime::TimelineQueryPosition,
) -> dim_agent_runtime::TimelineQuery {
    dim_agent_runtime::TimelineQuery {
        scope,
        position,
        direction: Default::default(),
        kinds: vec![],
        flags: vec![],
        event_id: None,
        predicate: None,
        limit: std::num::NonZeroU64::new(16).unwrap(),
    }
}

fn queried(events: &[Arc<AgentEvent>]) -> Vec<String> {
    events.iter().map(|event| event.id.clone()).collect()
}

fn branch_create(branch: &str, at: u64) -> EventDraft {
    EventKind::BranchCreate(BranchCreateData {
        at: BranchCreateAt::Sequence { sequence: at },
        source: BranchSource::Host {
            operation_id: "test".into(),
        },
        branch: branch.into(),
        prompt: "continue".into(),
    })
    .into()
}

/// `main` with `main_head` facts whose last is the `child` branch create, and
/// `child` cut at that head with `child_facts` facts.
async fn chain_with_a_tall_ancestor(main_head: u64, child_facts: u64) -> (Timeline, Timeline) {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let main = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    for sequence in 1..=main_head {
        if sequence == main_head {
            main.append(branch_create("child", sequence)).unwrap();
        } else {
            let id = format!("main-{sequence}");
            main.append(input(&id).with_id(id)).unwrap();
        }
    }
    let child = Timeline::new(
        ctx,
        TimelineKey {
            session_id: "session".into(),
            branch_id: "child".into(),
        },
        vec![("main".into(), main_head)],
        store,
    );
    for sequence in 1..=child_facts {
        let id = format!("child-{sequence}");
        child.append(input(&id).with_id(id)).unwrap();
    }
    (main, child)
}

#[tokio::test]
async fn a_query_cursor_ends_a_batch_and_its_next_continues_after_it() {
    use dim_agent_runtime::{
        TimelineCursor, TimelineQueryPosition::Cursor, TimelineQueryPosition::Range,
        TimelineQueryScope::Branch,
    };

    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let key = TimelineKey {
        session_id: "session".into(),
        branch_id: "main".into(),
    };
    let timeline = Timeline::new(Ctx::root().unwrap(), key, vec![], store);
    timeline.append(input("one").with_id("main-1")).unwrap();
    timeline.append(input("two").with_id("main-2")).unwrap();
    timeline
        .append_batch_registered(
            vec![
                input("a").with_id("main-3"),
                input("b").with_id("main-4"),
                input("c").with_id("main-5"),
            ],
            |_| Ok(()),
        )
        .unwrap();
    timeline.append(input("six").with_id("main-6")).unwrap();
    timeline.append(input("seven").with_id("main-7")).unwrap();

    let first = timeline
        .query(dim_agent_runtime::TimelineQuery {
            limit: std::num::NonZeroU64::new(1).unwrap(),
            ..branch_query(
                Branch,
                Range {
                    start: 1,
                    end: Some(7),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(queried(&first.events), ["main-1"]);
    assert_eq!(
        first.next,
        Some(TimelineCursor {
            branch_id: "main".into(),
            sequence: 1
        })
    );

    let second = timeline
        .query(dim_agent_runtime::TimelineQuery {
            position: Cursor {
                cursor: first.next.clone().unwrap(),
                through: Some(7),
            },
            limit: std::num::NonZeroU64::new(2).unwrap(),
            ..branch_query(
                Branch,
                Range {
                    start: 1,
                    end: Some(7),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(
        queried(&second.events),
        ["main-2", "main-3", "main-4", "main-5"],
        "the limit is a target: the reply ends at the batch boundary"
    );
    assert_eq!(
        second.next,
        Some(TimelineCursor {
            branch_id: "main".into(),
            sequence: 5
        })
    );

    let third = timeline
        .query(dim_agent_runtime::TimelineQuery {
            position: Cursor {
                cursor: second.next.clone().unwrap(),
                through: Some(7),
            },
            ..branch_query(
                Branch,
                Range {
                    start: 1,
                    end: Some(7),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(queried(&third.events), ["main-6", "main-7"]);
    assert_eq!(third.next, None);
}

#[tokio::test]
async fn a_chain_query_orders_by_chain_position_then_sequence() {
    use dim_agent_runtime::{
        TimelineCursor, TimelineQueryPosition::Cursor, TimelineQueryPosition::Range,
        TimelineQueryScope::Chain,
    };

    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let main = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    main.append(input("root one").with_id("main-1")).unwrap();
    main.append(input("root two").with_id("main-2")).unwrap();
    main.append(branch_create("child", 2)).unwrap();
    let child = Timeline::new(
        ctx,
        TimelineKey {
            session_id: "session".into(),
            branch_id: "child".into(),
        },
        vec![("main".into(), 2)],
        store,
    );
    child.append(input("child one").with_id("child-1")).unwrap();
    child.append(input("child two").with_id("child-2")).unwrap();

    let whole = child
        .query(branch_query(
            Chain,
            Range {
                start: 1,
                end: Some(2),
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        queried(&whole.events),
        ["main-1", "main-2", "child-1", "child-2"]
    );
    assert_eq!(whole.next, None);

    let first = child
        .query(dim_agent_runtime::TimelineQuery {
            limit: std::num::NonZeroU64::new(3).unwrap(),
            ..branch_query(
                Chain,
                Range {
                    start: 1,
                    end: Some(2),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(
        queried(&first.events),
        ["main-1", "main-2", "child-1"],
        "the ancestor's events come first"
    );
    assert_eq!(
        first.next,
        Some(TimelineCursor {
            branch_id: "child".into(),
            sequence: 1
        })
    );

    let rest = child
        .query(dim_agent_runtime::TimelineQuery {
            position: Cursor {
                cursor: first.next.unwrap(),
                through: Some(2),
            },
            ..branch_query(
                Chain,
                Range {
                    start: 1,
                    end: Some(2),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(queried(&rest.events), ["child-2"]);
    assert_eq!(rest.next, None);
}

#[tokio::test]
async fn a_query_selects_kinds_and_the_flags_it_names() {
    use dim_agent_runtime::TimelineQueryPosition::Range;
    use dim_agent_runtime::TimelineQueryScope::Branch;

    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let timeline = Timeline::new(
        Ctx::root().unwrap(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store,
    );
    timeline.append(input("message").with_id("input")).unwrap();
    timeline
        .append(
            EventDraft::new(EventKind::Text(DeltaData {
                turn_id: "turn".into(),
                request_id: "request".into(),
                delta: "text".into(),
            }))
            .with_id("text"),
        )
        .unwrap();
    timeline
        .append(
            EventDraft::new(EventKind::StopInterrupt(StopInterruptData {
                reason: "stop".into(),
                scope: InterruptScope::Turn,
            }))
            .with_id("stop"),
        )
        .unwrap();

    let mut query = branch_query(
        Branch,
        Range {
            start: 1,
            end: Some(3),
        },
    );
    query.kinds = vec!["input/message".into()];
    let by_kind = timeline.query(query).await.unwrap();
    assert_eq!(queried(&by_kind.events), ["input"]);

    let render = timeline
        .query(dim_agent_runtime::TimelineQuery {
            flags: vec![dim_agent_runtime::TimelineQueryFlag::Render],
            ..branch_query(
                Branch,
                Range {
                    start: 1,
                    end: Some(3),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(queried(&render.events), ["text"]);

    let state = timeline
        .query(dim_agent_runtime::TimelineQuery {
            flags: vec![dim_agent_runtime::TimelineQueryFlag::State],
            ..branch_query(
                Branch,
                Range {
                    start: 1,
                    end: Some(3),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(queried(&state.events), ["input", "text"]);

    let union = timeline
        .query(dim_agent_runtime::TimelineQuery {
            flags: vec![
                dim_agent_runtime::TimelineQueryFlag::Render,
                dim_agent_runtime::TimelineQueryFlag::State,
            ],
            ..branch_query(
                Branch,
                Range {
                    start: 1,
                    end: Some(3),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(
        queried(&union.events),
        ["input", "text"],
        "naming both flags selects the union: a row carrying one matches"
    );

    let any = timeline
        .query(dim_agent_runtime::TimelineQuery {
            ..branch_query(
                Branch,
                Range {
                    start: 1,
                    end: Some(3),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(queried(&any.events), ["input", "text", "stop"]);

    let unknown = timeline
        .query(dim_agent_runtime::TimelineQuery {
            kinds: vec!["test/absent".into()],
            ..branch_query(
                Branch,
                Range {
                    start: 1,
                    end: Some(3),
                },
            )
        })
        .await
        .unwrap();
    assert!(unknown.events.is_empty(), "an unknown kind selects no rows");
}

#[tokio::test]
async fn a_query_predicate_matches_one_dotted_path_and_an_unknown_path_matches_nothing() {
    use dim_agent_runtime::{
        TimelineQueryPosition::Range, TimelineQueryPredicate, TimelineQueryScope::Branch,
    };

    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let timeline = Timeline::new(
        Ctx::root().unwrap(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store,
    );
    let marked = |id: &str, tool: &str| {
        EventDraft::new(EventKind::Extension(
            ExtensionEvent::new(
                "test",
                "marked",
                serde_json::json!({"tool": tool, "nested": {"level": 3}}),
            )
            .unwrap(),
        ))
        .with_id(id)
    };
    timeline.append(marked("search", "search")).unwrap();
    timeline.append(marked("read", "read")).unwrap();

    let matching = |path: &str, value: serde_json::Value| dim_agent_runtime::TimelineQuery {
        predicate: Some(TimelineQueryPredicate {
            path: path.into(),
            value,
        }),
        ..branch_query(
            Branch,
            Range {
                start: 1,
                end: Some(2),
            },
        )
    };
    let reply = timeline
        .query(matching("tool", serde_json::json!("search")))
        .await
        .unwrap();
    assert_eq!(queried(&reply.events), ["search"]);
    let reply = timeline
        .query(matching("nested.level", serde_json::json!(3)))
        .await
        .unwrap();
    assert_eq!(queried(&reply.events), ["search", "read"]);
    let reply = timeline
        .query(matching("nested.absent", serde_json::json!(3)))
        .await
        .unwrap();
    assert!(reply.events.is_empty(), "an unknown path selects no rows");
    let reply = timeline
        .query(matching("tool", serde_json::json!("absent")))
        .await
        .unwrap();
    assert!(reply.events.is_empty());
}

#[tokio::test]
async fn a_query_rejects_a_future_position_a_cursor_inside_a_batch_and_a_cursor_outside_its_scope()
{
    use dim_agent_runtime::{
        TimelineCursor, TimelineQueryPosition::Cursor, TimelineQueryPosition::Range,
        TimelineQueryScope::Branch,
    };

    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let main = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    main.append(input("one")).unwrap();
    main.append(branch_create("child", 1)).unwrap();
    let batch = main
        .append_batch_registered(vec![input("a"), input("b")], |_| Ok(()))
        .unwrap();
    let child = Timeline::new(
        ctx,
        TimelineKey {
            session_id: "session".into(),
            branch_id: "child".into(),
        },
        vec![("main".into(), 1)],
        store,
    );

    assert!(
        matches!(
            main.query(branch_query(
                Branch,
                Range {
                    start: 1,
                    end: Some(5)
                }
            ))
            .await,
            Err(AgentError::InvalidInput(_))
        ),
        "a position past the published head is invalid input"
    );
    assert!(
        matches!(
            main.query(branch_query(
                Branch,
                Cursor {
                    cursor: TimelineCursor {
                        branch_id: "main".into(),
                        sequence: batch[0].sequence,
                    },
                    through: Some(batch[1].sequence),
                }
            ))
            .await,
            Err(AgentError::InvalidInput(_))
        ),
        "a cursor inside an atomic batch is invalid input"
    );
    assert!(
        matches!(
            child
                .query(branch_query(
                    Branch,
                    Cursor {
                        cursor: TimelineCursor {
                            branch_id: "main".into(),
                            sequence: 1,
                        },
                        through: Some(0),
                    }
                ))
                .await,
            Err(AgentError::InvalidInput(_))
        ),
        "a branch-scoped read does not cover its ancestor"
    );
}

#[tokio::test]
async fn a_descending_page_returns_the_newest_facts_in_ascending_order() {
    use dim_agent_runtime::TimelineQueryDirection::Descending;
    use dim_agent_runtime::TimelineQueryPosition::{Cursor, Range};
    use dim_agent_runtime::TimelineQueryScope::Branch;

    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let timeline = Timeline::new(
        Ctx::root().unwrap(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store,
    );
    timeline.append(input("one").with_id("main-1")).unwrap();
    timeline.append(input("two").with_id("main-2")).unwrap();
    timeline
        .append_batch_registered(
            vec![
                input("a").with_id("main-3"),
                input("b").with_id("main-4"),
                input("c").with_id("main-5"),
            ],
            |_| Ok(()),
        )
        .unwrap();
    timeline.append(input("six").with_id("main-6")).unwrap();

    let first = timeline
        .query(dim_agent_runtime::TimelineQuery {
            direction: Descending,
            limit: std::num::NonZeroU64::new(2).unwrap(),
            ..branch_query(
                Branch,
                Range {
                    start: 1,
                    end: Some(6),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(
        queried(&first.events),
        ["main-3", "main-4", "main-5", "main-6"],
        "the newest page is ordered ascending and ends at a batch start"
    );
    assert_eq!(
        first.next,
        Some(dim_agent_runtime::TimelineCursor {
            branch_id: "main".into(),
            sequence: 3
        }),
        "the cursor continues strictly below the oldest returned event"
    );

    let second = timeline
        .query(dim_agent_runtime::TimelineQuery {
            direction: Descending,
            position: Cursor {
                cursor: first.next.unwrap(),
                through: Some(1),
            },
            ..branch_query(
                Branch,
                Range {
                    start: 1,
                    end: Some(6),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(queried(&second.events), ["main-1", "main-2"]);
    assert_eq!(second.next, None);
}

#[tokio::test]
async fn a_range_ending_inside_a_batch_is_rejected_in_both_directions() {
    use dim_agent_runtime::TimelineQueryDirection::Descending;
    use dim_agent_runtime::TimelineQueryPosition::Range;
    use dim_agent_runtime::TimelineQueryScope::Branch;

    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let timeline = Timeline::new(
        Ctx::root().unwrap(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store,
    );
    timeline
        .append_batch_registered(vec![input("a"), input("b"), input("c")], |_| Ok(()))
        .unwrap();
    for direction in [
        dim_agent_runtime::TimelineQueryDirection::Ascending,
        Descending,
    ] {
        assert!(
            matches!(
                timeline
                    .query(dim_agent_runtime::TimelineQuery {
                        direction,
                        ..branch_query(
                            Branch,
                            Range {
                                start: 1,
                                end: Some(2)
                            }
                        )
                    })
                    .await,
                Err(AgentError::InvalidInput(_))
            ),
            "{direction:?}: a page must not cross an atomic batch"
        );
    }
}

#[tokio::test]
async fn a_query_event_id_selects_one_fact_and_reports_its_branch() {
    use dim_agent_runtime::TimelineQueryPosition::Range;
    use dim_agent_runtime::TimelineQueryScope::Chain;

    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let main = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    main.append(input("root one").with_id("main-1")).unwrap();
    main.append(input("root two").with_id("main-2")).unwrap();
    main.append(branch_create("child", 2)).unwrap();
    let child = Timeline::new(
        ctx,
        TimelineKey {
            session_id: "session".into(),
            branch_id: "child".into(),
        },
        vec![("main".into(), 2)],
        store,
    );
    child.append(input("child one").with_id("child-1")).unwrap();

    let ancestor = child
        .query(dim_agent_runtime::TimelineQuery {
            event_id: Some("main-2".into()),
            ..branch_query(
                Chain,
                Range {
                    start: 1,
                    end: None,
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(queried(&ancestor.events), ["main-2"]);
    assert_eq!(
        ancestor.positions,
        vec![TimelinePosition {
            branch_id: "main".into(),
            sequence: 2
        }],
        "a by-id lookup reports the branch the event lives on"
    );

    let local = child
        .query(dim_agent_runtime::TimelineQuery {
            event_id: Some("child-1".into()),
            ..branch_query(
                Chain,
                Range {
                    start: 1,
                    end: None,
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(queried(&local.events), ["child-1"]);
    assert_eq!(local.positions[0].branch_id, "child");

    let absent = child
        .query(dim_agent_runtime::TimelineQuery {
            event_id: Some("absent".into()),
            ..branch_query(
                Chain,
                Range {
                    start: 1,
                    end: None,
                },
            )
        })
        .await
        .unwrap();
    assert!(
        absent.events.is_empty(),
        "an id outside the scope selects nothing"
    );
    assert!(absent.positions.is_empty());
    assert_eq!(absent.next, None);
}

#[tokio::test]
async fn a_chain_range_applies_to_each_branch_and_defaults_to_the_published_head() {
    use dim_agent_runtime::TimelineQueryPosition::Range;
    use dim_agent_runtime::TimelineQueryScope::Chain;

    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("session", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let main = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "session".into(),
            branch_id: "main".into(),
        },
        vec![],
        store.clone(),
    );
    for id in ["main-1", "main-2", "main-3", "main-4"] {
        main.append(input(id).with_id(id)).unwrap();
    }
    main.append(branch_create("child", 2)).unwrap();
    let child = Timeline::new(
        ctx,
        TimelineKey {
            session_id: "session".into(),
            branch_id: "child".into(),
        },
        vec![("main".into(), 2)],
        store,
    );
    child.append(input("child-1").with_id("child-1")).unwrap();
    child.append(input("child-2").with_id("child-2")).unwrap();

    let defaulted = child
        .query(branch_query(
            Chain,
            Range {
                start: 1,
                end: None,
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        queried(&defaulted.events),
        ["main-1", "main-2", "child-1", "child-2"],
        "the range's end defaults to the published head, and the ancestor reads to its cut"
    );
    assert_eq!(
        defaulted
            .positions
            .iter()
            .map(|position| (position.branch_id.as_str(), position.sequence))
            .collect::<Vec<_>>(),
        [("main", 1), ("main", 2), ("child", 1), ("child", 2)],
        "a chain read reports the ancestor's branch for an ancestor's fact"
    );

    let clipped = child
        .query(branch_query(
            Chain,
            Range {
                start: 1,
                end: Some(1),
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        queried(&clipped.events),
        ["main-1", "child-1"],
        "each branch is read to the lower of the range's end and its cut"
    );

    let from_second = child
        .query(branch_query(
            Chain,
            Range {
                start: 2,
                end: None,
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        queried(&from_second.events),
        ["main-2", "child-2"],
        "the range's start applies to each branch's own sequence space"
    );
}

#[tokio::test]
async fn a_paged_chain_read_returns_every_ancestor_fact_once() {
    use dim_agent_runtime::TimelineQueryPosition::{Cursor, Range};
    use dim_agent_runtime::TimelineQueryScope::Chain;

    let (_, child) = chain_with_a_tall_ancestor(5, 3).await;
    let whole = child
        .query(branch_query(
            Chain,
            Range {
                start: 1,
                end: None,
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        whole.positions.len(),
        8,
        "five ancestor facts and three local ones"
    );

    let mut position = Range {
        start: 1,
        end: None,
    };
    let mut paged: Vec<TimelinePosition> = Vec::new();
    for page in 0..8 {
        let reply = child
            .query(dim_agent_runtime::TimelineQuery {
                position,
                limit: std::num::NonZeroU64::new(2).unwrap(),
                ..branch_query(
                    Chain,
                    Range {
                        start: 1,
                        end: None,
                    },
                )
            })
            .await
            .unwrap();
        paged.extend(reply.positions.iter().cloned());
        match reply.next {
            Some(next) => {
                position = Cursor {
                    cursor: next,
                    through: None,
                }
            }
            None => {
                assert_eq!(page, 3, "four pages cover the whole chain");
                break;
            }
        }
    }
    assert_eq!(
        paged, whole.positions,
        "the pages compose to the single-page read, every ancestor fact exactly once"
    );
}

#[tokio::test]
async fn a_cursor_on_an_ancestor_is_checked_against_that_ancestor() {
    use dim_agent_runtime::TimelineQueryPosition::{Cursor, Range};
    use dim_agent_runtime::TimelineQueryScope::Chain;

    let (_, child) = chain_with_a_tall_ancestor(5, 2).await;
    let first = child
        .query(dim_agent_runtime::TimelineQuery {
            limit: std::num::NonZeroU64::new(3).unwrap(),
            ..branch_query(
                Chain,
                Range {
                    start: 1,
                    end: None,
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(
        first
            .positions
            .iter()
            .map(|position| (position.branch_id.as_str(), position.sequence))
            .collect::<Vec<_>>(),
        [("main", 1), ("main", 2), ("main", 3)]
    );
    let cursor = first.next.unwrap();

    let rest = child
        .query(dim_agent_runtime::TimelineQuery {
            position: Cursor {
                cursor,
                through: Some(7),
            },
            ..branch_query(
                Chain,
                Range {
                    start: 1,
                    end: None,
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(
        rest.positions
            .iter()
            .map(|position| (position.branch_id.as_str(), position.sequence))
            .collect::<Vec<_>>(),
        [("main", 4), ("main", 5), ("child", 1), ("child", 2)],
        "a through above the ancestor's head is clamped to its cut, not rejected"
    );
    assert_eq!(rest.next, None);

    assert!(
        matches!(
            child
                .query(dim_agent_runtime::TimelineQuery {
                    position: Cursor {
                        cursor: dim_agent_runtime::TimelineCursor {
                            branch_id: "main".into(),
                            sequence: 6,
                        },
                        through: None,
                    },
                    ..branch_query(
                        Chain,
                        Range {
                            start: 1,
                            end: None
                        }
                    )
                })
                .await,
            Err(AgentError::InvalidInput(_))
        ),
        "a cursor past the ancestor's cut is invalid input"
    );
}

#[tokio::test]
async fn the_next_cursor_names_the_effective_end_of_the_segment() {
    use dim_agent_runtime::TimelineQueryPosition::{Cursor, Range};
    use dim_agent_runtime::TimelineQueryScope::Chain;

    let (_, child) = chain_with_a_tall_ancestor(5, 2).await;
    let first = child
        .query(dim_agent_runtime::TimelineQuery {
            limit: std::num::NonZeroU64::new(2).unwrap(),
            ..branch_query(
                Chain,
                Range {
                    start: 1,
                    end: Some(2),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(queried(&first.events), ["main-1", "main-2"]);
    let cursor = first.next.unwrap();
    assert_eq!(
        (cursor.branch_id.as_str(), cursor.sequence),
        ("main", 2),
        "the cursor names the segment's range end, not its cut"
    );

    let rest = child
        .query(dim_agent_runtime::TimelineQuery {
            position: Cursor {
                cursor,
                through: Some(2),
            },
            ..branch_query(
                Chain,
                Range {
                    start: 1,
                    end: Some(2),
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(queried(&rest.events), ["child-1", "child-2"]);
    assert_eq!(rest.next, None);
}

#[tokio::test]
async fn a_range_start_above_the_published_head_is_invalid_input() {
    use dim_agent_runtime::TimelineQueryPosition::Range;
    use dim_agent_runtime::TimelineQueryScope::{Branch, Chain};

    let (_, child) = chain_with_a_tall_ancestor(5, 2).await;
    for scope in [Branch, Chain] {
        assert!(
            matches!(
                child
                    .query(branch_query(
                        scope,
                        Range {
                            start: 3,
                            end: None
                        }
                    ))
                    .await,
                Err(AgentError::InvalidInput(_))
            ),
            "{scope:?}: a start past the queried branch's head is invalid input"
        );
    }
}

#[test]
fn the_query_crosses_languages_with_its_serde_shape() {
    use dim_agent_runtime::{
        TimelineCursor, TimelineQueryPosition, TimelineQueryPredicate, TimelineQueryScope,
    };

    let query = dim_agent_runtime::TimelineQuery {
        scope: TimelineQueryScope::Chain,
        position: TimelineQueryPosition::Cursor {
            cursor: TimelineCursor {
                branch_id: "main".into(),
                sequence: 4,
            },
            through: Some(9),
        },
        direction: dim_agent_runtime::TimelineQueryDirection::Descending,
        kinds: vec!["input/message".into()],
        flags: vec![
            dim_agent_runtime::TimelineQueryFlag::Render,
            dim_agent_runtime::TimelineQueryFlag::State,
        ],
        event_id: Some("event-1".into()),
        predicate: Some(TimelineQueryPredicate {
            path: "nested.level".into(),
            value: serde_json::json!(3),
        }),
        limit: std::num::NonZeroU64::new(4).unwrap(),
    };
    let wire = serde_json::to_value(&query).unwrap();
    assert_eq!(wire["scope"], "chain");
    assert_eq!(wire["direction"], "descending");
    assert_eq!(wire["position"]["kind"], "cursor");
    assert_eq!(wire["position"]["cursor"]["sequence"], 4);
    assert_eq!(wire["flags"], serde_json::json!(["render", "state"]));
    assert_eq!(wire["event_id"], "event-1");
    assert_eq!(wire["limit"], 4);
    assert_eq!(wire["predicate"]["path"], "nested.level");
    assert_eq!(
        serde_json::from_value::<dim_agent_runtime::TimelineQuery>(wire).unwrap(),
        query
    );
    let defaulted: dim_agent_runtime::TimelineQuery = serde_json::from_value(serde_json::json!({
        "scope": "branch",
        "position": {"kind": "range", "start": 1},
        "limit": 2
    }))
    .unwrap();
    assert_eq!(
        defaulted.direction,
        dim_agent_runtime::TimelineQueryDirection::Ascending,
        "ascending is the default"
    );
    assert_eq!(
        defaulted.position,
        TimelineQueryPosition::Range {
            start: 1,
            end: None
        }
    );
    assert_eq!(defaulted.kinds, Vec::<String>::new());
    assert_eq!(defaulted.flags, Vec::new());
    assert_eq!(defaulted.event_id, None);
    assert!(
        serde_json::from_value::<dim_agent_runtime::TimelineQuery>(serde_json::json!({
            "scope": "branch",
            "position": {"kind": "range", "start": 1, "end": 2},
            "limit": 0
        }))
        .is_err(),
        "a zero limit is not a query"
    );
}
