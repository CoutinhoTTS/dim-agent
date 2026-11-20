import type { ChatUiTimelineBlockStatus } from '../model/timeline'

/** Minimal tool block view used by artifact/result extractors. */
export interface ChatUiToolResultBlock {
  id: string
  status: ChatUiTimelineBlockStatus
  content: string
  toolName?: string
  toolParams?: Readonly<Record<string, unknown>>
  toolResult?: unknown
}
