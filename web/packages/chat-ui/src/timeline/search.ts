/**
 * Minimal search match contract shared by the timeline search renderers.
 * Hosts may extend it with row/source metadata; the package only reads the
 * offsets and identity fields below.
 */
export interface ChatUiTimelineSearchMatch {
  id: string
  sourceMatchIndex: number
  startOffset: number
  endOffset: number
}

export function sanitizeTimelineSearchInput(value: string): string {
  return value.replace(/[\r\n]+/g, '')
}

export function normalizeTimelineSearchQuery(value: string): string {
  return sanitizeTimelineSearchInput(value).trim()
}

export function hasTimelineSearchLineBreak(value: string): boolean {
  return /[\r\n]/.test(value)
}

export function timelineMessageSearchSourceId(messageId: string): string {
  return `message:${messageId}`
}

export function timelineBlockSearchSourceId(blockId: string): string {
  return `block:${blockId}`
}
