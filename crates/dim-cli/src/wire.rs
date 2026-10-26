//! Wire types of the App Server session notifications.
//!
//! These are decoded once at the boundary and handed to the projection and
//! the scrollback printer as plain values. Nothing else parses these shapes.

use dim_agent_types::*;
use serde::Deserialize;

pub struct Event {
    /// The Timeline's event id, used by held-input and delegation controls.
    pub id: String,
    pub sequence: u64,
    /// Unix milliseconds the runtime wrote the event at.
    pub create_at: u64,
    pub content: EventContent,
}
#[derive(Clone, Deserialize)]
pub struct Configuration {
    pub provider_id: String,
    pub model_id: String,
}
pub enum EventContent {
    BranchCreate(BranchCreateData),
    BranchStart(BranchStartData),
    BranchCreateFailed(BranchCreateFailedData),
    Configuration(Configuration),
    Input(InputMessageData),
    /// InputQueue released the input to the turn.
    InputApplied(InputAppliedData),
    /// InputQueue dropped a held input, on withdrawal.
    InputDeleted(InputDeletedData),
    TurnEnd(TurnEndData),
    /// A command moved from the `exec` call to the background.
    ExecStarted(ExecStartedData),
    /// A background command ended.
    ExecEnded(ExecEndedData),
    /// The leader delegated a task to a member agent.
    CollaborationRequest(CollaborationRequestData),
    /// Delivery binds a delegation's request event to its actual member.
    CollaborationDelivered(CollaborationDeliveredData),
    /// A member's final result reached the leader.
    CollaborationResult(CollaborationResultData),
    /// The branch's permission policy changed, from any client.
    PermissionChanged(PermissionChangedData),
    ModelStart(ModelStartData),
    Thinking(ThinkingData),
    Text(TextData),
    ToolUse(ToolUseData),
    ToolCall(Box<ToolCallData>),
    ToolCompleted(Box<ToolCompletedData>),
    ToolResult(Box<ToolResultData>),
    ModelEnd(Box<ModelOutputEndData>),
    Other,
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            id: String,
            sequence: u64,
            create_at: u64,
            r#type: String,
            name: String,
            data: serde_json::Value,
        }
        let wire = Wire::deserialize(deserializer)?;
        if wire.sequence > JS_MAX_SAFE_INTEGER {
            return Err(serde::de::Error::custom(
                "Event sequence exceeds the protocol range",
            ));
        }
        let content = match (wire.r#type.as_str(), wire.name.as_str()) {
            ("branch", "create") => EventContent::BranchCreate(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("branch", "start") => EventContent::BranchStart(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("branch", "create-failed") => EventContent::BranchCreateFailed(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("context", "configured") => EventContent::Configuration(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("input", "message") => EventContent::Input(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("input", "applied") => EventContent::InputApplied(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("input", "deleted") => EventContent::InputDeleted(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("turn", "end") => EventContent::TurnEnd(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("exec", "started") => EventContent::ExecStarted(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("exec", "ended") => EventContent::ExecEnded(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("collaboration", "request") => EventContent::CollaborationRequest(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("collaboration", "delivered") => EventContent::CollaborationDelivered(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("collaboration", "result") => EventContent::CollaborationResult(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("permission", "changed") => EventContent::PermissionChanged(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("step", "model-start") => EventContent::ModelStart(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("step", "thinking") => EventContent::Thinking(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("step", "text") => EventContent::Text(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("step", "tool-use") => EventContent::ToolUse(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("step", "tool-call") => EventContent::ToolCall(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("step", "tool-completed") => EventContent::ToolCompleted(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("step", "tool-result") => EventContent::ToolResult(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            ("step", "model-output-end") => EventContent::ModelEnd(
                serde_json::from_value(wire.data).map_err(serde::de::Error::custom)?,
            ),
            _ => EventContent::Other,
        };
        Ok(Self {
            id: wire.id,
            sequence: wire.sequence,
            create_at: wire.create_at,
            content,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub session_id: String,
    pub branch_id: String,
    pub request_id: Option<String>,
    pub text: String,
    pub text_offset: u64,
    /// Reasoning not written to the Timeline yet: it follows the first
    /// `thinking_offset` characters of the request's `step/thinking` deltas.
    pub thinking: String,
    pub thinking_offset: u64,
}

/// A Workflow run as `workflow.status` and `workflow.stop` return it: the
/// fields the task view reads. The run owner validates the whole record.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkflowRun {
    pub run_id: String,
    pub name: String,
    pub source: String,
    #[serde(deserialize_with = "timestamp")]
    pub created_at: u64,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub ended_at: Option<u64>,
    pub state: WorkflowState,
    pub stages: Vec<WorkflowStage>,
    pub log: Vec<WorkflowLog>,
    pub report: WorkflowReport,
    #[serde(default)]
    report_to: Option<WorkflowAddress>,
    started_by: WorkflowStarter,
}

impl WorkflowRun {
    /// The branch of `session_id` that started the run or receives its
    /// report; `None` when the run belongs to no branch of that Session.
    pub fn branch_of(&self, session_id: &str) -> Option<&str> {
        match &self.started_by {
            WorkflowStarter::Tool(starter) if starter.session_id == session_id => {
                Some(&starter.branch_id)
            }
            _ => self
                .report_to
                .as_ref()
                .filter(|to| to.session_id == session_id)
                .map(|to| to.branch_id.as_str()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowState {
    Running,
    Stopping,
    Completed,
    Failed,
    Stopped,
    Interrupted,
}

impl WorkflowState {
    pub fn unfinished(self) -> bool {
        matches!(self, Self::Running | Self::Stopping)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkflowStage {
    pub id: String,
    pub step: u32,
    pub state: StageState,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub result_text: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StageState {
    Waiting,
    Launching,
    Running,
    Completed,
    Failed,
    Stopped,
    Skipped,
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkflowLog {
    #[serde(deserialize_with = "timestamp")]
    pub at: u64,
    pub message: String,
}

/// Report settlement, apart from the run's own outcome.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum WorkflowReport {
    None {},
    Pending {},
    Delivered {},
    Undeliverable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct WorkflowAddress {
    session_id: String,
    branch_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
enum WorkflowStarter {
    Tool(WorkflowAddress),
    Management(Management),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Management {
    Management,
}

fn timestamp<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let text = String::deserialize(deserializer)?;
    crate::usage::parse_rfc3339_ms(&text)
        .ok_or_else(|| serde::de::Error::custom("Invalid Workflow timestamp"))
}

fn optional_timestamp<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    timestamp(deserializer).map(Some)
}

/// The run a successful `workflow` tool result names and the state it saw:
/// `start` returns `{run_id, status}`, `status` and `stop` the stored `{run}`.
pub fn workflow_tool_run(result: &ToolResult) -> Option<(String, WorkflowState)> {
    #[derive(Deserialize)]
    struct Observed {
        run_id: String,
        state: WorkflowState,
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Reply {
        Start {
            run_id: String,
            status: WorkflowState,
        },
        Record {
            run: Observed,
        },
    }
    if result.is_error == Some(true) {
        return None;
    }
    let structured = serde_json::Value::Object(result.structured_content.clone()?);
    Some(match serde_json::from_value(structured).ok()? {
        Reply::Start { run_id, status } => (run_id, status),
        Reply::Record { run } => (run.run_id, run.state),
    })
}

/// The run and outcome of a `<dim-workflow kind="report" ...>` input block.
pub fn workflow_report(text: &str) -> Option<(String, WorkflowState)> {
    let rest = text.strip_prefix(r#"<dim-workflow kind="report" run=""#)?;
    let (run, rest) = rest.split_once('"')?;
    let (status, _) = rest.strip_prefix(r#" status=""#)?.split_once('"')?;
    let status = serde_json::from_value(serde_json::Value::String(status.to_owned())).ok()?;
    Some((unescape(run), status))
}

/// Reverses the marker's attribute escaping.
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}
