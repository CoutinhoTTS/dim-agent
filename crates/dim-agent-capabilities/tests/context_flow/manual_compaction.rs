//! The Loop's compaction step and the record contract (ContextCompaction
//! §5.3–§5.8, acceptance 3–6 and 8 at the capability level).
use super::*;
use dim_agent_capabilities::{
    agent_loop::AgentLoop,
    history::{context_messages, summary_message},
};
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, Default, PartialEq)]
enum Mode {
    /// The legal cut within the retention limit, one summary message.
    #[default]
    Cut,
    /// The same cut with a two-message replacement.
    TwoMessages,
    /// A cut after the limit: inside the retained turns.
    InsideRetained,
    /// The first call fails; later calls cut.
    FailFirst,
}

/// A test strategy that can hold its next call until released or cancelled.
#[derive(Default)]
struct Scripted {
    mode: Mode,
    calls: AtomicUsize,
    hold: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Scripted {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            ..Self::default()
        })
    }
}
impl CompactionStrategy for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }
    fn compact(
        &self,
        input: CompactionInput,
        cancel: CancellationToken,
    ) -> HostFuture<'_, StrategyOutcome> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.hold.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                tokio::select! {
                    _ = cancel.cancelled() => {
                        return Err(HostError::Cancelled("strategy cancelled".into()));
                    }
                    _ = self.release.notified() => {}
                }
            }
            if self.mode == Mode::FailFirst && call == 0 {
                return Err(HostError::failed("summarizer unavailable"));
            }
            let cut = match self.mode {
                Mode::InsideRetained => input.cuts.last(),
                _ => input.cut(),
            };
            let Some(cut) = cut else {
                return Ok(StrategyOutcome::Skipped {
                    reason: "nothing_to_compact".into(),
                    diagnostics: test_diagnostics(),
                });
            };
            let replacement = match self.mode {
                Mode::TwoMessages => vec![
                    Message::text(Role::User, "first replacement"),
                    Message::text(Role::User, "second replacement"),
                ],
                _ => vec![summary_message("short")],
            };
            Ok(StrategyOutcome::Completed {
                retain_from: cut.retain_from.clone(),
                replacement,
                diagnostics: test_diagnostics(),
            })
        })
    }
}

/// Emits one `echo` call for an input that asks for it, holding it until
/// released when armed; echoes otherwise.
#[derive(Default)]
struct ToolOnRequest {
    hold: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    text: std::sync::Mutex<String>,
}
impl ModelDriver for ToolOnRequest {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        let asks = matches!(&request.messages.last().unwrap().content,
            MessageContent::Text(text) if text == "use the tool");
        Box::pin(async move {
            if !asks {
                return EchoModel.invoke(request, connection, cancel).await;
            }
            if self.hold.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            let text = self.text.lock().unwrap().clone();
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ModelChunk::MessageStart {
                    message_id: "tool".into(),
                }),
                Ok(ModelChunk::ToolStart {
                    id: "echo-call".into(),
                    name: "echo".into(),
                }),
                Ok(ModelChunk::ToolUse(ToolCall {
                    id: "echo-call".into(),
                    name: "echo".into(),
                    input: serde_json::from_value(serde_json::json!({ "text": text })).unwrap(),
                })),
                Ok(ModelChunk::MessageStop {
                    message_id: "tool".into(),
                    reason: ModelStopReason::ToolUse,
                }),
            ])) as ModelStream)
        })
    }
}

/// Provides the strategy from a child plugin; restarting it reloads the
/// Loop and ContextPreparation of every branch.
#[derive(Clone, Default)]
struct StrategyReload(Arc<std::sync::Mutex<Vec<rutis::FiberView>>>);
struct StrategyOwner {
    scope: SessionScope,
    strategy: Arc<Scripted>,
    views: Arc<std::sync::Mutex<Vec<rutis::FiberView>>>,
}
impl TypedPlugin for StrategyOwner {
    type Deps = ();
    fn name(&self) -> &str {
        "test.strategy-owner"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let view = ctx.plugin(capability_plugin(StrategyProvider(
                self.scope,
                self.strategy.clone(),
            )));
            self.views.lock().unwrap().push(view);
            Ok(Effect::Done)
        })
    }
}
impl Capability for StrategyOwner {
    fn keys(&self) -> KeysOf<Self> {}
}
struct StrategyProvider(SessionScope, Arc<Scripted>);
impl TypedPlugin for StrategyProvider {
    type Deps = ();
    fn name(&self) -> &str {
        "test.strategy"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as(
                self.0.key::<CompactionStrategyHost>(),
                Arc::new(CompactionStrategyHost(Some(self.1.clone()))),
            )?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for StrategyProvider {
    fn keys(&self) -> KeysOf<Self> {}
}

/// No automatic compaction (a high threshold); `strategy` serves manual
/// steps and budget recovery. With `reload`, the strategy is provided by a
/// restartable plugin.
fn builder_with_strategy(
    model: Arc<dyn ModelDriver>,
    history: HistoryRenderer,
    strategy: Arc<Scripted>,
    reload: Option<&StrategyReload>,
) -> AgentBuilder {
    let mut builder = AgentBuilder::new(AgentConfig::default());
    builder
        .session_value(Arc::new(ModelHost(model)))
        .session_value(Arc::new(ToolHost(Arc::new(MockTools))))
        .session_value(Arc::new(history))
        .session_value(Arc::new(PrepareContextHost(Arc::new(IdentityContext))))
        .session_value(Arc::new(ContextModelHost(Some(Arc::new(
            Context::default(),
        )))))
        .session_plugin(DefaultPrepareModelPlugin::new);
    match reload {
        Some(reload) => {
            let views = reload.0.clone();
            builder.session_plugin(move |scope| StrategyOwner {
                scope: *scope,
                strategy: strategy.clone(),
                views: views.clone(),
            });
        }
        None => {
            builder.session_value(Arc::new(CompactionStrategyHost(Some(strategy))));
        }
    }
    // A manual step builds its request from the adopted configuration.
    defaults::install_with_prompt(
        &mut builder,
        Arc::new(EmptyPromptHost),
        Some(ConfigureUpdate {
            sources: Vec::new(),
            model: Some(ModelSelection {
                provider_id: "mock".into(),
                model_id: "echo".into(),
                capability_snapshot: None,
                config: None,
                metadata: None,
            }),
        }),
    );
    builder
}

/// A first turn long enough that replacing it shrinks the request.
fn first() -> String {
    "first ".repeat(100)
}

async fn turns(session: &Session, texts: &[&str]) {
    for text in texts {
        session
            .submit((*text).into(), InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
    }
}

/// Waits until the branch history satisfies `done`.
async fn until(
    session: &Session,
    done: impl Fn(&[Arc<AgentEvent>]) -> bool,
) -> Vec<Arc<AgentEvent>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let events = session.history("main", None).await.unwrap();
            if done(&events) {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the expected facts are committed")
}

/// The result covering `command`, as the method's waiter reads it: the
/// first result with this command or, while no step started for it, the
/// first result with a later one.
fn result_for(events: &[Arc<AgentEvent>], command: u64) -> Option<&ContextCompactionData> {
    let mut started = false;
    events.iter().find_map(|event| match &event.kind {
        EventKind::ContextCompactionStart(data) => {
            started |= data.command == Some(command);
            None
        }
        EventKind::ContextCompaction(data)
            if data
                .command
                .is_some_and(|c| c == command || (c > command && !started)) =>
        {
            Some(data)
        }
        _ => None,
    })
}

/// Waits until the Loop has consumed the event at `sequence`.
async fn consumed(session: &Session, sequence: u64) {
    let mut dispatch = session.dispatch_watch("main").unwrap();
    while dispatch.borrow().sequence < sequence {
        dispatch.changed().await.unwrap();
    }
}

fn starts_of(events: &[Arc<AgentEvent>]) -> Vec<Option<u64>> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ContextCompactionStart(data) => Some(data.command),
            _ => None,
        })
        .collect()
}

/// Every start has exactly one result with its `start` and the same ids,
/// and no result names a start that does not exist (acceptance 8).
pub(crate) fn assert_paired(events: &[Arc<AgentEvent>]) {
    for event in events {
        match &event.kind {
            EventKind::ContextCompactionStart(start) => {
                let results: Vec<_> = events
                    .iter()
                    .filter_map(|other| match &other.kind {
                        EventKind::ContextCompaction(data)
                            if data.start == Some(event.sequence) =>
                        {
                            Some(data)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(results.len(), 1, "start {}", event.sequence);
                let result = results[0];
                assert_eq!(
                    (
                        &result.turn_id,
                        &result.request_id,
                        result.command,
                        result.through
                    ),
                    (
                        &start.turn_id,
                        &start.request_id,
                        start.command,
                        start.through
                    )
                );
            }
            EventKind::ContextCompaction(ContextCompactionData {
                start: Some(start), ..
            }) => assert!(events.iter().any(|other| other.sequence == *start
                && matches!(other.kind, EventKind::ContextCompactionStart(_)))),
            _ => {}
        }
    }
}

/// The messages a preparation at the head would start from.
async fn projected(timeline_session: &Session, branch: &str) -> Vec<Message> {
    let through = timeline_session.sequence(branch).unwrap();
    let context = timeline_session.context(branch, through).await.unwrap();
    let history = project_context_history(&context).unwrap();
    context_messages(&context, &history)
}

/// Acceptance 3: with no turn, `context/compact` is served at once and
/// writes only the command, the start and the result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_manual_compaction_with_no_turn_writes_only_its_facts() {
    let strategy = Scripted::new(Mode::Cut);
    let agent = builder_with_strategy(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        strategy.clone(),
        None,
    )
    .build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    turns(&session, &[first().as_str(), "second", "third"]).await;
    let command = session
        .service::<AgentLoop>("main")
        .unwrap()
        .compact()
        .unwrap();
    let events = until(&session, |events| result_for(events, command).is_some()).await;
    let after: Vec<_> = events
        .iter()
        .filter(|event| event.sequence >= command)
        .collect();
    assert_eq!(after.len(), 3, "{after:#?}");
    let EventKind::ContextCompactionStart(start) = &after[1].kind else {
        panic!("expected the start, got {:?}", after[1].kind)
    };
    assert_eq!(
        start,
        &ContextCompactionStartData {
            turn_id: None,
            request_id: None,
            command: Some(command),
            through: command,
            trigger: CompactionTrigger::Manual,
            strategy: "scripted".into(),
        }
    );
    let EventKind::ContextCompaction(result) = &after[2].kind else {
        panic!("expected the result")
    };
    assert_eq!(
        (result.command, result.start, result.through),
        (Some(command), Some(after[1].sequence), command)
    );
    let ContextCompactionResult::Completed {
        retain_from,
        strategy: name,
        diagnostics,
        ..
    } = &result.outcome
    else {
        panic!("expected completion, got {:?}", result.outcome)
    };
    // The cut keeps the last two turns: it is the second turn's start.
    let starts: Vec<_> = events
        .iter()
        .filter(|event| matches!(event.kind, EventKind::TurnStart(_)))
        .collect();
    assert_eq!(retain_from, &starts[1].id);
    assert_eq!(name, "scripted");
    assert_eq!(diagnostics.reason, CompactionReason::Manual);
    assert_eq!(diagnostics.retain_limit.as_ref(), Some(&starts[1].id));
    // The next turn starts from the compacted context.
    turns(&session, &["fourth"]).await;
    let events = session.history("main", None).await.unwrap();
    let request = events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(&data.request),
            _ => None,
        })
        .unwrap();
    assert_eq!(*request.messages[0], summary_message("short"));
    let text = serde_json::to_string(&request.messages).unwrap();
    assert!(!text.contains("first") && text.contains("second") && text.contains("third"));
    assert_paired(&events);
    agent.close().await.unwrap();
}

/// Acceptance 4: inside a turn the step waits for the next step boundary,
/// then the turn goes on from the compacted context.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_manual_compaction_inside_a_turn_runs_at_the_next_step_boundary() {
    let model = Arc::new(ToolOnRequest::default());
    *model.text.lock().unwrap() = "tool output".into();
    let agent = builder_with_strategy(
        model.clone(),
        HistoryRenderer::default(),
        Scripted::new(Mode::Cut),
        None,
    )
    .build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    turns(&session, &[first().as_str(), "second"]).await;
    model.hold.store(true, Ordering::SeqCst);
    session
        .submit("use the tool".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    model.entered.notified().await;
    let command = session
        .service::<AgentLoop>("main")
        .unwrap()
        .compact()
        .unwrap();
    // The command is consumed while the request is in flight: pending.
    let mut dispatch = session.dispatch_watch("main").unwrap();
    while dispatch.borrow().sequence < command {
        dispatch.changed().await.unwrap();
    }
    assert!(
        result_for(&session.history("main", None).await.unwrap(), command).is_none(),
        "no step runs while a request is in flight"
    );
    model.release.notify_one();
    session.idle("main").await.unwrap();
    let events = until(&session, |events| {
        matches!(
            events.last().map(|event| &event.kind),
            Some(EventKind::TurnEnd(_))
        )
    })
    .await;
    let position = |predicate: &dyn Fn(&EventKind) -> bool| {
        events
            .iter()
            .position(|event| predicate(&event.kind))
            .unwrap()
    };
    let turn = match &events[position(&|kind| matches!(kind, EventKind::ToolCall(_)))].kind {
        EventKind::ToolCall(data) => data.turn_id.clone(),
        _ => unreachable!(),
    };
    let result = position(
        &|kind| matches!(kind, EventKind::ToolResult(data) if data.call_event_id.is_some()),
    );
    let start = position(&|kind| matches!(kind, EventKind::ContextCompactionStart(_)));
    let fact = position(&|kind| matches!(kind, EventKind::ContextCompaction(_)));
    let next = events
        .iter()
        .enumerate()
        .skip(fact)
        .find(|(_, event)| matches!(event.kind, EventKind::ModelStart(_)))
        .map(|(index, _)| index)
        .unwrap();
    assert!(result < start && start < fact && fact < next);
    let EventKind::ContextCompactionStart(data) = &events[start].kind else {
        unreachable!()
    };
    assert_eq!(
        (data.turn_id.as_ref(), data.command, data.through),
        (Some(&turn), Some(command), events[result].sequence)
    );
    let EventKind::ContextCompaction(data) = &events[fact].kind else {
        unreachable!()
    };
    assert_eq!(
        (data.turn_id.as_ref(), data.command, data.start),
        (Some(&turn), Some(command), Some(events[start].sequence))
    );
    assert!(matches!(
        data.outcome,
        ContextCompactionResult::Completed { .. }
    ));
    // The turn's next request is prepared from the compacted context and
    // keeps the tool call with its result.
    let request = events[next..]
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(&data.request),
            _ => None,
        })
        .unwrap();
    assert_eq!(*request.messages[0], summary_message("short"));
    let text = serde_json::to_string(&request.messages).unwrap();
    assert!(text.contains("tool output") && !text.contains("first"));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::TurnStart(_)))
            .count(),
        3
    );
    assert!(matches!(
        &events.last().unwrap().kind,
        EventKind::TurnEnd(TurnEndData {
            outcome: TurnOutcome::Completed { .. },
            ..
        })
    ));
    assert_paired(&events);
    agent.close().await.unwrap();
}

/// Renders the default history with another entry id scheme, keeping each
/// entry's source event.
fn renamed_history(context: &ContextHistory) -> std::result::Result<Vec<HistoryEntry>, String> {
    let mut history = project_context_history(context)?;
    for (index, entry) in history.iter_mut().enumerate() {
        entry.id = format!("other-scheme-{index}");
    }
    Ok(history)
}

/// Acceptance 5: one replay rule for every strategy. A two-message
/// replacement is committed and replayed as is; reload, a fork and another
/// renderer project the same messages; an illegal cut is rejected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replaced_strategy_is_replayed_by_the_same_rule_and_checked_by_the_policy() {
    let store = Arc::new(MemoryTimelineStore::default());
    let mut builder = builder_with_strategy(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        Scripted::new(Mode::TwoMessages),
        None,
    );
    builder.timeline_store(store.clone());
    let agent = builder.build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    turns(&session, &[first().as_str(), "second", "third"]).await;
    let command = session
        .service::<AgentLoop>("main")
        .unwrap()
        .compact()
        .unwrap();
    let events = until(&session, |events| result_for(events, command).is_some()).await;
    let fact = events.last().unwrap().sequence;
    let messages = projected(&session, "main").await;
    assert_eq!(
        messages[..2],
        [
            Message::text(Role::User, "first replacement"),
            Message::text(Role::User, "second replacement"),
        ]
    );
    // A branch forked after the compaction inherits the same projection.
    let child = session
        .branch(
            "main".into(),
            fact,
            BranchOptions {
                prompt: "continue".into(),
                source: BranchSource::Host {
                    operation_id: "fork-after-compaction".into(),
                },
            },
        )
        .await
        .unwrap();
    session.idle(&child).await.unwrap();
    assert!(projected(&session, &child).await.starts_with(&messages));
    // Another renderer finds the cut from the event id, not the entry id.
    let context = session.context("main", fact).await.unwrap();
    let renamed = renamed_history(&context).unwrap();
    assert_eq!(context_messages(&context, &renamed), messages);
    let id = session.id().to_owned();
    agent.close().await.unwrap();

    // A restored Session projects the same messages, from facts or snapshot.
    let mut builder = builder_with_strategy(
        Arc::new(EchoModel),
        HistoryRenderer {
            render: renamed_history,
            ..HistoryRenderer::default()
        },
        Scripted::new(Mode::InsideRetained),
        None,
    );
    builder.timeline_store(store.clone());
    let agent = builder.build();
    let session = agent.restore_session(id).await.unwrap();
    let timeline_context = session.context("main", fact).await.unwrap();
    let history = renamed_history(&timeline_context).unwrap();
    assert_eq!(context_messages(&timeline_context, &history), messages);
    // A cut inside the retained turns is rejected: the record fails and the
    // projection stays as it was.
    let command = session
        .service::<AgentLoop>("main")
        .unwrap()
        .compact()
        .unwrap();
    let events = until(&session, |events| result_for(events, command).is_some()).await;
    assert!(matches!(
        &result_for(&events, command).unwrap().outcome,
        ContextCompactionResult::Failed { error, .. }
            if error == "Compaction cut lies inside the retained turns."
    ));
    assert_eq!(projected(&session, "main").await, messages);
    assert_paired(&events);
    agent.close().await.unwrap();
}

/// Acceptance 6: an interrupt cancels a running step, and a pending one ends
/// without a start.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_interrupt_cancels_a_running_step_and_a_pending_command() {
    let strategy = Scripted::new(Mode::Cut);
    let model = Arc::new(ToolOnRequest::default());
    *model.text.lock().unwrap() = "tool output".into();
    let agent = builder_with_strategy(
        model.clone(),
        HistoryRenderer::default(),
        strategy.clone(),
        None,
    )
    .build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    turns(&session, &[first().as_str(), "second", "third"]).await;
    // Running, with no turn.
    strategy.hold.store(true, Ordering::SeqCst);
    let loop_service = session.service::<AgentLoop>("main").unwrap();
    let command = loop_service.compact().unwrap();
    strategy.entered.notified().await;
    session.interrupt("main", "user stop".into()).unwrap();
    let events = until(&session, |events| result_for(events, command).is_some()).await;
    let result = result_for(&events, command).unwrap();
    assert!(result.start.is_some());
    assert!(matches!(&result.outcome,
        ContextCompactionResult::Cancelled { reason } if reason == "user stop"));
    assert_eq!(strategy.calls.load(Ordering::SeqCst), 1);
    // Pending inside a turn: the interrupt settles it without a start and
    // the turn ends as interrupted turns do.
    model.hold.store(true, Ordering::SeqCst);
    session
        .submit("use the tool".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    model.entered.notified().await;
    let command = loop_service.compact().unwrap();
    let mut dispatch = session.dispatch_watch("main").unwrap();
    while dispatch.borrow().sequence < command {
        dispatch.changed().await.unwrap();
    }
    session.interrupt("main", "user stop".into()).unwrap();
    model.release.notify_one();
    session.idle("main").await.unwrap();
    let events = until(&session, |events| result_for(events, command).is_some()).await;
    let result = result_for(&events, command).unwrap();
    assert_eq!(result.start, None);
    assert!(matches!(&result.outcome,
        ContextCompactionResult::Cancelled { reason } if reason == "user stop"));
    assert_eq!(strategy.calls.load(Ordering::SeqCst), 1);
    assert_paired(&events);
    agent.close().await.unwrap();
}

/// Acceptance 6: a plugin reload during a step settles it `cancelled:
/// capability reloading` without a second strategy call; a new command
/// compacts normally.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reload_during_a_step_settles_it_without_another_strategy_call() {
    let strategy = Scripted::new(Mode::Cut);
    let reload = StrategyReload::default();
    let agent = builder_with_strategy(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        strategy.clone(),
        Some(&reload),
    )
    .build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    turns(&session, &[first().as_str(), "second", "third"]).await;
    let before = projected(&session, "main").await;
    strategy.hold.store(true, Ordering::SeqCst);
    let command = session
        .service::<AgentLoop>("main")
        .unwrap()
        .compact()
        .unwrap();
    strategy.entered.notified().await;
    let view = reload.0.lock().unwrap().last().cloned().unwrap();
    view.restart().await.unwrap();
    let events = until(&session, |events| result_for(events, command).is_some()).await;
    let result = result_for(&events, command).unwrap();
    assert!(result.start.is_some());
    assert!(matches!(&result.outcome,
        ContextCompactionResult::Cancelled { reason } if reason == CAPABILITY_RELOADING));
    assert_eq!(projected(&session, "main").await, before);
    // The reloaded Loop does not resume the step; a new command compacts.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(strategy.calls.load(Ordering::SeqCst), 1);
    let command = session
        .service::<AgentLoop>("main")
        .unwrap()
        .compact()
        .unwrap();
    let events = until(&session, |events| result_for(events, command).is_some()).await;
    assert!(matches!(
        result_for(&events, command).unwrap().outcome,
        ContextCompactionResult::Completed { .. }
    ));
    assert_eq!(strategy.calls.load(Ordering::SeqCst), 2);
    assert_paired(&events);
    agent.close().await.unwrap();
}

/// Acceptance 6: a manual compaction failing inside a turn lets the turn go
/// on uncompacted; when the next request exceeds the window, budget
/// recovery compacts with a fallback cut.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_manual_compaction_lets_the_turn_continue_and_recovery_compacts() {
    let model = Arc::new(ToolOnRequest::default());
    *model.text.lock().unwrap() = "t".repeat(1_500);
    let agent = builder_with_strategy(
        model.clone(),
        HistoryRenderer::default(),
        Scripted::new(Mode::FailFirst),
        None,
    )
    .build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let first = "f".repeat(1_000);
    turns(&session, &[first.as_str(), "second"]).await;
    model.hold.store(true, Ordering::SeqCst);
    session
        .submit("use the tool".into(), InputMode::Queue, "main")
        .await
        .unwrap();
    model.entered.notified().await;
    let command = session
        .service::<AgentLoop>("main")
        .unwrap()
        .compact()
        .unwrap();
    let mut dispatch = session.dispatch_watch("main").unwrap();
    while dispatch.borrow().sequence < command {
        dispatch.changed().await.unwrap();
    }
    model.release.notify_one();
    session.idle("main").await.unwrap();
    let events = until(&session, |events| {
        matches!(
            events.last().map(|event| &event.kind),
            Some(EventKind::TurnEnd(_))
        )
    })
    .await;
    let manual = result_for(&events, command).unwrap();
    assert!(manual.start.is_some());
    assert!(matches!(&manual.outcome,
        ContextCompactionResult::Failed { error, .. } if error.contains("summarizer unavailable")));
    let recovered = events
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::ContextCompaction(data) if data.command.is_none() => Some(data),
            _ => None,
        })
        .expect("budget recovery compacts");
    let ContextCompactionResult::Completed { diagnostics, .. } = &recovered.outcome else {
        panic!("expected completion, got {:?}", recovered.outcome)
    };
    assert_eq!(
        diagnostics.trigger,
        CompactionTrigger::ProviderOutputBudgetExhausted
    );
    assert!(matches!(
        &events.last().unwrap().kind,
        EventKind::TurnEnd(TurnEndData {
            outcome: TurnOutcome::Completed { .. },
            ..
        })
    ));
    assert_paired(&events);
    agent.close().await.unwrap();
}

/// Two commands interleaved with a running step (§5.4): commands consumed
/// while the step runs coalesce into one pending command, the running
/// step's result retires only its own command, and the pending one runs
/// next under its own sequence before the Loop advances. The coalesced
/// command is answered by that next step's result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn commands_consumed_during_a_step_run_as_the_next_step() {
    let strategy = Scripted::new(Mode::Cut);
    let agent = builder_with_strategy(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        strategy.clone(),
        None,
    )
    .build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    turns(&session, &[first().as_str(), "second", "third"]).await;
    let loop_service = session.service::<AgentLoop>("main").unwrap();
    strategy.hold.store(true, Ordering::SeqCst);
    let c1 = loop_service.compact().unwrap();
    strategy.entered.notified().await;
    let c2 = loop_service.compact().unwrap();
    let c3 = loop_service.compact().unwrap();
    consumed(&session, c3).await;
    strategy.release.notify_one();
    let events = until(&session, |events| result_for(events, c3).is_some()).await;
    // One step per active command: c1, then c3 with c2 coalesced into it.
    assert_eq!(starts_of(&events), [Some(c1), Some(c3)]);
    let first_result = result_for(&events, c1).unwrap();
    assert_eq!(first_result.command, Some(c1));
    assert!(matches!(
        first_result.outcome,
        ContextCompactionResult::Completed { .. }
    ));
    let next = result_for(&events, c3).unwrap();
    assert_eq!(next.command, Some(c3));
    assert!(next.start.is_some());
    // The coalesced command is answered by the step that served c3.
    assert_eq!(result_for(&events, c2), Some(next));
    assert_eq!(strategy.calls.load(Ordering::SeqCst), 2);
    // Normal advancement resumes after the pending step.
    turns(&session, &["fourth"]).await;
    let events = session.history("main", None).await.unwrap();
    assert!(matches!(
        &events.last().unwrap().kind,
        EventKind::TurnEnd(TurnEndData {
            outcome: TurnOutcome::Completed { .. },
            ..
        })
    ));
    assert_paired(&events);
    agent.close().await.unwrap();
}

/// An interrupt with both an active and a pending command settles both as
/// `cancelled`: the active one with its start, the pending one without.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_interrupt_settles_the_active_and_the_pending_command() {
    let strategy = Scripted::new(Mode::Cut);
    let agent = builder_with_strategy(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        strategy.clone(),
        None,
    )
    .build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    turns(&session, &[first().as_str(), "second", "third"]).await;
    let loop_service = session.service::<AgentLoop>("main").unwrap();
    strategy.hold.store(true, Ordering::SeqCst);
    let c1 = loop_service.compact().unwrap();
    strategy.entered.notified().await;
    let c2 = loop_service.compact().unwrap();
    consumed(&session, c2).await;
    session.interrupt("main", "user stop".into()).unwrap();
    let events = until(&session, |events| {
        result_for(events, c1).is_some() && result_for(events, c2).is_some()
    })
    .await;
    let active = result_for(&events, c1).unwrap();
    assert_eq!((active.command, active.start.is_some()), (Some(c1), true));
    assert!(matches!(&active.outcome,
        ContextCompactionResult::Cancelled { reason } if reason == "user stop"));
    let pending = result_for(&events, c2).unwrap();
    assert_eq!((pending.command, pending.start), (Some(c2), None));
    assert!(matches!(&pending.outcome,
        ContextCompactionResult::Cancelled { reason } if reason == "user stop"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    let events = session.history("main", None).await.unwrap();
    assert_eq!(starts_of(&events), [Some(c1)]);
    assert_eq!(strategy.calls.load(Ordering::SeqCst), 1);
    assert_paired(&events);
    agent.close().await.unwrap();
}

/// A plugin reload with both an active and a pending command settles both as
/// `cancelled: capability reloading`, with no further strategy call.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reload_settles_the_active_and_the_pending_command() {
    let strategy = Scripted::new(Mode::Cut);
    let reload = StrategyReload::default();
    let agent = builder_with_strategy(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        strategy.clone(),
        Some(&reload),
    )
    .build();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    turns(&session, &[first().as_str(), "second", "third"]).await;
    let loop_service = session.service::<AgentLoop>("main").unwrap();
    strategy.hold.store(true, Ordering::SeqCst);
    let c1 = loop_service.compact().unwrap();
    strategy.entered.notified().await;
    let c2 = loop_service.compact().unwrap();
    consumed(&session, c2).await;
    let view = reload.0.lock().unwrap().last().cloned().unwrap();
    view.restart().await.unwrap();
    let events = until(&session, |events| {
        result_for(events, c1).is_some() && result_for(events, c2).is_some()
    })
    .await;
    let active = result_for(&events, c1).unwrap();
    assert_eq!((active.command, active.start.is_some()), (Some(c1), true));
    let pending = result_for(&events, c2).unwrap();
    assert_eq!((pending.command, pending.start), (Some(c2), None));
    for result in [active, pending] {
        assert!(matches!(&result.outcome,
            ContextCompactionResult::Cancelled { reason } if reason == CAPABILITY_RELOADING));
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let events = session.history("main", None).await.unwrap();
    assert_eq!(starts_of(&events), [Some(c1)]);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(&event.kind,
                EventKind::ContextCompaction(data) if data.command == Some(c2)))
            .count(),
        1
    );
    assert_eq!(strategy.calls.load(Ordering::SeqCst), 1);
    assert_paired(&events);
    agent.close().await.unwrap();
}
