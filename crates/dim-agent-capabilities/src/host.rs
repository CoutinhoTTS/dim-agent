use crate::history::HistoryEntry;
use dim_agent_runtime::PromptSourceTarget;
use dim_agent_types::*;
use futures_util::Stream;
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};
use tokio_util::sync::CancellationToken;

pub(crate) use dim_agent_runtime::panic_message;

pub type HostFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, HostError>> + Send + 'a>>;
pub type ModelStream = Pin<Box<dyn Stream<Item = Result<ModelChunk, HostError>> + Send>>;

#[derive(Debug, Clone, thiserror::Error)]
pub enum HostError {
    #[error("{message}")]
    Failed {
        message: String,
        budget: Option<ProviderOutputBudgetFacts>,
        kind: HostErrorKind,
        model: Option<Box<ModelFailureFacts>>,
        presentation: Option<Box<ProviderPresentation>>,
    },
    #[error("{0}")]
    Cancelled(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFailureOrigin {
    Transport,
    HttpResponse,
    StreamEvent,
}
#[derive(Debug, Clone)]
pub struct ModelFailureFacts {
    pub origin: ModelFailureOrigin,
    /// Trimmed, ASCII lowercase, and nonempty when present; normalized at ingress.
    pub error_type: Option<String>,
    pub message: Option<String>,
    pub request_id: Option<String>,
    pub transient: bool,
    pub retry_after: Option<std::time::Duration>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostErrorKind {
    Other,
    Http {
        status: Option<u16>,
        /// Trimmed, ASCII lowercase, and nonempty when present; normalized at ingress.
        code: Option<String>,
    },
    InvalidRequest,
    Unsupported,
    Transport,
    OwnerClosed,
}
impl HostError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
            budget: None,
            kind: HostErrorKind::Other,
            model: None,
            presentation: None,
        }
    }
    pub fn classified(kind: HostErrorKind, message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
            budget: None,
            kind,
            model: None,
            presentation: None,
        }
    }
}

/// Request settings deliberately omit messages: the loop owns the fixed prefix.
#[derive(Debug, Clone)]
pub struct ModelSettings {
    pub provider_id: String,
    pub model_id: String,
    pub capability_snapshot: Option<ModelCapabilitySnapshot>,
    pub tools: Option<Vec<ToolDefinition>>,
    pub system: Option<SystemPrompt>,
    pub config: Option<ModelConfig>,
    pub metadata: Option<RequestMetadata>,
}
impl ModelSettings {
    pub fn new(provider_id: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            model_id: model_id.into(),
            capability_snapshot: None,
            tools: None,
            system: None,
            config: None,
            metadata: None,
        }
    }
    pub fn request(self, messages: Vec<Message>) -> ModelRequest {
        ModelRequest {
            provider_id: self.provider_id,
            model_id: self.model_id,
            capability_snapshot: self.capability_snapshot,
            messages: messages.into_iter().map(Arc::new).collect(),
            tools: self.tools.map(Arc::new),
            system: self.system.map(Arc::new),
            config: self.config,
            metadata: self.metadata,
        }
    }
}

pub trait PrepareModel: Send + Sync + 'static {
    fn prepare(
        &self,
        turn_id: &str,
        request_id: &str,
        history: &[Message],
    ) -> Result<ModelSettings, HostError>;
}
pub struct PrepareModelHost(pub Arc<dyn PrepareModel>);

#[derive(Debug, Clone)]
pub struct ContextSource {
    pub turn_id: String,
    pub request_id: String,
    pub through: u64,
}
pub trait PrepareContext: Send + Sync + 'static {
    fn prepare(
        &self,
        request: Arc<ModelRequest>,
        source: ContextSource,
        cancel: CancellationToken,
    ) -> HostFuture<'_, Arc<ModelRequest>>;
}
pub struct PrepareContextHost(pub Arc<dyn PrepareContext>);

pub trait LoadPrompt: Send + Sync + 'static {
    fn load(
        &self,
        target: PromptSourceTarget,
        cancel: CancellationToken,
    ) -> HostFuture<'_, Option<PromptEntry>>;
}
pub struct PromptHost(pub Arc<dyn LoadPrompt>);

pub struct EmptyPromptHost;
impl LoadPrompt for EmptyPromptHost {
    fn load(
        &self,
        target: PromptSourceTarget,
        cancel: CancellationToken,
    ) -> HostFuture<'_, Option<PromptEntry>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                Err(HostError::Cancelled("Prompt loading cancelled".into()))
            } else {
                Err(HostError::failed(format!(
                    "unknown prompt source: {}",
                    target.source
                )))
            }
        })
    }
}

/// Host adapters validate external wire chunks before producing this type.
#[derive(Debug, Clone)]
pub enum ModelChunk {
    MessageStart {
        message_id: String,
    },
    /// A streamed fragment of the current thinking block, for display.
    Thinking(String),
    /// The current thinking block is complete, with its whole content.
    ThinkingEnd {
        thinking: String,
    },
    /// A streamed fragment of the current text block.
    Text(String),
    /// The current text block is complete.
    TextEnd,
    /// A tool call has started; it precedes the call's arguments and `ToolUse`.
    ToolStart {
        id: String,
        name: String,
    },
    /// A fragment of a started call's argument text, for display.
    ToolArguments {
        id: String,
        name: String,
        delta: String,
    },
    ToolUse(ToolCall),
    /// Complete native blocks, including continuation metadata, before MessageStop.
    Content(Vec<MessagePart>),
    MessageStop {
        message_id: String,
        reason: ModelStopReason,
    },
    Usage(Usage),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelStopReason {
    EndTurn,
    ToolUse,
    StopSequence,
    MaxTokens,
}
/// Connection facts are resolved for one final request and never serialized.
/// Local is the explicit development host profile, with no network credentials.
#[derive(Clone)]
pub enum ModelConnection {
    Local,
    Http(HttpModelConnection),
    Bedrock(BedrockModelConnection),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelProtocol {
    #[serde(rename = "openai-chat")]
    OpenaiChat,
    #[serde(rename = "openai-compatible")]
    OpenaiCompatible,
    #[serde(rename = "openai-responses")]
    OpenaiResponses,
    #[serde(rename = "xai")]
    Xai,
    #[serde(rename = "xai-responses")]
    XaiResponses,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "bedrock")]
    Bedrock,
}
/// Request-local transport facts. Providers validate these at their input
/// boundary; the model host owns clients, not persisted provider records.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelNetwork {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub proxy: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub timeout_ms: Option<u64>,
}
#[derive(Clone)]
pub struct HttpModelConnection {
    pub protocol: ModelProtocol,
    pub base_url: String,
    pub network: ModelNetwork,
    pub headers: std::collections::HashMap<String, String>,
    pub bearer: Option<String>,
    pub api_key: Option<String>,
    pub preset: Option<String>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "optional_non_null"
    )]
    pub session_token: Option<String>,
}
#[derive(Clone)]
pub enum BedrockAuth {
    Bearer(String),
    SigV4 {
        credentials: AwsCredentials,
        region: String,
    },
}
#[derive(Clone)]
pub struct BedrockModelConnection {
    pub base_url: String,
    pub network: ModelNetwork,
    pub headers: HashMap<String, String>,
    pub auth: BedrockAuth,
}
fn optional_non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}
/// Request-local credentials stay with the caller, never with the transport.
pub struct ResolvedModelConnection {
    pub connection: ModelConnection,
    pub failure_policy: Option<Arc<dyn ModelFailurePolicy>>,
    pub recovery: Option<Arc<dyn RecoverModelConnection>>,
}
/// Recovery replaces credentials, never the request's product policy.
pub struct RecoveredModelConnection {
    pub connection: ModelConnection,
    pub recovery: Arc<dyn RecoverModelConnection>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CredentialRecoveryPhase {
    #[default]
    Initial,
    Recovered,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialRejection {
    Unrelated,
    Recoverable,
    Terminal,
}
pub trait RecoverModelConnection: Send + Sync + 'static {
    /// Pure authority-specific interpretation; a terminal rejection requires
    /// settlement even when output or policy prevents replay.
    fn rejection(&self, error: &HostError, phase: CredentialRecoveryPhase) -> CredentialRejection;
    fn recover(&self, cancel: CancellationToken) -> HostFuture<'_, RecoveredModelConnection>;
    fn finish_rejected(&self, cancel: CancellationToken) -> HostFuture<'_, ()>;
}
/// Pure request policy, independent of credential recovery support.
pub trait ModelFailurePolicy: Send + Sync + 'static {
    fn forbids_retry(&self, _error: &HostError) -> bool {
        false
    }
    fn presentation(&self, _error: &HostError) -> Option<ProviderPresentation> {
        None
    }
}
pub trait ResolveModelConnection: Send + Sync + 'static {
    fn resolve(
        &self,
        request: Arc<ModelRequest>,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ResolvedModelConnection>;
}
pub struct ProviderConnectionResolver(pub Arc<dyn ResolveModelConnection>);

/// Resolves a configuration once. Executing an adopted request must not call
/// this port again to replace its capabilities with the latest catalog.
pub trait ResolveModelSelection: Send + Sync + 'static {
    fn resolve(
        &self,
        explicit: Option<ModelId>,
        cwd: Option<std::path::PathBuf>,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelSelection>;
}
pub struct ModelSelectionHost(pub Arc<dyn ResolveModelSelection>);

/// One non-streaming request outside any Session: no tools, no Timeline.
#[derive(Debug, Clone)]
pub struct AuxiliaryRequest {
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub max_output_tokens: Option<u32>,
}
#[derive(Debug, Clone)]
pub struct AuxiliaryReply {
    /// The text blocks only; thinking output is dropped.
    pub text: String,
    pub stop_reason: ModelStopReason,
    pub usage: Option<Usage>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuxiliaryErrorCode {
    InvalidRequest,
    ModelUnavailable,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct AuxiliaryError {
    pub code: AuxiliaryErrorCode,
    pub message: String,
}
/// The caller chooses the model; the owner of credentials never falls back
/// to another one and never returns the connection.
pub trait CompleteAuxiliary: Send + Sync + 'static {
    fn complete(
        &self,
        model: ModelId,
        request: AuxiliaryRequest,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<AuxiliaryReply, AuxiliaryError>> + Send + '_>>;
}
pub struct AuxiliaryModelHost(pub Arc<dyn CompleteAuxiliary>);

pub trait ModelDriver: Send + Sync + 'static {
    fn invoke(
        &self,
        request: Arc<ModelRequest>,
        connection: ModelConnection,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ModelStream>;
}
pub struct ModelHost(pub Arc<dyn ModelDriver>);

#[derive(Debug, Clone)]
pub struct ToolExecutionContext {
    pub turn_id: String,
    pub request_id: String,
    pub branch_id: String,
}
pub trait ToolDriver: Send + Sync + 'static {
    fn definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }
    fn execute(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult>;
}
pub struct ToolHost(pub Arc<dyn ToolDriver>);

/// What ContextPolicy hands a strategy: the entries from the previous cut
/// on, the previous result, the request whose context is compacted and the
/// cuts the retention rule allows.
#[derive(Clone)]
pub struct CompactionInput {
    /// The projected entries from the previous `retain_from` on.
    pub history: Vec<HistoryEntry>,
    pub checkpoint: Option<CompactionCheckpoint>,
    pub request: Arc<ModelRequest>,
    pub model: Arc<dyn ContextModel>,
    /// Event id of the first event of the oldest of the last K turns.
    pub retain_limit: Option<String>,
    /// Every legal cut, in order; a strategy returns one of their
    /// `retain_from`.
    pub cuts: Vec<CompactionCut>,
    pub trigger: CompactionTrigger,
    pub estimated_input_tokens: f64,
    pub baseline_source: BaselineSource,
    pub baseline_prompt_tokens: Option<f64>,
    pub delta_tokens: Option<f64>,
    pub provider_budget: Option<ProviderOutputBudgetFacts>,
}
/// A legal cut: `history[..entry]` is replaced, `history[entry..]` is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionCut {
    pub retain_from: String,
    pub entry: usize,
    /// At or before `retain_limit`.
    pub within_limit: bool,
}
impl CompactionInput {
    /// The latest cut within the retention limit or, when none is within it
    /// (a single long turn), the latest cut that keeps something, so the
    /// branch compacts instead of skipping until overflow. Only budget
    /// recovery may replace everything, the latest input included.
    pub fn cut(&self) -> Option<&CompactionCut> {
        self.cuts
            .iter()
            .rev()
            .find(|cut| cut.within_limit)
            .or_else(|| {
                self.cuts
                    .iter()
                    .rev()
                    .find(|cut| self.may_fall_back_to(cut))
            })
    }
    /// Whether a cut outside the retention limit is allowed when none is
    /// within it.
    pub fn may_fall_back_to(&self, cut: &CompactionCut) -> bool {
        self.provider_budget.is_some() || cut.entry < self.history.len()
    }
}
#[derive(Debug, Clone)]
pub struct CompactionCheckpoint {
    pub retain_from: String,
    pub replacement: Arc<[Message]>,
    pub context_baseline: Arc<[ContextBaselineEntry]>,
}
/// Window facts of the model whose context is prepared.
pub trait ContextModel: Send + Sync + 'static {
    fn context_window(&self) -> f64;
    fn output_tokens(&self) -> f64;
    fn threshold_tokens(&self) -> f64;
    fn count_tokens(&self, request: &ModelRequest) -> f64;
}
/// A strategy's diagnostics: its model calls and its own record.
#[derive(Debug, Clone)]
pub struct StrategyDiagnostics {
    pub model_call_count: u64,
    pub record: CompactionStrategyRecord,
}
#[derive(Debug, Clone)]
pub enum StrategyOutcome {
    Completed {
        retain_from: String,
        replacement: Vec<Message>,
        diagnostics: StrategyDiagnostics,
    },
    Skipped {
        reason: String,
        diagnostics: StrategyDiagnostics,
    },
}
/// Produces the replacement of the history before a cut. Replay never calls
/// it: a committed result is replayed by its record alone.
pub trait CompactionStrategy: Send + Sync + 'static {
    /// Written into `context/compaction-start` and the result.
    fn name(&self) -> &str;
    fn compact(
        &self,
        input: CompactionInput,
        cancel: CancellationToken,
    ) -> HostFuture<'_, StrategyOutcome>;
}
/// An explicit optional binding, like `ContextModelHost`; the policy exists
/// when both are `Some`.
pub struct CompactionStrategyHost(pub Option<Arc<dyn CompactionStrategy>>);
pub trait ResolveContextModel: Send + Sync + 'static {
    fn resolve(
        &self,
        request: Arc<ModelRequest>,
        cancel: CancellationToken,
    ) -> HostFuture<'_, Arc<dyn ContextModel>>;
}
/// An explicit optional policy binding keeps dependencies visible in every profile.
pub struct ContextModelHost(pub Option<Arc<dyn ResolveContextModel>>);

/// A Session file SessionStore stored. `path` is absolute on the Session's
/// resource node and names the same bytes for the file's whole life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSessionFile {
    pub file_id: String,
    pub path: std::path::PathBuf,
    pub bytes: u64,
}
/// The `sessionFiles` error codes: `not_found`, `invalid`, `too_large`,
/// `node_unavailable`, `cancelled` and `storage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFileErrorCode {
    NotFound,
    Invalid,
    TooLarge,
    NodeUnavailable,
    Cancelled,
    Storage,
}
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct SessionFileError {
    pub code: SessionFileErrorCode,
    pub message: String,
}
/// `sessionFiles.put`: stores `content` once, read-only, as the file `name`
/// owned by `session_id`. Cancellation takes effect until the file is
/// committed; afterwards the reply stands.
pub trait PutSessionFile: Send + Sync + 'static {
    fn put(
        &self,
        session_id: String,
        name: String,
        content: Vec<u8>,
        cancel: CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<StoredSessionFile, SessionFileError>> + Send + '_>>;
}
/// The process service `sessionFiles`, provided by SessionStore, so that its
/// consumers name no App Server type.
pub struct SessionFilesHost(pub Arc<dyn PutSessionFile>);

pub struct IdentityContext;
impl PrepareContext for IdentityContext {
    fn prepare(
        &self,
        request: Arc<ModelRequest>,
        _: ContextSource,
        cancel: CancellationToken,
    ) -> HostFuture<'_, Arc<ModelRequest>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                Err(HostError::Cancelled("Context preparation cancelled".into()))
            } else {
                Ok(request)
            }
        })
    }
}
