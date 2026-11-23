use crate::{host::*, steer::REPLACED_BY_STEER};
use dim_agent_runtime::{
    ModelProgress, RetryProgress, StateCell, Timeline, ToolProgress, core::unix_millis,
};
use dim_agent_types::*;
use futures_util::StreamExt;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Text of a block is committed at the block end, or earlier once this much
/// is buffered (ADR agent-029, implementation record).
const TEXT_FLUSH_BYTES: usize = 4 * 1024;
/// ... or once the oldest buffered text is this old, checked as text arrives.
const TEXT_FLUSH_AGE: Duration = Duration::from_secs(1);
/// Thinking progress is shown at most once per this interval, so a long
/// thought does not send its whole text on every delta; the block end and a
/// switch to text or a tool call show it at once, a timer shows the last
/// delta within one interval, and a retry or a cleared progress drops it.
const THINKING_SHOW_AGE: Duration = Duration::from_millis(250);

/// Progress that an event now holds; it leaves the progress once that event
/// is published, so a client never sees neither (ADR agent-029, decision 2).
enum Handoff {
    /// The first bytes of the progress text.
    Text(usize),
    /// The first bytes of the progress thinking.
    Thinking(usize),
    Tool(String),
}

/// Whether a request received visible model output or was replaced by a
/// steer first; whichever comes first holds (ADR agent-029 decision 5).
#[derive(Default)]
pub(crate) struct OutputGate(AtomicU8);
const WAITING: u8 = 0;
const RECEIVED: u8 = 1;
const REPLACED: u8 = 2;
impl OutputGate {
    /// Replaces the request unless it already received visible output.
    pub fn replace(&self) -> bool {
        matches!(
            self.0
                .compare_exchange(WAITING, REPLACED, Ordering::AcqRel, Ordering::Acquire),
            Ok(_) | Err(REPLACED)
        )
    }
    /// Marks visible output received; false once the request was replaced.
    fn receive(&self) -> bool {
        !matches!(
            self.0
                .compare_exchange(WAITING, RECEIVED, Ordering::AcqRel, Ordering::Acquire),
            Err(REPLACED)
        )
    }
    pub fn replaced(&self) -> bool {
        self.0.load(Ordering::Acquire) == REPLACED
    }
    /// A retried attempt starts over: output the replaced attempt showed no
    /// longer blocks a steer. A request already replaced stays replaced.
    fn restart(&self) {
        let _ = self
            .0
            .compare_exchange(RECEIVED, WAITING, Ordering::AcqRel, Ordering::Acquire);
    }
}

/// A call of this request whose `step/tool-use-start` is committed.
pub(crate) struct StartedCall {
    pub id: String,
    /// Its `step/tool-use` is committed too.
    pub complete: bool,
}

/// The single result batch closing every started call of an output that did
/// not complete, in start order (ADR agent-029 decision 3).
pub(crate) fn closure(
    turn_id: &str,
    request_id: &str,
    started: &[StartedCall],
) -> Option<ToolResultData> {
    (!started.is_empty()).then(|| ToolResultData {
        turn_id: turn_id.to_owned(),
        request_id: request_id.to_owned(),
        call_event_id: None,
        results: started
            .iter()
            .map(|call| ToolCallResult {
                call_id: call.id.clone(),
                result: ToolResult::error(if call.complete {
                    "Tool was not executed: the model output ended"
                } else {
                    "Tool was not executed: the model output ended before its arguments were complete"
                }),
            })
            .collect(),
    })
}

/// What one request committed to the Timeline, and its progress: received
/// output not committed yet.
pub(crate) struct Output {
    timeline: Arc<Timeline>,
    gate: Arc<OutputGate>,
    progress: Arc<StateCell<ModelProgress>>,
    scope: ModelCallData,
    /// Text of the current block not committed yet.
    text: String,
    /// Stable identity of the current text block, across flush chunks.
    text_part_id: Option<String>,
    /// Whether the first text event carrying `text_part_id` was committed.
    text_part_committed: bool,
    /// Stable identity of the current thinking block.
    thinking_part_id: Option<String>,
    /// Whether the first thinking event carrying `thinking_part_id` was committed.
    thinking_part_committed: bool,
    /// Unix milliseconds at which the current thinking block started.
    thinking_started_at: Option<u64>,
    text_since: Option<Instant>,
    /// Thinking of the open block not shown as progress yet.
    thinking_pending: String,
    /// When the pending thinking is shown at the latest.
    thinking_show_at: Option<Instant>,
    /// Whether this attempt committed an event to the Timeline.
    committed: bool,
    pub started: Vec<StartedCall>,
    committed_text: String,
    committed_thinking: String,
    committed_calls: HashMap<String, ToolCall>,
    /// Committed output still shown as progress, by event sequence.
    handoffs: VecDeque<(u64, Handoff)>,
}
impl Output {
    pub fn new(
        timeline: Arc<Timeline>,
        gate: Arc<OutputGate>,
        progress: Arc<StateCell<ModelProgress>>,
        scope: ModelCallData,
    ) -> Self {
        progress.set(ModelProgress {
            turn_id: Some(scope.turn_id.clone()),
            request_id: Some(scope.request_id.clone()),
            ..ModelProgress::default()
        });
        Self {
            timeline,
            gate,
            progress,
            scope,
            text: String::new(),
            text_part_id: None,
            text_part_committed: false,
            thinking_part_id: None,
            thinking_part_committed: false,
            thinking_started_at: None,
            text_since: None,
            thinking_pending: String::new(),
            thinking_show_at: None,
            committed: false,
            started: Vec::new(),
            committed_text: String::new(),
            committed_thinking: String::new(),
            committed_calls: HashMap::new(),
            handoffs: VecDeque::new(),
        }
    }
    /// Changes the progress while it still shows this request; a request
    /// started later owns it otherwise.
    /// Published output leaves the progress first.
    fn show(&mut self, change: impl FnOnce(&mut ModelProgress)) {
        let published = self.timeline.sequence();
        let mut due = Vec::new();
        while self
            .handoffs
            .front()
            .is_some_and(|(sequence, _)| *sequence <= published)
        {
            due.push(self.handoffs.pop_front().expect("checked front").1);
        }
        self.progress.update(|progress| {
            if progress.request_id.as_deref() == Some(self.scope.request_id.as_str()) {
                for handoff in due {
                    match handoff {
                        Handoff::Text(len) => {
                            progress.text_offset += progress.text.drain(..len).count() as u64
                        }
                        Handoff::Thinking(len) => {
                            progress.thinking_offset +=
                                progress.thinking.drain(..len).count() as u64
                        }
                        Handoff::Tool(id) => progress.tools.retain(|tool| tool.call_id != id),
                    }
                }
                change(progress);
            }
        });
    }
    /// Bytes at the start of the progress text (or thinking) that committed
    /// events still hold.
    fn handed_off(&self, thinking: bool) -> usize {
        self.handoffs
            .iter()
            .map(|(_, handoff)| match (handoff, thinking) {
                (Handoff::Text(len), false) | (Handoff::Thinking(len), true) => *len,
                _ => 0,
            })
            .sum()
    }
    /// The progress after its handed-off part becomes `tail`: what a just
    /// committed event holds, so the offsets count exactly the published
    /// deltas (ADR agent-029 decision 2).
    fn replace_tail(&self, thinking: bool, tail: &str) {
        let earlier = self.handed_off(thinking);
        self.progress.update(|progress| {
            if progress.request_id.as_deref() == Some(self.scope.request_id.as_str()) {
                if thinking {
                    progress.text_part_id = None;
                    progress.thinking_part_id = self.thinking_part_id.clone();
                    progress.thinking_started_at = self.thinking_started_at;
                    progress.thinking.truncate(earlier);
                    progress.thinking.push_str(tail);
                } else {
                    progress.thinking_part_id = None;
                    progress.thinking_started_at = None;
                    progress.text_part_id = self.text_part_id.clone();
                    progress.text.truncate(earlier);
                    progress.text.push_str(tail);
                }
            }
        });
    }
    /// Its output stays shown until the events holding it are published.
    fn hand_off(&mut self, sequence: u64, handoff: Handoff) {
        self.handoffs.push_back((sequence, handoff));
        self.show(|_| {});
    }
    /// The request wrote its terminal: once everything it committed is
    /// published, its progress is gone.
    /// Unloading stops the wait: its progress goes with this generation.
    pub async fn release(&mut self, unloading: &CancellationToken) {
        let target = self.timeline.committed();
        let mut published = self.timeline.watch();
        while !published.borrow().1 && published.borrow().0 < target {
            tokio::select! {
                changed = published.changed() => if changed.is_err() { break },
                _ = unloading.cancelled() => break,
            }
        }
        self.clear_progress();
    }
    /// The request ended: its progress is gone.
    pub fn clear_progress(&mut self) {
        self.thinking_pending.clear();
        self.thinking_show_at = None;
        self.progress.update(|progress| {
            if progress.request_id.as_deref() == Some(self.scope.request_id.as_str()) {
                *progress = ModelProgress::default();
            }
        });
    }
    /// Forgets the failed attempt's output. None of it was committed, so the
    /// retry replaces it and the Timeline keeps nothing of it.
    fn forget_attempt(&mut self) {
        self.gate.restart();
        self.committed = false;
        self.text.clear();
        self.text_part_id = None;
        self.text_part_committed = false;
        self.text_since = None;
        self.thinking_part_id = None;
        self.thinking_part_committed = false;
        self.thinking_started_at = None;
        self.thinking_pending.clear();
        self.thinking_show_at = None;
        let (turn_id, request_id) = (&self.scope.turn_id, &self.scope.request_id);
        self.progress.update(|progress| {
            if progress.request_id.as_deref() == Some(request_id.as_str()) {
                *progress = ModelProgress {
                    turn_id: Some(turn_id.clone()),
                    request_id: Some(request_id.clone()),
                    ..ModelProgress::default()
                };
            }
        });
    }
    /// Shows the thinking received since the last time it was shown; this is
    /// where the thought becomes visible output.
    fn show_thinking(&mut self) -> Result<bool, HostError> {
        self.thinking_show_at = None;
        if self.thinking_pending.is_empty() {
            return Ok(false);
        }
        self.receive()?;
        let thinking = std::mem::take(&mut self.thinking_pending);
        let part_id = self.thinking_part_id.clone();
        let started_at = self.thinking_started_at;
        self.show(|progress| {
            progress.text_part_id = None;
            progress.thinking_part_id = part_id;
            progress.thinking_started_at = started_at;
            progress.thinking.push_str(&thinking);
        });
        Ok(true)
    }
    /// Visible output arrived; a request replaced by a steer stops here.
    fn receive(&self) -> Result<(), HostError> {
        if self.gate.receive() {
            Ok(())
        } else {
            Err(HostError::Cancelled(REPLACED_BY_STEER.into()))
        }
    }
    /// Commits one output event; the event is checked against before-commit
    /// rewrites.
    fn commit(
        &mut self,
        kind: EventKind,
        id: Option<String>,
        create_at: Option<u64>,
    ) -> Result<Arc<AgentEvent>, HostError> {
        let mut draft = EventDraft::new(kind);
        if let Some(id) = id {
            draft = draft.with_id(id);
        }
        let event = match create_at {
            Some(create_at) => self.timeline.append_with_create_at(draft, create_at),
            None => self.timeline.append(draft),
        }
        .map_err(|error| HostError::failed(error.to_string()))?;
        self.committed = true;
        let (turn_id, request_id) = match &event.kind {
            EventKind::Text(d) | EventKind::Thinking(d) => (&d.turn_id, &d.request_id),
            EventKind::ToolUseStart(d) => (&d.turn_id, &d.request_id),
            EventKind::ToolUse(d) => (&d.turn_id, &d.request_id),
            _ => return Ok(event),
        };
        if *turn_id != self.scope.turn_id || *request_id != self.scope.request_id {
            return Err(HostError::failed(
                "Committed output changed request identity",
            ));
        }
        Ok(event)
    }
    fn text(&mut self, delta: String) -> Result<(), HostError> {
        if self.text_part_id.is_none() {
            self.text_part_id = Some(uuid::Uuid::new_v4().to_string());
            self.text_part_committed = false;
        }
        let pending = std::mem::take(&mut self.thinking_pending);
        self.thinking_show_at = None;
        self.thinking_part_id = None;
        self.thinking_part_committed = false;
        self.thinking_started_at = None;
        let part_id = self.text_part_id.clone();
        self.show(|progress| {
            if !pending.is_empty() {
                progress.thinking.push_str(&pending);
            }
            progress.text_part_id = part_id;
            progress.thinking_part_id = None;
            progress.thinking_started_at = None;
            progress.text.push_str(&delta);
        });
        self.text.push_str(&delta);
        let since = *self.text_since.get_or_insert_with(Instant::now);
        if self.text.len() >= TEXT_FLUSH_BYTES || since.elapsed() >= TEXT_FLUSH_AGE {
            self.flush_text()?;
        }
        Ok(())
    }
    /// Ends the provider's text block. The block identity stays open because
    /// `TextEnd` is not durable: adjacent text events with no thinking or tool
    /// event between them are one block in history.
    fn end_text(&mut self) -> Result<(), HostError> {
        self.flush_text()
    }
    /// Commits the buffered text as one `step/text`.
    pub fn flush_text(&mut self) -> Result<(), HostError> {
        if self.text.is_empty() {
            return Ok(());
        }
        self.text_since = None;
        let delta = std::mem::take(&mut self.text);
        let event_id = if self.text_part_committed {
            None
        } else {
            self.text_part_committed = true;
            self.text_part_id.clone()
        };
        let event = self.commit(
            EventKind::Text(TextData {
                turn_id: self.scope.turn_id.clone(),
                request_id: self.scope.request_id.clone(),
                delta,
            }),
            event_id,
            None,
        )?;
        let EventKind::Text(data) = &event.kind else {
            return Err(HostError::failed("Committed output changed its kind"));
        };
        self.committed_text.push_str(&data.delta);
        self.replace_tail(false, &data.delta);
        self.hand_off(event.sequence, Handoff::Text(data.delta.len()));
        Ok(())
    }
    /// Opens the current thinking block, or returns the one already open.
    fn open_thinking(&mut self) -> Option<u64> {
        if self.thinking_part_id.is_none() {
            self.thinking_part_id = Some(uuid::Uuid::new_v4().to_string());
            self.thinking_part_committed = false;
            self.thinking_started_at = Some(unix_millis());
        }
        self.thinking_started_at
    }
    fn thinking_delta(&mut self, thinking: String) -> Result<(), HostError> {
        self.flush_text()?;
        self.text_part_id = None;
        self.text_part_committed = false;
        self.open_thinking();
        self.thinking_pending.push_str(&thinking);
        if !self.thinking_pending.is_empty() {
            self.thinking_show_at
                .get_or_insert_with(|| Instant::now() + THINKING_SHOW_AGE);
        }
        Ok(())
    }
    fn thinking(&mut self, thinking: String) -> Result<(), HostError> {
        self.flush_text()?;
        self.show_thinking()?;
        self.text_part_id = None;
        self.text_part_committed = false;
        if thinking.is_empty() {
            self.thinking_part_id = None;
            self.thinking_part_committed = false;
            self.thinking_started_at = None;
            self.replace_tail(true, "");
            return Ok(());
        }
        self.receive()?;
        let started_at = self.open_thinking();
        let first = !self.thinking_part_committed;
        let event_id = if first {
            self.thinking_part_committed = true;
            self.thinking_part_id.clone()
        } else {
            None
        };
        let event = self.commit(
            EventKind::Thinking(ThinkingData {
                turn_id: self.scope.turn_id.clone(),
                request_id: self.scope.request_id.clone(),
                delta: thinking,
            }),
            event_id,
            if first { started_at } else { None },
        )?;
        let EventKind::Thinking(data) = &event.kind else {
            return Err(HostError::failed("Committed output changed its kind"));
        };
        self.committed_thinking.push_str(&data.delta);
        self.replace_tail(true, &data.delta);
        self.hand_off(event.sequence, Handoff::Thinking(data.delta.len()));
        Ok(())
    }
    fn tool_start(&mut self, call_id: String, name: String) -> Result<(), HostError> {
        self.flush_text()?;
        self.show_thinking()?;
        self.text_part_id = None;
        self.text_part_committed = false;
        self.thinking_part_id = None;
        self.thinking_part_committed = false;
        self.thinking_started_at = None;
        self.commit(
            EventKind::ToolUseStart(ToolUseStartData {
                turn_id: self.scope.turn_id.clone(),
                request_id: self.scope.request_id.clone(),
                call_id: call_id.clone(),
                name: name.clone(),
            }),
            None,
            None,
        )?;
        self.started.push(StartedCall {
            id: call_id.clone(),
            complete: false,
        });
        self.show(|progress| {
            progress.text_part_id = None;
            progress.thinking_part_id = None;
            progress.thinking_started_at = None;
            progress.tools.push(ToolProgress {
                call_id,
                name,
                arguments: String::new(),
            })
        });
        Ok(())
    }
    fn tool_use(&mut self, call: ToolCall) -> Result<(), HostError> {
        self.flush_text()?;
        self.show_thinking()?;
        self.text_part_id = None;
        self.text_part_committed = false;
        self.thinking_part_id = None;
        self.thinking_part_committed = false;
        self.thinking_started_at = None;
        let event = self.commit(
            EventKind::ToolUse(ToolUseData {
                turn_id: self.scope.turn_id.clone(),
                request_id: self.scope.request_id.clone(),
                call: call.clone(),
            }),
            None,
            None,
        )?;
        if let EventKind::ToolUse(data) = &event.kind {
            self.committed_calls
                .insert(data.call.id.clone(), data.call.clone());
        }
        if let Some(started) = self
            .started
            .iter_mut()
            .find(|started| started.id == call.id)
        {
            started.complete = true;
        }
        self.hand_off(event.sequence, Handoff::Tool(call.id));
        Ok(())
    }
    fn content(&mut self, content: Vec<MessagePart>) -> Result<(), HostError> {
        self.flush_text()?;
        self.show_thinking()?;
        let text = content
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        let thinking = content
            .iter()
            .filter_map(|part| match part {
                MessagePart::Thinking { thinking, .. } => Some(thinking.as_str()),
                _ => None,
            })
            .collect::<String>();
        let mut seen_calls = std::collections::HashSet::new();
        let calls_match = content
            .iter()
            .filter_map(|part| match part {
                MessagePart::ToolUse {
                    id, name, input, ..
                } => Some((id, name, input)),
                _ => None,
            })
            .all(|(id, name, input)| {
                seen_calls.insert(id)
                    && self
                        .committed_calls
                        .get(id)
                        .is_some_and(|call| &call.name == name && &call.input == input)
            });
        if text != self.committed_text
            || thinking != self.committed_thinking
            || !calls_match
            || seen_calls.len() != self.committed_calls.len()
            || content.iter().any(|part| {
                !matches!(
                    part,
                    MessagePart::Text { .. }
                        | MessagePart::Thinking { .. }
                        | MessagePart::ToolUse { .. }
                )
            })
        {
            return Err(HostError::failed(
                "Complete model content differs from committed output",
            ));
        }
        let data = ModelContentData {
            turn_id: self.scope.turn_id.clone(),
            request_id: self.scope.request_id.clone(),
            content,
        };
        let event = self.commit(EventKind::ModelContent(data.clone()), None, None)?;
        if event.kind != EventKind::ModelContent(data) {
            return Err(HostError::failed(
                "Complete model content was changed before commit",
            ));
        }
        Ok(())
    }
}

impl crate::model_provider::RetryObserver for Output {
    fn waiting(&mut self, retry: Option<crate::model_provider::RetryWaiting<'_>>) {
        self.show(|progress| {
            progress.retry = retry.map(|retry| RetryProgress {
                attempt: retry.attempt,
                max_attempts: retry.max_attempts,
                retry_at_ms: retry.retry_at_ms,
                kind: retry.kind,
                reason: retry.reason.map(str::to_owned),
            });
        });
    }
    fn retrying(&mut self) {
        self.forget_attempt();
    }
}

pub(crate) async fn consume_output(
    stream: ModelStream,
    output: &mut Output,
    cancel: CancellationToken,
    replay_safe: &mut bool,
) -> Result<FinishReason, HostError> {
    let mut stream = stream.fuse();
    let result = parse_output(&mut stream, output, cancel.clone()).await;
    let error = match result {
        Ok(finish) => return Ok(finish),
        Err(error) => error,
    };
    cancel.cancel();
    let flushed = output.flush_text();
    // A retry may follow only an attempt that left no fact behind and whose
    // shown text reached the Timeline. What the attempt showed stays until
    // the recovery policy starts another one: only a retry may clear it and
    // let a steer replace the request again.
    *replay_safe = flushed.is_ok() && !output.committed;
    output.thinking_pending.clear();
    output.thinking_show_at = None;
    while stream.next().await.is_some() {}
    // A failed final flush is a local write failure, not the provider's cut:
    // the text the client already saw must not be replaced by a retry.
    if !matches!(error, HostError::Cancelled(_))
        && let Err(storage) = flushed
    {
        return Err(storage);
    }
    Err(error)
}

async fn parse_output(
    stream: &mut (impl futures_util::Stream<Item = Result<ModelChunk, HostError>> + Unpin),
    output: &mut Output,
    cancel: CancellationToken,
) -> Result<FinishReason, HostError> {
    let mut message_id = None;
    let mut finish = None;
    let mut partial = HashMap::<String, String>::new();
    let mut visible = false;
    let mut content_seen = false;
    let mut stop_seen = false;
    let mut published = output.timeline.watch();
    loop {
        // A cancelled request ends at once; a stop lets the host end its
        // stream, so the calls it started still close.
        if !stop_seen && cancel.is_cancelled() {
            if output.gate.replaced() {
                return Err(HostError::Cancelled("Model invocation cancelled".into()));
            }
            stop_seen = true;
        }
        let show_at = output.thinking_show_at;
        if show_at.is_some_and(|at| Instant::now() >= at) {
            if output.show_thinking()? {
                visible = true;
            }
            continue;
        }
        let until = show_at.unwrap_or_else(Instant::now);
        let next = tokio::select! {
            biased;
            _ = cancel.cancelled(), if !stop_seen => {
                if output.gate.replaced() {
                    return Err(HostError::Cancelled("Model invocation cancelled".into()));
                }
                stop_seen = true;
                continue;
            }
            _ = tokio::time::sleep_until(until), if show_at.is_some() => {
                if output.show_thinking()? {
                    visible = true;
                }
                continue;
            }
            changed = published.changed(), if !output.handoffs.is_empty() => {
                if changed.is_ok() {
                    output.show(|_| {});
                } else {
                    output.handoffs.clear();
                }
                continue;
            }
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = next else { break };
        let chunk = match chunk {
            Err(HostError::Failed {
                message,
                budget,
                kind,
                model,
                presentation,
            }) => {
                return Err(HostError::Failed {
                    message,
                    budget: if visible { None } else { budget },
                    kind,
                    model,
                    presentation,
                });
            }
            other => other?,
        };
        if let ModelChunk::Usage(usage) = chunk {
            output.commit(
                EventKind::ModelUsage(ModelUsageData {
                    turn_id: output.scope.turn_id.clone(),
                    request_id: output.scope.request_id.clone(),
                    usage,
                }),
                None,
                None,
            )?;
            continue;
        }
        if finish.is_some() {
            return Err(HostError::failed(
                "Provider emitted output after message_stop",
            ));
        }
        if let ModelChunk::MessageStart { message_id: id } = chunk {
            if message_id.replace(id).is_some() {
                return Err(HostError::failed(
                    "Provider emitted duplicate message_start",
                ));
            }
            continue;
        }
        if message_id.is_none() {
            return Err(HostError::failed("Provider output preceded message_start"));
        }
        if content_seen && !matches!(chunk, ModelChunk::MessageStop { .. }) {
            return Err(HostError::failed(
                "Provider emitted output after complete content",
            ));
        }
        match chunk {
            ModelChunk::Thinking(delta) => output.thinking_delta(delta)?,
            ModelChunk::ThinkingEnd { thinking } => {
                output.thinking(thinking)?;
                visible = true;
            }
            ModelChunk::Text(delta) => {
                output.receive()?;
                visible = true;
                output.text(delta)?;
            }
            ModelChunk::TextEnd => output.end_text()?,
            ModelChunk::ToolStart { id, name } => {
                output.receive()?;
                visible = true;
                if !output
                    .scope
                    .request
                    .tools
                    .as_deref()
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .any(|tool| tool.name == name)
                {
                    return Err(HostError::failed(format!(
                        "Tool was not offered in this request: {name}"
                    )));
                }
                if partial.contains_key(&id) || output.started.iter().any(|call| call.id == id) {
                    return Err(HostError::failed(
                        "Provider changed or reused a tool call identity",
                    ));
                }
                partial.insert(id.clone(), name.clone());
                output.tool_start(id, name)?;
            }
            ModelChunk::ToolArguments { id, name, delta } => {
                output.receive()?;
                visible = true;
                if partial.get(&id) != Some(&name) {
                    return Err(HostError::failed(
                        "Provider sent arguments for a tool call that is not in progress",
                    ));
                }
                output.show(|progress| {
                    if let Some(tool) = progress.tools.iter_mut().find(|tool| tool.call_id == id) {
                        tool.arguments.push_str(&delta);
                    }
                });
            }
            ModelChunk::ToolUse(call) => {
                output.receive()?;
                visible = true;
                if partial.remove(&call.id).as_ref() != Some(&call.name) {
                    return Err(HostError::failed(
                        "Provider completed a tool call that is not in progress",
                    ));
                }
                output.tool_use(call)?;
            }
            ModelChunk::Content(content) => {
                output.receive()?;
                output.content(content)?;
                content_seen = true;
            }
            ModelChunk::MessageStop {
                message_id: id,
                reason,
            } => {
                if message_id.as_ref() != Some(&id) {
                    return Err(HostError::failed("Provider message_stop identity mismatch"));
                }
                if reason == ModelStopReason::MaxTokens {
                    let request = &output.scope.request;
                    let message = format!(
                        "Model {}/{} stopped at max_tokens",
                        request.provider_id, request.model_id
                    );
                    return Err(HostError::Failed {
                        message: message.clone(),
                        budget: None,
                        kind: HostErrorKind::Other,
                        model: Some(Box::new(ModelFailureFacts {
                            kind: ModelFailureKind::OutputLimit,
                            reason: Some(message.clone()),
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
                        presentation: Some(Box::new(ProviderPresentation {
                            source: request.provider_id.clone(),
                            reason: "max_tokens".into(),
                            server_message: Some(message),
                            request_id: None,
                            action: Some("Increase the output token limit and retry.".into()),
                        })),
                    });
                }
                if !partial.is_empty() {
                    // The same shape as a stream that reports unparseable
                    // arguments (InvalidToolInput): one user-visible defect,
                    // one failure kind.
                    return Err(HostError::Failed {
                        message: "Provider left incomplete tool arguments".into(),
                        budget: None,
                        kind: HostErrorKind::Other,
                        model: Some(Box::new(ModelFailureFacts {
                            kind: ModelFailureKind::MalformedResponse,
                            reason: Some(
                                "the model output ended before its tool arguments were complete"
                                    .into(),
                            ),
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
                    });
                }
                if (reason == ModelStopReason::ToolUse) != !output.started.is_empty() {
                    return Err(HostError::failed(
                        "Provider stop reason does not match tool output",
                    ));
                }
                output.flush_text()?;
                finish = Some(match reason {
                    ModelStopReason::EndTurn => FinishReason::EndTurn,
                    ModelStopReason::ToolUse => FinishReason::ToolUse,
                    ModelStopReason::StopSequence => FinishReason::StopSequence,
                    ModelStopReason::MaxTokens => unreachable!(),
                });
            }
            ModelChunk::MessageStart { .. } | ModelChunk::Usage(_) => unreachable!(),
        }
    }
    if let Some(finish) = finish {
        return Ok(finish);
    }
    if cancel.is_cancelled() {
        return Err(HostError::Cancelled("Model invocation cancelled".into()));
    }
    Err(HostError::failed(
        "Provider stream ended without message_stop",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dim_agent_runtime::{
        MemoryTimelineStore, SessionConfig, Timeline, TimelineKey, TimelineStore,
    };
    use dim_agent_types::{ModelFailureKind, ModelRequest};
    use futures_util::stream;
    use rutis::Ctx;
    use rutis::host::CtxHost;
    use std::time::Duration;

    fn scope() -> ModelCallData {
        ModelCallData {
            turn_id: "turn".into(),
            request_id: "request".into(),
            request: Arc::new(ModelRequest::new("provider", "model", Vec::new())),
            configuration: None,
            prompt: None,
            failure_chain: None,
        }
    }

    async fn timeline(branch: &str) -> Arc<Timeline> {
        let store = Arc::new(MemoryTimelineStore::default());
        store
            .create_session("session", &SessionConfig::default())
            .await
            .unwrap();
        Arc::new(Timeline::new(
            Ctx::root().unwrap(),
            TimelineKey {
                session_id: "session".into(),
                branch_id: branch.into(),
            },
            Vec::new(),
            store,
        ))
    }

    fn cut() -> HostError {
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

    /// A text delta smaller than the flush threshold and younger than its age
    /// leaves the final flush to the cut. Its append fails on a branch the
    /// store does not hold.
    #[tokio::test]
    async fn a_failed_final_flush_is_not_replayable() {
        let gate = Arc::new(OutputGate::default());
        let progress = Arc::new(StateCell::new(ModelProgress::default()));
        let mut output = Output::new(timeline("missing").await, gate, progress, scope());
        let chunks: Vec<std::result::Result<ModelChunk, HostError>> = vec![
            Ok(ModelChunk::MessageStart {
                message_id: "m".into(),
            }),
            Ok(ModelChunk::Text("shown".into())),
            Err(cut()),
        ];
        let stream: ModelStream = Box::pin(stream::iter(chunks));
        let mut replay_safe = true;
        let result = consume_output(
            stream,
            &mut output,
            CancellationToken::new(),
            &mut replay_safe,
        )
        .await;
        assert!(
            !replay_safe,
            "a failed final flush is a local failure: the attempt may not be retried"
        );
        let Err(HostError::Failed { message, model, .. }) = result else {
            panic!("a cut after a failed flush is a failure")
        };
        assert!(
            model.is_none(),
            "the local write error is returned instead of the provider's cut: {message}"
        );
    }

    /// A thinking delta stays private until the buffer is shown; until then a
    /// steer still replaces the request.
    #[tokio::test(start_paused = true)]
    async fn a_private_thinking_delta_leaves_the_request_replaceable() {
        let gate = Arc::new(OutputGate::default());
        let progress = Arc::new(StateCell::new(ModelProgress::default()));
        let mut output = Output::new(
            timeline("main").await,
            gate.clone(),
            progress.clone(),
            scope(),
        );
        let replaceable = gate.clone();
        let read = progress.clone();
        let stream: ModelStream = Box::pin(stream::unfold(0u8, move |step| {
            let replaceable = replaceable.clone();
            let read = read.clone();
            async move {
                match step {
                    0 => Some((
                        Ok(ModelChunk::MessageStart {
                            message_id: "m".into(),
                        }),
                        1,
                    )),
                    1 => Some((Ok(ModelChunk::Thinking("private".into())), 2)),
                    2 => {
                        assert!(
                            read.get().thinking.is_empty(),
                            "nothing was shown before the interval elapsed"
                        );
                        assert!(
                            replaceable.replace(),
                            "a request whose thinking was never shown is still replaceable"
                        );
                        Some((
                            Ok(ModelChunk::ThinkingEnd {
                                thinking: "private".into(),
                            }),
                            3,
                        ))
                    }
                    3 => Some((
                        Ok(ModelChunk::MessageStop {
                            message_id: "m".into(),
                            reason: ModelStopReason::EndTurn,
                        }),
                        4,
                    )),
                    _ => None,
                }
            }
        }));
        let mut replay_safe = true;
        let result = consume_output(
            stream,
            &mut output,
            CancellationToken::new(),
            &mut replay_safe,
        )
        .await;
        assert!(
            matches!(&result, Err(HostError::Cancelled(reason)) if reason == REPLACED_BY_STEER),
            "the replacement ends the attempt before its thinking is shown: {result:?}"
        );
    }

    /// A cancelled request ends before an item that is ready at the same
    /// time; with both branches ready the choice must not be left to chance.
    #[tokio::test(start_paused = true)]
    async fn a_cancelled_request_ends_before_a_ready_item() {
        for attempt in 0..32 {
            let gate = Arc::new(OutputGate::default());
            let progress = Arc::new(StateCell::new(ModelProgress::default()));
            let mut output = Output::new(timeline("main").await, gate.clone(), progress, scope());
            gate.replace();
            let cancel = CancellationToken::new();
            cancel.cancel();
            let chunks: Vec<std::result::Result<ModelChunk, HostError>> = vec![Err(cut())];
            let stream: ModelStream = Box::pin(stream::iter(chunks));
            let mut replay_safe = true;
            let result = consume_output(stream, &mut output, cancel, &mut replay_safe).await;
            assert!(
                matches!(result, Err(HostError::Cancelled(_))),
                "attempt {attempt}: cancellation wins over a ready item: {result:?}"
            );
        }
    }

    /// Thinking that keeps arriving still shows within the interval: the
    /// deadline is not starved by the next chunk.
    #[tokio::test(start_paused = true)]
    async fn a_continuous_thinking_stream_shows_within_the_interval() {
        let gate = Arc::new(OutputGate::default());
        let progress = Arc::new(StateCell::new(ModelProgress::default()));
        let mut output = Output::new(timeline("main").await, gate, progress.clone(), scope());
        let read = progress.clone();
        let stream: ModelStream = Box::pin(stream::unfold(0u8, move |step| {
            let read = read.clone();
            async move {
                match step {
                    0 => Some((
                        Ok(ModelChunk::MessageStart {
                            message_id: "m".into(),
                        }),
                        1,
                    )),
                    1..=3 => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        Some((Ok(ModelChunk::Thinking("abc".into())), step + 1))
                    }
                    4 => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        assert_eq!(
                            read.get().thinking,
                            "abcabcabc",
                            "the interval shows the buffered thinking while deltas keep arriving"
                        );
                        Some((Ok(ModelChunk::Thinking("d".into())), 5))
                    }
                    5 => Some((
                        Ok(ModelChunk::ThinkingEnd {
                            thinking: "abcabcd".into(),
                        }),
                        6,
                    )),
                    6 => Some((
                        Ok(ModelChunk::MessageStop {
                            message_id: "m".into(),
                            reason: ModelStopReason::EndTurn,
                        }),
                        7,
                    )),
                    _ => None,
                }
            }
        }));
        let mut replay_safe = true;
        let result = consume_output(
            stream,
            &mut output,
            CancellationToken::new(),
            &mut replay_safe,
        )
        .await;
        assert!(result.is_ok(), "the attempt completes: {result:?}");
        assert_eq!(output.committed_thinking, "abcabcd");
    }

    /// An expired thinking interval wins over an item that is ready at the
    /// same time: the buffered thought is shown before the next delta joins
    /// it. With both branches ready the choice must not be left to chance.
    #[tokio::test(start_paused = true)]
    async fn an_expired_thinking_deadline_shows_before_a_ready_item() {
        for attempt in 0..32 {
            let gate = Arc::new(OutputGate::default());
            let progress = Arc::new(StateCell::new(ModelProgress::default()));
            let mut output = Output::new(timeline("main").await, gate, progress.clone(), scope());
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let stream: ModelStream = Box::pin(stream::unfold(rx, |mut rx| async move {
                rx.recv().await.map(|chunk| (chunk, rx))
            }));
            tx.send(Ok(ModelChunk::MessageStart {
                message_id: "m".into(),
            }))
            .unwrap();
            tx.send(Ok(ModelChunk::Thinking("first".into()))).unwrap();
            let mut replay_safe = true;
            let run = consume_output(
                stream,
                &mut output,
                CancellationToken::new(),
                &mut replay_safe,
            );
            tokio::pin!(run);
            let first =
                std::future::poll_fn(|cx| std::task::Poll::Ready(run.as_mut().poll(cx))).await;
            assert!(
                first.is_pending(),
                "attempt {attempt}: the read loop waits for the interval"
            );
            tokio::time::advance(THINKING_SHOW_AGE + Duration::from_millis(1)).await;
            tx.send(Ok(ModelChunk::Thinking("second".into()))).unwrap();
            let second =
                std::future::poll_fn(|cx| std::task::Poll::Ready(run.as_mut().poll(cx))).await;
            assert!(
                second.is_pending(),
                "attempt {attempt}: the read loop waits for the next item"
            );
            assert_eq!(
                progress.get().thinking,
                "first",
                "attempt {attempt}: the expired interval shows its thought before a ready item joins it"
            );
            drop(tx);
            let _ = run.await;
        }
    }
}
