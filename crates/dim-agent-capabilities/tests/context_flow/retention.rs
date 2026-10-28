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
    let agent = builder_with_context(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        Context {
            threshold: Some(3_600.),
            replacement,
            ..Context::default()
        },
    )
    .build();
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
    let agent = builder_with_context(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        Context {
            threshold: Some(3_600.),
            ..Context::default()
        },
    )
    .build();
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
            let agent = builder(Arc::new(ReportedPrompt(prompt))).build();
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
    let agent = builder_with_context(
        Arc::new(EchoModel),
        HistoryRenderer::default(),
        Context {
            threshold: Some(900.),
            ..Context::default()
        },
    )
    .build();
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
