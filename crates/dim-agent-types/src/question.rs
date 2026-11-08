use crate::ToolCall;
use serde::{Deserialize, Serialize};

/// The question calls of one tool batch without a terminal result at the read cutoff.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingQuestionBatch {
    pub event_id: String,
    #[serde(with = "crate::safe_integer")]
    pub sequence: u64,
    pub request_id: String,
    pub calls: Vec<ToolCall>,
}
