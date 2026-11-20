import removeMd from 'remove-markdown'
import { resolveTurnRunId } from './turn-artifacts'

export const USER_MESSAGE_TIMELINE_MIN_TURNS = 3
export const USER_MESSAGE_TIMELINE_PREVIEW_MAX_CHARS = 180

export function toAssistantPreview(markdown: string): string {
  return removeMd(markdown).replace(/\s+/g, ' ').trim().slice(0, USER_MESSAGE_TIMELINE_PREVIEW_MAX_CHARS)
}

export interface ChatUiTimelineUserMessageMarker {
  id: string
  rowKey?: string
  visibleRowKeys: string[]
  userMessageId: string
  preview: string
  assistantPreview?: string
  previewLoaded?: boolean
  runId?: string
  /** Raw `turn/end` sequence, when the completed turn still carries it. */
  turnEndSequence?: number
}

export type ChatUiRunChangedPathChangeType = 'created' | 'modified' | 'deleted' | 'unknown'

/**
 * The only per-path facts the rail card renders. The host projects its
 * authoritative changed-path result onto this shape so the package stays
 * independent of the desktop protocol.
 */
export interface ChatUiRunChangedPathEntry {
  path: string
  changeType?: ChatUiRunChangedPathChangeType
}

export interface ChatUiTimelineMessageDetail {
  message: { content: string }
  assistantPreview?: string
}

export interface ChatUiTimelineUserMessageBlock {
  kind: string
  content: string
}

export interface ChatUiTimelineUserMessageRow {
  kind: string
  key: string
  turn?: {
    userMessage: { id: string; content: string } | null
    blocks: readonly ChatUiTimelineUserMessageBlock[]
    forkAfterSequence?: number | null
  }
}

function positiveSequence(value: number | null | undefined): number | undefined {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0 ? value : undefined
}

export function createTimelineUserMessageMarkers(
  messageIds: string[],
): ChatUiTimelineUserMessageMarker[] {
  return messageIds.map((messageId, index) => ({
    id: messageId,
    visibleRowKeys: [],
    userMessageId: messageId,
    preview: `User message ${index + 1}`,
    previewLoaded: false,
  }))
}

export function deriveTimelineUserMessageMarkers(
  rows: readonly ChatUiTimelineUserMessageRow[],
): ChatUiTimelineUserMessageMarker[] {
  const markers: ChatUiTimelineUserMessageMarker[] = []
  let activeMarker: ChatUiTimelineUserMessageMarker | null = null
  for (const row of rows) {
    if (row.kind !== 'turn' || row.turn === undefined) {
      continue
    }
    if (!row.turn.userMessage) {
      if (activeMarker && row.turn.blocks.length > 0) {
        activeMarker.visibleRowKeys.push(row.key)
      }
      continue
    }
    const userMessage = row.turn.userMessage
    const runId = resolveTurnRunId(row.turn)
    const assistantMarkdown = row.turn.blocks
      .filter(block => block.kind === 'text')
      .map(block => block.content)
      .join('\n')
      .trim()
    const marker: ChatUiTimelineUserMessageMarker = {
      id: userMessage.id,
      rowKey: row.key,
      visibleRowKeys: [row.key],
      userMessageId: userMessage.id,
      preview: truncateUserMessagePreview(userMessage.content),
      previewLoaded: true,
    }
    if (assistantMarkdown) {
      marker.assistantPreview = toAssistantPreview(assistantMarkdown)
    }
    if (runId !== undefined) {
      marker.runId = runId
    }
    const turnEndSequence = positiveSequence(row.turn.forkAfterSequence)
    if (turnEndSequence !== undefined) {
      marker.turnEndSequence = turnEndSequence
    }
    markers.push(marker)
    activeMarker = marker
  }
  return markers
}

export function shouldShowTimelineUserMessageRail(
  markers: ChatUiTimelineUserMessageMarker[],
): boolean {
  return markers.length >= USER_MESSAGE_TIMELINE_MIN_TURNS
}

export function truncateUserMessagePreview(content: string): string {
  const normalized = content.replace(/\s+/g, ' ').trim()
  if (normalized.length <= USER_MESSAGE_TIMELINE_PREVIEW_MAX_CHARS) {
    return normalized
  }
  return `${normalized.slice(0, USER_MESSAGE_TIMELINE_PREVIEW_MAX_CHARS - 1)}…`
}
