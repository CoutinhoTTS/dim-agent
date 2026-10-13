export type CompactionCoverage = "full" | "partial_recent"

export type CompactionFallbackReason =
  | "chunk_overflow"
  | "empty_summary"
  | "budget_preflight_empty"

export type CompactionDiagnosticsRecord = {
  reason: "auto" | "manual"
  trigger?:
    | "threshold"
    | "provider_output_budget_exhausted"
    | "manual"
  sourceRunId?: string
  sourceTurnId?: number
  /* RFC §6.5: decision source of estimatedInputTokens. */
  baselineSource?: "provider_baseline" | "full_estimate"
  baselinePromptTokens?: number
  deltaTokens?: number
  coverage?: CompactionCoverage
  candidateMessageCount: number
  inputMessageCount: number
  retainedMessageCount: number
  retainedTokenBudget: number
  retainedTokens: number
  serializedChars: number
  estimatedInputTokens: number
  plannedOutputTokens?: number
  minVisibleOutputTokens?: number
  requestedThinkingBudgetTokens?: number
  adjustedThinkingBudgetTokens?: number
  thinkingBudgetClamped?: boolean
  reasoningEffortBudgetEstimateTokens?: number
  effectivePlannedOutputTokens?: number
  systemPromptTokens?: number
  estimatedCompressedTokens: number
  estimatedPostCompactionTokens: number
  summaryInputBudget: number
  chunkTargetInputBudget?: number
  maxChunkInputTokens?: number
  summaryRequestContextRatio?: number
  requestEnvelopeMargin?: number
  budgetEstimateSource?: "actual" | "estimated"
  compactionReasoningEffort?: string
  preflightInputTokens: number
  preflightOutputTokens: number
  compactedToolPayloadCount: number
  droppedMessageCount: number
  tailTruncatedMessageId?: string
  chunkCount: number
  recursiveDepth?: number
  summaryChars: number
  estimatedSummaryTokens: number
  modelCallCount: number
  usage?: {
    inputTokens?: number
    outputTokens?: number
  }
  failureCode?: string
  fallbackReason?: CompactionFallbackReason
  omittedTokenEstimate?: number
  durationMs: number
}
