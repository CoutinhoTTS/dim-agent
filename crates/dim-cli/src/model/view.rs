//! The view model: everything the terminal UI knows about a session.
//!
//! These types are owned by the UI. The renderer, the prompts and the panels
//! read them and nothing from the backend crates (`dim-client`,
//! `dim-agent-types`, `dim-app-protocol`); `view_adapter` is the one place a
//! backend projection is turned into them, so a change below the wire lands
//! there and not in a widget. `xtask check` enforces the boundary.

use std::collections::BTreeMap;

/// The kind of one transcript row, as far as drawing is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    UserMessage,
    /// A shell-mode run the user sent to the model (`!command`).
    Shell,
    /// Assistant text that is still arriving.
    StreamingAssistant,
    AssistantMessage,
    Thinking,
    ToolCall,
    Error,
    Status,
    /// A row kind the UI does not draw specially; it falls back to a status
    /// line.
    #[allow(dead_code)] // The projection maps every kind it emits.
    Other,
}

impl RowKind {
    /// Rows counted as messages by the history window.
    pub fn is_message(self) -> bool {
        matches!(
            self,
            Self::UserMessage | Self::Shell | Self::AssistantMessage | Self::ToolCall
        )
    }
}

/// How a status row is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowTone {
    Danger,
    #[allow(dead_code)] // The projection emits no warning statuses yet.
    Warning,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewRow {
    pub row_id: String,
    pub kind: RowKind,
    pub text: String,
    pub streaming: bool,
    /// A settled thought: how long it ran, in milliseconds.
    pub elapsed_ms: Option<u64>,
    pub tone: Option<RowTone>,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    /// The model request a tool call came from: the calls of one request are
    /// drawn as one group.
    pub batch: Option<String>,
    /// A tool call that has not produced its result yet.
    pub pending: bool,
    /// A call that waits on the user (an approval names it, or it is the
    /// question tool itself): it is part of the transcript at once, and the
    /// prompt owns the waiting.
    pub parked: bool,
    pub result_text: Option<String>,
    pub result_is_error: bool,
    pub diff: Option<String>,
    /// A finished command's exit code (the exec tool reports it).
    pub exit_code: Option<i64>,
}

/// A tool call that asks the user for permission.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalView {
    pub request_id: String,
    pub branch_id: String,
    pub reason: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    /// The simple command an "always" answer would save an allow rule for,
    /// when the daemon keeps command rules.
    pub always: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionKind {
    #[allow(dead_code)] // Questions wait for the App Server's ask methods.
    SingleSelect,
    MultiSelect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub header: String,
    pub question: String,
    pub kind: QuestionKind,
    pub options: Vec<QuestionOption>,
}

/// An open request for the user to answer questions.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionRequestView {
    pub request_id: String,
    pub branch_id: String,
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Single(String),
    Multiple(Vec<String>),
}

/// Answers keyed by question id.
pub type Answers = BTreeMap<String, Answer>;

/// A message submitted while a turn runs, not yet consumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedInput {
    pub id: String,
    pub text: String,
}

/// Work of the session that runs apart from the turn: a command the `exec`
/// tool moved to the background, or a member agent on a delegated task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskView {
    /// The Timeline event that started the command or delegation.
    pub id: String,
    pub kind: TaskKind,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskTarget {
    pub branch_id: String,
    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOutputState {
    Loading,
    Live,
    Unavailable,
    Unsupported,
    ReadError(String),
    Finished { status: &'static str, failed: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskKind {
    /// Read and stopped on the originating branch.
    Command(TaskTarget),
    /// The Session is known only after successful delivery of the request.
    Member { session_id: Option<String> },
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEnd {
    Completed,
    Failed,
    Cancelled,
}

/// The session as the screen draws it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewSnapshot {
    /// Shared with the worker's projection: publishing copies pointers.
    pub rows: Vec<std::sync::Arc<ViewRow>>,
    pub running: bool,
    pub approvals: Vec<ApprovalView>,
    pub questions: Vec<QuestionRequestView>,
    /// Oldest first.
    pub queued: Vec<QueuedInput>,
    /// Background commands and member agents still running, oldest first.
    pub tasks: Vec<TaskView>,
    /// The newest turn that ended on main: its sequence and how it ended.
    pub ended: Option<(u64, TurnEnd)>,
}
