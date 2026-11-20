export interface ChatUiApproval {
  requestId: string
  kind: 'permission' | 'member' | 'other'
  title?: string
  detail?: string
}

export interface ChatUiQuestionItem {
  id: string
  question: string
  options?: readonly string[]
}

export interface ChatUiQuestion {
  requestId: string
  questions: readonly ChatUiQuestionItem[]
}
