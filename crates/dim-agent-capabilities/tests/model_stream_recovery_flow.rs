use dim_agent_capabilities::{
    adapters::{DefaultPrepareModelPlugin, MockTools},
    defaults,
    host::*,
};
use dim_agent_runtime::testing::MinimalHost;
use dim_agent_runtime::testing::chain_events;
use dim_agent_runtime::*;
use dim_agent_types::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

/// The interval `output.rs` shows thinking progress at.
const THINKING_SHOW_INTERVAL: Duration = Duration::from_millis(250);

fn builder(model: Arc<dyn ModelDriver>, tools: Arc<dyn ToolDriver>) -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder
        .process_value(Arc::new(ModelHost(model)))
        .session_value(Arc::new(ToolHost(tools)))
        .session_value(Arc::new(
            dim_agent_capabilities::history::HistoryRenderer::default(),
        ))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(None)))
        .session_value(Arc::new(CompactionStrategyHost(None)))
        .require_feature(
            Feature::new("PrepareModelHost").session("PrepareModelHost", |names| {
                Ok(Box::new(DefaultPrepareModelPlugin::new(names)?))
            }),
        );
    defaults::install(&mut builder);
    builder
}

/// Hands each attempt the next scripted stream; a request that runs out of
/// scripted streams fails the test.
struct ScriptedModel {
    calls: AtomicUsize,
    streams: Mutex<Vec<mpsc::UnboundedReceiver<std::result::Result<ModelChunk, HostError>>>>,
    second_attempt: Notify,
    release: Notify,
}
impl ScriptedModel {
    fn new(
        streams: Vec<mpsc::UnboundedReceiver<std::result::Result<ModelChunk, HostError>>>,
    ) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            streams: Mutex::new(streams),
            second_attempt: Notify::new(),
            release: Notify::new(),
        }
    }
}
impl ModelDriver for ScriptedModel {
    fn invoke(
        &self,
        _: Arc<ModelRequest>,
        _: ModelConnection,
        _: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let receiver = {
                let mut streams = self.streams.lock().unwrap();
                assert!(
                    !streams.is_empty(),
                    "attempt {} has no scripted stream",
                    call + 1
                );
                streams.remove(0)
            };
            if call == 1 {
                self.second_attempt.notify_one();
                self.release.notified().await;
            }
            let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
                receiver.recv().await.map(|chunk| (chunk, receiver))
            });
            Ok(Box::pin(stream) as ModelStream)
        })
    }
}

fn malformed_cut() -> HostError {
    HostError::Failed {
        message: "the stream ended without a finish reason".into(),
        budget: None,
        kind: HostErrorKind::Transport,
        model: Some(Box::new(ModelFailureFacts {
            kind: ModelFailureKind::MalformedResponse,
            reason: Some("the stream ended without a finish reason".into()),
            status: None,
            provider_code: None,
            request_id: None,
            cause: Vec::new(),
            retry_after: None,
            attempts: 1,
            waited: Duration::ZERO,
            retry_after_stop: None,
            retry_forbidden: false,
        })),
        presentation: None,
    }
}

fn usage(total: u8) -> ModelChunk {
    ModelChunk::Usage(
        serde_json::from_value(serde_json::json!({
            "promptTokens": 7,
            "completionTokens": total,
            "totalTokens": 7 + total,
        }))
        .unwrap(),
    )
}

fn send(
    chunks: &mpsc::UnboundedSender<std::result::Result<ModelChunk, HostError>>,
    list: Vec<std::result::Result<ModelChunk, HostError>>,
) {
    for chunk in list {
        chunks.send(chunk).unwrap();
    }
}

/// Waits until the branch published `count` usage facts, so everything the
/// stream sent before the last one was consumed without moving the clock.
async fn wait_for_usage(session: &Session, count: usize) {
    let mut dispatch = session.dispatch_watch("main").unwrap();
    loop {
        let events = chain_events(session, "main", ..).await.unwrap();
        if events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::ModelUsage(_)))
            .count()
            >= count
        {
            return;
        }
        dispatch.changed().await.unwrap();
    }
}

/// Waits until the branch ends its turn; its outcome tells a missing retry
/// apart from a retry still starting.
async fn turn_end(session: &Session) -> String {
    let mut dispatch = session.dispatch_watch("main").unwrap();
    loop {
        let events = chain_events(session, "main", ..).await.unwrap();
        if let Some(outcome) = events.iter().find_map(|event| match &event.kind {
            EventKind::TurnEnd(data) => Some(format!("{:?}", data.outcome)),
            _ => None,
        }) {
            return outcome;
        }
        dispatch.changed().await.unwrap();
    }
}

/// Takes every progress snapshot the cell published since the last call.
fn snapshots(progress: &mut tokio::sync::watch::Receiver<ModelProgress>) -> Vec<ModelProgress> {
    let mut taken = Vec::new();
    while progress.has_changed().unwrap_or(false) {
        taken.push(progress.borrow_and_update().clone());
    }
    taken
}

/// Adds the snapshots a subscriber received since the last call, and their
/// serialized size, to `updates` and `bytes`.
fn drain(
    progress: &mut tokio::sync::watch::Receiver<ModelProgress>,
    updates: &mut usize,
    bytes: &mut usize,
) {
    for snapshot in snapshots(progress) {
        *updates += 1;
        *bytes += serde_json::to_vec(&snapshot).unwrap().len();
    }
}

fn thinking_events(events: &[Arc<AgentEvent>]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Thinking(data) => Some(data.delta.clone()),
            _ => None,
        })
        .collect()
}

fn text_events(events: &[Arc<AgentEvent>]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Text(data) => Some(data.delta.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_stream_cut_before_any_committed_event_retries_in_place() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (cut, cut_stream) = mpsc::unbounded_channel();
        let (retry, retry_stream) = mpsc::unbounded_channel();
        let model = Arc::new(ScriptedModel::new(vec![cut_stream, retry_stream]));
        let agent = MinimalHost::start(&builder(model.clone(), Arc::new(MockTools)))
            .await
            .unwrap();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let mut progress = session.on::<ModelProgress>("main").unwrap().unwrap();
        send(
            &cut,
            vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "cut".into(),
                }),
                Ok(ModelChunk::Thinking("partial ".into())),
                Ok(ModelChunk::Thinking("thought".into())),
            ],
        );
        progress
            .wait_for(|progress| progress.thinking == "partial thought")
            .await
            .unwrap();
        assert!(
            thinking_events(&chain_events(&session, "main", ..).await.unwrap()).is_empty(),
            "thinking in progress is not a committed fact"
        );
        send(&cut, vec![Err(malformed_cut())]);
        drop(cut);

        let ended = tokio::select! {
            _ = model.second_attempt.notified() => None,
            outcome = turn_end(&session) => Some(outcome),
        };
        assert!(
            ended.is_none(),
            "a cut before any committed output retries in place; the turn ended with {ended:?}"
        );
        let cleared = session.get::<ModelProgress>("main").unwrap().unwrap();
        assert!(
            cleared.thinking.is_empty() && cleared.text.is_empty(),
            "the failed attempt's progress is gone before the retry: {cleared:?}"
        );
        assert!(cleared.turn_id.is_some() && cleared.request_id.is_some());

        send(
            &retry,
            vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "retry".into(),
                }),
                Ok(ModelChunk::Thinking("retry thought".into())),
                Ok(ModelChunk::ThinkingEnd {
                    thinking: "retry thought".into(),
                }),
                Ok(ModelChunk::Text("retry answer".into())),
                Ok(ModelChunk::TextEnd),
                Ok(ModelChunk::MessageStop {
                    message_id: "retry".into(),
                    reason: ModelStopReason::EndTurn,
                }),
            ],
        );
        drop(retry);
        model.release.notify_one();
        session.idle("main").await.unwrap();

        assert_eq!(model.calls.load(Ordering::SeqCst), 2);
        let events = chain_events(&session, "main", ..).await.unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::ModelCall(_)))
                .count(),
            1,
            "the retry belongs to the same request"
        );
        assert_eq!(thinking_events(&events), ["retry thought"]);
        assert_eq!(text_events(&events), ["retry answer"]);
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            EventKind::TurnEnd(TurnEndData {
                outcome: TurnOutcome::Completed { .. },
                ..
            })
        )));
        assert_eq!(
            session.get::<ModelProgress>("main").unwrap().unwrap(),
            ModelProgress::default()
        );
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_stream_cut_after_a_committed_event_ends_the_turn_without_retrying() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (cut, cut_stream) = mpsc::unbounded_channel();
        let model = Arc::new(ScriptedModel::new(vec![cut_stream]));
        let agent = MinimalHost::start(&builder(model.clone(), Arc::new(MockTools)))
            .await
            .unwrap();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let committed = "x".repeat(4096);
        send(
            &cut,
            vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "cut".into(),
                }),
                Ok(ModelChunk::Text(committed.clone())),
                Err(malformed_cut()),
            ],
        );
        drop(cut);
        session.idle("main").await.unwrap();

        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        let events = chain_events(&session, "main", ..).await.unwrap();
        assert_eq!(text_events(&events), [committed]);
        let failure = events
            .iter()
            .find_map(|event| match &event.kind {
                EventKind::ModelOutputEnd(ModelOutputEndData {
                    outcome:
                        ModelOutputOutcome::Failed {
                            failure: Some(failure),
                            ..
                        },
                    ..
                }) => Some(failure.as_ref().clone()),
                _ => None,
            })
            .expect("a failed model output with facts");
        assert_eq!(failure.kind, ModelFailureKind::MalformedResponse);
        assert_eq!(failure.attempts, 1, "committed text stops the retry");
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_usage_fact_before_a_cut_stops_the_retry() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (cut, cut_stream) = mpsc::unbounded_channel();
        let model = Arc::new(ScriptedModel::new(vec![cut_stream]));
        let agent = MinimalHost::start(&builder(model.clone(), Arc::new(MockTools)))
            .await
            .unwrap();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        send(
            &cut,
            vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "usage".into(),
                }),
                Ok(usage(1)),
                Err(malformed_cut()),
            ],
        );
        drop(cut);
        session.idle("main").await.unwrap();

        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
        let events = chain_events(&session, "main", ..).await.unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::ModelUsage(_)))
                .count(),
            1,
            "the failed attempt's usage stays the only one"
        );
        let failure = events
            .iter()
            .find_map(|event| match &event.kind {
                EventKind::ModelOutputEnd(ModelOutputEndData {
                    outcome:
                        ModelOutputOutcome::Failed {
                            failure: Some(failure),
                            ..
                        },
                    ..
                }) => Some(failure.as_ref().clone()),
                _ => None,
            })
            .expect("a failed model output with facts");
        assert_eq!(
            failure.attempts, 1,
            "a committed usage fact stops the retry"
        );
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn thinking_progress_is_shown_once_per_interval() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (chunks, receiver) = mpsc::unbounded_channel();
        let model = Arc::new(ScriptedModel::new(vec![receiver]));
        let agent = MinimalHost::start(&builder(model, Arc::new(MockTools)))
            .await
            .unwrap();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let mut progress = session.on::<ModelProgress>("main").unwrap().unwrap();
        progress
            .wait_for(|progress| progress.request_id.is_some())
            .await
            .unwrap();
        snapshots(&mut progress);
        send(
            &chunks,
            vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "throttle".into(),
                }),
                Ok(ModelChunk::Thinking("a".into())),
                Ok(ModelChunk::Thinking("b".into())),
                Ok(ModelChunk::Thinking("c".into())),
                Ok(usage(1)),
            ],
        );
        wait_for_usage(&session, 1).await;
        let taken = snapshots(&mut progress);
        assert!(
            taken.is_empty(),
            "no snapshot for three deltas inside the interval: {taken:?}"
        );
        let shown = progress
            .wait_for(|progress| progress.thinking == "abc")
            .await
            .unwrap()
            .clone();
        assert_eq!((shown.thinking.as_str(), shown.thinking_offset), ("abc", 0));

        send(
            &chunks,
            vec![
                Ok(ModelChunk::Thinking("d".into())),
                Ok(ModelChunk::Thinking("e".into())),
                Ok(usage(2)),
            ],
        );
        wait_for_usage(&session, 2).await;
        let taken = snapshots(&mut progress);
        assert!(
            taken.is_empty(),
            "no snapshot for the next interval's deltas: {taken:?}"
        );
        let shown = progress
            .wait_for(|progress| progress.thinking == "abcde")
            .await
            .unwrap()
            .clone();
        assert_eq!(shown.thinking.as_str(), "abcde");

        send(
            &chunks,
            vec![
                Ok(ModelChunk::ThinkingEnd {
                    thinking: "abcde".into(),
                }),
                Ok(ModelChunk::Text("answer".into())),
                Ok(ModelChunk::TextEnd),
                Ok(ModelChunk::MessageStop {
                    message_id: "throttle".into(),
                    reason: ModelStopReason::EndTurn,
                }),
            ],
        );
        drop(chunks);
        session.idle("main").await.unwrap();
        assert_eq!(
            session.get::<ModelProgress>("main").unwrap().unwrap(),
            ModelProgress::default()
        );
        let events = chain_events(&session, "main", ..).await.unwrap();
        assert_eq!(
            thinking_events(&events),
            ["abcde"],
            "thinking is committed once, at its block end"
        );
        assert_eq!(text_events(&events), ["answer"]);
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_text_delta_shows_buffered_thinking_at_once() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (chunks, receiver) = mpsc::unbounded_channel();
        let model = Arc::new(ScriptedModel::new(vec![receiver]));
        let agent = MinimalHost::start(&builder(model, Arc::new(MockTools)))
            .await
            .unwrap();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let mut progress = session.on::<ModelProgress>("main").unwrap().unwrap();
        progress
            .wait_for(|progress| progress.request_id.is_some())
            .await
            .unwrap();
        snapshots(&mut progress);
        send(
            &chunks,
            vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "switch".into(),
                }),
                Ok(ModelChunk::Thinking("plan".into())),
                Ok(usage(1)),
            ],
        );
        wait_for_usage(&session, 1).await;
        let taken = snapshots(&mut progress);
        assert!(
            taken.is_empty(),
            "the buffer waits for the interval: {taken:?}"
        );
        send(&chunks, vec![Ok(ModelChunk::Text("answer".into()))]);
        let shown = progress
            .wait_for(|progress| progress.text == "answer")
            .await
            .unwrap()
            .clone();
        assert_eq!(shown.thinking, "plan", "the switch shows the buffer");
        assert_eq!(shown.thinking_offset, 0);
        send(
            &chunks,
            vec![
                Ok(ModelChunk::TextEnd),
                Ok(ModelChunk::MessageStop {
                    message_id: "switch".into(),
                    reason: ModelStopReason::EndTurn,
                }),
            ],
        );
        drop(chunks);
        session.idle("main").await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_tool_start_shows_buffered_thinking_at_once() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (chunks, receiver) = mpsc::unbounded_channel();
        let model = Arc::new(ScriptedModel::new(vec![receiver]));
        let agent = MinimalHost::start(&builder(model, Arc::new(MockTools)))
            .await
            .unwrap();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let mut progress = session.on::<ModelProgress>("main").unwrap().unwrap();
        progress
            .wait_for(|progress| progress.request_id.is_some())
            .await
            .unwrap();
        snapshots(&mut progress);
        send(
            &chunks,
            vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "call".into(),
                }),
                Ok(ModelChunk::Thinking("plan".into())),
                Ok(usage(1)),
            ],
        );
        wait_for_usage(&session, 1).await;
        let taken = snapshots(&mut progress);
        assert!(
            taken.is_empty(),
            "the buffer waits for the interval: {taken:?}"
        );
        send(
            &chunks,
            vec![Ok(ModelChunk::ToolStart {
                id: "call".into(),
                name: "echo".into(),
            })],
        );
        let shown = progress
            .wait_for(|progress| progress.tools.len() == 1)
            .await
            .unwrap()
            .clone();
        assert_eq!(shown.thinking, "plan", "the tool start shows the buffer");
        assert_eq!(shown.thinking_offset, 0);
        drop(chunks);
        session.stop("main", "done".into()).await.unwrap();
        session.idle("main").await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_thinking_block_end_shows_the_buffer_at_once() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (chunks, receiver) = mpsc::unbounded_channel();
        let model = Arc::new(ScriptedModel::new(vec![receiver]));
        let agent = MinimalHost::start(&builder(model, Arc::new(MockTools)))
            .await
            .unwrap();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let mut progress = session.on::<ModelProgress>("main").unwrap().unwrap();
        progress
            .wait_for(|progress| progress.request_id.is_some())
            .await
            .unwrap();
        snapshots(&mut progress);
        send(
            &chunks,
            vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "end".into(),
                }),
                Ok(ModelChunk::Thinking("abc".into())),
                Ok(ModelChunk::ThinkingEnd {
                    thinking: "abc".into(),
                }),
            ],
        );
        let shown = progress
            .wait_for(|progress| progress.thinking_offset + progress.thinking.len() as u64 == 3)
            .await
            .unwrap()
            .clone();
        assert_eq!(
            shown.thinking_offset + shown.thinking.len() as u64,
            3,
            "the block end shows the whole block: {shown:?}"
        );
        tokio::time::advance(THINKING_SHOW_INTERVAL).await;
        tokio::task::yield_now().await;
        let settled = session.get::<ModelProgress>("main").unwrap().unwrap();
        assert_eq!(
            settled.thinking_offset + settled.thinking.len() as u64,
            3,
            "the interval does not show the block's thinking again: {settled:?}"
        );
        send(
            &chunks,
            vec![
                Ok(ModelChunk::Text("answer".into())),
                Ok(ModelChunk::TextEnd),
                Ok(ModelChunk::MessageStop {
                    message_id: "end".into(),
                    reason: ModelStopReason::EndTurn,
                }),
            ],
        );
        drop(chunks);
        session.idle("main").await.unwrap();
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_pending_thinking_buffer_is_not_shown_after_a_retry() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (cut, cut_stream) = mpsc::unbounded_channel();
        let (retry, retry_stream) = mpsc::unbounded_channel();
        let model = Arc::new(ScriptedModel::new(vec![cut_stream, retry_stream]));
        let agent = MinimalHost::start(&builder(model.clone(), Arc::new(MockTools)))
            .await
            .unwrap();
        let session = agent
            .create_session(SessionConfig::default())
            .await
            .unwrap();
        session
            .submit("go".into(), InputMode::Queue, "main")
            .await
            .unwrap();
        let mut progress = session.on::<ModelProgress>("main").unwrap().unwrap();
        progress
            .wait_for(|progress| progress.request_id.is_some())
            .await
            .unwrap();
        snapshots(&mut progress);
        send(
            &cut,
            vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "cut".into(),
                }),
                Ok(ModelChunk::Thinking("discarded".into())),
                Err(malformed_cut()),
            ],
        );
        drop(cut);

        let ended = tokio::select! {
            _ = model.second_attempt.notified() => None,
            outcome = turn_end(&session) => Some(outcome),
        };
        assert!(
            ended.is_none(),
            "a cut before any committed output retries in place; the turn ended with {ended:?}"
        );
        let cleared = session.get::<ModelProgress>("main").unwrap().unwrap();
        assert!(
            cleared.thinking.is_empty(),
            "the failed attempt's buffered thinking is gone before the retry: {cleared:?}"
        );

        model.release.notify_one();
        tokio::time::advance(THINKING_SHOW_INTERVAL).await;
        tokio::task::yield_now().await;
        let retrying = session.get::<ModelProgress>("main").unwrap().unwrap();
        assert!(
            retrying.thinking.is_empty(),
            "the retry does not show the failed attempt's buffer: {retrying:?}"
        );

        send(
            &retry,
            vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "retry".into(),
                }),
                Ok(ModelChunk::Thinking("kept".into())),
                Ok(ModelChunk::ThinkingEnd {
                    thinking: "kept".into(),
                }),
                Ok(ModelChunk::Text("answer".into())),
                Ok(ModelChunk::TextEnd),
                Ok(ModelChunk::MessageStop {
                    message_id: "retry".into(),
                    reason: ModelStopReason::EndTurn,
                }),
            ],
        );
        drop(retry);
        session.idle("main").await.unwrap();
        assert_eq!(model.calls.load(Ordering::SeqCst), 2);
        let events = chain_events(&session, "main", ..).await.unwrap();
        assert_eq!(thinking_events(&events), ["kept"]);
        assert_eq!(text_events(&events), ["answer"]);
        agent.close().await.unwrap();
    })
    .await
    .unwrap();
}

/// The comparison the pull request reports: 2,000 deltas of 8 bytes, 20 ms
/// apart, measure the updates and serialized bytes one subscriber sees.
#[tokio::test(start_paused = true)]
async fn thinking_progress_volume_is_throttled() {
    let (chunks, receiver) = mpsc::unbounded_channel();
    let model = Arc::new(ScriptedModel::new(vec![receiver]));
    let agent = MinimalHost::start(&builder(model.clone(), Arc::new(MockTools)))
        .await
        .unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    session
        .submit("go".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    let mut progress = session.on::<ModelProgress>("main").unwrap().unwrap();
    progress
        .wait_for(|progress| progress.request_id.is_some())
        .await
        .unwrap();
    let (mut updates, mut bytes) = (0, 0);
    drain(&mut progress, &mut updates, &mut bytes);
    send(
        &chunks,
        vec![Ok(ModelChunk::MessageStart {
            message_id: "volume".into(),
        })],
    );
    tokio::task::yield_now().await;
    for _ in 0..2000 {
        send(&chunks, vec![Ok(ModelChunk::Thinking("abcdefgh".into()))]);
        tokio::time::sleep(Duration::from_millis(20)).await;
        drain(&mut progress, &mut updates, &mut bytes);
    }
    send(
        &chunks,
        vec![
            Ok(ModelChunk::ThinkingEnd {
                thinking: "abcdefgh".repeat(2000),
            }),
            Ok(ModelChunk::Text("answer".into())),
            Ok(ModelChunk::TextEnd),
            Ok(ModelChunk::MessageStop {
                message_id: "volume".into(),
                reason: ModelStopReason::EndTurn,
            }),
        ],
    );
    drop(chunks);
    session.idle("main").await.unwrap();
    drain(&mut progress, &mut updates, &mut bytes);
    println!("thinking progress: {updates} updates, {bytes} bytes");
    assert!(
        updates < 300,
        "the interval throttles 2,000 deltas to {updates} updates"
    );
    assert!(
        bytes < 2_000_000,
        "the throttle keeps the snapshots under 2 MB, got {bytes}"
    );
    agent.close().await.unwrap();
}
