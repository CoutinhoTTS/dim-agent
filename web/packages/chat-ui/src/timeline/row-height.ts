import type { ChatUiTimelineBlockKind, ChatUiTimelineBlockStatus } from '../model/timeline'

export interface ChatUiTimelineEstimateBlock {
  kind: ChatUiTimelineBlockKind
  status: ChatUiTimelineBlockStatus
  content: string
  generatedImages?: readonly unknown[]
}

export type ChatUiTimelineEstimateRow =
  | { kind: 'running' }
  | { kind: 'message'; message: { role: string } }
  | { kind: 'display' }
  | {
      kind: 'turn'
      turn: {
        userMessage: { content: string } | null
        blocks: readonly ChatUiTimelineEstimateBlock[]
        isRunning: boolean
      }
      runningStartedAt?: Date | null
    }

const ESTIMATED_TEXT_WRAP_CHARS = 92
const ESTIMATED_LIST_WRAP_CHARS = 82
const ESTIMATED_TEXT_LINE_HEIGHT = 22
const ESTIMATED_CODE_LINE_HEIGHT = 20
const ESTIMATED_MARKDOWN_FLOW_GAP = 4
const ESTIMATED_CODE_CONTENT_EXTRA_HEIGHT = 18
const ESTIMATED_CODE_BLOCK_HEADER_HEIGHT = 39
const ESTIMATED_CODE_BLOCK_MIN_CONTENT_HEIGHT = 55
const ESTIMATED_CODE_BLOCK_MAX_CONTENT_HEIGHT = 584
const ESTIMATED_GENERATED_IMAGE_HEIGHT = 440
// One work block (tool call or thinking step) is a single compact row while
// the turn details are open. Expanded turn rows have to account for them:
// a restore window sized from a collapsed-style estimate would mount several
// thousand-pixel turns before the first frame.
const ESTIMATED_WORK_BLOCK_HEIGHT = 44
const ESTIMATED_ROW_MAX_HEIGHT = 20000

export function estimateTimelineRowHeight(row: ChatUiTimelineEstimateRow | undefined): number {
  if (!row) {
    return 180
  }

  if (row.kind === 'running') {
    return 32
  }

  if (row.kind === 'message') {
    return row.message.role === 'divider' || row.message.role === 'user' ? 48 : 80
  }

  if (row.kind === 'display') {
    return 48
  }

  if (row.turn.userMessage && row.turn.blocks.length === 0) {
    return row.runningStartedAt !== undefined ? 128 : 120
  }

  let height = 32
  if (row.turn.userMessage) {
    height += estimateUserMessageHeight(row.turn.userMessage.content)
  }

  if (row.turn.userMessage && row.turn.blocks.length > 0) {
    height += 16
  }

  const hasCollapsedWorkSummary = row.turn.blocks.some(block =>
    block.kind !== 'text'
    && block.kind !== 'notification'
    && block.kind !== 'retry'
    && block.kind !== 'status'
    && block.kind !== 'plan-draft'
    && block.kind !== 'generated-image'
    && !(row.turn.isRunning || block.status === 'running'),
  )
  for (const block of row.turn.blocks) {
    if (block.kind === 'text') {
      // Fully-rendered height estimate: rows the user has seen keep their
      // rendered height (scrollTop, the scrollbar and the CV
      // contain-intrinsic-size memory all live in rendered space). This is
      // used as the final fallback when a row has never been mounted.
      height += estimateMarkdownContentHeight(block.content) + 16
      continue
    }
    if (block.kind === 'status') {
      height += estimatePlainTextHeight(block.content, ESTIMATED_TEXT_WRAP_CHARS) + 18
      continue
    }
    if (block.kind === 'notification' || block.kind === 'retry') {
      height += 34
      continue
    }
    if (block.kind === 'plan-draft') {
      height += estimateMarkdownContentHeight(block.content) + 64
      continue
    }
    if (block.kind === 'generated-image') {
      height += estimateGeneratedImageBlockHeight(block)
      continue
    }
    if (row.turn.isRunning || block.status === 'running') {
      height += Math.min(680, 52 + estimateMarkdownContentHeight(block.content) * 0.4)
      continue
    }
    if (block.kind === 'tool' || block.kind === 'thinking') {
      height += ESTIMATED_WORK_BLOCK_HEIGHT
      continue
    }
  }

  if (hasCollapsedWorkSummary) {
    height += 52
  }

  return clampEstimate(Math.ceil(height), 180, ESTIMATED_ROW_MAX_HEIGHT)
}

/**
 * Off-screen height estimate for a timeline row that is not mounted (or a
 * mounted row whose text block markstream has deferred until visible).
 *
 * While a row is off-screen its text block is not rendered at all, so the
 * real off-screen height is user message + work summary + status — NOT the
 * full text height. Placeholders must use this value, otherwise the scrollbar
 * and scrollHeight collapse as rows realize during a first pass.
 */
export function estimateOffscreenTimelineRowHeight(row: ChatUiTimelineEstimateRow | undefined): number {
  if (!row) {
    return 180
  }

  if (row.kind === 'running') {
    return 32
  }

  if (row.kind === 'message') {
    return row.message.role === 'divider' || row.message.role === 'user' ? 48 : 80
  }

  if (row.kind === 'display') {
    return 48
  }

  if (row.turn.userMessage && row.turn.blocks.length === 0) {
    return row.runningStartedAt !== undefined ? 128 : 120
  }

  let height = 32
  if (row.turn.userMessage) {
    height += estimateUserMessageHeight(row.turn.userMessage.content)
  }

  if (row.turn.userMessage && row.turn.blocks.length > 0) {
    height += 16
  }

  const hasCollapsedWorkSummary = row.turn.blocks.some(block =>
    block.kind !== 'text'
    && block.kind !== 'notification'
    && block.kind !== 'retry'
    && block.kind !== 'status'
    && block.kind !== 'plan-draft'
    && block.kind !== 'generated-image'
    && !(row.turn.isRunning || block.status === 'running'),
  )
  for (const block of row.turn.blocks) {
    if (block.kind === 'text') {
      // Deferred off-screen: the text block contributes no height. The clamp
      // floor (180px) keeps the placeholder in the same ballpark as a short
      // user message + summary row.
      continue
    }
    if (block.kind === 'status') {
      height += estimatePlainTextHeight(block.content, ESTIMATED_TEXT_WRAP_CHARS) + 18
      continue
    }
    if (block.kind === 'notification' || block.kind === 'retry') {
      height += 34
      continue
    }
    if (block.kind === 'plan-draft') {
      height += estimateMarkdownContentHeight(block.content) + 64
      continue
    }
    if (block.kind === 'generated-image') {
      height += estimateGeneratedImageBlockHeight(block)
      continue
    }
    if (row.turn.isRunning || block.status === 'running') {
      height += Math.min(680, 52 + estimateMarkdownContentHeight(block.content) * 0.4)
      continue
    }
  }

  if (hasCollapsedWorkSummary) {
    height += 52
  }

  return clampEstimate(Math.ceil(height), 180, ESTIMATED_ROW_MAX_HEIGHT)
}

/**
 * Height estimate for a row whose real height is not known yet, in the layout
 * state it is currently in.
 *
 * The two turn layouts are not interchangeable: a collapsed row hides its work
 * blocks and text behind the summary, so its placeholder has to stay short.
 * An expanded row renders those blocks, so the collapsed estimate would let a
 * restore window mount several multi-thousand-pixel turns in one batch.
 */
export function estimateTimelineRowHeightForLayout(
  row: ChatUiTimelineEstimateRow | undefined,
  layoutState: 'static' | 'collapsed' | 'expanded',
): number {
  return layoutState === 'expanded'
    ? estimateTimelineRowHeight(row)
    : estimateOffscreenTimelineRowHeight(row)
}

function estimateUserMessageHeight(content: string): number {
  return clampEstimate(28 + estimatePlainTextHeight(content, 52), 52, 360)
}

function estimatePlainTextHeight(content: string, wrapChars: number): number {
  const lines = content.split('\n')
  let height = 0
  for (const line of lines) {
    const trimmed = line.trim()
    height += trimmed
      ? estimateWrappedLineCount(trimmed, wrapChars) * ESTIMATED_TEXT_LINE_HEIGHT
      : 10
  }
  return Math.max(24, height)
}

function estimateMarkdownContentHeight(content: string): number {
  const lines = content.split('\n')
  let height = 0
  let inFence = false
  let fenceLanguage = ''
  let fenceLineCount = 0

  for (const line of lines) {
    const fenceMatch = line.match(/^\s*(```|~~~)\s*([A-Za-z0-9_-]+)?/)
    if (fenceMatch) {
      if (inFence) {
        height += estimateCodeFenceHeight(fenceLanguage, fenceLineCount)
        inFence = false
        fenceLanguage = ''
        fenceLineCount = 0
      } else {
        inFence = true
        fenceLanguage = fenceMatch[2] ?? ''
        fenceLineCount = 0
      }
      continue
    }

    if (inFence) {
      fenceLineCount += 1
      continue
    }

    const trimmed = line.trim()
    if (!trimmed) {
      height += 6
      continue
    }

    if (/^#{1,6}\s/.test(trimmed)) {
      height += 18 + estimateWrappedLineCount(trimmed, 72) * 30
      continue
    }

    if (/^\|.*\|$/.test(trimmed)) {
      height += 34
      continue
    }

    if (/^[-*_]{3,}$/.test(trimmed)) {
      height += 28
      continue
    }

    const wrapChars = /^(\s*[-*+]\s|\s*\d+\.\s|>)/.test(line)
      ? ESTIMATED_LIST_WRAP_CHARS
      : ESTIMATED_TEXT_WRAP_CHARS
    height += estimateWrappedLineCount(trimmed, wrapChars) * ESTIMATED_TEXT_LINE_HEIGHT + 6
  }

  if (inFence) {
    height += estimateCodeFenceHeight(fenceLanguage, fenceLineCount)
  }

  const flowGap = height < 8000 ? lines.length * ESTIMATED_MARKDOWN_FLOW_GAP : 0
  return Math.max(32, height + flowGap)
}

function estimateCodeFenceHeight(language: string, lineCount: number): number {
  if (language.toLowerCase() === 'mermaid') {
    return Math.max(280, 180 + lineCount * 18)
  }
  const contentHeight = Math.min(
    ESTIMATED_CODE_BLOCK_MAX_CONTENT_HEIGHT,
    Math.max(
      ESTIMATED_CODE_BLOCK_MIN_CONTENT_HEIGHT,
      Math.ceil(Math.max(1, lineCount) * ESTIMATED_CODE_LINE_HEIGHT + ESTIMATED_CODE_CONTENT_EXTRA_HEIGHT),
    ),
  )
  return contentHeight + ESTIMATED_CODE_BLOCK_HEADER_HEIGHT
}

function estimateGeneratedImageBlockHeight(block: { content: string; generatedImages?: readonly unknown[] }): number {
  const imageCount = block.generatedImages?.length
    ? block.generatedImages.length
    : Math.max(1, block.content.split('\n').filter(line => line.trim()).length)
  return imageCount * ESTIMATED_GENERATED_IMAGE_HEIGHT
}

function estimateWrappedLineCount(value: string, wrapChars: number): number {
  return Math.max(1, Math.ceil(readVisualLength(value) / wrapChars))
}

function readVisualLength(value: string): number {
  let length = 0
  for (const char of value) {
    length += char.charCodeAt(0) > 255 ? 2 : 1
  }
  return length
}

function clampEstimate(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value))
}
