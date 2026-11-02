//! The retention rule and budget recovery (ContextCompaction §5.3, #128):
//! a compaction keeps the longest tail that fits a quarter of the input
//! budget or what the `0.5 × B` bound leaves after the resident parts, and
//! recovery compares the trigger's own estimate with the input budget.
use super::*;
use dim_agent_capabilities::context_block;

/// Runs a large first turn, carrying `block`, then small turns until an
/// automatic compaction completes; returns the history and the number of
/// turns whose start lies at or after the cut.
async fn compact_after_small_turns(
    block: InputBlock,
    replacement: Option<f64>,
) -> (Vec<Arc<AgentEvent>>, usize) {
    let agent = MinimalHost::start(&builder_with_context(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        Context {
            threshold: Some(3_600.),
            replacement,
            ..Context::default()
        },
    ))
    .await
    .unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    session
        .input(
            InputMessageData {
                blocks: vec![block],
                ..InputMessageData::text("x".repeat(1_200), Some(InputMode::Queue))
            },
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    for index in 0.. {
        assert!(index < 30, "no automatic compaction");
        session
            .submit(format!("small {index}"), InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
        let events = session.history("main", None).await.unwrap();
        let cut = events.iter().find_map(|event| match &event.kind {
            EventKind::ContextCompaction(ContextCompactionData {
                outcome: ContextCompactionResult::Completed { retain_from, .. },
                ..
            }) => Some(retain_from.clone()),
            _ => None,
        });
        if let Some(cut) = cut {
            let cut = events.iter().position(|event| event.id == cut).unwrap();
            let kept = events[cut..]
                .iter()
                .filter(|event| matches!(event.kind, EventKind::TurnStart(_)))
                .count();
            agent.close().await.unwrap();
            return (events, kept);
        }
    }
    unreachable!()
}

fn first_call_after_compaction(events: &[Arc<AgentEvent>]) -> Arc<ModelRequest> {
    let compaction = events
        .iter()
        .position(|event| matches!(event.kind, EventKind::ContextCompaction(_)))
        .unwrap();
    events[compaction..]
        .iter()
        .find_map(|event| match &event.kind {
            EventKind::ModelCall(data) => Some(data.request.clone()),
            _ => None,
        })
        .unwrap()
}

/// The kept messages after the replacement, as the window facts count them.
fn kept_tokens(request: &ModelRequest) -> f64 {
    Context::default().count_tokens(&ModelRequest {
        messages: request.messages[1..].to_vec(),
        ..ModelRequest::new("mock", "echo", Vec::new())
    })
}

#[tokio::test]
async fn retention_keeps_the_longest_tail_the_bound_allows() {
    let block = "k".repeat(400);
    let (events, plain) =
        compact_after_small_turns(context_block::raw(block.clone(), None), None).await;
    assert!(plain > 2, "kept {plain} turns");
    let request = first_call_after_compaction(&events);
    let sent = serde_json::to_string(&request.messages).unwrap();
    assert!(!sent.contains("xxxx") && !sent.contains("kkkk"));
    assert!(
        kept_tokens(&request) <= 0.5 * (4096. - 64.),
        "{:?}",
        request.messages
    );

    // The keyed baseline is a resident part: with the same history and the
    // same planned replacement, the keyed compaction keeps fewer turns and
    // replays the block from its baseline (ContextCompaction §5.3).
    let (_, plain) =
        compact_after_small_turns(context_block::raw(block.clone(), None), Some(1_400.)).await;
    let (events, keyed) = compact_after_small_turns(
        context_block::raw(block.clone(), Some("state".into())),
        Some(1_400.),
    )
    .await;
    assert!(keyed < plain, "keyed {keyed}, plain {plain}");
    let request = first_call_after_compaction(&events);
    assert_eq!(*request.messages[1], Message::text(Role::User, &block));
    assert!(
        kept_tokens(&request) <= 0.5 * (4096. - 64.),
        "{:?}",
        request.messages
    );
}

#[tokio::test]
async fn a_tail_that_cannot_fit_the_bound_is_replaced_entirely() {
    // The keyed block, the planned replacement and the running turn's pinned
    // input leave only a small part of the `0.5 × B` bound, and every kept
    // tail is larger: the compaction keeps no verbatim tail and the request
    // is the resident parts alone (ContextCompaction §5.3).
    let agent = MinimalHost::start(&builder_with_context(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        Context {
            threshold: Some(3_600.),
            ..Context::default()
        },
    ))
    .await
    .unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    session
        .input(
            InputMessageData {
                blocks: vec![context_block::raw("k".repeat(900), Some("state".into()))],
                ..InputMessageData::text("x".repeat(1_200), Some(InputMode::Queue))
            },
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    for text in ["z".repeat(800), "w".repeat(800), "final".into()] {
        session
            .submit(text, InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
    }
    let events = session.history("main", None).await.unwrap();
    let at = events
        .iter()
        .position(|event| {
            matches!(
                &event.kind,
                EventKind::ContextCompaction(ContextCompactionData {
                    outcome: ContextCompactionResult::Completed { .. },
                    ..
                })
            )
        })
        .expect("no automatic compaction");
    let (retain_from, replaced) = match &events[at].kind {
        EventKind::ContextCompaction(ContextCompactionData {
            outcome:
                ContextCompactionResult::Completed {
                    retain_from,
                    diagnostics,
                    ..
                },
            ..
        }) => (retain_from.clone(), diagnostics.entries_replaced),
        _ => unreachable!(),
    };
    let start = events[..at]
        .iter()
        .rposition(|event| matches!(event.kind, EventKind::ContextCompactionStart(_)))
        .unwrap();
    let cut = events
        .iter()
        .position(|event| event.id == retain_from)
        .unwrap();
    assert!(cut < start, "the cut replaces every event before it");
    let context = ContextHistory {
        checkpoint: None,
        ancestry: Vec::new(),
        positions: events[..at]
            .iter()
            .map(|event| TimelinePosition {
                branch_id: "main".into(),
                sequence: event.sequence,
            })
            .collect(),
        events: events[..at].to_vec(),
    };
    assert_eq!(
        replaced as usize,
        project_context_history(&context).unwrap().len()
    );
    let request = first_call_after_compaction(&events);
    assert_eq!(request.messages.len(), 3, "{:?}", request.messages);
    assert!(
        serde_json::to_string(&request.messages[0])
            .unwrap()
            .contains("<dim-summary>")
    );
    assert!(
        serde_json::to_string(&request.messages[1])
            .unwrap()
            .contains("[input "),
        "the running turn's input is pinned"
    );
    assert!(
        !serde_json::to_string(&request.messages[1])
            .unwrap()
            .contains("kkkk"),
        "a keyed block is replayed from the baseline, never pinned"
    );
    assert_eq!(
        *request.messages[2],
        Message::text(Role::User, "k".repeat(900))
    );
    agent.close().await.unwrap();
}

/// Echoes, reporting `prompt_tokens` as the provider's count of each request.
struct ReportedPrompt(f64);
impl ModelDriver for ReportedPrompt {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        _: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream> {
        let usage = Usage {
            prompt_tokens: self.0,
            completion_tokens: 1.,
            total_tokens: self.0 + 1.,
            cache_read_tokens: None,
            cache_write_tokens: None,
            cost: None,
            currency: None,
        };
        Box::pin(async move {
            let stream = EchoModel
                .invoke(request, ModelConnection::Local, cancel)
                .await?;
            Ok(
                Box::pin(stream.chain(futures_util::stream::once(async move {
                    Ok(ModelChunk::Usage(usage))
                }))) as ModelStream,
            )
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_compares_the_provider_baseline_estimate_with_the_input_budget() {
    tokio::time::timeout(Duration::from_secs(10), async {
        for (prompt, recovers) in [(4_000., true), (2_000., false)] {
            let agent = MinimalHost::start(&builder(Arc::new(ReportedPrompt(prompt))))
                .await
                .unwrap();
            let session = agent
                .create_session(SessionConfig::default())
                .await
                .unwrap();
            for text in ["x".repeat(1_500), "second".into()] {
                session
                    .submit(text, InputMode::Queue, "main")
                    .await
                    .unwrap();
                session.idle("main").await.unwrap();
            }
            let events = session.history("main", None).await.unwrap();
            assert!(
                events.iter().all(|event| !matches!(
                    &event.kind,
                    EventKind::ModelOutputEnd(ModelOutputEndData {
                        outcome: ModelOutputOutcome::Failed { .. },
                        ..
                    })
                )),
                "local budget recovery must not publish an intermediate failure"
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(event.kind, EventKind::ContextPrepare(_)))
                    .count(),
                2,
                "local budget recovery stays in the original preparation"
            );
            let budget = events
                .iter()
                .filter_map(|event| match &event.kind {
                    EventKind::ContextBudget(data) => Some(data),
                    _ => None,
                })
                .nth(1)
                .unwrap();
            assert_eq!(budget.baseline_source, BaselineSource::ProviderBaseline);
            assert_eq!(
                budget.estimated_input_tokens > 4096. - 64.,
                recovers,
                "{budget:?}"
            );
            let starts: Vec<_> = events
                .iter()
                .filter_map(|event| match &event.kind {
                    EventKind::ContextCompactionStart(data) => Some(data.trigger),
                    _ => None,
                })
                .collect();
            if recovers {
                assert_eq!(
                    starts,
                    [CompactionTrigger::ProviderOutputBudgetExhausted],
                    "prompt={prompt}"
                );
                assert!(events.iter().any(|event| matches!(
                    &event.kind,
                    EventKind::ContextCompaction(ContextCompactionData {
                        outcome: ContextCompactionResult::Completed { .. },
                        ..
                    })
                )));
            } else {
                assert!(starts.is_empty(), "prompt={prompt}");
            }
            assert!(
                events
                    .iter()
                    .filter_map(|event| match &event.kind {
                        EventKind::TurnEnd(data) => Some(&data.outcome),
                        _ => None,
                    })
                    .all(|outcome| matches!(outcome, TurnOutcome::Completed { .. })),
                "prompt={prompt}"
            );
            agent.close().await.unwrap();
        }
    })
    .await
    .unwrap();
}

/// Automatic compaction does not run again until `0.25 × B` of new history
/// has accumulated since the branch's latest start; the estimate alone does
/// not trigger it (ContextCompaction §5.6, acceptance 11).
#[tokio::test]
async fn automatic_compaction_waits_for_a_quarter_of_the_budget_of_new_history() {
    let agent = MinimalHost::start(&builder_with_context(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        Context {
            threshold: Some(900.),
            ..Context::default()
        },
    ))
    .await
    .unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    let starts = |events: &[Arc<AgentEvent>]| {
        events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::ContextCompactionStart(_)))
            .count()
    };
    session
        .submit("x".repeat(1_500), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let events = session.history("main", None).await.unwrap();
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        EventKind::ContextCompaction(ContextCompactionData {
            outcome: ContextCompactionResult::Completed { .. },
            ..
        })
    )));
    assert_eq!(starts(&events), 1);

    // The estimate reaches the threshold, but only ~690 bytes of new
    // history lie after the start's through.
    session
        .submit("y".repeat(200), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let events = session.history("main", None).await.unwrap();
    assert_eq!(
        starts(&events),
        1,
        "no automatic repeat on the same history"
    );

    // Past the quarter, the automatic trigger runs again.
    session
        .submit("z".repeat(400), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let events = session.history("main", None).await.unwrap();
    assert_eq!(starts(&events), 2, "the quarter of new history is reached");
    agent.close().await.unwrap();
}

/// Runs one keyed block, then small turns until an automatic compaction
/// completes under the given window facts.
async fn compact_marked(
    window: f64,
    output: f64,
    threshold: f64,
    block: usize,
    keyed: bool,
) -> Vec<Arc<AgentEvent>> {
    let agent = MinimalHost::start(&builder_with_context(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        Context {
            threshold: Some(threshold),
            window,
            output,
            ..Context::default()
        },
    ))
    .await
    .unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    session
        .input(
            InputMessageData {
                blocks: vec![context_block::raw(
                    "k".repeat(block),
                    keyed.then(|| "state".to_owned()),
                )],
                ..InputMessageData::text("x".repeat(200), Some(InputMode::Queue))
            },
            "main",
        )
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    for index in 0..30 {
        let events = session.history("main", None).await.unwrap();

        if events.iter().any(|event| {
            matches!(
                &event.kind,
                EventKind::ContextCompaction(ContextCompactionData {
                    outcome: ContextCompactionResult::Completed { .. },
                    ..
                })
            )
        }) {
            agent.close().await.unwrap();
            return events;
        }
        session
            .submit(format!("small {index}"), InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
    }
    panic!("no automatic compaction");
}

fn completed_mark(events: &[Arc<AgentEvent>]) -> Option<InsufficientSpace> {
    events.iter().find_map(|event| match &event.kind {
        EventKind::ContextCompaction(ContextCompactionData {
            outcome: ContextCompactionResult::Completed { diagnostics, .. },
            ..
        }) => diagnostics.insufficient_space.clone(),
        _ => None,
    })
}

#[tokio::test]
async fn a_compaction_whose_resident_parts_exceed_half_the_budget_is_marked() {
    // 32,000 / 8,000: `0.5 × B` is 12,000, and the keyed baseline alone is
    // larger, so the completed compaction names it and nothing small
    // (ContextCompaction §11 test 10).
    let events = compact_marked(32_000., 8_000., 13_000., 12_400, true).await;
    let mark = completed_mark(&events).expect("a mark");
    assert_eq!(mark.parts[0].part, InsufficientSpacePartName::KeyedBaseline);
    assert!(mark.parts[0].tokens > 12_000., "{:?}", mark.parts);
    assert!(mark.parts.iter().all(|part| part.tokens > 1_000.));

    // 200,000 / 64,000: the same parts are within `0.5 × B`, so no mark.
    let events = compact_marked(200_000., 64_000., 60_600., 60_000, false).await;
    assert!(completed_mark(&events).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_summary_that_overshoots_the_budget_fails_the_recovery_request() {
    let budget = 8_192. - 2_048.;
    let agent = MinimalHost::start(&builder_with_context(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        Context {
            window: 8_192.,
            output: 2_048.,
            summary: Some(Box::leak("s".repeat(6_000).into_boxed_str())),
            ..Context::default()
        },
    ))
    .await
    .unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    session
        .submit("x".repeat(8_000), InputMode::Queue, "main")
        .await
        .unwrap();
    session.idle("main").await.unwrap();
    let events = session.history("main", None).await.unwrap();
    let starts = events
        .iter()
        .filter(|event| matches!(event.kind, EventKind::ContextCompactionStart(_)))
        .count();
    assert_eq!(starts, 1, "budget recovery compacts once, and never again");
    let mark = completed_mark(&events).expect("the completed compaction is marked");
    assert_eq!(mark.parts[0].part, InsufficientSpacePartName::Summary);
    let tokens = mark.parts[0].tokens;
    assert!(tokens > budget / 2., "{tokens}");
    let error = events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::TurnEnd(data) => match &data.outcome {
                TurnOutcome::Failed { error } => Some(error.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("the turn failed");
    assert!(
        error.contains("Context recovery could not fit the request"),
        "{error}"
    );
    assert!(
        error.contains(&format!("summary {tokens:.0} tokens")),
        "{error}"
    );
    let (failed, facts) = events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ModelOutputEnd(data) => match &data.outcome {
                ModelOutputOutcome::Failed { error, budget, .. } => {
                    Some((error.clone(), budget.clone()))
                }
                _ => None,
            },
            _ => None,
        })
        .expect("the recovered request failed");
    assert!(
        failed.contains("Context recovery could not fit"),
        "{failed}"
    );
    assert!(
        facts.is_none(),
        "terminal recovery failure must not request another recovery"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::ModelOutputEnd(_)))
            .count(),
        1,
        "only the terminal recovery failure is published"
    );
    agent.close().await.unwrap();
}

/// A child branch forked after the parent's completed compaction counts the
/// history it writes as new, although its local sequences are smaller than
/// the inherited attempt's `through`: the attempt's boundary is read in the
/// context's own order (ContextCompaction §5.6, acceptance 11).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_branch_regrows_from_its_own_history() {
    let agent = MinimalHost::start(&builder_with_context(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        Context {
            threshold: Some(3_000.),
            summary: Some(Box::leak("s".repeat(1_100).into_boxed_str())),
            ..Context::default()
        },
    ))
    .await
    .unwrap();
    let session = agent
        .create_session(SessionConfig::default())
        .await
        .unwrap();
    for text in ["p".repeat(1_500), "q".repeat(1_500)] {
        session
            .submit(text, InputMode::Queue, "main")
            .await
            .unwrap();
        session.idle("main").await.unwrap();
    }
    let events = session.history("main", None).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                &event.kind,
                EventKind::ContextCompaction(ContextCompactionData {
                    outcome: ContextCompactionResult::Completed { .. },
                    ..
                })
            ))
            .count(),
        1,
        "the parent compacts once"
    );
    let at = session.sequence("main").unwrap();
    let child = session
        .branch(
            "main".into(),
            at,
            BranchOptions {
                prompt: "child".into(),
                source: BranchSource::Host {
                    operation_id: "test-branch".into(),
                },
            },
        )
        .await
        .unwrap();
    session.idle(&child).await.unwrap();
    session
        .submit("c".repeat(1_000), InputMode::Queue, &child)
        .await
        .unwrap();
    session.idle(&child).await.unwrap();
    let child_events = session.history(&child, None).await.unwrap();
    let inherited = events
        .iter()
        .filter(|event| matches!(event.kind, EventKind::ContextCompactionStart(_)))
        .count();
    let starts: Vec<_> = child_events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::ContextCompactionStart(data) => Some(data.trigger),
            _ => None,
        })
        .collect();
    assert_eq!(
        starts.len(),
        inherited + 1,
        "the child compacts on the history it wrote: {starts:?}"
    );
    assert_eq!(
        starts.last(),
        Some(&CompactionTrigger::Threshold),
        "at the threshold, without overflow recovery"
    );
    let budget = child_events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::ContextBudget(data) => Some(data.estimated_input_tokens),
            _ => None,
        })
        .expect("the child's preparation");
    assert!(
        (3_000.0..=4_032.0).contains(&budget),
        "the estimate reaches the threshold within the input budget: {budget}"
    );
    agent.close().await.unwrap();
}
