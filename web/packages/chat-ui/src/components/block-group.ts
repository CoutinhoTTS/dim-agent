import type { Component } from 'vue'
import type { ChatUiTimelineBlockKind, ChatUiTimelineBlockStatus } from '../model/timeline'

export type ChatUiTimelineToolSummaryCategory =
  | 'read'
  | 'edit'
  | 'command'
  | 'search'
  | 'webSearch'
  | 'webFetch'
  | 'subagent'
  | 'browser'
  | 'computerUse'
  | 'wait'
  | 'skill'
  | 'unknown'

export type ChatUiTimelineBlockGroupSummaryItemKey =
  | ChatUiTimelineToolSummaryCategory
  | 'thinking'
  | 'notifications'

export interface ChatUiTimelineBlockGroupSummaryItem {
  key: ChatUiTimelineBlockGroupSummaryItemKey
  count: number
  separator: string
}

export type ChatUiBackgroundTaskNoticeStatus = 'completed' | 'failed' | 'cancelled'

export interface ChatUiBackgroundTaskNotice {
  taskId: string
  status: ChatUiBackgroundTaskNoticeStatus
  outputPath?: string
  outputBasename?: string
  toolName?: string
  label?: string
  summary?: string
  agentName?: string
  taskTitle?: string
  providerId?: string
  modelId?: string
  externalAgentType?: string
  childSessionId?: string
}

/** Minimal block shape the shared work-group renderer owns. */
export interface ChatUiTimelineBlockGroupBlock {
  id: string
  kind: ChatUiTimelineBlockKind
  status: ChatUiTimelineBlockStatus
  content: string
  toolName?: string
  toolParams?: Readonly<Record<string, unknown>>
  toolResult?: unknown
  toolTarget?: string | null
  toolSource?: { kind?: string }
  /** Pass-through metadata used to read background-task notices. */
  notificationMessage?: { metadata?: unknown }
  startedAt?: Date | null
  endedAt?: Date | null
  durationMs?: number | null
}

export interface ChatUiTimelineSubagentTarget {
  subagentSessionId: string
  title: string
}

export interface ChatUiTimelineBlockGroupLabels {
  backgroundTaskCommand: string
  open: string
  separator: string
  workActionSeparator: string
  backgroundTaskStatus: Readonly<Record<ChatUiBackgroundTaskNoticeStatus, string>>
}

export interface ChatUiTimelineBlockGroupIcons {
  workflow: Component
  chevron: Component
  bot: Component
  terminal: Component
  layers: Component
}
