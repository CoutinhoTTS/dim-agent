import type { Component, Ref } from 'vue'
import type { ChatUiMessage } from '../model'
import type {
  ChatUiRunChangedPathEntry,
  ChatUiTimelineMessageDetail,
} from '../timeline/user-message-rail'

export interface ChatUiTimelineMessageActionPayload<MessageT> {
  message: MessageT
  content: string
  onSubmitted?: () => void
}

export interface ChatUiTimelineMessageActions {
  canEdit: (messageId: string) => boolean
  canFork: (messageId: string) => boolean
  pending: Readonly<Ref<boolean>>
}

export interface ChatUiTimelineAgentModelInfo {
  providerId?: string
  modelId?: string
  externalAgentType?: string
}

/**
 * Per-timeline scope facts. A side chat can display a synthetic session key
 * while RPC reads still target the real parent session.
 */
export interface ChatUiTimelineScope {
  sessionId?: Readonly<Ref<string | null | undefined>>
  branchId?: Readonly<Ref<string | null>>
  detailSessionId?: Readonly<Ref<string | null>>
  restoreActive?: Readonly<Ref<boolean>>
}

export interface ChatUiTimelineSlots {
  userContent?: Component
  turnOutput?: Component
  markdown?: Component
  media?: Component
  toolResult?: Component
}

export interface ChatUiLatestRun {
  runId?: string
  anchorMessageId?: string
  changesRunId?: string
}

export interface ChatUiTimelineHooks {
  hasUserContent?: (messageId: string) => boolean
  hasToolMedia?: (callId: string) => boolean
  systemNoticeText?: (message: ChatUiMessage) => string | undefined
  isSystemNoticeMessage?: (message: ChatUiMessage) => boolean
  isNonRestPointInput?: (message: ChatUiMessage) => boolean
  getAgentModelLabel?: (info: ChatUiTimelineAgentModelInfo) => string | null
  resolveLatestRun?: (sessionId: string, messageId?: string) => ChatUiLatestRun | null
  getSubagentDurationMs?: (sessionId: string, taskId: string) => number | null
}

export interface ChatUiTimelineMessageDetailInput {
  sessionId: string
  messageId: string
  branchId?: string
}

export interface ChatUiTimelineRunChangedPathsInput {
  sessionId: string
  runId: string
  /** Caller-resolved run end sequence; omitted when the marker has none. */
  throughSequence?: number
}

export interface ChatUiTimelineIo {
  readTextAttachment?: (sessionId: string, path: string) => Promise<string>
  readTimelineMessageDetail?: (
    input: ChatUiTimelineMessageDetailInput,
  ) => Promise<ChatUiTimelineMessageDetail>
  /**
   * Run-finalized changed paths for one user-message marker. `null` means the
   * run is not finalized (or unknown) yet; the rail retries on the next hover.
   */
  readRunChangedPaths?: (
    input: ChatUiTimelineRunChangedPathsInput,
  ) => Promise<ChatUiRunChangedPathEntry[] | null>
}

export interface ChatUiTimelinePerf {
  start: () => number
  finish: (name: string, startedAt: number, data?: Record<string, unknown>) => void
  checkpoint: (name: string, data?: Record<string, unknown>) => void
}

export interface ChatUiTimelineEffects {
  openPlan?: () => void | Promise<void>
  notify?: {
    success: (message: string) => void
    error: (message: string) => void
  }
  visibility?: {
    fileChanges: Readonly<Ref<boolean>>
  }
  perf?: ChatUiTimelinePerf
}

/**
 * Optional timeline surface on `ChatUiHost`. Each field is additive so a host
 * can provide only the slices its timeline needs.
 */
export interface ChatUiTimelineHost {
  scope?: ChatUiTimelineScope
  actions?: ChatUiTimelineMessageActions
  slots?: ChatUiTimelineSlots
  hooks?: ChatUiTimelineHooks
  io?: ChatUiTimelineIo
  effects?: ChatUiTimelineEffects
}
