import type { ChatUiMessage, ChatUiMessageStatus } from './chat-message'
import type { GeneratedImageResult } from '../timeline/generated-image-results'
import type { PlanDraftResult } from '../timeline/plan-draft-result'

export type ChatUiTimelineBlockKind =
  | 'media'
  | 'thinking'
  | 'tool'
  | 'text'
  | 'notification'
  | 'retry'
  | 'status'
  | 'generated-image'
  | 'plan-draft'

export type ChatUiTimelineBlockStatus = 'queued' | 'running' | 'complete' | 'error'

export interface ChatUiTimelineBlock {
  id: string
  kind: ChatUiTimelineBlockKind
  status: ChatUiTimelineBlockStatus
  content: string
  toolName?: string
  toolParams?: Readonly<Record<string, unknown>>
  toolResult?: unknown
  toolSource?: unknown
  toolTarget?: string | null
  toolCallId?: string
  backgroundTaskNames?: Readonly<Record<string, string>>
  notificationMessage?: ChatUiMessage
  retryNotice?: unknown
  generatedImages?: readonly GeneratedImageResult[]
  planDraft?: PlanDraftResult
  affectedPaths?: readonly string[]
  childCalls?: readonly ChatUiTimelineBlock[]
  runId?: string
  instanceId?: string
  protocolId?: string
  inputText?: string
  toolInputText?: string
  toolInputComplete?: boolean
  startedAt?: Date
  endedAt?: Date
  durationMs?: number
  statusType?: ChatUiMessageStatus
}

export interface ChatUiTimelineTurn {
  id: string
  userMessage: ChatUiMessage | null
  messageIds: readonly string[]
  forkAnchorMessageId?: string | null
  forkAfterSequence?: number | null
  blocks: readonly ChatUiTimelineBlock[]
  model?: string
  canShowWorkDuration?: boolean
  workDurationMs?: number | null
  turnStartedAt?: Date | null
  turnEndedAt?: Date | null
  isRunning: boolean
  terminal?: 'completed' | 'failed' | 'cancelled'
}

export type ChatUiTimelineEntry =
  | { kind: 'turn'; turn: ChatUiTimelineTurn; key?: string }
  | { kind: 'message'; message: ChatUiMessage; key?: string }
