use dim_agent_capabilities::{defaults, timeline_tree::TreeCommands};
use dim_agent_runtime::{
    AgentBuilder, AgentConfig, AgentError, BranchOptions, Dispatcher, Feature, SessionConfig,
    Timeline, TimelineIdentity, TimelineKey, TimelineStore,
};
use dim_agent_types::*;
use dim_app_server::storage::{SqliteStorage, SqliteTimelineStore};
use rusqlite::Connection;
use rutis::{BoxFuture, CordisError, Ctx, Effect, Keyed, TypedPlugin};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

struct CaptureTimeline {
    timelines: Arc<Mutex<BTreeMap<String, Timeline>>>,
}

impl TypedPlugin for CaptureTimeline {
    type Deps = (Keyed<Timeline>, Keyed<TimelineIdentity>);
    fn name(&self) -> &str {
        "storage-session.capture-timeline"
    }
    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        (Keyed(timeline), Keyed(identity)): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            self.timelines
                .lock()
                .unwrap()
                .insert(identity.branch_id.clone(), timeline.as_ref().clone());
            Ok(Effect::Done)
        })
    }
}

struct GenericCleanupFault {
    tree: Arc<Mutex<Option<Arc<TreeCommands>>>>,
}
impl TypedPlugin for GenericCleanupFault {
    type Deps = (
        Keyed<Timeline>,
        Keyed<TimelineIdentity>,
        Keyed<Dispatcher>,
        Keyed<TreeCommands>,
    );
    fn name(&self) -> &str {
        "storage-session.generic-cleanup"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(timeline), Keyed(identity), Keyed(dispatcher), Keyed(tree)): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            if identity.branch_id != "main" {
                return Ok(Effect::Done);
            }
            *self.tree.lock().unwrap() = Some(tree);
            dispatcher.subscribe(ctx, self.name(), &["input/message"], |event| {
                Box::pin(async move {
                    if matches!(&event.kind, EventKind::InputMessage(data) if matches!(
                        data.user.as_ref().map(|user| user.content.as_slice()),
                        Some([ContentBlock::Text { text }]) if text == "generic cleanup fault"
                    )) {
                        return Err(AgentError::fault(
                            "cleanup fixture",
                            "first generic failure",
                        ));
                    }
                    Ok(())
                })
            })?;
            ctx.effect(|| {
                Effect::Disposer(Box::new(move || {
                    timeline.append(EventDraft {
                        tags: Some(vec!["cleanup".into()]),
                        kind: EventKind::PermissionChanged(PermissionChangedData {
                            policy: PermissionPolicy::default(),
                        }),
                    })?;
                    Ok(())
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_failure_during_generic_cleanup_supersedes_the_session_fault() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/storage-session-flow").join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("dimcode.sqlite");
        let db = SqliteStorage::open(path.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        let timelines = Arc::new(Mutex::new(BTreeMap::new()));
        let captured = timelines.clone();
        let tree = Arc::new(Mutex::new(None));
        let captured_tree = tree.clone();
        let mut builder = AgentBuilder::new(AgentConfig::default());
        defaults::mock_hosts(&mut builder);
        defaults::install(&mut builder);
        builder
            .timeline_store(store.clone())
            .require_feature(Feature::new("storage-session.capture-timeline").branch(
                "storage-session.capture-timeline",
                move |names| names.part(CaptureTimeline { timelines: captured.clone() }),
            ))
            .require_feature(Feature::new("storage-session.generic-cleanup").branch(
                "storage-session.generic-cleanup",
                move |names| names.part(GenericCleanupFault { tree: captured_tree.clone() }),
            ));
        let agent = builder.build();
        let session = agent.create_session(SessionConfig::default()).await.unwrap();
        let idle = session.branch("main".into(), 0, BranchOptions {
            prompt: "idle sibling".into(), source: BranchSource::Host { operation_id: "idle sibling".into() },
        }).await.unwrap();
        session.idle(&idle).await.unwrap();
        let idle_handle = tree.lock().unwrap().as_ref().unwrap().get(&idle).unwrap();
        idle_handle.close().await.unwrap();
        assert!(idle_handle.state().unwrap().closed);
        session.idle("main").await.unwrap();
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TRIGGER fail_cleanup BEFORE INSERT ON timeline_events WHEN NEW.kind = 'permission/changed' BEGIN SELECT RAISE(ABORT, 'cleanup storage failure'); END").unwrap();
        session.submit("generic cleanup fault".into(), InputMode::Queue, "main").await.unwrap();
        assert!(session.closed().await.is_err());
        for branch in ["main", idle.as_str()] {
            let timeline = timelines.lock().unwrap().get(branch).unwrap().clone();
            assert!(matches!(timeline.failure(), Some(AgentError::Storage(message)) if message.contains("cleanup storage failure")));
            assert!(matches!(timeline.append(EventKind::StopInterrupt(StopInterruptData { reason: "filler".into(), scope: InterruptScope::Turn }).into()), Err(AgentError::Storage(message)) if message.contains("cleanup storage failure")));
            assert!(matches!(session.submit("late input".into(), InputMode::Queue, branch).await, Err(AgentError::Storage(message)) if message.contains("cleanup storage failure")));
        }
        assert!(matches!(session.failure(), Some(AgentError::Storage(message)) if message.contains("cleanup storage failure")));
        assert!(!session.local("main", ..).await.unwrap().iter().any(|event| event.tags.as_deref() == Some(&["cleanup".into()])));
        let _ = agent.close().await;
        drop(connection);
        timelines.lock().unwrap().clear();
        store.close().await.unwrap();
        db.close().await.unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }).await.expect("generic cleanup with a later storage fault completed");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_session_failure_rejects_writes_on_an_idle_sibling_timeline() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let path = std::env::temp_dir().join(format!("dimagent-storage-siblings-{}.sqlite", uuid::Uuid::new_v4()));
        let db = SqliteStorage::open(path.clone()).unwrap();
        let store = SqliteTimelineStore::new(db.clone());
        let timelines = Arc::new(Mutex::new(BTreeMap::new()));
        let captured = timelines.clone();
        let mut builder = AgentBuilder::new(AgentConfig::default());
        defaults::mock_hosts(&mut builder);
        defaults::install(&mut builder);
        builder
            .timeline_store(store.clone())
            .require_feature(Feature::new("storage-session.capture-timeline").branch(
                "storage-session.capture-timeline",
                move |names| names.part(CaptureTimeline { timelines: captured.clone() }),
            ));
        let agent = builder.build();
        let session = agent.create_session(SessionConfig::default()).await.unwrap();
        let mut siblings = Vec::new();
        for name in ["failed sibling", "idle sibling"] {
            let branch = session.branch("main".into(), 0, BranchOptions {
                prompt: name.into(),
                source: BranchSource::Host { operation_id: name.into() },
            }).await.unwrap();
            session.idle(&branch).await.unwrap();
            siblings.push(branch);
        }
        let failed = &siblings[0];
        let idle = &siblings[1];
        let idle_through = session.sequence(idle).unwrap();
        let failed_through = session.sequence(failed).unwrap();
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE injected_branch(session_id TEXT, branch_id TEXT); CREATE TRIGGER fail_sibling_output BEFORE INSERT ON timeline_events WHEN NEW.kind = 'step/text' AND NEW.session_id = (SELECT session_id FROM injected_branch) AND NEW.branch_id = (SELECT branch_id FROM injected_branch) BEGIN SELECT RAISE(ABORT, 'injected sibling output failure'); END").unwrap();
        connection.execute("INSERT INTO injected_branch VALUES (?1, ?2)", [session.id(), failed]).unwrap();
        session.submit("fail this output".into(), InputMode::Queue, failed).await.unwrap();
        assert!(session.closed().await.unwrap_err().to_string().contains("injected sibling output failure"));
        for branch in [failed, idle] {
            let timeline = timelines.lock().unwrap().get(branch).unwrap().clone();
            assert!(matches!(timeline.failure(), Some(AgentError::Storage(message)) if message.contains("injected sibling output failure")));
            assert!(matches!(timeline.append(EventKind::InputMessage(InputMessageData::text(
                "late direct write", Some(InputMode::Queue),
            )).into()), Err(AgentError::Storage(message)) if message.contains("injected sibling output failure")));
            assert!(matches!(session.submit("late input".into(), InputMode::Queue, branch).await,
                Err(AgentError::Storage(message)) if message.contains("injected sibling output failure")));
        }
        assert_eq!(session.sequence(idle).unwrap(), idle_through);
        assert!(!session.local(failed, (failed_through + 1)..).await.unwrap().iter().any(|event|
            matches!(event.kind, EventKind::Text(_) | EventKind::ModelOutputEnd(_) | EventKind::TurnEnd(_))));
        assert!(!session.local(idle, (idle_through + 1)..).await.unwrap().iter().any(|event|
            matches!(event.kind, EventKind::ModelOutputEnd(_) | EventKind::TurnEnd(_))));
        let _ = agent.close().await;
        drop(connection);
        drop(session);
        drop(agent);
        drop(builder);
        timelines.lock().unwrap().clear();
        store.close().await.unwrap();
        db.close().await.unwrap();
        std::fs::remove_file(path).unwrap();
    }).await.expect("SQLite sibling failure and cleanup completed");
}

struct RecordCleanup;
impl TypedPlugin for RecordCleanup {
    type Deps = Keyed<Timeline>;
    fn name(&self) -> &str {
        "storage-session.record-cleanup"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        Keyed(timeline): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.effect(|| {
                Effect::Disposer(Box::new(move || {
                    timeline.append(EventDraft {
                        tags: Some(vec!["finalizer".into()]),
                        kind: EventKind::PermissionChanged(PermissionChangedData {
                            policy: PermissionPolicy::default(),
                        }),
                    })?;
                    Ok(())
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}

#[tokio::test]
async fn session_close_and_idle_unload_persist_finalizer_events() {
    tokio::time::timeout(Duration::from_secs(10), async {
        for unloading in [false, true] {
            let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target/storage-session-flow")
                .join(uuid::Uuid::new_v4().to_string());
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("dimcode.sqlite");
            let db = SqliteStorage::open(path.clone()).unwrap();
            let store = SqliteTimelineStore::new(db.clone());
            let mut builder = AgentBuilder::new(AgentConfig::default());
            defaults::mock_hosts(&mut builder);
            defaults::install(&mut builder);
            builder.timeline_store(store.clone()).require_feature(
                Feature::new("storage-session.record-cleanup")
                    .branch("storage-session.record-cleanup", |names| {
                        names.part(RecordCleanup)
                    }),
            );
            let agent = builder.build();
            let session = agent
                .create_session(SessionConfig::default())
                .await
                .unwrap();
            session
                .submit("settle before cleanup".into(), InputMode::Queue, "main")
                .await
                .unwrap();
            session.idle("main").await.unwrap();
            let key = TimelineKey {
                session_id: session.id().into(),
                branch_id: "main".into(),
            };
            if unloading {
                assert!(session.unload_if_idle().await.unwrap());
            } else {
                session.close().await.unwrap();
                session.close().await.unwrap();
            }
            // Completion includes SQLite publication, not just append admission.
            let head = store.head(&key).await.unwrap();
            let records = store.read(&key, 1..=head).await.unwrap();
            assert_eq!(
                records
                    .iter()
                    .filter(|r| r.event.tags.as_deref() == Some(&["finalizer".into()]))
                    .count(),
                1
            );
            let terminal = records.last().unwrap().event.clone();
            assert_eq!(terminal.tags, Some(vec!["finalizer".into()]));
            agent.close().await.unwrap();
            store.close().await.unwrap();
            db.close().await.unwrap();
            drop(session);
            drop(agent);
            drop(builder);
            drop(store);
            drop(db);
            let reopened = SqliteStorage::open(path).unwrap();
            let store = SqliteTimelineStore::new(reopened.clone());
            assert_eq!(store.head(&key).await.unwrap(), head);
            assert_eq!(
                store.read(&key, head..=head).await.unwrap()[0].event,
                terminal
            );
            store.close().await.unwrap();
            reopened.close().await.unwrap();
            std::fs::remove_dir_all(directory).unwrap();
        }
    })
    .await
    .expect("session cleanup and durable publication completed");
}
