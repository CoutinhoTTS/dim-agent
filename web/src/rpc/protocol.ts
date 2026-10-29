/**
 * App Server protocol v3: the methods and notifications the web workspace uses.
 *
 * Hand-written from `crates/dim-app-server/src/protocol.rs` and
 * `tree_protocol.rs`. Values of these types come only from the readers in
 * `read.ts`; the rest of the app uses them as they are.
 */

export const PROTOCOL_VERSION = 3

/** The Session's main line; other branches are its lines. */
export const MAIN_BRANCH = "main"

export type ModelId = { providerId: string; modelId: string }

export type HelloResult = {
  instanceId: string
  serverVersion: string
  protocolVersion: number
  /** Methods and capabilities this connection may use (v3 always sends the list). */
  features: string[]
}

/** `session.summaries`: SessionInfo flattened with the summary fields. */
export type SessionSummary = {
  sessionId: string
  cwd: string
  model: ModelId
  /** Unix milliseconds. */
  createdAt: number
  running: boolean
  sequence: number
  preview?: string
  /** Unix milliseconds of the last main-branch event. */
  updatedAt: number
}

export type CreateResult = { sessionId: string; model: ModelId; sequence: number }

export type HistoryPage = {
  events: SessionEvent[]
  through: number
  nextAfter: number
  done: boolean
}

export type ModelUpdateResult = { model: ModelId; outcome: string }

/** `provider.list` entry; the reply carries no credentials. */
export type ProviderSummary = {
  providerId: string
  displayName: string
  enabled: boolean
  credentialConfigured: boolean
}

export type ProviderModel = {
  id: string
  name: string
  /** Set when the model cannot be selected right now. */
  unavailableReason?: string
}

export type ProviderCatalog = {
  models: ProviderModel[]
  disabledModelIds: string[]
}

export type ApprovalDecision = "allow" | "deny"

export type BranchSource =
  | { kind: "tool"; turnId: string; requestId: string; callId: string }
  | { kind: "host"; operationId: string }

export type ToolResult = {
  /** Text the model read; image blocks are dropped at the boundary. */
  text: string
  isError: boolean
  /** The edit tool's display diff (`structuredContent.diff`). */
  diff?: string
}

/** The Timeline facts the projection folds; every other kind is `other`. */
export type EventContent =
  | { kind: "input"; text?: string }
  | { kind: "source-input"; text: string; mode?: string }
  | { kind: "input-deleted"; inputId: string }
  | { kind: "branch-create"; branchId: string; at: number | undefined; prompt: string; source: BranchSource }
  | { kind: "branch-create-failed"; branchId: string; status: "failed" | "cancelled"; error: string }
  | { kind: "branch-end"; source: BranchSource; ancestor?: string }
  | { kind: "branch-ended"; branchId: string; source: BranchSource; ancestor?: string }
  | { kind: "turn-end"; outcome: "completed" | "failed" | "cancelled"; error?: string }
  | { kind: "model-start"; requestId: string }
  | { kind: "thinking"; requestId: string; delta: string }
  | { kind: "text"; requestId: string; delta: string }
  | { kind: "tool-use"; requestId: string; callId: string; name: string; input: Record<string, unknown> }
  | { kind: "tool-completed"; callId: string; result: ToolResult }
  | { kind: "tool-result"; results: Array<{ callId: string; result: ToolResult }> }
  | { kind: "model-end"; requestId: string; error?: string }
  | { kind: "context-configured"; model: ModelId }
  | { kind: "other" }

export type SessionEvent = {
  id: string
  sequence: number
  /** Unix milliseconds the runtime wrote the event at. */
  createAt: number
  content: EventContent
}

/** `session.branch_create`: the new line and where it was cut. */
export type BranchCreateResult = {
  branchId: string
  parentBranchId?: string
  branchPoint?: number
  createEvent?: number
}


/** Model output of a request not written to the Timeline yet; each notice replaces the last. */
export type ModelProgress = {
  sessionId: string
  branchId: string
  requestId?: string
  text: string
  /** Unicode scalar values of the request's `step/text` deltas before `text`. */
  textOffset: number
  thinking: string
  /** Unicode scalar values of the request's `step/thinking` deltas before `thinking`. */
  thinkingOffset: number
}

export type ApprovalRequest = {
  sessionId: string
  branchId: string
  requestId: string
  /** The tool call that asks, when a tool asks. */
  callId?: string
  call: { name: string; input: Record<string, unknown> }
  reason: string
}

export type Notification =
  | { method: "session.event"; sessionId: string; branchId: string; event: SessionEvent }
  | { method: "session.state"; sessionId: string; branchId: string; running: boolean; stopping: boolean }
  | { method: "session.model.progress"; progress: ModelProgress }
  | { method: "approval.request"; request: ApprovalRequest }
  | { method: "approval.cancelled"; sessionId: string; branchId: string; requestId: string }
  | { method: "session.closed"; sessionId: string; reason?: string }
