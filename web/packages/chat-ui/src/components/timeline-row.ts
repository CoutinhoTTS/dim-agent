import type { ChatUiAttachment, ChatUiMessage } from '../model/chat-message'
import type { ChatUiAssistantMessageBlock, ChatUiTurnTerminalState } from './assistant-message'

/**
 * Message shape the row needs from the host projection. `orderKey` carries the
 * App Server sequence used for fork cuts; desktop and web both have it.
 */
export interface ChatUiTimelineRowMessage extends ChatUiMessage {
  orderKey?: string
}

/**
 * Block shape the row forwards to the assistant renderer and notification
 * callbacks. Only `notificationMessage` is narrowed to the row message type so
 * hosts can resolve transcript notices from the source message.
 */
export interface ChatUiTimelineRowBlock extends ChatUiAssistantMessageBlock {
  notificationMessage?: ChatUiTimelineRowMessage
}

/**
 * Turn shape rendered by the row. It stays a structural superset of the
 * `ChatUiTimelineTurn` model: blocks carry assistant-message details, while
 * fork/terminal/run metadata belongs to the row's footer and fallback paths.
 */
export interface ChatUiTimelineRowTurn {
  id: string
  userMessage: ChatUiTimelineRowMessage | null
  messageIds: readonly string[]
  forkAnchorMessageId: string | null
  /** `turn/end` sequence used when forking after this completed turn. */
  forkAfterSequence?: number | null
  blocks: readonly ChatUiTimelineRowBlock[]
  model?: string
  canShowWorkDuration: boolean
  workDurationMs: number | null
  turnStartedAt: Date | null
  turnEndedAt: Date | null
  isRunning: boolean
  /** Terminal state from a run-terminal / legacy user-interrupt message. */
  terminal?: ChatUiTurnTerminalState
}

/**
 * UI-owned display row. The row only distinguishes the divider kind; hosts
 * keep the payload and pass the same object through to their divider widget.
 */
export type ChatUiTimelineDisplayRow =
  | { kind: 'context_compression' }
  | { kind: 'window_truncation' }

export type ChatUiTimelineRow =
  | {
      kind: 'turn'
      key: string
      turn: ChatUiTimelineRowTurn
      runningStartedAt?: Date | null
    }
  | {
      kind: 'message'
      key: string
      message: ChatUiTimelineRowMessage
    }
  | {
      kind: 'display'
      key: string
      row: ChatUiTimelineDisplayRow
    }
  | {
      kind: 'running'
      key: string
      startedAt: Date | null
    }

/** A file clicked in the timeline: opens (or re-targets) the preview panel. */
export interface ChatUiTimelinePreviewTarget {
  path: string
  /** 1-indexed line to bring into view; `read` rows pass their offset. */
  startLine?: number
}

/** Fork cut emitted from a user message or the assistant footer. */
export interface ChatUiTimelineForkMessage {
  id: string
  role: string
  content?: string
  attachments?: readonly ChatUiAttachment[]
  metadata?: Readonly<Record<string, unknown>>
  sequence?: number
  /** Source message for user forks; hosts use it to rebuild the cut. */
  message?: ChatUiTimelineRowMessage
}
