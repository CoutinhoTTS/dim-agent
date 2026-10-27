//! The retention rule and budget recovery (ContextCompaction §5.3, #128):
//! a compaction keeps the longest tail, keyed baseline included, that fits
//! a quarter of the input budget, and recovery compares the trigger's own
//! estimate with the input budget.
use super::*;
use dim_agent_capabilities::context_block;

/// The test window facts: `4096 − 64` input tokens, so the retention budget
/// is `0.25 × 4032 = 1008` serialized bytes.
const RETAINED: f64 = 0.25 * (4096. - 64.);

/// Runs a large first turn, carrying `block`, then small turns until an
/// automatic compaction completes; returns the history and the number of
/// turns whose start lies at or after the cut.
async fn compact_after_small_turns(block: InputBlock) -> (Vec<Arc<AgentEvent>>, usize) {
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
async fn retention_keeps_the_longest_tail_that_fits_its_token_budget_baseline_included() {
    let block = "k".repeat(400);
    let (events, plain) = compact_after_small_turns(context_block::raw(block.clone(), None)).await;
    assert!(plain > 2, "kept {plain} turns");
    let request = first_call_after_compaction(&events);
    let sent = serde_json::to_string(&request.messages).unwrap();
    assert!(!sent.contains("xxxx") && !sent.contains("kkkk"));
    assert!(kept_tokens(&request) <= RETAINED + request.messages.len() as f64);

    let (events, keyed) =
        compact_after_small_turns(context_block::raw(block.clone(), Some("state".into()))).await;
    assert!(keyed < plain, "keyed {keyed}, plain {plain}");
    let request = first_call_after_compaction(&events);
    assert_eq!(*request.messages[1], Message::text(Role::User, &block));
    assert!(kept_tokens(&request) <= RETAINED + request.messages.len() as f64);
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
