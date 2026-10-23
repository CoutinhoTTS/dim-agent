//! Folds App Server session events into the transcript the screen draws.
//!
//! The worker applies every event once, in sequence order, and publishes
//! [`ViewSnapshot`]s built here. Only the rows the screen needs are kept; the
//! durable history stays with the App Server.

use std::collections::HashMap;
use std::sync::Arc;

use dim_agent_types::{
    CollaborationDelivery, CollaborationOutcome, ContentBlock, ExecStatus, InputMode, JsonObject,
    ModelOutputOutcome, ToolContent, ToolResult, TurnOutcome, UserInput,
};
use serde_json::Value;

use crate::model::shell::ShellRun;
use crate::model::view::{
    ApprovalView, Question, QuestionKind, QuestionOption, QuestionRequestView, QueuedInput,
    RowKind, RowTone, TaskKind, TaskTarget, TaskView, ViewRow, ViewSnapshot,
};
use crate::question::{self, Call, Pending};
use crate::wire::{Event, EventContent, Progress};

/// Reasoning that is still arriving: the row shows what the Timeline holds
/// plus the progress not written yet.
struct OpenThought {
    index: usize,
    /// The request's `step/thinking` deltas so far.
    written: String,
}

#[derive(Default)]
pub struct Projection {
    /// Shared with the published snapshots: a snapshot copies pointers, and
    /// a row is copied only when it changes while a snapshot still holds it
    /// (in practice the one that is streaming).
    rows: Vec<Arc<ViewRow>>,
    /// The assistant row each model request streams into.
    assistant_rows: HashMap<String, usize>,
    /// When each running model request started, to time its reasoning.
    request_starts: HashMap<String, u64>,
    /// The reasoning row each model request streams into while it is open.
    thinking_rows: HashMap<String, OpenThought>,
    /// The tool row each call id opened.
    tool_rows: HashMap<String, usize>,
    running: bool,
    approvals: Vec<ApprovalView>,
    /// Messages InputQueue still holds: submitted while a turn ran and not
    /// applied yet. Shown in the queue strip; each enters the transcript
    /// when its `input/applied` comes, so the transcript keeps the order
    /// the model sees. Withdrawal re-submits the kept `UserInput` as Steer.
    held: Vec<(String, UserInput)>,
    /// Background commands and delegated tasks still running.
    tasks: Vec<TaskView>,
    /// The `exec` call each background task came from, until it ends.
    background: HashMap<TaskTarget, String>,
    /// `ask_user` calls of any branch that wait for an answer.
    questions: Pending,
}

/// Names one waiting `ask_user` call in the snapshot: a batch may ask twice.
fn question_key(call: &Call) -> String {
    serde_json::to_string(&(&call.branch_id, &call.request_id, &call.call_id))
        .expect("question identity contains only strings")
}

fn view_question((index, asked): (usize, &question::Question)) -> Question {
    Question {
        id: asked.id.clone(),
        header: asked
            .header
            .clone()
            .unwrap_or_else(|| format!("Q{}", index + 1)),
        question: asked.question.clone(),
        kind: if asked.multi_select {
            QuestionKind::MultiSelect
        } else {
            QuestionKind::SingleSelect
        },
        options: asked
            .options
            .iter()
            .map(|choice| QuestionOption {
                label: choice.label.clone(),
                description: choice.description.clone(),
            })
            .collect(),
        allow_custom: asked.allow_custom,
    }
}

impl Projection {
    pub fn apply(&mut self, branch_id: &str, event: &Event) {
        // Any branch may ask; the prompt answers it on its own branch.
        match &event.content {
            EventContent::ToolCall(data) => self.questions.asked(branch_id, &event.id, data),
            EventContent::ToolResult(data) => {
                self.questions.answered(branch_id, data);
            }
            _ => {}
        }
        // The transcript is main; the Session's task list also includes other branches.
        if branch_id != "main"
            && !matches!(
                &event.content,
                EventContent::ExecStarted(_)
                    | EventContent::ExecEnded(_)
                    | EventContent::CollaborationRequest(_)
                    | EventContent::CollaborationDelivered(_)
                    | EventContent::CollaborationResult(_)
            )
        {
            return;
        }
        let sequence = event.sequence;
        match &event.content {
            // Inputs without `user` carry context blocks, not a message; of
            // those only a shell run the App Server recorded is shown.
            EventContent::Input(data) => {
                if let Some(user) = &data.user {
                    // While a turn runs InputQueue holds a queued message
                    // until the turn ends, and a steer until the next tool
                    // result. An append (a notice the server adds for the
                    // model) is applied at once and is nobody's queue.
                    let held = self.running && data.mode != Some(InputMode::Append);
                    if held {
                        self.held.push((event.id.clone(), user.clone()));
                    } else {
                        self.push_user(sequence, user);
                    }
                } else if let Some(text) = data.blocks.iter().find_map(|block| {
                    let dim_agent_types::InputBlock::Raw { text, .. } = block;
                    ShellRun::parse(text).is_some().then(|| text.clone())
                }) {
                    self.push(format!("user-{sequence}"), RowKind::Shell, text);
                }
            }
            EventContent::InputApplied(data) => {
                if let Some(index) = self.held.iter().position(|(id, _)| *id == data.input_id) {
                    let (_, user) = self.held.remove(index);
                    self.push_user(sequence, &user);
                }
            }
            EventContent::InputDeleted(data) => {
                self.held.retain(|(id, _)| *id != data.input_id);
            }
            EventContent::ExecStarted(data) => {
                let label = self
                    .rows
                    .iter()
                    .find(|row| row.row_id == data.call_id)
                    .and_then(|row| row.tool_input.as_ref()?.get("command")?.as_str())
                    .filter(|command| !command.is_empty())
                    .unwrap_or(data.description.as_str())
                    .to_owned();
                let target = TaskTarget {
                    branch_id: branch_id.to_owned(),
                    task_id: data.task_id.clone(),
                };
                self.tasks.push(TaskView {
                    id: event.id.clone(),
                    kind: TaskKind::Command(target.clone()),
                    label,
                });
                self.background.insert(target, data.call_id.clone());
            }
            EventContent::ExecEnded(data) => {
                let target = TaskTarget {
                    branch_id: branch_id.to_owned(),
                    task_id: data.task_id.clone(),
                };
                self.tasks
                    .retain(|task| !matches!(&task.kind, TaskKind::Command(at) if at == &target));
                // The call's own result only said the command detached; its
                // end is what the command block shows.
                if let Some(call_id) = self.background.remove(&target)
                    && let Some(index) = self.rows.iter().position(|row| row.row_id == call_id)
                {
                    let row = self.row(index);
                    row.result_is_error =
                        data.status != ExecStatus::Completed || data.result.is_error == Some(true);
                    row.result_text = Some(result_text(&data.result));
                    row.exit_code = data
                        .result
                        .structured_content
                        .as_ref()
                        .and_then(|value| value.get("exitCode"))
                        .and_then(Value::as_i64);
                }
            }
            EventContent::CollaborationRequest(data) => {
                self.tasks.push(TaskView {
                    id: event.id.clone(),
                    kind: TaskKind::Member { session_id: None },
                    label: data.prompt.lines().next().unwrap_or("").to_owned(),
                });
            }
            EventContent::CollaborationDelivered(data) => match &data.delivery {
                CollaborationDelivery::Ok { session_id, .. } => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| {
                        task.id == data.op_event_id && matches!(task.kind, TaskKind::Member { .. })
                    }) {
                        task.kind = TaskKind::Member {
                            session_id: Some(session_id.clone()),
                        };
                    }
                }
                CollaborationDelivery::Error { .. } => self.tasks.retain(|task| {
                    task.id != data.op_event_id || !matches!(task.kind, TaskKind::Member { .. })
                }),
            },
            EventContent::CollaborationResult(data) => {
                self.tasks.retain(|task| task.id != data.request_event_id);
                let (text, tone) = match &data.outcome {
                    CollaborationOutcome::Completed => {
                        (format!("agent result: {}", data.text), None)
                    }
                    CollaborationOutcome::Failed { reason } => {
                        (format!("agent failed: {reason}"), Some(RowTone::Danger))
                    }
                    CollaborationOutcome::Cancelled { reason } => {
                        (format!("agent cancelled: {reason}"), None)
                    }
                };
                let index = self.push(format!("agent-{sequence}"), RowKind::Status, text);
                self.row(index).tone = tone;
            }
            EventContent::ModelStart(data) => {
                self.request_starts
                    .insert(data.request_id.clone(), event.create_at);
            }
            EventContent::Thinking(data) => {
                let index = self.open_thinking(&data.request_id);
                let elapsed = self
                    .request_starts
                    .get(&data.request_id)
                    .map(|start| event.create_at.saturating_sub(*start));
                let thought = self
                    .thinking_rows
                    .get_mut(&data.request_id)
                    .expect("opened above");
                thought.written.push_str(&data.delta);
                let text = thought.written.clone();
                let row = self.row(index);
                row.text = text;
                // A thought is written whole when it ends.
                row.elapsed_ms = elapsed;
            }
            EventContent::Text(data) => {
                self.close_thinking(&data.request_id);
                let index = match self.assistant_rows.get(&data.request_id) {
                    Some(index) => *index,
                    None => {
                        let index = self.push(
                            format!("assistant-{}", data.request_id),
                            RowKind::StreamingAssistant,
                            String::new(),
                        );
                        self.row(index).streaming = true;
                        self.assistant_rows.insert(data.request_id.clone(), index);
                        index
                    }
                };
                self.row(index).text.push_str(&data.delta);
            }
            EventContent::ToolUse(data) => {
                self.close_thinking(&data.request_id);
                self.close_assistant(&data.request_id);
                self.request_starts.remove(&data.request_id);
                let index = self.push(data.call.id.clone(), RowKind::ToolCall, String::new());
                let row = self.row(index);
                row.tool_name = Some(data.call.name.clone());
                row.tool_input = Some(Value::Object(data.call.input.clone()));
                row.batch = Some(data.request_id.clone());
                row.pending = true;
                self.tool_rows.insert(data.call.id.clone(), index);
            }
            EventContent::ToolCompleted(data) => {
                self.complete_tool(&data.call_id, &data.result);
            }
            EventContent::ToolResult(data) => {
                for result in &data.results {
                    self.complete_tool(&result.call_id, &result.result);
                }
            }
            EventContent::ModelEnd(data) => {
                self.close_thinking(&data.request_id);
                self.close_assistant(&data.request_id);
                self.request_starts.remove(&data.request_id);
                if let ModelOutputOutcome::Failed { error, .. } = &data.outcome {
                    self.push_error(format!("model failed: {error}"));
                }
            }
            EventContent::TurnEnd(data) => {
                // A turn that ends mid-thought settles what it had.
                let open: Vec<String> = self.thinking_rows.keys().cloned().collect();
                for request_id in open {
                    self.close_thinking(&request_id);
                }
                self.request_starts.clear();
                self.turn_end(sequence, &data.outcome);
            }
            // Calls show up as `tool-use` rows; the questions read them above.
            EventContent::Configuration(_)
            | EventContent::PermissionChanged(_)
            | EventContent::ToolCall(_)
            | EventContent::Other => {}
        }
    }

    fn turn_end(&mut self, sequence: u64, outcome: &TurnOutcome) {
        match outcome {
            TurnOutcome::Failed { error } => self.push_error(format!("turn failed: {error}")),
            TurnOutcome::Cancelled { .. } => {
                self.push(
                    format!("stopped-{sequence}"),
                    RowKind::Status,
                    "stopped".into(),
                );
            }
            TurnOutcome::Completed { .. } => {}
        }
    }

    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    /// A user message as a transcript row: a shell run's record draws as one.
    fn push_user(&mut self, sequence: u64, user: &UserInput) {
        let text = text_of(&user.content);
        let kind = if ShellRun::parse(&text).is_some() {
            RowKind::Shell
        } else {
            RowKind::UserMessage
        };
        self.push(format!("user-{sequence}"), kind, text);
    }

    /// The content of a held message, to re-submit it as Steer.
    pub fn held_input(&self, input_id: &str) -> Option<&UserInput> {
        self.held
            .iter()
            .find(|(id, _)| id == input_id)
            .map(|(_, user)| user)
    }

    pub fn add_approval(
        &mut self,
        branch_id: String,
        request_id: String,
        name: String,
        input: JsonObject,
        reason: String,
        always: Option<String>,
    ) {
        if let Some(index) = self.tool_rows.get(&request_id).copied() {
            self.row(index).parked = true;
        }
        self.approvals.push(ApprovalView {
            request_id,
            branch_id,
            reason,
            tool_name: name,
            tool_input: Value::Object(input),
            always,
        });
    }

    /// The command an "always" answer to `request_id` saves a rule for.
    pub fn approval_always(&self, request_id: &str) -> Option<String> {
        self.approvals
            .iter()
            .find(|approval| approval.request_id == request_id)?
            .always
            .clone()
    }

    /// The waiting `ask_user` call a snapshot question request names.
    pub fn question(&self, branch_id: &str, key: &str) -> Option<&Call> {
        self.questions
            .calls()
            .find(|call| call.branch_id == branch_id && question_key(call) == key)
    }

    pub fn remove_approval(&mut self, request_id: &str) {
        self.approvals
            .retain(|approval| approval.request_id != request_id);
    }

    pub fn snapshot(&self) -> ViewSnapshot {
        ViewSnapshot {
            rows: self.rows.clone(),
            running: self.running,
            approvals: self.approvals.clone(),
            questions: self
                .questions
                .calls()
                .map(|call| QuestionRequestView {
                    request_id: question_key(call),
                    branch_id: call.branch_id.clone(),
                    questions: call
                        .questions
                        .iter()
                        .enumerate()
                        .map(view_question)
                        .collect(),
                })
                .collect(),
            queued: self
                .held
                .iter()
                .map(|(id, user)| QueuedInput {
                    id: id.clone(),
                    text: text_of(&user.content),
                })
                .collect(),
            tasks: self.tasks.clone(),
        }
    }

    fn row(&mut self, index: usize) -> &mut ViewRow {
        Arc::make_mut(&mut self.rows[index])
    }

    fn push(&mut self, row_id: String, kind: RowKind, text: String) -> usize {
        self.rows.push(Arc::new(ViewRow {
            row_id,
            kind,
            text,
            streaming: false,
            elapsed_ms: None,
            tone: None,
            tool_name: None,
            tool_input: None,
            batch: None,
            pending: false,
            parked: false,
            result_text: None,
            result_is_error: false,
            diff: None,
            exit_code: None,
        }));
        self.rows.len() - 1
    }

    fn push_error(&mut self, text: String) {
        let index = self.push(format!("error-{}", self.rows.len()), RowKind::Error, text);
        self.row(index).tone = Some(RowTone::Danger);
    }

    /// Shows a model request's reasoning that is not written yet. Progress
    /// that the written deltas overtook, or that runs ahead of them, waits
    /// for the next notice.
    pub fn progress(&mut self, progress: &Progress) {
        let Some(request_id) = &progress.request_id else {
            return;
        };
        if progress.thinking.is_empty() {
            return;
        }
        let index = self.open_thinking(request_id);
        let thought = &self.thinking_rows[request_id];
        if thought.written.chars().count() as u64 != progress.thinking_offset {
            return;
        }
        let text = format!("{}{}", thought.written, progress.thinking);
        self.row(index).text = text;
    }

    /// The open reasoning row of a request, pushed on its first output.
    fn open_thinking(&mut self, request_id: &str) -> usize {
        if let Some(thought) = self.thinking_rows.get(request_id) {
            return thought.index;
        }
        let index = self.push(
            format!("thinking-{request_id}-{}", self.rows.len()),
            RowKind::Thinking,
            String::new(),
        );
        self.row(index).streaming = true;
        self.thinking_rows.insert(
            request_id.to_owned(),
            OpenThought {
                index,
                written: String::new(),
            },
        );
        index
    }

    /// A thought settles when the request moves on, as the Timeline holds
    /// it: progress that was never written (an interrupted thought) goes.
    fn close_thinking(&mut self, request_id: &str) {
        if let Some(thought) = self.thinking_rows.remove(request_id) {
            let row = self.row(thought.index);
            row.streaming = false;
            row.text = thought.written;
        }
    }

    fn close_assistant(&mut self, request_id: &str) {
        if let Some(index) = self.assistant_rows.remove(request_id) {
            self.row(index).streaming = false;
            self.row(index).kind = RowKind::AssistantMessage;
        }
    }

    fn complete_tool(&mut self, call_id: &str, result: &ToolResult) {
        let Some(index) = self.tool_rows.remove(call_id) else {
            return;
        };
        let row = self.row(index);
        row.pending = false;
        row.parked = false;
        row.result_is_error = result.is_error == Some(true);
        row.result_text = Some(result_text(result));
        let structured = |key: &str| result.structured_content.as_ref()?.get(key);
        // The edit tool reports the display diff of what it changed.
        row.diff = structured("diff")
            .and_then(Value::as_str)
            .map(str::to_owned);
        row.exit_code = structured("exitCode").and_then(Value::as_i64);
    }
}

pub(crate) fn result_text(result: &ToolResult) -> String {
    match &result.content {
        ToolContent::Text(text) => text.clone(),
        ToolContent::Blocks(blocks) => text_of(blocks),
    }
}

fn text_of(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            ContentBlock::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(projection: &mut Projection, events: &[serde_json::Value]) {
        for event in events {
            let event: Event = serde_json::from_value(event.clone()).unwrap();
            projection.apply("main", &event);
        }
    }

    fn step(sequence: u64, name: &str, delta: &str) -> serde_json::Value {
        serde_json::json!({
            "sequence": sequence,
            "create_at": sequence * 1000,
            "type": "step",
            "name": name,
            "data": { "turn_id": "t1", "request_id": "r1", "delta": delta },
        })
    }

    fn model_start(sequence: u64) -> serde_json::Value {
        serde_json::json!({
            "sequence": sequence,
            "create_at": sequence * 1000,
            "type": "step",
            "name": "model-start",
            "data": { "turn_id": "t1", "request_id": "r1" },
        })
    }

    fn progress(thinking: &str, offset: u64) -> Progress {
        serde_json::from_value(serde_json::json!({
            "sessionId": "s", "branchId": "main", "requestId": "r1",
            "text": "", "textOffset": 0,
            "thinking": thinking, "thinkingOffset": offset,
        }))
        .unwrap()
    }

    #[test]
    fn reasoning_streams_from_progress_and_settles_as_written() {
        let mut projection = Projection::default();
        apply(
            &mut projection,
            &[
                // Context-only input: not a user message.
                serde_json::json!({ "id": "in-1", "sequence": 1, "create_at": 1000, "type": "input", "name": "message", "data": {} }),
                model_start(2),
            ],
        );
        // Unwritten reasoning shows while it streams.
        projection.progress(&progress("Look at the\nparser", 0));
        let rows = projection.snapshot().rows;
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].kind, RowKind::Thinking);
        assert!(rows[0].streaming);
        assert_eq!(rows[0].text, "Look at the\nparser");

        // The thought is written whole when it ends, timed from model start.
        apply(
            &mut projection,
            &[step(5, "thinking", "Look at the\nparser first")],
        );
        // Progress the written deltas overtook changes nothing.
        projection.progress(&progress("Look at the\nparser", 0));
        let rows = projection.snapshot().rows;
        assert_eq!(rows[0].text, "Look at the\nparser first");
        assert_eq!(rows[0].elapsed_ms, Some(3000));

        apply(&mut projection, &[step(6, "text", "Done.")]);
        let rows = projection.snapshot().rows;
        assert!(!rows[0].streaming);
        assert_eq!(rows[1].kind, RowKind::StreamingAssistant);
    }

    #[test]
    fn an_interrupted_thought_keeps_only_what_was_written() {
        let mut projection = Projection::default();
        apply(&mut projection, &[model_start(1)]);
        projection.progress(&progress("half a thought", 0));
        apply(
            &mut projection,
            &[serde_json::json!({
                "sequence": 2, "create_at": 2000, "type": "turn", "name": "end",
                "data": { "turn_id": "t1", "status": "cancelled", "reason": "user" },
            })],
        );
        let rows = projection.snapshot().rows;
        assert!(!rows[0].streaming);
        // Never written: the transcript draws nothing for it.
        assert_eq!(rows[0].text, "");
    }

    #[test]
    fn snapshots_share_rows_and_a_delta_copies_only_its_own() {
        let mut projection = Projection::default();
        apply(
            &mut projection,
            &[step(1, "thinking", "plan"), step(2, "text", "Hel")],
        );
        let before = projection.snapshot().rows;
        apply(&mut projection, &[step(3, "text", "lo")]);
        let after = projection.snapshot().rows;
        // The settled thought is the same allocation in both snapshots; the
        // streaming answer was copied once, and the old snapshot kept its text.
        assert!(Arc::ptr_eq(&before[0], &after[0]));
        assert!(!Arc::ptr_eq(&before[1], &after[1]));
        assert_eq!(
            (before[1].text.as_str(), after[1].text.as_str()),
            ("Hel", "Hello")
        );
    }

    #[test]
    fn a_shell_run_the_app_server_recorded_is_a_shell_row() {
        let marker = "<dim-shell-input kind=\"command\" status=\"completed\" exit=\"0\" truncated=\"false\">\n$ ls\na\n\n</dim-shell-input>";
        let mut projection = Projection::default();
        apply(
            &mut projection,
            &[
                serde_json::json!({ "id": "in-1", "sequence": 1, "create_at": 1000, "type": "input", "name": "message",
                    "data": { "blocks": [{ "type": "raw", "text": marker }], "mode": "append" } }),
                serde_json::json!({ "id": "in-2", "sequence": 2, "create_at": 2000, "type": "input", "name": "message",
                    "data": { "blocks": [{ "type": "raw", "text": "<other>context</other>" }] } }),
            ],
        );
        let rows = projection.snapshot().rows;
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].kind, RowKind::Shell);
        assert_eq!(rows[0].text, marker);
    }

    #[test]
    fn ask_user_calls_of_any_branch_are_questions_until_their_results() {
        let mut projection = Projection::default();
        let mut call = |branch: &str| {
            let event: Event = serde_json::from_value(serde_json::json!({
                "id": format!("{branch}-call"), "sequence": 1, "create_at": 1000,
                "type": "step", "name": "tool-call",
                "data": { "turn_id": "t1", "request_id": "r1", "calls": [
                    { "id": "c1", "name": "ask_user", "input": { "questions": [
                        { "header": "Dinner", "question": "What should we eat?",
                          "options": [{ "label": "Hotpot" }, { "label": "Ramen" }] },
                        { "question": "Sides?", "multiSelect": true, "allowCustom": false,
                          "options": [{ "label": "Rice" }, { "label": "Soup" }] }
                    ] } },
                    { "id": "c2", "name": "read", "input": { "path": "a" } }
                ] },
            }))
            .unwrap();
            projection.apply(branch, &event);
        };
        call("main");
        call("child");
        let questions = projection.snapshot().questions;
        assert_eq!(questions.len(), 2, "{questions:?}");
        let asked = &questions[0];
        assert_eq!(asked.branch_id, "main");
        assert_eq!(asked.questions[0].header, "Dinner");
        assert_eq!(asked.questions[0].kind, QuestionKind::SingleSelect);
        assert!(asked.questions[0].allow_custom);
        assert_eq!(asked.questions[1].header, "Q2");
        assert_eq!(asked.questions[1].kind, QuestionKind::MultiSelect);
        assert!(!asked.questions[1].allow_custom);
        assert_ne!(asked.request_id, questions[1].request_id);
        assert_eq!(
            projection
                .question("child", &questions[1].request_id)
                .unwrap()
                .branch_id,
            "child"
        );
        assert!(
            projection
                .question("main", &questions[1].request_id)
                .is_none()
        );

        let result: Event = serde_json::from_value(serde_json::json!({
            "id": "main-result", "sequence": 2, "create_at": 2000,
            "type": "step", "name": "tool-result",
            "data": { "turn_id": "t1", "request_id": "r1", "call_event_id": "main-call",
                "results": [{ "call_id": "c1", "result": { "content": "Hotpot" } }] },
        }))
        .unwrap();
        projection.apply("main", &result);
        let questions = projection.snapshot().questions;
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].branch_id, "child");
    }
}
