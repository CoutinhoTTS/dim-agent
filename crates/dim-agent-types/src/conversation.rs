use crate::{InputMessageData, ModelId, PermissionPolicy};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationBegin {
    pub key: String,
    pub fingerprint: String,
    pub cwd: String,
    pub model: Option<ModelId>,
    pub policy: Option<PermissionPolicy>,
    pub title: Option<String>,
    #[serde(with = "crate::safe_integer")]
    pub chunks: u64,
    #[serde(with = "crate::safe_integer")]
    pub files: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationChunk {
    pub key: String,
    #[serde(with = "crate::safe_integer")]
    pub index: u64,
    pub input: InputMessageData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationFileChunk {
    pub key: String,
    pub id: String,
    pub name: String,
    pub sha256: String,
    #[serde(with = "crate::safe_integer")]
    pub bytes: u64,
    #[serde(with = "crate::safe_integer")]
    pub offset: u64,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationKey {
    pub key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationStatus {
    Preparing,
    Committed,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationFileReceipt {
    pub id: String,
    pub file_id: String,
    pub path: String,
    #[serde(with = "crate::safe_integer")]
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationReceipt {
    pub agent_id: String,
    pub session_id: String,
    pub status: ConversationStatus,
    #[serde(with = "crate::safe_integer")]
    pub next_chunk: u64,
    pub files: Vec<ConversationFileReceipt>,
}
