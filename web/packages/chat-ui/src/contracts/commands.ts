import type { ChatUiAttachment } from '../model'

export type ChatUiInputMode = 'queue' | 'steer' | 'append'
export type ChatUiApprovalDecision = 'allow' | 'deny'

export interface ChatUiSendInput {
  text: string
  attachments?: readonly ChatUiAttachment[]
  mode?: ChatUiInputMode
  branchId?: string
}

export interface ChatUiAnswer {
  questionId: string
  answer: string
}

/**
 * The only write surface the shared renderer receives. The host adapter owns
 * transport, optimistic state, and error mapping; the renderer never opens a
 * socket or reads Electron APIs.
 */
export interface ChatUiCommands {
  send(input: ChatUiSendInput): Promise<void>
  stop(): Promise<void>
  retry(messageId: string): Promise<void>
  fork(messageId: string): Promise<void>
  branch(messageId: string): Promise<void>
  edit(messageId: string, content: string): Promise<void>
  approve(requestId: string, decision: ChatUiApprovalDecision): Promise<void>
  answer(requestId: string, answers: readonly ChatUiAnswer[]): Promise<void>
  openFile(path: string, line?: number): void
  copy(text: string): Promise<void>
}
