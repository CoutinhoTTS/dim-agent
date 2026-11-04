use dim_agent_runtime::testing::timeline_events;
use dim_agent_runtime::{
    AgentError, MemoryTimelineStore, SessionConfig, Timeline, TimelineKey, TimelineQueryScope,
    TimelineStore,
};
use dim_agent_types::*;
use rutis::Ctx;
use rutis::host::CtxHost;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn input(text: &str) -> EventDraft {
    EventKind::InputMessage(InputMessageData::text(text, None)).into()
}

fn text(event: &dim_agent_types::AgentEvent) -> &str {
    let EventKind::InputMessage(data) = &event.kind else {
        panic!("expected input/message event");
    };
    let ContentBlock::Text { text } = &data.user.as_ref().expect("text input").content[0] else {
        panic!("expected text input");
    };
    text
}

#[tokio::test]
async fn batch_is_registered_and_published_as_one_snapshot_unit() {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("test", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let key = TimelineKey {
        session_id: "test".into(),
        branch_id: "main".into(),
    };
    let timeline = Timeline::new(ctx.clone(), key.clone(), vec![], store.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let _before = timeline
        .before(&ctx, "number", &["input/message"], move |mut draft| {
            let call = observed.fetch_add(1, Ordering::Relaxed) + 1;
            if let EventKind::InputMessage(data) = &mut draft.kind {
                let ContentBlock::Text { text } =
                    &mut data.user.as_mut().expect("text input").content[0]
                else {
                    panic!("expected text input");
                };
                *text = format!("{text}:{call}");
            }
            Ok(draft)
        })
        .unwrap();

    let error = timeline
        .append_batch_registered(vec![input("first"), input("second")], |events| {
            assert_eq!(events.len(), 2);
            assert_eq!(text(&events[0]), "first:1");
            assert_eq!(text(&events[1]), "second:2");
            Err(AgentError::InvalidInput("producer rejected".into()))
        })
        .unwrap_err();
    assert!(matches!(error, AgentError::InvalidInput(_)));
    assert_eq!(timeline.sequence(), 0);

    let events = timeline
        .append_batch_registered(vec![input("first"), input("second")], |_| Ok(()))
        .unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(text(&events[0]), "first:3");
    assert_eq!(text(&events[1]), "second:4");
    assert_eq!(store.trace(&key, 1).await.unwrap().unwrap().steps.len(), 1);
    assert_eq!(store.trace(&key, 2).await.unwrap().unwrap().steps.len(), 1);
    assert!(timeline.batch_end(1).await.is_err());
    assert_eq!(
        timeline_events(&timeline, TimelineQueryScope::Branch, ..=2)
            .await
            .unwrap(),
        events
    );

    assert!(matches!(
        timeline.append_batch_registered(vec![input("uncommitted")], |_| {
            panic!("registration failed")
        }),
        Err(AgentError::Fault { .. })
    ));
    assert_eq!(timeline.sequence(), 2);
    assert_eq!(
        timeline_events(&timeline, TimelineQueryScope::Chain, ..)
            .await
            .unwrap(),
        events
    );
    assert_eq!(timeline.append(input("next")).unwrap().sequence, 3);
}

#[tokio::test]
async fn batch_rejects_empty_without_partial_commit() {
    let store = Arc::new(MemoryTimelineStore::default());
    store
        .create_session("test", &SessionConfig::default())
        .await
        .unwrap();
    let ctx = Ctx::root().unwrap();
    let timeline = Timeline::new(
        ctx.clone(),
        TimelineKey {
            session_id: "test".into(),
            branch_id: "main".into(),
        },
        vec![],
        store,
    );
    assert!(matches!(
        timeline.append_batch_registered(Vec::new(), |_| Ok(())),
        Err(AgentError::InvalidInput(_))
    ));
}
