use crate::{
    history::{
        HistoryEntry, HistoryRenderer, context_compaction, context_configuration, context_messages,
        retained_entry,
    },
    host::*,
};
use dim_agent_runtime::{ContextHistory, Timeline};
use dim_agent_types::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

/// Share of the input budget a compaction keeps verbatim, the keyed baseline
/// included; provisional (#128).
const RETAINED_SHARE: f64 = 0.25;

pub(crate) struct ContextPolicy {
    pub timeline: Arc<Timeline>,
    pub resolver: Arc<dyn ResolveContextModel>,
    pub strategy: Arc<dyn CompactionStrategy>,
    pub history: Arc<HistoryRenderer>,
    /// The latest prepared request and its estimate, so `validate` checks
    /// it without reading the context again; derived, a miss reads it.
    prepared: Mutex<Option<(Arc<ModelRequest>, f64)>>,
}
pub(crate) struct Estimate {
    pub input: f64,
    pub source: BaselineSource,
    baseline: Option<f64>,
    delta: Option<f64>,
}

/// The caller of a compaction step and its reason; the ids and `through`
/// are written into both facts.
pub(crate) struct CompactionStep {
    pub turn_id: Option<String>,
    pub request_id: Option<String>,
    pub command: Option<u64>,
    pub through: u64,
    pub trigger: CompactionTrigger,
    pub provider_budget: Option<ProviderOutputBudgetFacts>,
}

/// A request's context at a fixed boundary, projected under the compaction
/// in effect for it.
pub(crate) struct View {
    /// The request as built from the Timeline, with the history prefix.
    request: Arc<ModelRequest>,
    prefix_len: usize,
    context: ContextHistory,
    /// The entries from the cut in effect on.
    history: Vec<HistoryEntry>,
    /// The fixed context and the facts a retry may reuse.
    events: Vec<Arc<AgentEvent>>,
    checkpoint: Option<CompactionCheckpoint>,
    /// `request` projected under `checkpoint`.
    pub prepared: Arc<ModelRequest>,
}

/// The committed result of a step and, when it completed, the request
/// projected on it, or why the committed result cannot be adopted.
pub(crate) struct Compacted {
    pub fact: Arc<AgentEvent>,
    pub request: Option<Result<Arc<ModelRequest>, HostError>>,
}

fn checkpoint(data: &ContextCompactionData) -> Option<CompactionCheckpoint> {
    match &data.outcome {
        ContextCompactionResult::Completed {
            retain_from,
            replacement,
            context_baseline,
            ..
        } => Some(CompactionCheckpoint {
            retain_from: retain_from.clone(),
            replacement: replacement.clone().into(),
            context_baseline: context_baseline.clone().into(),
        }),
        _ => None,
    }
}

/// The request a preparation at this boundary would start from: the latest
/// adopted configuration and system with the projected messages. Tools are
/// declared but not bound; nothing executes them.
pub(crate) fn configured_request(
    context: &ContextHistory,
    history: &[HistoryEntry],
) -> Option<Arc<ModelRequest>> {
    let data = context_configuration(context)?;
    Some(Arc::new(
        ModelSettings {
            provider_id: data.settings.provider_id.clone(),
            model_id: data.settings.model_id.clone(),
            capability_snapshot: data.settings.capability_snapshot.clone(),
            tools: data.settings.tools.clone(),
            system: data.system.clone(),
            config: data.settings.config.clone(),
            metadata: data.settings.metadata.clone(),
        }
        .request(context_messages(context, history)),
    ))
}

impl ContextPolicy {
    /// The policy exists when both hosts are bound; one without the other
    /// is an assembly defect.
    pub fn bind(
        timeline: &Arc<Timeline>,
        model: &ContextModelHost,
        strategy: &CompactionStrategyHost,
        history: &Arc<HistoryRenderer>,
    ) -> Result<Option<Self>, String> {
        match (&model.0, &strategy.0) {
            (Some(resolver), Some(strategy)) => Ok(Some(Self {
                timeline: timeline.clone(),
                resolver: resolver.clone(),
                strategy: strategy.clone(),
                history: history.clone(),
                prepared: Mutex::new(None),
            })),
            (None, None) => Ok(None),
            _ => Err("ContextModelHost and CompactionStrategyHost must be bound together".into()),
        }
    }

    /// Projects `request` under the compaction in effect: the context's own,
    /// or a later one of the same source that a retry reuses (`reusable`).
    pub fn view(
        &self,
        request: Arc<ModelRequest>,
        context: ContextHistory,
        history: Vec<HistoryEntry>,
        reusable: Vec<Arc<AgentEvent>>,
    ) -> Result<View, HostError> {
        let prefix_len = context_messages(&context, &history).len();
        let reused = reusable.iter().rev().find_map(|event| match &event.kind {
            EventKind::ContextCompaction(data) => checkpoint(data).and_then(|checkpoint| {
                retained_entry(&context, &history, &checkpoint.retain_from)
                    .ok()
                    .map(|kept| (checkpoint, kept))
            }),
            _ => None,
        });
        let (history, checkpoint, prepared) = match reused {
            Some((checkpoint, kept)) => {
                let history = history[kept..].to_vec();
                let prepared = project(
                    &request,
                    prefix_len,
                    &checkpoint.replacement,
                    &checkpoint.context_baseline,
                    &history,
                );
                (history, Some(checkpoint), prepared)
            }
            None => (
                history,
                context_compaction(&context).and_then(checkpoint),
                request.clone(),
            ),
        };
        let mut events = context.events.clone();
        events.extend(reusable);
        Ok(View {
            request,
            prefix_len,
            context,
            history,
            events,
            checkpoint,
            prepared,
        })
    }

    pub async fn prepare(
        &self,
        request: Arc<ModelRequest>,
        source: &ContextPrepareData,
        context: ContextHistory,
        history: Vec<HistoryEntry>,
        cancel: CancellationToken,
        cancelled: &(dyn Fn() -> String + Sync),
    ) -> Result<Arc<ModelRequest>, HostError> {
        let (request, input) = self
            .prepare_estimated(request, source, context, history, cancel, cancelled)
            .await?;
        *self.prepared.lock().unwrap() = Some((request.clone(), input));
        Ok(request)
    }

    /// The prepared request and the trigger's estimate of it.
    async fn prepare_estimated(
        &self,
        request: Arc<ModelRequest>,
        source: &ContextPrepareData,
        context: ContextHistory,
        history: Vec<HistoryEntry>,
        cancel: CancellationToken,
        cancelled: &(dyn Fn() -> String + Sync),
    ) -> Result<(Arc<ModelRequest>, f64), HostError> {
        let boundary = history.last().map(|entry| entry.id.clone());
        // A retry may reuse only compactions belonging to the fixed source.
        let reusable = self
            .timeline
            .local(source.through.saturating_add(1)..)
            .await
            .map_err(|e| HostError::failed(e.to_string()))?
            .into_iter()
            .filter(|event| match &event.kind {
                EventKind::ContextCompaction(data) => {
                    data.through == source.through && data.turn_id.as_ref() == Some(&source.turn_id)
                }
                EventKind::ContextBudget(data) => data.turn_id == source.turn_id,
                _ => false,
            })
            .collect();
        let view = self.view(request, context, history, reusable)?;
        let model = self
            .resolver
            .resolve(view.prepared.clone(), cancel.clone())
            .await?;
        check_cancel(&cancel)?;
        let estimate = self.estimate(&view.prepared, model.as_ref(), &view.events)?;
        let checked = view.events.iter().any(|event| {
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
        if source.recovery.is_none() && (checked || estimate.input < model.threshold_tokens()) {
            return Ok((view.prepared, estimate.input));
        }
        let step = CompactionStep {
            turn_id: Some(source.turn_id.clone()),
            request_id: Some(source.request_id.clone()),
            command: None,
            through: source.through,
            trigger: match source.recovery {
                Some(_) => CompactionTrigger::ProviderOutputBudgetExhausted,
                None => CompactionTrigger::Threshold,
            },
            provider_budget: source.recovery.clone(),
        };
        let compacted = self
            .compact_step(
                step,
                &view,
                model.clone(),
                &estimate,
                cancel.clone(),
                cancelled,
            )
            .await?;
        check_cancel(&cancel)?;
        let EventKind::ContextCompaction(data) = &compacted.fact.kind else {
            unreachable!("a compaction step commits a compaction")
        };
        match (&data.outcome, compacted.request) {
            // After a completed compaction the estimate is a full count.
            (_, Some(request)) => request.map(|request| {
                let input = model.count_tokens(&request);
                (request, input)
            }),
            (ContextCompactionResult::Cancelled { reason }, _) => {
                Err(HostError::Cancelled(reason.clone()))
            }
            (ContextCompactionResult::Failed { error, .. }, _) if source.recovery.is_some() => {
                Err(HostError::failed(error.clone()))
            }
            (ContextCompactionResult::Skipped { reason, .. }, _) if source.recovery.is_some() => {
                Err(HostError::failed(format!(
                    "Context recovery could not compact: {reason}"
                )))
            }
            _ => Ok((view.prepared, estimate.input)),
        }
    }

    /// The one compaction path, for the automatic trigger in preparation and
    /// the Loop's manual step: the start, one strategy call, validation of
    /// any strategy's result, the result, then the optional snapshot.
    /// A cancellation before the strategy returns commits `cancelled` with
    /// `cancelled()`'s reason; a result the strategy returned is kept.
    pub async fn compact_step(
        &self,
        step: CompactionStep,
        view: &View,
        model: Arc<dyn ContextModel>,
        estimate: &Estimate,
        cancel: CancellationToken,
        cancelled: &(dyn Fn() -> String + Sync),
    ) -> Result<Compacted, HostError> {
        let started = Instant::now();
        let mut cuts = legal_cuts(&view.context, &view.history);
        let limit = retain_limit(
            view,
            model.as_ref(),
            &cuts,
            step.trigger == CompactionTrigger::Manual,
        );
        if let Some(limit) = limit {
            for cut in &mut cuts[..=limit] {
                cut.within_limit = true;
            }
        }
        let retain_limit = limit.map(|limit| cuts[limit].retain_from.clone());
        let name = self.strategy.name().to_owned();
        let start = self
            .timeline
            .append(
                EventKind::ContextCompactionStart(ContextCompactionStartData {
                    turn_id: step.turn_id.clone(),
                    request_id: step.request_id.clone(),
                    command: step.command,
                    through: step.through,
                    trigger: step.trigger,
                    strategy: name.clone(),
                })
                .into(),
            )
            .map_err(|e| HostError::failed(e.to_string()))?;
        let result = self
            .strategy
            .compact(
                CompactionInput {
                    history: view.history.clone(),
                    checkpoint: view.checkpoint.clone(),
                    request: view.prepared.clone(),
                    model: model.clone(),
                    retain_limit: retain_limit.clone(),
                    cuts: cuts.clone(),
                    trigger: step.trigger,
                    estimated_input_tokens: estimate.input,
                    baseline_source: estimate.source,
                    baseline_prompt_tokens: estimate.baseline,
                    delta_tokens: estimate.delta,
                    provider_budget: step.provider_budget.clone(),
                },
                cancel.clone(),
            )
            .await;
        let record = |retain_from: Option<String>,
                      output: Option<f64>,
                      replaced: usize,
                      diagnostics: StrategyDiagnostics| {
            Box::new(CompactionDiagnosticsRecord {
                reason: match step.trigger {
                    CompactionTrigger::Manual => CompactionReason::Manual,
                    _ => CompactionReason::Auto,
                },
                trigger: step.trigger,
                retain_from,
                retain_limit: retain_limit.clone(),
                estimated_input_tokens: estimate.input,
                estimated_output_tokens: output,
                entries_replaced: replaced as u64,
                model_call_count: diagnostics.model_call_count,
                duration_ms: started.elapsed().as_millis() as u64,
                strategy: diagnostics.record,
            })
        };
        let outcome = match result {
            Ok(StrategyOutcome::Completed {
                retain_from,
                replacement,
                diagnostics,
            }) => {
                let within = cuts.iter().any(|cut| cut.within_limit);
                let checked = cuts
                    .iter()
                    .find(|cut| cut.retain_from == retain_from)
                    .ok_or_else(|| {
                        if view
                            .context
                            .events
                            .iter()
                            .any(|event| event.id == retain_from)
                        {
                            "Compaction cut separates a tool call from its results or context."
                        } else {
                            "Compaction cut is outside the fixed context history."
                        }
                    })
                    .and_then(|cut| {
                        let recovery = step.provider_budget.is_some();
                        if cut.within_limit
                            || (!within && (recovery || cut.entry < view.history.len()))
                        {
                            Ok(cut)
                        } else {
                            Err("Compaction cut lies inside the retained tail.")
                        }
                    })
                    .and_then(|cut| {
                        // The in-effect state just before the cut.
                        let baseline = view.history[cut.entry - 1].baseline.clone();
                        view.compacted(model.as_ref(), cut.entry, &replacement, &baseline)
                            .map(|(_, tokens)| (cut.entry, baseline, tokens))
                    });
                match checked {
                    Ok((replaced, baseline, tokens)) => ContextCompactionResult::Completed {
                        retain_from: retain_from.clone(),
                        replacement,
                        strategy: name,
                        diagnostics: record(Some(retain_from), Some(tokens), replaced, diagnostics),
                        context_baseline: baseline.to_vec(),
                    },
                    Err(error) => ContextCompactionResult::Failed {
                        error: error.into(),
                        diagnostics: Some(record(Some(retain_from), None, 0, diagnostics)),
                    },
                }
            }
            Ok(StrategyOutcome::Skipped {
                reason,
                diagnostics,
            }) => ContextCompactionResult::Skipped {
                reason,
                diagnostics: record(None, None, 0, diagnostics),
            },
            Err(_) if cancel.is_cancelled() => ContextCompactionResult::Cancelled {
                reason: cancelled(),
            },
            Err(error) => ContextCompactionResult::Failed {
                error: error.to_string(),
                diagnostics: None,
            },
        };
        let fact = self
            .timeline
            .append(
                EventKind::ContextCompaction(ContextCompactionData {
                    turn_id: step.turn_id,
                    request_id: step.request_id,
                    command: step.command,
                    start: Some(start.sequence),
                    through: step.through,
                    outcome,
                })
                .into(),
            )
            .map_err(|e| HostError::failed(e.to_string()))?;
        // The committed fact is adopted, as middleware left it.
        let EventKind::ContextCompaction(data) = &fact.kind else {
            unreachable!("before preserves event kind")
        };
        let ContextCompactionResult::Completed {
            retain_from,
            replacement,
            context_baseline,
            ..
        } = &data.outcome
        else {
            return Ok(Compacted {
                fact: fact.clone(),
                request: None,
            });
        };
        let request = retained_entry(&view.context, &view.history, retain_from)
            .map_err(HostError::failed)
            .and_then(|kept| {
                view.compacted(model.as_ref(), kept, replacement, context_baseline)
                    .map(|(request, _)| request)
                    .map_err(HostError::failed)
            });
        if request.is_ok() {
            // Saved independently of the immutable fact; a cache failure
            // neither discards the result nor repeats model work.
            let saved = async {
                let snapshot = self.history.snapshot(&view.context, retain_from, &fact)?;
                self.timeline
                    .save_snapshot(snapshot)
                    .await
                    .map_err(|error| error.to_string())
            }
            .await;
            if saved.is_err() {
                tracing::warn!("projection snapshot unavailable; using fact replay");
            }
        }
        Ok(Compacted {
            fact: fact.clone(),
            request: Some(request),
        })
    }

    /// Fails with budget facts when the trigger's estimate of `request`,
    /// over the branch's committed events, exceeds the input budget.
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
        let prepared = self
            .prepared
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(prepared, _)| Arc::ptr_eq(prepared, &request) || **prepared == *request)
            .map(|(_, input)| *input);
        let input = match prepared {
            Some(input) => input,
            None => {
                let context = self
                    .history
                    .context(&self.timeline, self.timeline.committed())
                    .await
                    .map_err(|e| HostError::failed(e.to_string()))?;
                self.estimate(&request, model.as_ref(), &context.events)?
                    .input
            }
        };
        if input <= input_budget(model.as_ref()) {
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

    pub fn estimate(
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

impl View {
    pub fn events(&self) -> &[Arc<AgentEvent>] {
        &self.events
    }
    /// The request with `history[..kept]` replaced, if it is smaller than
    /// before and below the window.
    fn compacted(
        &self,
        model: &dyn ContextModel,
        kept: usize,
        replacement: &[Message],
        baseline: &[ContextBaselineEntry],
    ) -> Result<(Arc<ModelRequest>, f64), &'static str> {
        let request = project(
            &self.request,
            self.prefix_len,
            replacement,
            baseline,
            &self.history[kept..],
        );
        let tokens = model.count_tokens(&request);
        if tokens < model.count_tokens(&self.prepared) && tokens < model.context_window() {
            Ok((request, tokens))
        } else {
            Err("Compacted context does not reduce input within the model budget.")
        }
    }
}

/// The replay rule applied to a request: the replacement, the keyed
/// baseline, the kept entries, then what preparation appended.
fn project(
    request: &Arc<ModelRequest>,
    prefix_len: usize,
    replacement: &[Message],
    baseline: &[ContextBaselineEntry],
    kept: &[HistoryEntry],
) -> Arc<ModelRequest> {
    let mut result = (**request).clone();
    result.messages = replacement
        .iter()
        .cloned()
        .map(Arc::new)
        .chain(baseline.iter().map(|entry| Arc::new(entry.message.clone())))
        .chain(kept.iter().map(|entry| Arc::new(entry.message.clone())))
        .chain(request.messages[prefix_len..].iter().cloned())
        .collect();
    Arc::new(result)
}

/// The input budget `context_window − output_tokens`.
fn input_budget(model: &dyn ContextModel) -> f64 {
    (model.context_window() - model.output_tokens()).max(0.)
}

/// The index in `cuts` of the earliest cut that keeps something and whose
/// kept tail, the keyed baseline at the cut included, fits `RETAINED_SHARE`
/// of the input budget; for a manual step, the latest turn's start when it
/// keeps less. None when no cut qualifies.
fn retain_limit(
    view: &View,
    model: &dyn ContextModel,
    cuts: &[CompactionCut],
    manual: bool,
) -> Option<usize> {
    let count = |messages: Vec<Message>| model.count_tokens(&ModelRequest::new("", "", messages));
    let empty = count(Vec::new());
    // Kept entries, counted once each and summed from the end.
    let mut tail = vec![0.; view.history.len() + 1];
    for (index, entry) in view.history.iter().enumerate().rev() {
        tail[index] = tail[index + 1] + count(vec![entry.message.clone()]) - empty;
    }
    let budget = RETAINED_SHARE * input_budget(model);
    let fits = cuts.iter().position(|cut| {
        cut.entry < view.history.len()
            && tail[cut.entry] <= budget
            && tail[cut.entry]
                + count(
                    view.history[cut.entry - 1]
                        .baseline
                        .iter()
                        .map(|entry| entry.message.clone())
                        .collect(),
                )
                <= budget
    });
    let latest_turn = manual
        .then(|| {
            let start = view
                .context
                .events
                .iter()
                .rev()
                .find(|event| matches!(event.kind, EventKind::TurnStart(_)))?;
            cuts.iter().position(|cut| cut.retain_from == start.id)
        })
        .flatten();
    fits.max(latest_turn)
}

/// Every event boundary that replaces at least one entry without separating
/// a tool call from its results or context: turn starts, the events that
/// place entries and the boundary's own last event (replacing everything),
/// in order. None is marked within the retention limit yet.
fn legal_cuts(context: &ContextHistory, history: &[HistoryEntry]) -> Vec<CompactionCut> {
    let sources: HashMap<&str, ()> = history
        .iter()
        .map(|entry| (entry.source.as_str(), ()))
        .collect();
    let mut kept = 0;
    let order: HashMap<&str, usize> = context
        .events
        .iter()
        .enumerate()
        .map(|(index, event)| (event.id.as_str(), index))
        .collect();
    let mut cuts = Vec::new();
    for (index, event) in context.events.iter().enumerate() {
        if !matches!(event.kind, EventKind::TurnStart(_))
            && !sources.contains_key(event.id.as_str())
            && index + 1 != context.events.len()
        {
            continue;
        }
        while kept < history.len()
            && order
                .get(history[kept].source.as_str())
                .is_some_and(|source| *source < index)
        {
            kept += 1;
        }
        if kept == 0 || !separates_nothing(history, kept) {
            continue;
        }
        cuts.push(CompactionCut {
            retain_from: event.id.clone(),
            entry: kept,
            within_limit: false,
        });
    }
    cuts
}

/// No kept entry closes a replaced tool call, and the first kept entry is
/// not the context of a replaced result.
fn separates_nothing(history: &[HistoryEntry], kept: usize) -> bool {
    let replaced: std::collections::HashSet<_> = history[..kept]
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
    !history[kept..]
        .iter()
        .any(|entry| match &entry.message.content {
            MessageContent::Parts(parts) => parts.iter().any(|part| {
                matches!(part, MessagePart::ToolResult { tool_use_id, .. }
                if replaced.contains(tool_use_id.as_str()))
            }),
            MessageContent::Text(_) => false,
        })
        && !history.get(kept).is_some_and(|entry| entry.tool_context)
}

pub(crate) fn check_cancel(cancel: &CancellationToken) -> Result<(), HostError> {
    if cancel.is_cancelled() {
        Err(HostError::Cancelled("Operation cancelled".into()))
    } else {
        Ok(())
    }
}
