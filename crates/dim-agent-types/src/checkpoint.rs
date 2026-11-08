use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelinePosition {
    pub branch_id: String,
    #[serde(with = "crate::safe_integer")]
    pub sequence: u64,
}

/// Disposable interpreter state anchored to an immutable committed event.
/// Inspect `interpreter` before decoding `state`; storage does not interpret it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverySnapshot {
    #[serde(with = "crate::safe_integer")]
    pub sequence: u64,
    pub event_id: String,
    pub interpreter: String,
    pub state: String,
}

/// The event a projection resumes at: the event it names and where that event
/// sits in the branch chain. A chain read begins at the event the id names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumePoint {
    pub branch_id: String,
    #[serde(with = "crate::safe_integer")]
    pub sequence: u64,
    pub event_id: String,
}

/// Internal projection state immediately before replaying `resume`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionCheckpoint {
    pub resume: ResumePoint,
    pub state: ProjectionCheckpointState,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PromptCheckpointState {
    pub entries: Vec<PromptEntry>,
    pub revision: Option<PromptReference>,
    pub system_revision: Option<PromptReference>,
    pub system: String,
    pub turns: Vec<String>,
    pub tasks: Vec<PromptScope>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProjectionCheckpointState {
    /// Timeline event id of the cut the anchoring compaction made.
    pub retain_from: String,
    pub prompts: PromptCheckpointState,
    pub delivered: Vec<ContextBaselineEntry>,
    pub inputs: Vec<(String, InputMessageData)>,
    pub configuration: Option<ContextConfiguredData>,
    pub output: Vec<MessagePart>,
    pub open_tools: Vec<String>,
    /// A model output started and has not ended.
    pub output_open: bool,
}
