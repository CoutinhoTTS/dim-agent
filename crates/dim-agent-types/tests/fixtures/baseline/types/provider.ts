import type { Logger } from "@archships/dim-log"
import type { Message } from "./content"
import type { ToolDefinition } from "./tool"

export type ProviderOutputBudgetFacts = {
  inputTokens: number
  contextWindow: number
  requestedMaxTokens: number
  allowedMaxTokens: number
  ratio: number
}

export function parseProviderOutputBudgetFacts(
  value: unknown,
): ProviderOutputBudgetFacts | undefined {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return undefined
  }
  const facts = value as Record<string, unknown>
  const inputTokens = facts["inputTokens"]
  const contextWindow = facts["contextWindow"]
  const requestedMaxTokens = facts["requestedMaxTokens"]
  const allowedMaxTokens = facts["allowedMaxTokens"]
  const ratio = facts["ratio"]
  if (
    typeof inputTokens !== "number" || !Number.isFinite(inputTokens) || inputTokens < 0 ||
    typeof contextWindow !== "number" || !Number.isFinite(contextWindow) || contextWindow <= 0 ||
    typeof requestedMaxTokens !== "number" ||
    !Number.isFinite(requestedMaxTokens) || requestedMaxTokens <= 0 ||
    typeof allowedMaxTokens !== "number" ||
    !Number.isFinite(allowedMaxTokens) || allowedMaxTokens >= 1 ||
    typeof ratio !== "number" || !Number.isFinite(ratio) || ratio <= 0 || ratio > 1
  ) {
    return undefined
  }
  return {
    inputTokens,
    contextWindow,
    requestedMaxTokens,
    allowedMaxTokens,
    ratio,
  }
}

export function findProviderOutputBudgetFacts(
  error: unknown,
): ProviderOutputBudgetFacts | undefined {
  let current = error
  const seen = new Set<unknown>()
  for (let depth = 0; depth < 5; depth += 1) {
    if (current === null || typeof current !== "object" || seen.has(current)) {
      return undefined
    }
    seen.add(current)
    const record = current as Record<string, unknown>
    if (record["code"] === "PROVIDER_OUTPUT_BUDGET_EXHAUSTED") {
      return parseProviderOutputBudgetFacts(record["metadata"])
    }
    current = record["cause"]
  }
  return undefined
}

export type AuthConfig = {
  type: "api_key" | "bearer" | "oauth" | "custom" | "aws_bedrock"
  headerName?: string
  prefix?: string
}

export type Credential = {
  type: "apiKey" | "bearer" | "oauth" | "custom" | "awsBedrockAccessKey"
  apiKey?: string
  token?: string
  headerName?: string
  prefix?: string
  customHeaders?: Record<string, string>
  accessKeyId?: string
  secretAccessKey?: string
  sessionToken?: string
  baseUrl?: string
  headers?: Record<string, string>
  timeout?: number
}

export type CredentialResolverResult = {
  status: "available" | "missing" | "error"
  credential?: Credential
  hint?: string
}

export type CredentialResolutionRequest =
  | { reason: "request" }
  | { reason: "rejected"; rejectedAccessToken: string }
  | {
      reason: "terminal"
      authReason: "unauthorized_after_retry"
      rejectedAccessToken: string
    }

export type CredentialResolver = (
  providerId: string,
  driverKind: string,
  credentialScope: string,
  request?: CredentialResolutionRequest,
) => Promise<CredentialResolverResult>

export type Modality = "text" | "image" | "audio" | "video"

export type ModelCapability = {
  modalities: Modality[]
  toolCalling: boolean
  streaming: boolean
  structuredOutput: boolean
  vision: boolean
  audio: boolean
  reasoning: boolean
  contextWindow: number
  maxOutputTokens: number
  providerState?: Record<string, unknown>
}

export type ModelCapabilityOverrideField = "contextWindow" | "maxOutputTokens"

export type ModelStatus = "active" | "beta" | "deprecated" | "unavailable"

export type ModelMetadata = {
  description?: string
  releaseDate?: string
  pricing?: {
    inputTokenPrice?: number
    outputTokenPrice?: number
    currency?: string
  }
  rateLimit?: {
    requestsPerMinute?: number
    tokensPerMinute?: number
  }
  reasoning?: ModelReasoningMetadata
  serviceTier?: ModelServiceTierMetadata
  capabilityOverrideFields?: ModelCapabilityOverrideField[]
  raw?: Record<string, unknown>
}

export type ModelReasoningMetadata = {
  supported: boolean
  defaultEnabled?: boolean
  mode?: "effort" | "fixed" | "mixed" | "budget" | "level"
  effort?: string
  effortOptions?: string[]
  level?: string
  levelOptions?: string[]
  verbosity?: string
  verbosityOptions?: string[]
  interleaved?: boolean
  summaries?: boolean
  visibility?: "hidden" | "summary" | "full" | "omitted"
  continuation?: string[]
  budget?: {
    min?: number
    max?: number
    default?: number
    unit?: string
  }
  notes?: string[]
  off?: ModelReasoningOffMetadata
}

export type ModelReasoningOffMetadata =
  | { kind: "thinking_type"; value: "disabled" }
  | { kind: "thinking_budget"; value: 0 }
  | { kind: "enable_thinking"; value: false }
  | { kind: "chat_template_kwargs"; keys: ("thinking" | "enable_thinking")[] }

export type ModelServiceTierMetadata = {
  supported: boolean
  default?: ServiceTier
  options?: ServiceTier[]
}

export type ModelEntry = {
  modelId: string
  displayName: string
  status: ModelStatus
  capabilities: ModelCapability
  metadata?: ModelMetadata
}

export type ProviderMetadata = {
  adapter?: "openai" | "openai-responses" | "anthropic" | "gemini"
  customModels?: boolean
  defaultModelId?: string
  remoteModelList?: boolean
  reasoningToggle?: ProviderReasoningToggleMapping
  ccmock?: boolean
  mergeBuiltinModelDefaults?: boolean
  source?: "goatagent" | "public-provider-conf"
  description?: string
  website?: string
  apiDocsUrl?: string
  raw?: Record<string, unknown>
}

export type ProviderReasoningToggleMapping =
  | {
      field: "thinking.type"
      enabled: "enabled" | "adaptive"
      disabled: "disabled"
    }
  | {
      field: "enable_thinking"
      enabled: true
      disabled: false
    }

export type ProviderEntry = {
  providerId: string
  displayName: string
  driverKind: string
  defaultBaseUrl?: string
  auth: AuthConfig
  headers?: Record<string, string>
  models: ModelEntry[]
  capabilityDefaults?: ModelCapability
  metadata?: ProviderMetadata
}

export type ProviderCatalog = {
  updatedAt: string
  providers: ProviderEntry[]
}

export type Provider = ProviderEntry
export type ProviderConfig = ProviderEntry
export type Model = ModelEntry

export type ModelConfig = {
  temperature?: number
  maxTokens?: number
  topP?: number
  topK?: number
  stopSequences?: string[]
  reasoning?: ProviderReasoningConfig
  providerOptions?: ProviderOptions
}

export type KnownReasoningEffort =
  | "none"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "max"

export type ReasoningEffort = KnownReasoningEffort | (string & {})

export type KnownServiceTier =
  | "auto"
  | "flex"
  | "priority"
  | "default"

export type ServiceTier = KnownServiceTier | (string & {})

export type OpenAIProviderOptions = {
  reasoningEffort?: ReasoningEffort
  serviceTier?: ServiceTier
  summary?: "auto" | "concise" | "detailed"
  interleavedThinking?: boolean
}

export type OpenAICompatibleProviderOptions = {
  reasoningEffort?: ReasoningEffort
  serviceTier?: ServiceTier
  interleavedThinking?: boolean
  thinking?: {
    type?: "enabled" | "disabled"
  }
  enable_thinking?: boolean
}

export type AnthropicProviderOptions = {
  effort?: ReasoningEffort
  thinking?: {
    type?: "adaptive" | "enabled" | "disabled"
    budgetTokens?: number
  }
  sendReasoning?: boolean
  interleavedThinking?: boolean
}

export type DeepSeekProviderOptions = {
  reasoningEffort?: ReasoningEffort
  interleavedThinking?: boolean
}

export type XAIProviderOptions = {
  reasoningEffort?: ReasoningEffort
  interleavedThinking?: boolean
}

export type ZenmuxProviderOptions = {
  reasoningEffort?: ReasoningEffort
  interleavedThinking?: boolean
}

export type AIHubMixProviderOptions = {
  reasoningEffort?: ReasoningEffort
  interleavedThinking?: boolean
}

export type GeminiProviderOptions = {
  thinkingConfig?: {
    thinkingBudget?: number
    includeThoughts?: boolean
  }
}

export type MoonshotProviderOptions = {
  reasoningEffort?: ReasoningEffort
  interleavedThinking?: boolean
}

export type ProviderOptions = {
  openai?: OpenAIProviderOptions
  "openai-compatible"?: OpenAICompatibleProviderOptions
  anthropic?: AnthropicProviderOptions
  gemini?: GeminiProviderOptions
  deepseek?: DeepSeekProviderOptions
  moonshotai?: MoonshotProviderOptions
  xai?: XAIProviderOptions
  zenmux?: ZenmuxProviderOptions
  aihubmix?: AIHubMixProviderOptions
}

export type ProviderReasoningConfig = {
  enabled?: true
  effort?: ReasoningEffort
  budgetTokens?: number
  summary?: "auto" | "concise" | "detailed"
  includeThoughts?: boolean
  interleavedThinking?: boolean
}

export type ProviderInvocationConfig = ModelConfig

export type SystemPromptPart = {
  type: "text"
  text: string
  cache_control?: { type: "ephemeral" }
}

export type RequestMetadata = {
  sessionId?: string
  runId?: string
  traceId?: string
  turnId?: string
  [key: string]: unknown
}

export type ModelRequest = {
  providerId: string
  modelId: string
  messages: Message[]
  tools?: ToolDefinition[]
  system?: string | SystemPromptPart[]
  config?: ModelConfig
  metadata?: RequestMetadata
}

export interface ProviderDriver {
  readonly driverKind: string
  invokeModel(
    request: ModelRequest,
    credential: Credential,
    abortSignal?: AbortSignal,
  ): AsyncIterable<ProviderEvent>
}

export type ProviderDriverConfig = {
  provider: ProviderEntry
  logger?: Logger
}

export type ProviderDriverFactory = (config: ProviderDriverConfig) => ProviderDriver

export type ModelAvailability =
  | "available"
  | "provider_missing"
  | "model_missing"
  | "capability_missing"
  | "provider_error"

export type ProviderResolution = {
  providerId: string
  modelId: string
  provider?: ProviderEntry
  model?: ModelEntry
  capabilities?: ModelCapability
  driverKind?: string
  availability: ModelAvailability
  resolutionError?: string
  catalogSnapshotVersion: string
}

export type BaseProviderEvent = {
  eventType: string
  providerId: string
  modelId: string
}

export type ProviderMessageStartEvent = BaseProviderEvent & {
  eventType: "message_start"
  messageId: string
  role: "assistant"
}

export type MessageStartEvent = ProviderMessageStartEvent

export type ProviderContentDeltaEvent = BaseProviderEvent & {
  eventType: "content_delta"
  delta: string
}

export type ContentDeltaEvent = ProviderContentDeltaEvent

export type ProviderThinkingStartEvent = BaseProviderEvent & {
  eventType: "thinking_start"
}

export type ProviderThinkingEndEvent = BaseProviderEvent & {
  eventType: "thinking_end"
}

export type ProviderThinkingDeltaEvent = BaseProviderEvent & {
  eventType: "thinking_delta"
  delta: string
}

export type ProviderToolCallArgsDeltaEvent = BaseProviderEvent & {
  eventType: "tool_call_args_delta"
  toolCallId: string
  toolName: string
  delta: string
}

export type ProviderToolUseEvent = BaseProviderEvent & {
  eventType: "tool_use"
  toolCallId: string
  toolName: string
  toolInput: Record<string, unknown>
}

export type ToolUseEvent = ProviderToolUseEvent

export type StopReason =
  | "end_turn"
  | "max_tokens"
  | "stop_sequence"
  | "tool_use"

export type ProviderMessageStopEvent = BaseProviderEvent & {
  eventType: "message_stop"
  messageId: string
  stopReason: StopReason
}

export type MessageStopEvent = ProviderMessageStopEvent

export type ProviderUsageEvent = BaseProviderEvent & {
  eventType: "usage"
  usage: {
    /** Full input footprint, including cache reads and writes. */
    promptTokens: number
    completionTokens: number
    totalTokens: number
    cacheReadTokens?: number
    cacheWriteTokens?: number
    cost?: number
    currency?: string
  }
}

export type ProviderErrorEvent = BaseProviderEvent & {
  eventType: "error"
  error: Error
}

export type ErrorEvent = ProviderErrorEvent

export type ProviderEvent =
  | ProviderMessageStartEvent
  | ProviderContentDeltaEvent
  | ProviderThinkingStartEvent
  | ProviderThinkingDeltaEvent
  | ProviderThinkingEndEvent
  | ProviderToolCallArgsDeltaEvent
  | ProviderToolUseEvent
  | ProviderMessageStopEvent
  | ProviderUsageEvent
  | ProviderErrorEvent

export interface Transport {
  post(
    url: string,
    body: unknown,
    headers: Record<string, string>,
  ): AsyncIterable<TransportEvent>
}

export type TransportDataEvent = {
  type: "data"
  data: string
}

export type TransportErrorEvent = {
  type: "error"
  error: Error
  statusCode?: number
}

export type TransportDoneEvent = {
  type: "done"
}

export type TransportEvent =
  | TransportDataEvent
  | TransportErrorEvent
  | TransportDoneEvent
