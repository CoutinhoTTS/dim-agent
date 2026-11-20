import type { Component } from 'vue'
import type { ChatUiToolCallInlineAgentInfo } from './tool-call-inline-summary'

/**
 * Structural tool-call block the shared card reads. Desktop `TimelineBlock`
 * and the package `ChatUiTimelineBlock` are both assignable to it.
 */
export interface ChatUiTimelineToolCallBlock {
  id: string
  kind?: string
  status: 'queued' | 'running' | 'complete' | 'error'
  content: string
  toolName?: string
  toolParams?: Readonly<Record<string, unknown>>
  toolInputText?: string
  toolResult?: unknown
  toolTarget?: string | null
  toolCallId?: string
  backgroundTaskNames?: Readonly<Record<string, string>>
  childCalls?: readonly ChatUiTimelineToolCallBlock[]
  startedAt?: Date | null
  endedAt?: Date | null
  durationMs?: number | null
}

/** `dimTruncated` marker the App Server appends to bounded results. */
export interface ChatUiTimelineToolResultTruncation {
  bytes: number
  previewBytes?: number
  fields?: readonly string[]
}

export interface ChatUiTimelineToolCallLabels {
  expandMessage: string
  collapse: string
  loading: string
  retry: string
  askUserTimedOut: (duration: string) => string
  askUserDeclined: string
}

/** Host icon components. The package stays independent of the icon library. */
export interface ChatUiTimelineToolCallIcons {
  chevron: Component
  todoCompleted: Component
  todoInProgress: Component
  todoPending: Component
}

export interface ChatUiTimelineToolCallOpenAgentPayload {
  subagentSessionId: string
  title: string
}

export interface ChatUiTimelineToolCallFullOutputRequest {
  messageId: string
}

/** Raw detail response. Its shape is host-owned; the card reads `message`. */
export type ChatUiTimelineToolCallFullOutputDetail = {
  message?: {
    content?: string
    toolResult?: unknown
  }
} | null | undefined

export type ChatUiTimelineToolCallAgentModelInfo = ChatUiToolCallInlineAgentInfo

export type ChatUiTimelineToolCallAgentModelLabel = (
  info: ChatUiTimelineToolCallAgentModelInfo,
) => string | null
