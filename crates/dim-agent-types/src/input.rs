use crate::{InputMode, UserInput};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingInput {
    pub input_id: String,
    #[serde(with = "crate::safe_integer")]
    pub sequence: u64,
    pub mode: InputMode,
    pub user: Option<UserInput>,
    pub applied: bool,
}
