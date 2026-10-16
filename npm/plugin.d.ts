/** Named services implemented by Dim's App Server. No runtime code is exported. */
export type FileError = 'closed' | 'exists' | 'invalid_data' | 'invalid_path' | 'internal' | 'io';
export type FileResult<T> = { ok: true; value: T } | { ok: false; error: FileError };

export interface FileLease {
  read(): Promise<FileResult<string | null>>;
  replace(text: string): Promise<FileResult<null>>;
  backup(suffix: string): Promise<FileResult<null>>;
}

export interface FileScope {
  withLock<T>(path: string, work: (file: FileLease) => T | Promise<T>, cancelled?: Promise<void>): Promise<FileResult<T>>;
  quiesce(): null;
  close(): Promise<null>;
}

export interface PluginFiles {
  open(): Promise<FileScope>;
}

export type JsonValue = null | boolean | number | string | JsonValue[] | JsonObject;
export interface JsonObject { [key: string]: JsonValue }

export interface ModelId { providerId: string; modelId: string }
export type PermissionPreset = 'read-only' | 'workspace-write' | 'full-access';
export interface PermissionPolicy {
  preset: PermissionPreset;
  allowedTools: string[];
  disallowedTools: string[];
}

export interface ImageSource {
  type: 'base64' | 'url';
  mediaType: string;
  data?: string;
  url?: string;
}
export type ContentBlock = { type: 'text'; text: string } | { type: 'image'; source: ImageSource };
export interface SessionRef { sessionId: string; branchId: string }
export interface LaunchRequest {
  agent?: string | null;
  cwd: string;
  model?: ModelId | null;
  policy?: Partial<PermissionPolicy> | null;
  starter?: SessionRef | null;
}
export interface LaunchReply { sessionId: string; policy: PermissionPolicy }
export interface RawBlock {
  text: string;
  key?: { use: 'live' | 'retire'; name: string } | null;
}
export interface InputWrite extends SessionRef {
  mode: 'queue' | 'steer' | 'append';
  submissionId: string;
  rawBlocks: RawBlock[];
  content: ContentBlock[];
}
export interface InputReceipt { inputId: string; sequence: number }
export interface LookupRequest extends SessionRef { submissionId: string }
export type LookupReply = { status: 'accepted'; inputId: string; sequence: number } | { status: 'unknown' };
export interface HistoryRequest extends SessionRef {
  after?: number | null;
  through?: number | null;
  atLeast?: number | null;
  limit?: number | null;
}
/** Current native event envelope; each event owner defines its data fields. */
export interface AgentEvent {
  id: string;
  sequence: number;
  create_at: number;
  tags?: string[];
  type: string;
  name: string;
  data: JsonObject;
}
export interface HistoryPage { through: number; events: AgentEvent[] }
export interface WithdrawRequest extends SessionRef { inputId: string; reason?: string | null }
export type WithdrawReply = { status: 'deleted'; sequence: number } | { status: 'notHeld' };
export interface StopRequest extends SessionRef { reason?: string | null }
export interface SequenceReply { sequence: number }
export type SessionsErrorCode = 'rejected' | 'agent_unknown' | 'session_closed' | 'branch_missing' |
  'reloading' | 'future_sequence' | 'ambiguous' | 'publication_unknown' | 'failed';
export interface SessionsError { code: SessionsErrorCode; message: string; retryable: boolean }
export type SessionsResult<T> = { ok: true; value: T } | { ok: false; error: SessionsError };

/** Process service `sessions`; these methods are not terminal RPC methods. */
export interface Sessions {
  launch(request: LaunchRequest): Promise<SessionsResult<LaunchReply>>;
  input(request: InputWrite): Promise<SessionsResult<InputReceipt>>;
  lookup(request: LookupRequest): Promise<SessionsResult<LookupReply>>;
  history(request: HistoryRequest): Promise<SessionsResult<HistoryPage>>;
  withdraw(request: WithdrawRequest): Promise<SessionsResult<WithdrawReply>>;
  stop(request: StopRequest): Promise<SessionsResult<SequenceReply>>;
  idle(request: SessionRef): Promise<SessionsResult<SequenceReply>>;
  delete(request: { sessionId: string }): Promise<SessionsResult<Record<string, never>>>;
}
