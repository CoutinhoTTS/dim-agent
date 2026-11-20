export type ChatUiMessageRole = 'user' | 'assistant' | 'system' | 'tool' | 'thinking' | 'divider'
export type ChatUiMessageStatus = 'sending' | 'streaming' | 'complete' | 'error' | 'cancelled'
export type ChatUiAttachmentType = 'image' | 'video' | 'audio' | 'file'

export interface ChatUiAttachment {
  id: string
  name: string
  type: ChatUiAttachmentType
  size: number
  mime?: string
  url?: string
  absolutePath?: string
}

export interface ChatUiMessageEvent {
  id: string
  type: string
  status: string
  startTime: number
  endTime?: number
  content?: string
  toolName?: string
  toolParams?: Readonly<Record<string, unknown>>
  toolResult?: unknown
  toolSource?: unknown
  affectedPaths?: readonly string[]
  childCalls?: readonly ChatUiMessageEvent[]
}

/**
 * The initial normalized message shape. The desktop `Message` and the web
 * `TimelineRow` projection both map into it; timeline-specific blocks live in
 * `timeline.ts`.
 */
export interface ChatUiMessage {
  id: string
  /** App Server ordering key; preserved for fork/retry sequence derivation. */
  orderKey?: string
  role: ChatUiMessageRole
  content: string
  status: ChatUiMessageStatus
  createdAt: Date
  updatedAt: Date
  events?: readonly ChatUiMessageEvent[]
  attachments?: readonly ChatUiAttachment[]
  model?: string
  metadata?: Readonly<Record<string, unknown>>
  toolCallId?: string
  toolName?: string
  toolParams?: Readonly<Record<string, unknown>>
  toolResult?: unknown
  toolSource?: unknown
  affectedPaths?: readonly string[]
  childCalls?: readonly ChatUiMessage[]
}
