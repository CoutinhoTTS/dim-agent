/**
 * Renderer-only budget for one loaded timeline window. The host decides when
 * to trim; this module only estimates whether the window crossed the threshold.
 */
export interface ChatUiTimelineBudgetMessage {
  content: string
}

export const LOADED_TIMELINE_WINDOW_BYTE_BUDGET = 32 * 1024 * 1024
export const LOADED_TIMELINE_WINDOW_MESSAGE_BUDGET = 2_000

const estimateMessageBytes = (message: ChatUiTimelineBudgetMessage): number => {
  try {
    return JSON.stringify(message).length * 2
  } catch {
    return message.content.length * 2
  }
}

export const timelineWindowOverBudget = (
  messages: readonly ChatUiTimelineBudgetMessage[],
  byteBudget = LOADED_TIMELINE_WINDOW_BYTE_BUDGET,
  messageBudget = LOADED_TIMELINE_WINDOW_MESSAGE_BUDGET,
): boolean => {
  if (messages.length > messageBudget) return true
  let bytes = 0
  for (const message of messages) {
    bytes += estimateMessageBytes(message)
    if (bytes > byteBudget) return true
  }
  return false
}
