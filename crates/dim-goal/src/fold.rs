//! Goal's language-neutral fold and decisions (design §2, appendix A.4, B).
//!
//! The fold is the latest live block of each key over the branch's own consumed
//! events, from its opening on; a goal exists only while `goal:objective` is
//! live. The decisions read only that state.

use crate::text::{self, Judge, Marker, OBJECTIVE_KEY, STATUS_KEY};
use dim_agent_types::{
    AgentEvent, BranchSource, EventKind, InputBlock, InputMessageData, InputMode, InterruptScope,
    TurnOutcome,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Active,
    Paused,
    Completed,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusState {
    /// `None` when the block names an operation no status change defines: the
    /// state then holds no status, as the shipped package folded it.
    pub status: Option<Status>,
    pub summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub sequence: u64,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnEnd {
    pub sequence: u64,
    pub status: TurnStatus,
    /// The turn's id, which a progress query names (C7).
    pub turn_id: String,
    /// The `turn/start` sequence the end closes; a progress query starts here.
    pub turn_start: u64,
    /// Whether Goal's own continuation drove the turn that ended here.
    pub continuation_driven: bool,
}

/// Whether the turn a decision follows made a tool call (C7). `None` where the
/// decision needs the answer means it is not known yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    ToolCall,
    NoToolCall,
}

/// Consecutive continuation-driven turns without a tool call that pause a goal
/// (C7).
pub const NO_PROGRESS_TURNS: usize = 3;

/// One continuation input Goal wrote, and the streak it states (C7).
struct Continuation {
    sequence: u64,
    idle: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnStatus {
    Completed,
    Failed,
    Cancelled,
}

impl TurnStatus {
    fn of(outcome: &TurnOutcome) -> Self {
        match outcome {
            TurnOutcome::Completed { .. } => Self::Completed,
            TurnOutcome::Failed { .. } => Self::Failed,
            TurnOutcome::Cancelled { .. } => Self::Cancelled,
        }
    }
}

/// The result a conclusion states, and the evaluator's report of a failed turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConclusionKind {
    Done,
    Continue,
    Blocked,
    Failed,
}

impl ConclusionKind {
    fn of(value: &str) -> Option<Self> {
        match value {
            "done" => Some(Self::Done),
            "continue" => Some(Self::Continue),
            "blocked" => Some(Self::Blocked),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Done => "DONE:",
            Self::Continue => "CONTINUE:",
            Self::Blocked => "BLOCKED:",
            Self::Failed => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conclusion {
    pub evaluation: u64,
    pub evaluator: String,
    pub result: ConclusionKind,
    pub text: String,
}

impl Conclusion {
    /// The result a `done` or `blocked` conclusion states, without its prefix.
    pub fn result_text(&self) -> String {
        let prefix = self.result.prefix().encode_utf16().count();
        let units: Vec<u16> = self.text.encode_utf16().skip(prefix).collect();
        String::from_utf16_lossy(&units).trim().to_owned()
    }
}

/// A sequence as Goal writes one: decimal, without a leading zero.
pub fn sequence_of(value: &str) -> Option<u64> {
    if value.is_empty() || value.starts_with('0') || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

/// The operation id a `branch/create` records for evaluation `N` (§4.1).
pub fn evaluation_operation(evaluation: u64) -> String {
    format!("goal:evaluate:{evaluation}")
}

/// The evaluation a `branch/create` operation id names, if it is one.
pub fn evaluation_of_operation(operation: &str) -> Option<u64> {
    sequence_of(operation.strip_prefix("goal:evaluate:")?)
}

const CONCLUSION_LIMIT: usize = 4000;

/// Text cut to the 4000 characters a conclusion or a whole reply keeps.
pub fn truncate(text: &str) -> String {
    text.chars().take(CONCLUSION_LIMIT).collect()
}

/// The conclusion an evaluator reply ends with: the reply's last line that
/// begins `DONE:`, `CONTINUE:` or `BLOCKED:`, to the end.
pub fn reply_conclusion(reply: &str) -> Option<(ConclusionKind, String)> {
    let lines: Vec<&str> = reply.split('\n').collect();
    for index in (0..lines.len()).rev() {
        for kind in [
            ConclusionKind::Done,
            ConclusionKind::Continue,
            ConclusionKind::Blocked,
        ] {
            if lines[index].starts_with(kind.prefix()) {
                return Some((kind, truncate(&lines[index..].join("\n"))));
            }
        }
    }
    None
}

/// The conclusion a report block carries (§5). A report is a persisted
/// cross-plugin input, so every attribute it needs is checked here.
pub fn conclusion_of(text: &str) -> Option<Conclusion> {
    let attrs = text::marker(text).unwrap_or_default();
    let result = ConclusionKind::of(attrs.get("result").map(String::as_str)?)?;
    let evaluation = sequence_of(attrs.get("evaluation")?)?;
    let evaluator = attrs.get("evaluator").filter(|value| !value.is_empty())?;
    if attrs.get("kind").map(String::as_str) != Some("report") {
        return None;
    }
    Some(Conclusion {
        evaluation,
        evaluator: evaluator.clone(),
        result,
        text: text::body(text, "Evaluator report:"),
    })
}

/// Whether `event` opens a branch's own history: its `branch/start`, or the
/// `session/forked` a fork writes after the facts it copied.
pub fn is_opening(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::BranchStart(_) | EventKind::SessionForked(_)
    )
}

/// The status a live `goal:status` block sets. An operation no status change
/// defines sets a state without one, which no longer counts as active.
fn status_of(attrs: &Marker, text: &str) -> StatusState {
    if attrs.get("kind").map(String::as_str) == Some("status") {
        return StatusState {
            status: Some(Status::Active),
            summary: None,
        };
    }
    let (status, summary) = match attrs.get("op").map(String::as_str) {
        Some("pause") => (Some(Status::Paused), None),
        Some("resume") => (Some(Status::Active), None),
        Some("completed") => (Some(Status::Completed), text::value(text, "Result:")),
        Some("blocked") => (Some(Status::Blocked), text::value(text, "Blocker:")),
        _ => (None, None),
    };
    StatusState { status, summary }
}

/// One branch's goal state: the fold of its consumed local events (§2).
#[derive(Default)]
pub struct Fold {
    pub objective: Option<String>,
    pub status: Option<StatusState>,
    /// Sequence of the latest change that set `active` (`set` or `resume`);
    /// zero when none did, which every sequence exceeds.
    pub active: u64,
    pub last_operator: u64,
    pub turn_start: u64,
    pub turn_end: Option<TurnEnd>,
    /// How many local goal blocks the branch has: any means it has its own
    /// goal history.
    pub local_blocks: usize,
    /// At a `session/forked` opening: whether a goal key the copied facts left
    /// was live.
    pub copied_live: Option<bool>,
    /// The judge of the latest `set`, kept by `edit` (Pursuit §3).
    pub judge: Judge,
    /// Under the evaluator judge, the worker's `completed` blocks: a request,
    /// not a status change (A16).
    pub requests: Vec<Request>,
    /// Evaluations created on this branch: `N` → the evaluator branch.
    pub evaluations: BTreeMap<u64, String>,
    /// The first conclusion input from each evaluation's own evaluator.
    pub conclusions: BTreeMap<u64, u64>,
    /// The `continue` conclusions no input for their evaluation has answered
    /// yet: evaluation → (input sequence, conclusion). A continuation or pause
    /// for the evaluation, a newer turn end or an operator input drops the
    /// entry. Settling a deferred turn end re-derives it here, so a reload
    /// keeps it (Pursuit §5).
    pending_conclusions: BTreeMap<u64, (u64, Conclusion)>,
    /// Sequence of the latest live keyed goal block.
    pub last_block: u64,
    /// The lines this branch opened that ended.
    pub ended: HashSet<String>,
    /// The lines this branch could not create.
    pub failed: HashSet<String>,
    /// Turn ends in this branch's local events.
    pub turn_ends: usize,
    /// Background tasks this branch started and has not ended, by `task_id`.
    open_tasks: HashSet<String>,
    interrupts: Vec<u64>,
    continuations: Vec<Continuation>,
    /// The ids of the continuation inputs, which drive turns (C7).
    continuation_inputs: HashSet<String>,
    /// Whether the latest `turn/start` was driven by Goal's continuation; the
    /// `turn/end` that closes it keeps the answer.
    turn_continuation_driven: bool,
    decided: BTreeSet<String>,
    driving: HashMap<String, u64>,
    events: Vec<AgentEvent>,
    deleted: HashSet<String>,
}

impl Fold {
    /// Applies one consumed local event; `input/deleted` refolds without it.
    pub fn apply(&mut self, event: &AgentEvent) {
        if is_opening(&event.kind) {
            let copied_live = if matches!(event.kind, EventKind::SessionForked(_)) {
                Some(self.objective.is_some() || self.status.is_some())
            } else {
                None
            };
            let events = std::mem::take(&mut self.events);
            *self = Self::default();
            self.copied_live = copied_live;
            drop(events);
        }
        if let EventKind::InputDeleted(data) = &event.kind {
            self.deleted.insert(data.input_id.clone());
            self.events.push(event.clone());
            self.refold();
            return;
        }
        self.events.push(event.clone());
        self.step(event);
    }

    fn refold(&mut self) {
        let events = std::mem::take(&mut self.events);
        let copied_live = self.copied_live;
        *self = Self::default();
        self.copied_live = copied_live;
        self.deleted = events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::InputDeleted(data) => Some(data.input_id.clone()),
                _ => None,
            })
            .collect();
        for event in &events {
            self.step(event);
        }
        self.events = events;
    }

    fn step(&mut self, event: &AgentEvent) {
        let sequence = event.sequence;
        match &event.kind {
            EventKind::InputMessage(data) => self.step_input(event, data, sequence),
            EventKind::BranchCreate(data) => {
                let BranchSource::Host { operation_id } = &data.source else {
                    return;
                };
                let Some(evaluation) = evaluation_of_operation(operation_id) else {
                    return;
                };
                self.evaluations.insert(evaluation, data.branch.clone());
                self.decided.insert(evaluation.to_string());
            }
            EventKind::BranchEnded(data) => {
                self.ended.insert(data.branch.clone());
            }
            EventKind::BranchCreateFailed(data) => {
                let branch = match data {
                    dim_agent_types::BranchCreateFailedData::Failed { branch, .. } => branch,
                    dim_agent_types::BranchCreateFailedData::Cancelled { branch, .. } => branch,
                };
                self.failed.insert(branch.clone());
            }
            EventKind::InputConsumed(data) => {
                self.driving.remove(&data.input_id);
            }
            EventKind::TurnStart(_) => {
                self.turn_start = sequence;
                self.turn_continuation_driven = self
                    .driving
                    .keys()
                    .any(|id| self.continuation_inputs.contains(id));
            }
            EventKind::TurnEnd(data) => {
                self.turn_end = Some(TurnEnd {
                    sequence,
                    status: TurnStatus::of(&data.outcome),
                    turn_id: data.turn_id.clone(),
                    turn_start: self.turn_start,
                    continuation_driven: self.turn_continuation_driven,
                });
                self.turn_ends += 1;
                // A newer turn end supersedes a conclusion still waiting on a
                // background task: the newer end decides from its own state.
                self.pending_conclusions
                    .retain(|evaluation, _| *evaluation >= sequence);
            }
            EventKind::StopInterrupt(data) if data.scope == InterruptScope::Turn => {
                self.interrupts.push(sequence);
            }
            EventKind::ExecStarted(data) => {
                self.open_tasks.insert(data.task_id.clone());
            }
            EventKind::ExecEnded(data) => {
                self.open_tasks.remove(&data.task_id);
            }
            _ => {}
        }
    }

    fn step_input(&mut self, event: &AgentEvent, data: &InputMessageData, sequence: u64) {
        if self.deleted.contains(&event.id) {
            return;
        }
        if data.effective_mode() != InputMode::Append {
            self.driving.insert(event.id.clone(), sequence);
        }
        let first_text = data.blocks.first().map(block_text);
        let first = first_text.and_then(text::marker);
        if data.user.is_some() && !first_text.is_some_and(|text| text.starts_with("<dim-")) {
            self.last_operator = sequence;
            // An operator input supersedes a conclusion still waiting on a
            // background task: the operator's input decides what comes next.
            self.pending_conclusions
                .retain(|evaluation, _| *evaluation > sequence);
        }
        if let Some(after) = first.as_ref().and_then(|attrs| attrs.get("after")) {
            self.decided.insert(after.clone());
            // A continuation or pause for the evaluation answers its
            // conclusion; the deferred one is no longer pending.
            if let Some(evaluation) = sequence_of(after) {
                self.pending_conclusions.remove(&evaluation);
            }
        }
        if first
            .as_ref()
            .and_then(|attrs| attrs.get("kind"))
            .map(String::as_str)
            == Some("continuation")
        {
            self.continuation_inputs.insert(event.id.clone());
            self.continuations.push(Continuation {
                sequence,
                idle: first
                    .as_ref()
                    .and_then(|attrs| attrs.get("idle"))
                    .and_then(|idle| idle.parse().ok())
                    .unwrap_or(0),
            });
        }
        if let Some(conclusion) = first_text.and_then(conclusion_of) {
            let evaluation = conclusion.evaluation;
            if self.evaluations.get(&evaluation) == Some(&conclusion.evaluator)
                && !self.conclusions.contains_key(&evaluation)
            {
                self.conclusions.insert(evaluation, sequence);
                if conclusion.result == ConclusionKind::Continue {
                    self.pending_conclusions
                        .insert(evaluation, (sequence, conclusion));
                }
            }
        }
        for raw in &data.blocks {
            let InputBlock::Raw {
                text: raw_text,
                key,
                retire,
            } = raw;
            let Some(attrs) = text::marker(raw_text) else {
                continue;
            };
            let key = key.as_deref().unwrap_or_default();
            if key != OBJECTIVE_KEY && key != STATUS_KEY {
                continue;
            }
            self.local_blocks += 1;
            self.last_block = sequence;
            if key == OBJECTIVE_KEY {
                if *retire {
                    self.objective = None;
                    self.judge = Judge::Model;
                } else {
                    self.objective = text::value(raw_text, "Objective:");
                    if attrs.get("op").map(String::as_str) == Some("set") {
                        self.judge = Judge::of(&attrs);
                    }
                }
                continue;
            }
            if *retire {
                self.status = None;
                continue;
            }
            let status = status_of(&attrs, raw_text);
            if status.status == Some(Status::Completed)
                && attrs.get("by").map(String::as_str) != Some("evaluator")
                && self.judge == Judge::Evaluator
            {
                // A16: under the evaluator judge the worker's `completed` is a
                // request; only the evaluator completes the goal (§4.1).
                self.requests.push(Request {
                    sequence,
                    summary: status.summary.clone().unwrap_or_default(),
                });
                continue;
            }
            let active_now = status.status == Some(Status::Active)
                && (attrs.get("op").map(String::as_str) == Some("resume")
                    || attrs.get("kind").map(String::as_str) == Some("status"));
            self.status = Some(status);
            if active_now {
                self.active = sequence;
            }
        }
    }

    /// A goal exists and is active (C1).
    pub fn running(&self) -> bool {
        self.objective.is_some()
            && self.status.as_ref().and_then(|status| status.status) == Some(Status::Active)
    }
    pub fn interrupted(&self) -> bool {
        self.interrupts
            .iter()
            .any(|sequence| *sequence > self.active)
    }
    pub fn count(&self) -> usize {
        let since = self.active.max(self.last_operator);
        self.continuations
            .iter()
            .filter(|continuation| continuation.sequence > since)
            .count()
    }
    /// The streak the latest continuation after the count's cutoff states: the
    /// continuation-driven turns without a tool call ending at the turn it
    /// follows (C7). A continuation from before an operator input states none.
    fn idle(&self) -> usize {
        let since = self.active.max(self.last_operator);
        self.continuations
            .iter()
            .rev()
            .find(|continuation| continuation.sequence > since)
            .map_or(0, |continuation| continuation.idle)
    }
    pub fn pending_driving(&self) -> bool {
        self.driving.keys().any(|id| !self.deleted.contains(id))
    }
    /// Whether a background task this branch started is still open (A.4).
    pub fn open_tasks(&self) -> bool {
        !self.open_tasks.is_empty()
    }
    /// The `continue` conclusion still waiting for the branch to settle, with
    /// its input sequence. It is re-derived from the fold, so a reload keeps
    /// it (Pursuit §5).
    pub fn pending_conclusion(&self) -> Option<(u64, &Conclusion)> {
        let (evaluation, (sequence, conclusion)) = self.pending_conclusions.last_key_value()?;
        (self.conclusions.get(evaluation) == Some(sequence) && self.is_current(*evaluation))
            .then_some((*sequence, conclusion))
    }
    pub fn turn_open(&self) -> bool {
        self.turn_start > self.turn_end.as_ref().map_or(0, |end| end.sequence)
    }
    /// The worker's completion request inside the turn that ends at `sequence`.
    pub fn request_in_turn(&self, sequence: u64) -> Option<&Request> {
        self.requests
            .iter()
            .rev()
            .find(|request| request.sequence > self.turn_start && request.sequence < sequence)
    }
    /// The evaluation the branch `evaluator` was created for, if it is one.
    pub fn evaluation_of(&self, evaluator: &str) -> Option<u64> {
        self.evaluations
            .iter()
            .find(|(_, branch)| *branch == evaluator)
            .map(|(evaluation, _)| *evaluation)
    }
    /// Whether evaluation `N` is still current: no goal block follows it.
    pub fn is_current(&self, evaluation: u64) -> bool {
        self.last_block <= evaluation
    }
    pub fn is_decided(&self, sequence: u64) -> bool {
        self.decided.contains(&sequence.to_string())
    }
}

fn block_text(block: &InputBlock) -> &str {
    let InputBlock::Raw { text, .. } = block;
    text
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Continuation { count: usize, idle: usize },
    Pause { reason: &'static str },
}

/// C6 and C7 over a continuation C1–C5 allow (A.4). `progress` is the tool-call
/// answer for the turn the decision follows; it is needed only when a
/// continuation drove that turn.
fn decide_continuation(
    fold: &Fold,
    limit: Option<u64>,
    end: Option<&TurnEnd>,
    progress: Option<Progress>,
) -> Option<Decision> {
    if limit.is_some_and(|limit| fold.count() as u64 >= limit) {
        return Some(Decision::Pause { reason: "limit" });
    }
    let idle = match end {
        Some(end) if !end.continuation_driven => 0,
        Some(_) => match progress? {
            Progress::ToolCall => 0,
            Progress::NoToolCall => fold.idle() + 1,
        },
        None => 0,
    };
    if idle >= NO_PROGRESS_TURNS {
        return Some(Decision::Pause {
            reason: "no_progress",
        });
    }
    Some(Decision::Continuation {
        count: fold.count() + 1,
        idle,
    })
}

/// The decision for the latest `turn/end` (A.4), over the fold as consumed.
/// `limit` is the document's explicit `maxContinuations`; `None` is no limit.
pub fn decide_turn_end(
    fold: &Fold,
    limit: Option<u64>,
    progress: Option<Progress>,
) -> Option<Decision> {
    let end = fold.turn_end.as_ref()?;
    if fold.is_decided(end.sequence) || !fold.running() {
        return None;
    }
    // A turn end with a background task still open decides nothing: its wake
    // drives the next turn, or its cancelled end takes this decision later.
    if fold.open_tasks() {
        return None;
    }
    if fold.interrupted() {
        return Some(Decision::Pause {
            reason: "interrupted",
        });
    }
    if end.status == TurnStatus::Failed {
        return Some(Decision::Pause {
            reason: "turn_failed",
        });
    }
    if end.status != TurnStatus::Completed || fold.turn_open() || fold.pending_driving() {
        return None;
    }
    decide_continuation(fold, limit, Some(end), progress)
}

/// The decision for a `stop/interrupt {scope: turn}` at `sequence`.
pub fn decide_interrupt(fold: &Fold, sequence: u64) -> Option<Decision> {
    if fold.is_decided(sequence) || !fold.running() || fold.active >= sequence {
        return None;
    }
    Some(Decision::Pause {
        reason: "interrupted",
    })
}

/// Whether the turn ending at `sequence` creates an evaluator instead of
/// continuing (Pursuit §4.1).
pub fn decide_evaluation(fold: &Fold, sequence: u64) -> bool {
    let Some(end) = fold.turn_end.as_ref() else {
        return false;
    };
    if end.sequence != sequence || fold.is_decided(sequence) || !fold.running() {
        return false;
    }
    if fold.interrupted() || end.status != TurnStatus::Completed || fold.turn_open() {
        return false;
    }
    fold.judge == Judge::Evaluator && fold.request_in_turn(sequence).is_some()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConclusionAction {
    Completed(String),
    Blocked(String),
    Continue,
    Failed,
}

/// The action a conclusion input at `sequence` takes (Pursuit §4.1).
pub fn decide_conclusion(
    fold: &Fold,
    sequence: u64,
    conclusion: &Conclusion,
) -> Option<ConclusionAction> {
    let evaluation = conclusion.evaluation;
    if fold.conclusions.get(&evaluation).copied() != Some(sequence) {
        return None;
    }
    if !fold.is_current(evaluation) || !fold.running() {
        return None;
    }
    Some(match conclusion.result {
        ConclusionKind::Done => ConclusionAction::Completed(conclusion.result_text()),
        ConclusionKind::Blocked => ConclusionAction::Blocked(conclusion.result_text()),
        ConclusionKind::Continue => ConclusionAction::Continue,
        ConclusionKind::Failed => ConclusionAction::Failed,
    })
}

/// Goal's continuation for a `continue` conclusion: C1, C5, C6 and the
/// background wait. C7 measures the branch's own continuations; the evaluator
/// judged this turn with the whole of it in front of it, so the evaluated
/// path does not read progress and states `idle="0"` (Pursuit §5).
pub fn decide_evaluated_continuation(fold: &Fold, limit: Option<u64>) -> Option<Decision> {
    if !fold.running() || fold.pending_driving() || fold.open_tasks() {
        return None;
    }
    if limit.is_some_and(|limit| fold.count() as u64 >= limit) {
        return Some(Decision::Pause { reason: "limit" });
    }
    Some(Decision::Continuation {
        count: fold.count() + 1,
        idle: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::{Conclusion, conclusion_of};
    use crate::text;
    use dim_agent_types::InputBlock;

    fn report(result: &str, body: &str) -> Conclusion {
        let input = text::conclusion("main", "evaluator", "1", result, body);
        let [InputBlock::Raw { text, .. }] = &input.blocks[..] else {
            panic!("a report is one raw block");
        };
        conclusion_of(text).expect("a report parses")
    }

    #[test]
    fn an_empty_report_body_states_no_result() {
        assert_eq!(report("done", "").result_text(), "");
    }

    #[test]
    fn a_report_without_a_body_line_states_no_result() {
        let text = "<dim-goal kind=\"report\" result=\"done\" evaluation=\"1\" \
                    evaluator=\"evaluator\" branch=\"main\">\nEvaluator report:\n</dim-goal>";
        let conclusion = conclusion_of(text).expect("a report parses");
        assert_eq!(conclusion.result_text(), "");
    }

    #[test]
    fn a_report_body_shorter_than_the_prefix_states_no_result() {
        assert_eq!(report("done", "ok").result_text(), "");
        assert_eq!(report("blocked", "short").result_text(), "");
    }

    #[test]
    fn a_report_body_naming_another_kind_keeps_the_attribute_prefix() {
        assert_eq!(
            report("done", "BLOCKED: gated by policy").result_text(),
            "ED: gated by policy"
        );
    }

    #[test]
    fn a_non_ascii_report_body_slices_utf16_code_units() {
        assert_eq!(report("done", "已完成并验证").result_text(), "证");
        assert_eq!(report("done", "😀abcd").result_text(), "d");
        assert_eq!(report("blocked", "完成").result_text(), "");
    }
}
