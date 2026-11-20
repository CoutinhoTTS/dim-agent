import type { Component } from 'vue'
import type { ChatUiMessageStatus } from '../model/chat-message'
import type { ChatUiTimelineBlockKind, ChatUiTimelineBlockStatus } from '../model/timeline'
import type { GeneratedImageResult } from '../timeline/generated-image-results'
import type { PlanDraftResult } from '../timeline/plan-draft-result'
import type {
  ChatUiBackgroundTaskNoticeStatus,
  ChatUiTimelineToolSummaryCategory,
} from './block-group'

export interface ChatUiAssistantMessageBlock {
  id: string
  kind: ChatUiTimelineBlockKind
  status: ChatUiTimelineBlockStatus
  content: string
  toolName?: string
  toolSource?: { kind?: string; displayMode?: 'inline' | 'fullscreen' }
  notificationMessage?: { metadata?: unknown }
  retryNotice?: unknown
  generatedImages?: readonly GeneratedImageResult[]
  planDraft?: PlanDraftResult
  statusType?: ChatUiMessageStatus
  startedAt?: Date | null
  endedAt?: Date | null
  durationMs?: number | null
}

export interface ChatUiTurnTerminalDebugDetails {
  status?: string
  statusCode?: number
  code?: string
  providerCode?: string
  providerUrl?: string
  modelId?: string
  message?: string
  responseBody?: string
  responseHeaders?: Readonly<Record<string, string>>
  requestId?: string
}

export interface ChatUiTurnTerminalState {
  status: 'stopped' | 'failed'
  reason: string
  message?: string
  code?: string
  displayCode?: string
  debugDetails?: ChatUiTurnTerminalDebugDetails
  presentation?: { action?: string }
  runtimeHost?: {
    platform: string
    arch: string
    appVersion: string
    osVersion: string
  }
  sourceMessageId: string
}

export interface ChatUiAssistantMessageTerminalDebugLine {
  key: string
  label: string
  value: string
}

export interface ChatUiAssistantMessageWorkSummaryInput {
  terminalStatus: 'error' | 'cancelled' | null
  actionsText: string
  durationText: string | null
  detailsExpanded: boolean
}

export interface ChatUiAssistantMessageLabels {
  runFailedPlaceholder: string
  userInterrupt: string
  backgroundTaskCommand: string
  backgroundTaskStatus: Readonly<Record<ChatUiBackgroundTaskNoticeStatus, string>>
  open: string
  copyMessage: string
  copyEntireMessage: string
  terminalErrorDetailsTitle: string
  copy: string
  planRenew: string
  workAction: Readonly<Record<ChatUiTimelineToolSummaryCategory, string>>
  workActionSeparator: string
  workActionMoreSuffix: string
  workSummary: (input: ChatUiAssistantMessageWorkSummaryInput) => string
}

export interface ChatUiAssistantMessageIcons {
  chevron: Component
  copy: Component
  check: Component
  externalLink: Component
  listChecks: Component
  bot: Component
  terminal: Component
  layers: Component
  triangleAlert: Component
  octagonPause: Component
}

export interface ChatUiAssistantMessageAgentInfo {
  agentName?: string
  providerId?: string
  modelId?: string
  externalAgentType?: string
}

export interface ChatUiAssistantMessagePerfMetrics {
  blockCount: number
  textLength: number
  status: string
  viewportPriority: boolean
}
