use crate::{
    history::{HistoryEntry, HistoryRenderer, context_messages, summary_message},
    host::*,
};
use dim_agent_runtime::{ContextHistory, Timeline};
use dim_agent_types::*;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) struct ContextPolicy {
    pub timeline: Arc<Timeline>,
    pub resolver: Arc<dyn ResolveContextModel>,
    pub history: Arc<HistoryRenderer>,
}
struct Estimate {
    input: f64,
    source: BaselineSource,
    baseline: Option<f64>,
    delta: Option<f64>,
}

impl ContextPolicy {
    pub async fn prepare(
        &self,
        request: Arc<ModelRequest>,
        source: &ContextPrepareData,
        context: ContextHistory,
        history: Vec<HistoryEntry>,
        cancel: CancellationToken,
    ) -> Result<Arc<ModelRequest>, HostError> {
        let prefix_len = context_messages(&context, &history).len();
        // A retry may reuse only compactions belonging to the fixed source.
        let mut events = context.events.clone();
        events.extend(
            self.timeline
                .local(source.through.saturating_add(1)..)
                .await
                .map_err(|e| HostError::failed(e.to_string()))?
                .into_iter()
                .filter(|event| match &event.kind {
                    EventKind::ContextCompaction(data) => {
                        data.through == source.through && data.turn_id == source.turn_id
                    }
                    EventKind::ContextBudget(data) => data.turn_id == source.turn_id,
                    _ => false,
                }),
        );
        let checkpoint = events.iter().rev().find_map(|event| match &event.kind {
            EventKind::ContextCompaction(ContextCompactionData {
                outcome:
                    ContextCompactionResult::Completed {
                        cursor,
                        summary,
                        context_baseline,
                        ..
                    },
                ..
            }) if validate_cut(&history, cursor).is_ok() => Some(CompactionCheckpoint {
                cursor: cursor.clone(),
                summary: summary.clone(),
                context_baseline: context_baseline.clone().into(),
            }),
            _ => None,
        });
        let mut prepared = project(request.clone(), &history, prefix_len, checkpoint.as_ref())?;
        let model = self
            .resolver
            .resolve(prepared.clone(), cancel.clone())
            .await?;
        check_cancel(&cancel)?;
        let estimate = self.estimate(&prepared, model.as_ref(), &events)?;
        let boundary = history.last().map(|entry| entry.id.clone());
        let checked = events.iter().any(|event| {
            matches!(&event.kind, EventKind::ContextBudget(data)
            if data.boundary == boundary && data.turn_id == source.turn_id)
        });
        self.timeline
            .append(
                EventKind::ContextBudget(ContextBudgetData {
                    turn_id: source.turn_id.clone(),
                    request_id: source.request_id.clone(),
                    boundary,
                    estimated_input_tokens: estimate.input,
                    baseline_source: estimate.source,
                    baseline_prompt_tokens: estimate.baseline,
                    delta_tokens: estimate.delta,
                    context_window: model.context_window(),
                    output_tokens: model.output_tokens(),
                })
                .into(),
            )
            .map_err(|e| HostError::failed(e.to_string()))?;
        if source.recovery.is_some() || (!checked && estimate.input >= model.threshold_tokens()) {
            let outcome = model
                .compact(
                    CompactionInput {
                        history: history.clone(),
                        checkpoint: checkpoint.or_else(|| {
                            context
                                .checkpoint
                                .as_ref()
                                .and_then(|data| match &data.outcome {
                                    ContextCompactionResult::Completed {
                                        cursor,
                                        summary,
                                        context_baseline,
                                        ..
                                    } => Some(CompactionCheckpoint {
                                        cursor: cursor.clone(),
                                        summary: summary.clone(),
                                        context_baseline: context_baseline.clone().into(),
                                    }),
                                    _ => None,
                                })
                        }),
                        estimated_input_tokens: estimate.input,
                        baseline_source: estimate.source,
                        baseline_prompt_tokens: estimate.baseline,
                        delta_tokens: estimate.delta,
                        provider_budget: source.recovery.clone(),
                    },
                    cancel.clone(),
                )
                .await;
            // A compaction the host completed is a result to keep: it is
            // committed below even if this preparation was cancelled meanwhile.
            let mut outcome = match outcome {
                Ok(value) => value,
                Err(HostError::Cancelled(reason)) => return Err(HostError::Cancelled(reason)),
                Err(error) => ContextCompactionResult::Failed {
                    error: error.to_string(),
                    diagnostics: None,
                },
            };
            let baseline_tokens = model.count_tokens(&prepared);
            let compacted_request =
                |cursor: &str, summary: &str, context_baseline: Arc<[ContextBaselineEntry]>| {
                    let candidate = project(
                        request.clone(),
                        &history,
                        prefix_len,
                        Some(&CompactionCheckpoint {
                            cursor: cursor.to_owned(),
                            summary: summary.to_owned(),
                            context_baseline,
                        }),
                    )?;
                    let tokens = model.count_tokens(&candidate);
                    if !(tokens < baseline_tokens && tokens < model.context_window()) {
                        return Err(HostError::failed(
                            "Compacted context does not reduce input within the model budget.",
                        ));
                    }
                    Ok(candidate)
                };
            if let ContextCompactionResult::Completed {
                cursor,
                summary,
                context_baseline,
                diagnostics,
                checkpoint,
            } = &mut outcome
            {
                match authoritative_baseline(&history, cursor).and_then(|baseline| {
                    validate_cut(&history, cursor)?;
                    compacted_request(cursor, summary, baseline.clone()).map(|_| baseline)
                }) {
                    Ok(baseline) => {
                        *context_baseline = baseline.to_vec();
                        *checkpoint = Some(Box::new(
                            (self.history.checkpoint)(&context, cursor)
                                .map_err(HostError::failed)?,
                        ));
                    }
                    Err(error) => {
                        outcome = ContextCompactionResult::Failed {
                            error: error.to_string(),
                            diagnostics: Some(diagnostics.clone()),
                        }
                    }
                }
            }
            let event = self
                .timeline
                .append(
                    EventKind::ContextCompaction(ContextCompactionData {
                        turn_id: source.turn_id.clone(),
                        request_id: source.request_id.clone(),
                        through: source.through,
                        outcome,
                    })
                    .into(),
                )
                .map_err(|e| HostError::failed(e.to_string()))?;
            check_cancel(&cancel)?;
            let EventKind::ContextCompaction(data) = &event.kind else {
                unreachable!("before preserves event kind")
            };
            match &data.outcome {
                ContextCompactionResult::Completed {
                    cursor,
                    summary,
                    context_baseline,
                    ..
                } => {
                    prepared = compacted_request(cursor, summary, context_baseline.clone().into())?;
                }
                ContextCompactionResult::Failed { error, .. } if source.recovery.is_some() => {
                    return Err(HostError::failed(error.clone()));
                }
                ContextCompactionResult::Skipped { reason, .. } if source.recovery.is_some() => {
                    return Err(HostError::failed(format!(
                        "Context recovery could not compact: {reason}"
                    )));
                }
                _ => {}
            }
        }
        Ok(prepared)
    }

    pub async fn validate(
        &self,
        request: Arc<ModelRequest>,
        cancel: CancellationToken,
    ) -> Result<(), HostError> {
        let model = self
            .resolver
            .resolve(request.clone(), cancel.clone())
            .await?;
        check_cancel(&cancel)?;
        let input = self.estimate(&request, model.as_ref(), &[])?.input;
        if input < model.context_window() {
            return Ok(());
        }
        Err(HostError::Failed {
            kind: crate::host::HostErrorKind::Other,
            message: "Model input exhausts its context window.".into(),
            budget: Some(ProviderOutputBudgetFacts {
                input_tokens: input,
                context_window: model.context_window(),
                requested_max_tokens: model.output_tokens(),
                allowed_max_tokens: (model.context_window() - input).max(0.),
                ratio: 1.,
            }),
            model: None,
            presentation: None,
        })
    }

    fn estimate(
        &self,
        request: &ModelRequest,
        model: &dyn ContextModel,
        events: &[Arc<AgentEvent>],
    ) -> Result<Estimate, HostError> {
        let mut usage: Option<&ModelUsageData> = None;
        for event in events.iter().rev() {
            match &event.kind {
                EventKind::ContextCompaction(data)
                    if usage.is_none()
                        && matches!(data.outcome, ContextCompactionResult::Completed { .. }) =>
                {
                    break;
                }
                EventKind::ModelUsage(data) if usage.is_none() => usage = Some(data),
                EventKind::ModelCall(data)
                    if usage.is_some_and(|u| {
                        u.request_id == data.request_id && u.turn_id == data.turn_id
                    }) =>
                {
                    let value = &usage.expect("matched usage").usage;
                    let previous = &data.request;
                    if value.prompt_tokens > 0.
                        && value.prompt_tokens
                            >= value.cache_read_tokens.unwrap_or(0.)
                                + value.cache_write_tokens.unwrap_or(0.)
                        && previous.provider_id == request.provider_id
                        && previous.model_id == request.model_id
                        && previous.capability_snapshot == request.capability_snapshot
                        && previous.tools == request.tools
                        && previous.system == request.system
                        && previous.config == request.config
                        && previous.metadata == request.metadata
                        && request.messages.starts_with(&previous.messages)
                    {
                        let delta = if previous.messages.len() == request.messages.len() {
                            0.
                        } else {
                            model.count_tokens(&ModelRequest {
                                messages: request.messages[previous.messages.len()..].to_vec(),
                                ..ModelRequest::new(
                                    &request.provider_id,
                                    &request.model_id,
                                    Vec::new(),
                                )
                            })
                        };
                        return Ok(Estimate {
                            input: value.prompt_tokens + delta,
                            source: BaselineSource::ProviderBaseline,
                            baseline: Some(value.prompt_tokens),
                            delta: Some(delta),
                        });
                    }
                    break;
                }
                _ => {}
            }
        }
        Ok(Estimate {
            input: model.count_tokens(request),
            source: BaselineSource::FullEstimate,
            baseline: None,
            delta: None,
        })
    }
}

fn project(
    request: Arc<ModelRequest>,
    history: &[HistoryEntry],
    prefix_len: usize,
    checkpoint: Option<&CompactionCheckpoint>,
) -> Result<Arc<ModelRequest>, HostError> {
    let Some(checkpoint) = checkpoint else {
        return Ok(request);
    };
    let cursor = history
        .iter()
        .position(|entry| entry.id == checkpoint.cursor)
        .ok_or_else(|| HostError::failed("Compaction cursor is outside the request history."))?;
    let mut result = (*request).clone();
    result.messages = std::iter::once(Arc::new(summary_message(&checkpoint.summary)))
        .chain(
            checkpoint
                .context_baseline
                .iter()
                .map(|entry| Arc::new(entry.message.clone())),
        )
        .chain(
            history[cursor + 1..]
                .iter()
                .map(|entry| Arc::new(entry.message.clone())),
        )
        .chain(request.messages[prefix_len..].iter().cloned())
        .collect();
    Ok(Arc::new(result))
}

fn authoritative_baseline(
    history: &[HistoryEntry],
    cursor: &str,
) -> Result<Arc<[ContextBaselineEntry]>, HostError> {
    history
        .iter()
        .find(|entry| entry.id == cursor)
        .map(|entry| entry.baseline.clone())
        .ok_or_else(|| HostError::failed("Compaction cursor is outside the request history."))
}

fn validate_cut(history: &[HistoryEntry], cursor: &str) -> Result<(), HostError> {
    let cut = history
        .iter()
        .position(|entry| entry.id == cursor)
        .ok_or_else(|| HostError::failed("Compaction cursor is outside the request history."))?;
    let removed_tool_uses: std::collections::HashSet<_> = history[..=cut]
        .iter()
        .flat_map(|entry| match &entry.message.content {
            MessageContent::Parts(parts) => parts.iter(),
            MessageContent::Text(_) => [].iter(),
        })
        .filter_map(|part| match part {
            MessagePart::ToolUse { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let retained_result_for_removed_call =
        history[cut + 1..]
            .iter()
            .any(|entry| match &entry.message.content {
                MessageContent::Parts(parts) => parts.iter().any(|part| match part {
                    MessagePart::ToolResult { tool_use_id, .. } => {
                        removed_tool_uses.contains(tool_use_id.as_str())
                    }
                    _ => false,
                }),
                MessageContent::Text(_) => false,
            });
    if retained_result_for_removed_call
        || history.get(cut + 1).is_some_and(|entry| entry.tool_context)
    {
        return Err(HostError::failed(
            "Compaction cursor separates a tool call from its results or context.",
        ));
    }
    Ok(())
}
pub(crate) fn check_cancel(cancel: &CancellationToken) -> Result<(), HostError> {
    if cancel.is_cancelled() {
        Err(HostError::Cancelled("Operation cancelled".into()))
    } else {
        Ok(())
    }
}
