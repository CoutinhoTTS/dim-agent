use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoalView {
    pub objective: String,
    pub status: GoalViewStatus,
    pub evaluator: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum GoalViewStatus {
    Active,
    Paused(String),
    Completed,
    Blocked,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoalRead {
    pub branch_id: String,
    #[serde(with = "crate::safe_integer")]
    pub through: u64,
    pub goal: Option<GoalView>,
}
