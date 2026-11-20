import type { ChatUiMessage, ChatUiTimelineEntry } from '../model'
import type { ChatUiApproval, ChatUiQuestion } from './interactions'

export type ChatUiSessionStatus = 'loading' | 'ready' | 'failed' | 'closed'

/**
 * The normalized session view the renderer consumes. Each host adapter maps
 * its own projection into this shape; the package never reads a session store.
 */
export interface ChatUiSessionView {
  sessionId: string
  title?: string
  status: ChatUiSessionStatus
  messages: readonly ChatUiMessage[]
  timeline: readonly ChatUiTimelineEntry[]
  running: boolean
  stopping: boolean
  pendingApprovals: readonly ChatUiApproval[]
  pendingQuestions: readonly ChatUiQuestion[]
  error?: string
}
