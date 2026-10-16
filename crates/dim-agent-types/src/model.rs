use crate::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilities {
    pub modalities: Vec<String>,
    pub tool_calling: bool,
    pub streaming: bool,
    pub structured_output: bool,
    pub vision: bool,
    pub audio: bool,
    pub reasoning: bool,
    pub context_window: u64,
    pub max_output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<JsonObject>,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            modalities: vec!["text".into()],
            tool_calling: true,
            streaming: true,
            structured_output: true,
            vision: false,
            audio: false,
            reasoning: false,
            context_window: 128_000,
            max_output_tokens: 16_384,
            provider_state: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelReasoning {
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_options: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_options: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity_options: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interleaved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summaries: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<ReasoningBudget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off: Option<ReasoningOff>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReasoningBudget {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub default: Option<f64>,
    pub unit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReasoningOff {
    ThinkingType { value: String },
    ThinkingBudget { value: u64 },
    EnableThinking { value: bool },
    ChatTemplateKwargs { keys: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelServiceTier {
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCapabilitySnapshot {
    pub target: ModelId,
    pub capabilities: ModelCapabilities,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub reasoning: Option<ModelReasoning>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub service_tier: Option<ModelServiceTier>,
}

impl ModelCapabilitySnapshot {
    pub fn validate_for(&self, provider_id: &str, model_id: &str) -> Result<(), &'static str> {
        if provider_id.is_empty()
            || model_id.is_empty()
            || self.target.provider_id.is_empty()
            || self.target.model_id.is_empty()
        {
            return Err("model identity must be nonempty");
        }
        if self.target.provider_id != provider_id || self.target.model_id != model_id {
            return Err("capability snapshot target does not match request");
        }
        if self.capabilities.context_window == 0
            || self.capabilities.max_output_tokens == 0
            || self.capabilities.context_window > crate::JS_MAX_SAFE_INTEGER
            || self.capabilities.max_output_tokens > crate::JS_MAX_SAFE_INTEGER
        {
            return Err("model limits must be positive safe JSON integers");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SummaryStyle {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "concise")]
    Concise,
    #[serde(rename = "detailed")]
    Detailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThinkingType {
    #[serde(rename = "enabled")]
    Enabled,
    #[serde(rename = "disabled")]
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnthropicThinkingType {
    #[serde(rename = "adaptive")]
    Adaptive,
    #[serde(rename = "enabled")]
    Enabled,
    #[serde(rename = "disabled")]
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaselineSource {
    #[serde(rename = "provider_baseline")]
    ProviderBaseline,
    #[serde(rename = "full_estimate")]
    FullEstimate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionReason {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "manual")]
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionTrigger {
    #[serde(rename = "threshold")]
    Threshold,
    #[serde(rename = "provider_output_budget_exhausted")]
    ProviderOutputBudgetExhausted,
    #[serde(rename = "manual")]
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionCoverage {
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "partial_recent")]
    PartialRecent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetEstimateSource {
    #[serde(rename = "actual")]
    Actual,
    #[serde(rename = "estimated")]
    Estimated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionFallbackReason {
    #[serde(rename = "chunk_overflow")]
    ChunkOverflow,
    #[serde(rename = "empty_summary")]
    EmptySummary,
    #[serde(rename = "budget_preflight_empty")]
    BudgetPreflightEmpty,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderOutputBudgetFacts {
    #[serde(with = "crate::finite_number")]
    pub input_tokens: f64,
    #[serde(with = "crate::finite_number")]
    pub context_window: f64,
    #[serde(with = "crate::finite_number")]
    pub requested_max_tokens: f64,
    #[serde(with = "crate::finite_number")]
    pub allowed_max_tokens: f64,
    #[serde(with = "crate::finite_number")]
    pub ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Usage {
    #[serde(with = "crate::finite_number")]
    pub prompt_tokens: f64,
    #[serde(with = "crate::finite_number")]
    pub completion_tokens: f64,
    #[serde(with = "crate::finite_number")]
    pub total_tokens: f64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub cache_read_tokens: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub cache_write_tokens: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub cost: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub currency: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenAiProviderOptions {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub reasoning_effort: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub service_tier: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub summary: Option<SummaryStyle>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub interleaved_thinking: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompatibleThinking {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(rename = "type")]
    pub thinking_type: Option<ThinkingType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenAiCompatibleProviderOptions {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub reasoning_effort: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub service_tier: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub interleaved_thinking: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub thinking: Option<CompatibleThinking>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(rename = "enable_thinking")]
    pub enable_thinking: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnthropicThinking {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(rename = "type")]
    pub thinking_type: Option<AnthropicThinkingType>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub budget_tokens: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnthropicProviderOptions {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub effort: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub thinking: Option<AnthropicThinking>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub send_reasoning: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub interleaved_thinking: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffortProviderOptions {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub reasoning_effort: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub interleaved_thinking: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeminiThinkingConfig {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub thinking_budget: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub include_thoughts: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeminiProviderOptions {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub thinking_config: Option<GeminiThinkingConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderOptions {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub openai: Option<OpenAiProviderOptions>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(rename = "openai-compatible")]
    pub openai_compatible: Option<OpenAiCompatibleProviderOptions>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub anthropic: Option<AnthropicProviderOptions>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub gemini: Option<GeminiProviderOptions>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub deepseek: Option<EffortProviderOptions>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub moonshotai: Option<EffortProviderOptions>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub xai: Option<EffortProviderOptions>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub zenmux: Option<EffortProviderOptions>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub aihubmix: Option<EffortProviderOptions>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderReasoningConfig {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub enabled: Option<EnabledReasoning>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub effort: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub budget_tokens: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub summary: Option<SummaryStyle>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub include_thoughts: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub interleaved_thinking: Option<bool>,
}

/// The TypeScript contract permits `true` or omission, but never `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnabledReasoning;
impl Serialize for EnabledReasoning {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}
impl<'de> Deserialize<'de> for EnabledReasoning {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(
                "reasoning.enabled must be true when present",
            ))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConfig {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub temperature: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub max_tokens: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub top_p: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub top_k: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub reasoning: Option<ProviderReasoningConfig>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub provider_options: Option<ProviderOptions>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CacheControl {
    #[serde(rename = "ephemeral")]
    Ephemeral,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum SystemPromptPart {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "crate::optional_non_null"
        )]
        cache_control: Option<CacheControl>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemPrompt {
    Text(String),
    Parts(Vec<SystemPromptPart>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RequestMetadata {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub session_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub run_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub trace_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub turn_id: Option<String>,
    #[serde(flatten)]
    pub extensions: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRequest {
    pub provider_id: String,
    pub model_id: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub capability_snapshot: Option<ModelCapabilitySnapshot>,
    pub messages: Vec<Arc<Message>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub tools: Option<Arc<Vec<ToolDefinition>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub system: Option<Arc<SystemPrompt>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub config: Option<ModelConfig>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub metadata: Option<RequestMetadata>,
}

impl ModelRequest {
    pub fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        messages: Vec<Message>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            model_id: model_id.into(),
            capability_snapshot: None,
            messages: messages.into_iter().map(Arc::new).collect(),
            tools: None,
            system: None,
            config: None,
            metadata: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionUsage {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub input_tokens: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub output_tokens: Option<f64>,
}

/// Strategy-neutral diagnostics of a compaction: ContextPolicy fills the
/// common part, the strategy owns `strategy`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompactionDiagnosticsRecord {
    pub reason: CompactionReason,
    pub trigger: CompactionTrigger,
    /// Timeline event id of the first event kept verbatim; absent when the
    /// strategy found no cut.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub retain_from: Option<String>,
    /// Timeline event id the cut must not pass, unless under recovery.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    pub retain_limit: Option<String>,
    /// The request before compaction, from `count_tokens`.
    #[serde(with = "crate::finite_number")]
    pub estimated_input_tokens: f64,
    /// The projected request after compaction, from `count_tokens`; absent
    /// when nothing was replaced.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::optional_non_null"
    )]
    #[serde(serialize_with = "crate::finite_number::serialize_optional")]
    pub estimated_output_tokens: Option<f64>,
    #[serde(with = "crate::safe_integer")]
    pub entries_replaced: u64,
    #[serde(with = "crate::safe_integer")]
    pub model_call_count: u64,
    #[serde(with = "crate::safe_integer")]
    pub duration_ms: u64,
    pub strategy: CompactionStrategyRecord,
}

/// The strategy's own diagnostics: `kind` names the strategy, the other
/// fields are its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionStrategyRecord {
    pub kind: String,
    #[serde(flatten)]
    pub details: JsonObject,
}
