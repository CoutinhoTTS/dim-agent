use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelinePosition {
    pub branch_id: String,
    #[serde(with = "crate::safe_integer")]
    pub sequence: u64,
}

/// Internal projection state immediately before replaying `resume`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionCheckpoint {
    pub resume: TimelinePosition,
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
    pub compacted_through: String,
    pub prompts: PromptCheckpointState,
    pub delivered: Vec<ContextBaselineEntry>,
    pub inputs: Vec<(String, InputMessageData)>,
    pub configuration: Option<ContextConfiguredData>,
    pub output: Vec<MessagePart>,
    pub open_tools: Vec<String>,
    /// A model output started and has not ended.
    pub output_open: bool,
}
