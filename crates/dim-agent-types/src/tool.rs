use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchemaType {
    #[serde(rename = "object")]
    Object,
    #[serde(rename = "array")]
    Array,
    #[serde(rename = "string")]
    String,
    #[serde(rename = "number")]
    Number,
    #[serde(rename = "integer")]
    Integer,
    #[serde(rename = "boolean")]
    Boolean,
    #[serde(rename = "null")]
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolSource {
    #[serde(rename = "static")]
    Static,
    #[serde(rename = "builtin")]
    Builtin,
    #[serde(rename = "extension")]
    Extension,
    #[serde(rename = "dynamic")]
    Dynamic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolFilePathSelector {
    #[serde(rename = "path")]
    Path,
    #[serde(rename = "affectedPaths")]
    AffectedPaths,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WakePolicy {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "manual")]
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlobKind {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "binary")]
    Binary,
    #[serde(rename = "file")]
    File,
    #[serde(rename = "stream")]
    Stream,
    #[serde(rename = "remote")]
    Remote,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonSchema {
    #[serde(
        rename = "type",
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub schema_type: Option<SchemaType>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub properties: Option<std::collections::BTreeMap<String, JsonSchema>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub items: Option<Box<JsonSchema>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub required: Option<Vec<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(rename = "enum")]
    pub enum_values: Option<Vec<serde_json::Value>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub description: Option<String>,
    #[serde(flatten)]
    pub extensions: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolMetadata {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub version: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub author: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub docs_url: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub source_plugin_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub mcp_server_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub mcp_original_tool_name: Option<String>,
    #[serde(flatten)]
    pub extensions: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolPreExecutePaths {
    pub selector: ToolFilePathSelector,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolFileChangeDeclaration {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub pre_execute_paths: Option<ToolPreExecutePaths>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub post_execute_paths: Option<ToolFilePathSelector>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: JsonSchema,
    pub source: ToolSource,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub risk: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub metadata: Option<ToolMetadata>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub file_changes: Option<ToolFileChangeDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackgroundTaskInfo {
    pub task_id: String,
    pub wake_policy: WakePolicy,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub output_path: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlobRef {
    pub blob_id: String,
    pub session_id: String,
    pub kind: BlobKind,
    pub path: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub r#ref: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub mime_type: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub size_bytes: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolResult {
    pub content: ToolContent,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub structured_content: Option<JsonObject>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub is_error: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub blob_refs: Option<Vec<BlobRef>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub background_task: Option<BackgroundTaskInfo>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub context_messages: Option<Vec<Message>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub effects: Option<Vec<ToolEffect>>,
}

/// What a call did, declared by the tool on its own result
/// (session boundary A.6). The vocabulary is closed; anything else needs a
/// requirements change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ToolEffect {
    /// The call wrote the file at `path`, the absolute resolved path the
    /// result reports.
    FileWritten { path: String },
    /// The call ran `command`. At most one of `exit_code` and `signal` is
    /// present; both absent mean the tool observed no exit status.
    CommandRun {
        command: String,
        #[serde(
            rename = "exitCode",
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "crate::optional_non_null"
        )]
        exit_code: Option<i32>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "crate::optional_non_null"
        )]
        signal: Option<String>,
    },
}

impl ToolResult {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: ToolContent::Text(content.into()),
            structured_content: None,
            is_error: None,
            blob_refs: None,
            background_task: None,
            context_messages: None,
            effects: None,
        }
    }
    pub fn error(content: impl Into<String>) -> Self {
        Self {
            is_error: Some(true),
            ..Self::text(content)
        }
    }
}

impl Default for ToolResult {
    fn default() -> Self {
        Self::text("")
    }
}
