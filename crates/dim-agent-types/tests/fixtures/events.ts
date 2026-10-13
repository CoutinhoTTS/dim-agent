import type { AgentEvent } from './baseline-events.js';
import type { CompactionDiagnosticsRecord, ModelRequest, ToolResult } from '@archships/dim-types';

const objectInput = { nested: { enabled: true, amount: 2.5, absent: null }, values: [1, "二", false] };
const image = { type: "url" as const, mediaType: "image/png", url: "https://example.test/image.png" };
const blocks = [
  { type: "text" as const, text: "工具输出\nUnicode: 中文 🦀" },
  { type: "image" as const, source: image },
  { type: "image" as const, source: { type: "base64" as const, mediaType: "image/png", data: "AA==" } },
];
const call = { id: "call-1", name: "read", input: objectInput };
const step = { turn_id: "turn-1", request_id: "request-1" };
const budget = { inputTokens: 32768, contextWindow: 32768, requestedMaxTokens: 4096, allowedMaxTokens: -0.25, ratio: 0.9 };
export const diagnostics: CompactionDiagnosticsRecord = {
  reason: "auto", trigger: "provider_output_budget_exhausted", sourceRunId: "run-1", sourceTurnId: 1,
  baselineSource: "provider_baseline", baselinePromptTokens: 100, deltaTokens: -3.5, coverage: "partial_recent",
  candidateMessageCount: 8, inputMessageCount: 7, retainedMessageCount: 2, retainedTokenBudget: 256,
  retainedTokens: 200, serializedChars: 1024, estimatedInputTokens: 300.5, plannedOutputTokens: 128,
  minVisibleOutputTokens: 64, requestedThinkingBudgetTokens: 64, adjustedThinkingBudgetTokens: 32,
  thinkingBudgetClamped: true, reasoningEffortBudgetEstimateTokens: 12, effectivePlannedOutputTokens: 96,
  systemPromptTokens: 20, estimatedCompressedTokens: 40, estimatedPostCompactionTokens: 240,
  summaryInputBudget: 400, chunkTargetInputBudget: 300, maxChunkInputTokens: 250,
  summaryRequestContextRatio: 0.75, requestEnvelopeMargin: 16, budgetEstimateSource: "actual",
  compactionReasoningEffort: "custom-effort", preflightInputTokens: 200, preflightOutputTokens: 96,
  compactedToolPayloadCount: 1, droppedMessageCount: 3, tailTruncatedMessageId: "message-1",
  chunkCount: 2, recursiveDepth: 1, summaryChars: 80, estimatedSummaryTokens: 20, modelCallCount: 2,
  usage: { inputTokens: 200, outputTokens: 30 }, failureCode: "budget", fallbackReason: "chunk_overflow",
  omittedTokenEstimate: 15, durationMs: 1.25,
};
export const result: ToolResult = {
  content: blocks, structuredContent: objectInput, isError: false,
  blobRefs: ["text", "binary", "file", "stream", "remote"].map((kind) => ({
    blobId: `blob-${kind}`, sessionId: "session-1", kind: kind as "text" | "binary" | "file" | "stream" | "remote",
    path: `/tmp/${kind}`, ref: "opaque-ref", mimeType: "text/plain", sizeBytes: 23,
  })),
  backgroundTask: { taskId: "task-1", wakePolicy: "auto", outputPath: "/tmp/output", description: "working" },
  contextMessages: [{ role: "user", content: "Context retained" }],
};
export const request: ModelRequest = {
  providerId: "fixture-provider", modelId: "fixture-model",
  messages: [
    { role: "user", content: "Hello 中文" },
    { role: "assistant", content: [
      { type: "text", text: "Reply", startTime: "2026-09-20T01:02:03.004Z", endTime: "2026-09-20T01:02:04Z" },
      { type: "thinking", thinking: "Reasoning", startTime: "2026-09-20T01:02:00+00:00", endTime: "2026-09-20T01:02:02Z" },
      { type: "tool_use", ...call, startTime: "2026-09-20T01:02:05Z" },
      { type: "image", source: image },
    ] },
    { role: "tool_result", content: [
      { type: "tool_result", tool_use_id: "call-1", content: blocks, structuredContent: objectInput, is_error: false },
      { type: "tool_result", tool_use_id: "call-2", content: "plain result", is_error: true },
    ] },
    { role: "assistant", content: [{ type: "text", text: "no optional times" }, { type: "thinking", thinking: "no times" }] },
  ],
  tools: ["static", "builtin", "extension", "dynamic"].map((source) => ({
    name: `tool-${source}`, description: "Description", source: source as "static" | "builtin" | "extension" | "dynamic",
    risk: "low", metadata: { version: "1", author: "author", docsUrl: "https://example.test/docs", sourcePluginId: "plugin",
      mcpServerId: "mcp", mcpOriginalToolName: "original", custom: { vendor: true } },
    inputSchema: { type: "object", properties: { path: { type: "string", description: "path" },
      count: { type: "number" }, enabled: { type: "boolean" }, nil: { type: "null" },
      list: { type: "array", items: { type: "string", enum: ["a", "b"] } } },
      required: ["path"], additionalProperties: false },
    fileChanges: { preExecutePaths: { selector: "path" }, postExecutePaths: "affectedPaths" },
  })),
  system: [{ type: "text", text: "System", cache_control: { type: "ephemeral" } }, { type: "text", text: "Uncached" }],
  config: {
    temperature: 0.7, maxTokens: 1000, topP: 0.95, topK: 20, stopSequences: ["STOP"],
    reasoning: { enabled: true, effort: "future-effort", budgetTokens: 50, summary: "auto", includeThoughts: true, interleavedThinking: false },
    providerOptions: {
      openai: { reasoningEffort: "xhigh", serviceTier: "future-tier", summary: "detailed", interleavedThinking: true },
      "openai-compatible": { reasoningEffort: "low", serviceTier: "flex", interleavedThinking: false, thinking: { type: "disabled" }, enable_thinking: false },
      anthropic: { effort: "high", thinking: { type: "adaptive", budgetTokens: 32 }, sendReasoning: true, interleavedThinking: true },
      gemini: { thinkingConfig: { thinkingBudget: 24, includeThoughts: true } },
      deepseek: { reasoningEffort: "max", interleavedThinking: true }, moonshotai: { reasoningEffort: "medium", interleavedThinking: false },
      xai: { reasoningEffort: "low", interleavedThinking: true }, zenmux: { reasoningEffort: "high", interleavedThinking: true },
      aihubmix: { reasoningEffort: "none", interleavedThinking: false },
    },
  },
  metadata: { sessionId: "session-1", runId: "run-1", traceId: "trace-1", turnId: "turn-1", vendor: objectInput },
};

type Draft = AgentEvent extends infer E ? E extends AgentEvent ? Omit<E, "id" | "sequence" | "create_at"> : never : never;
const drafts = [
  { type: "input", name: "message", data: { text: "legacy default mode" } },
  { type: "input", name: "message", tags: [], data: { text: "queue", mode: "queue" } },
  { type: "input", name: "message", tags: ["external", "中文"], data: { text: "steer", mode: "steer" } },
  { type: "input", name: "message", data: { text: "append", mode: "append" } },
  { type: "input", name: "applied", data: { input_id: "event-1" } },
  { type: "turn", name: "start", data: { turn_id: "turn-1" } },
  { type: "step", name: "model-start", data: step },
  { type: "context", name: "prepare", data: { ...step, through: 7, request, recovery: budget } },
  { type: "context", name: "prepare", data: { ...step, through: 7, request: { providerId: "p", modelId: "m", messages: [] } } },
  { type: "context", name: "compaction", data: { ...step, through: 7, status: "completed", cursor: "event-7", summary: "summary", diagnostics } },
  { type: "context", name: "compaction", data: { ...step, through: 7, status: "skipped", reason: "small", diagnostics: { ...diagnostics, reason: "manual", trigger: "manual", coverage: "full", budgetEstimateSource: "estimated", fallbackReason: "empty_summary" } } },
  { type: "context", name: "compaction", data: { ...step, through: 7, status: "failed", error: "network" } },
  { type: "context", name: "compaction", data: { ...step, through: 7, status: "failed", error: "empty", diagnostics: { ...diagnostics, trigger: "threshold", fallbackReason: "budget_preflight_empty" } } },
  { type: "context", name: "budget", data: { ...step, boundary: "event-7", estimatedInputTokens: 20.5, baselineSource: "provider_baseline", baselinePromptTokens: 20, deltaTokens: 0.5, contextWindow: 1000, outputTokens: 100 } },
  { type: "context", name: "budget", data: { ...step, estimatedInputTokens: 20, baselineSource: "full_estimate", contextWindow: 1000, outputTokens: 100 } },
  { type: "model", name: "call", data: { ...step, request } },
  { type: "model", name: "call", data: { ...step, request: { providerId: "p", modelId: "m", messages: [], system: "plain", tools: [], config: {}, metadata: {} } } },
  { type: "model", name: "usage", data: { ...step, usage: { promptTokens: 100, completionTokens: 20, totalTokens: 120, cacheReadTokens: 30, cacheWriteTokens: 10, cost: 0.001, currency: "USD" } } },
  { type: "model", name: "usage", data: { ...step, usage: { promptTokens: 0, completionTokens: 0, totalTokens: 0 } } },
  { type: "step", name: "thinking", data: { ...step, delta: "Think" } },
  { type: "step", name: "text", data: { ...step, delta: "Hello 🦀" } },
  { type: "step", name: "tool-use", data: { ...step, call } },
  { type: "step", name: "model-output-end", data: { ...step, status: "completed", finish_reason: "end_turn" } },
  { type: "step", name: "model-output-end", data: { ...step, status: "completed", finish_reason: "tool_use" } },
  { type: "step", name: "model-output-end", data: { ...step, status: "completed", finish_reason: "stop_sequence" } },
  { type: "step", name: "model-output-end", data: { ...step, status: "failed", error: "budget", budget } },
  { type: "step", name: "model-output-end", data: { ...step, status: "failed", error: "other" } },
  { type: "step", name: "model-output-end", data: { ...step, status: "cancelled", reason: "user" } },
  { type: "step", name: "tool-call", data: { ...step, calls: [call, { id: "call-2", name: "write", input: {} }] } },
  { type: "step", name: "tool-start", data: { ...step, call_event_id: "batch-1", call_id: "call-1" } },
  { type: "step", name: "tool-completed", data: { ...step, call_event_id: "batch-1", call_id: "call-1", result } },
  { type: "step", name: "tool-result", data: { ...step, call_event_id: "batch-1", results: [{ call_id: "call-1", result }, { call_id: "call-2", result: { content: "Error", isError: true } }] } },
  { type: "turn", name: "end", data: { turn_id: "turn-1", status: "completed", reason: "done" } },
  { type: "turn", name: "end", data: { turn_id: "turn-1", status: "failed", error: "failure" } },
  { type: "turn", name: "end", data: { turn_id: "turn-1", status: "cancelled", reason: "stop" } },
  { type: "task", name: "started", data: { ...step, task_id: "task-1", call_id: "call-1", tool: "exec", wake: "auto", description: "work", outputPath: "/tmp/output" } },
  { type: "task", name: "started", data: { ...step, task_id: "task-2", call_id: "call-2", tool: "exec", wake: "manual" } },
  { type: "task", name: "ended", data: { ...step, task_id: "task-1", status: "completed", result: { content: "done" } } },
  { type: "task", name: "ended", data: { ...step, task_id: "task-2", status: "failed", result: { content: [], isError: true } } },
  { type: "task", name: "ended", data: { ...step, task_id: "task-3", status: "cancelled", result: { content: "stopped", backgroundTask: { taskId: "task-3", wakePolicy: "manual" } } } },
  { type: "branch", name: "create", data: { ...step, branch: "child", prompt: "Investigate", call_id: "call-1" } },
  { type: "branch", name: "start", data: { parent: "main", createEvent: 41 } },
] satisfies Draft[];

export const events = drafts.map((draft, i) => ({
  ...draft, id: `event-${i + 1}`, sequence: i + 1, create_at: 1_790_000_000_123,
})) satisfies AgentEvent[];
