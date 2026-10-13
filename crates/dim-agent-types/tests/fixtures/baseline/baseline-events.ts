import type { CompactionDiagnosticsRecord, ModelRequest, ProviderEvent, ProviderOutputBudgetFacts, ToolCall, ToolResult } from "@archships/dim-types";

/** A timeline record. Its producer defines the meaning of type, name, and data. */
export interface TimelineEvent<Data> {
  readonly id: string;
  readonly type: string;
  readonly name: string;
  readonly tags?: readonly string[];
  readonly data: Data;
  /** Append order within the owning timeline. */
  readonly sequence: number;
  /** Creation time in Unix milliseconds. */
  readonly create_at: number;
}

/** The turn and model request that own a step. */
type StepData = {
  readonly turn_id: string;
  readonly request_id: string;
};

export interface InputMessageEvent extends TimelineEvent<{
  readonly text: string;
  /** Omitted by older producers: queue. */
  readonly mode?: "queue" | "steer" | "append";
}> {
  readonly type: "input";
  readonly name: "message";
}

/** InputQueue has selected this external input to take effect. */
export interface InputAppliedEvent extends TimelineEvent<{
  /** The original input/message event id. */
  readonly input_id: string;
}> {
  readonly type: "input";
  readonly name: "applied";
}

export interface TurnStartEvent extends TimelineEvent<{
  readonly turn_id: string;
}> {
  readonly type: "turn";
  readonly name: "start";
}

/** Enters the local pre-model phase; it does not confirm remote receipt. */
export interface ModelStartEvent extends TimelineEvent<StepData> {
  readonly type: "step";
  readonly name: "model-start";
}

/** Fixed source material for asynchronous request preparation. */
export interface ContextPrepareEvent extends TimelineEvent<StepData & {
  readonly through: number;
  readonly request: ModelRequest;
  readonly recovery?: ProviderOutputBudgetFacts;
}> {
  readonly type: "context";
  readonly name: "prepare";
}

export type ContextCompactionResult =
  | { status: "completed"; cursor: string; summary: string; diagnostics: CompactionDiagnosticsRecord }
  | { status: "skipped"; reason: string; diagnostics: CompactionDiagnosticsRecord }
  | { status: "failed"; error: string; diagnostics?: CompactionDiagnosticsRecord };

export interface ContextCompactionEvent extends TimelineEvent<StepData & ContextCompactionResult & {
  readonly through: number;
}> {
  readonly type: "context";
  readonly name: "compaction";
}

export interface ContextBudgetEvent extends TimelineEvent<StepData & {
  readonly boundary?: string;
  readonly estimatedInputTokens: number;
  readonly baselineSource: "provider_baseline" | "full_estimate";
  readonly baselinePromptTokens?: number;
  readonly deltaTokens?: number;
  readonly contextWindow: number;
  readonly outputTokens: number;
}> {
  readonly type: "context";
  readonly name: "budget";
}

/** Provider-normalized usage for this physical request, including cached input. */
export interface ModelUsageEvent extends TimelineEvent<StepData & {
  readonly usage: Extract<ProviderEvent, { eventType: "usage" }>["usage"];
}> {
  readonly type: "model";
  readonly name: "usage";
}

/** A complete request handed to ModelProvider, without execution resources. */
export interface ModelCallEvent extends TimelineEvent<StepData & {
  readonly request: ModelRequest;
}> {
  readonly type: "model";
  readonly name: "call";
}

export interface ThinkingEvent extends TimelineEvent<StepData & {
  /** Newly received content, never a cumulative snapshot. */
  readonly delta: string;
}> {
  readonly type: "step";
  readonly name: "thinking";
}

export interface TextEvent extends TimelineEvent<StepData & {
  /** Newly received content, never a cumulative snapshot. */
  readonly delta: string;
}> {
  readonly type: "step";
  readonly name: "text";
}

/** A complete model-proposed call; execution has not started. */
export interface ToolUseEvent extends TimelineEvent<StepData & {
  readonly call: ToolCall;
}> {
  readonly type: "step";
  readonly name: "tool-use";
}

/** Emitted after output validation or failure/cancellation finalization. */
export interface ModelOutputEndEvent extends TimelineEvent<StepData & (
  | {
      readonly status: "completed";
      readonly finish_reason: "end_turn" | "tool_use" | "stop_sequence";
    }
  | { readonly status: "failed"; readonly error: string; readonly budget?: ProviderOutputBudgetFacts }
  | { readonly status: "cancelled"; readonly reason: string }
)> {
  readonly type: "step";
  readonly name: "model-output-end";
}

/** An ordered batch approved by Loop for execution as one step. */
export interface ToolCallEvent extends TimelineEvent<StepData & {
  readonly calls: readonly ToolCall[];
}> {
  readonly type: "step";
  readonly name: "tool-call";
}

export interface ToolCallResult {
  readonly call_id: string;
  readonly result: ToolResult;
}

/** Optional execution progress; it does not complete the batch. */
export interface ToolStartEvent extends TimelineEvent<StepData & {
  readonly call_event_id: string;
  readonly call_id: string;
}> {
  readonly type: "step";
  readonly name: "tool-start";
}

export interface ToolResultEvent extends TimelineEvent<StepData & {
  /** The step/tool-call event that owns this completed batch. */
  readonly call_event_id: string;
  /** Exactly one result per call, in the original call order. */
  readonly results: readonly ToolCallResult[];
}> {
  readonly type: "step";
  readonly name: "tool-result";
}

/** Optional early result for observers; only tool-result completes the step. */
export interface ToolCompletedEvent extends TimelineEvent<StepData & ToolCallResult & {
  readonly call_event_id: string;
}> {
  readonly type: "step";
  readonly name: "tool-completed";
}

/** Emitted once the turn has finalized, including failure or cancellation. */
export interface TurnEndEvent extends TimelineEvent<{
  readonly turn_id: string;
} & (
  | { readonly status: "completed"; readonly reason: string }
  | { readonly status: "failed"; readonly error: string }
  | { readonly status: "cancelled"; readonly reason: string }
)> {
  readonly type: "turn";
  readonly name: "end";
}

export interface TaskStartedEvent extends TimelineEvent<StepData & {
  readonly task_id: string;
  readonly call_id: string;
  readonly tool: string;
  readonly wake: "auto" | "manual";
  readonly description?: string;
  readonly outputPath?: string;
}> {
  readonly type: "task";
  readonly name: "started";
}

export interface TaskEndedEvent extends TimelineEvent<StepData & {
  readonly task_id: string;
  readonly status: "completed" | "failed" | "cancelled";
  readonly result: ToolResult;
}> {
  readonly type: "task";
  readonly name: "ended";
}

/** Branch anchor on the forking timeline; pure tree metadata. */
export interface BranchCreateEvent extends TimelineEvent<StepData & {
  /** The new branch's timeline id. */
  readonly branch: string;
  /** Initial instruction handed to the child branch as its first input. */
  readonly prompt: string;
  /** Pairs with the step/tool-use that triggered this branch. */
  readonly call_id: string;
}> {
  readonly type: "branch";
  readonly name: "create";
}

/** First event of a child branch; the authoritative existence record. */
export interface BranchStartEvent extends TimelineEvent<{
  /** The parent branch identifier. */
  readonly parent: string;
  /** Sequence number of the branch/create event on the parent branch. */
  readonly createEvent: number;
}> {
  readonly type: "branch";
  readonly name: "start";
}

export type AgentEvent =
  | TaskStartedEvent
  | TaskEndedEvent
  | InputMessageEvent
  | InputAppliedEvent
  | TurnStartEvent
  | ModelStartEvent
  | ContextPrepareEvent
  | ContextCompactionEvent
  | ContextBudgetEvent
  | ModelCallEvent
  | ModelUsageEvent
  | ThinkingEvent
  | TextEvent
  | ToolUseEvent
  | ModelOutputEndEvent
  | ToolCallEvent
  | ToolStartEvent
  | ToolCompletedEvent
  | ToolResultEvent
  | TurnEndEvent
  | BranchCreateEvent
  | BranchStartEvent;

export type EventsByName = {
  [Event in AgentEvent as `${Event["type"]}/${Event["name"]}`]: Event;
};

/** Producer-owned fields, retaining the name/payload association for each event. */
export type DraftsByName = {
  [Name in keyof EventsByName]: Omit<EventsByName[Name], "id" | "sequence" | "create_at">;
};

export type TimelineDraft = DraftsByName[keyof DraftsByName];
