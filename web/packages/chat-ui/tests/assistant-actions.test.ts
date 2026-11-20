import { describe, expect, it } from 'vitest'
import {
  buildTimelineAssistantCopyPayload,
  hasTimelineAssistantCopyPayload,
  timelineAssistantBlockCopyText,
} from '../src'
import type { ChatUiTimelineBlock } from '../src'

const labels = {
  runFailedPlaceholder: 'Run failed',
  userInterrupt: 'Stopped by user',
}

const block = (
  id: string,
  kind: ChatUiTimelineBlock['kind'],
  content: string,
  statusType?: ChatUiTimelineBlock['statusType'],
): ChatUiTimelineBlock => ({
  id,
  kind,
  status: 'complete',
  content,
  ...(statusType === undefined ? {} : { statusType }),
})

describe('timeline assistant copy helpers', () => {
  it('copies text, generated-image, and plan-draft blocks', () => {
    expect(timelineAssistantBlockCopyText(block('text', 'text', 'answer'), labels)).toBe('answer')
    expect(timelineAssistantBlockCopyText(block('image', 'generated-image', 'image alt'), labels))
      .toBe('image alt')
    expect(timelineAssistantBlockCopyText(block('plan', 'plan-draft', 'plan'), labels)).toBe('plan')
  })

  it('maps terminal status blocks to their localized fallback', () => {
    expect(timelineAssistantBlockCopyText(block('error', 'status', '', 'error'), labels))
      .toBe('Run failed')
    expect(timelineAssistantBlockCopyText(block('cancel', 'status', '', 'cancelled'), labels))
      .toBe('Stopped by user')
    expect(timelineAssistantBlockCopyText(block('empty', 'status', ''), labels)).toBeUndefined()
  })

  it('builds the payload in block order and ignores non-copyable blocks', () => {
    const blocks = [
      block('text', 'text', 'answer'),
      block('tool', 'tool', 'tool output'),
      block('error', 'status', '', 'error'),
    ]

    expect(hasTimelineAssistantCopyPayload(blocks, labels)).toBe(true)
    expect(buildTimelineAssistantCopyPayload(blocks, labels)).toBe('answer\n\nRun failed')
  })

  it('reports no copy payload for whitespace-only or non-copyable blocks', () => {
    const blocks = [
      block('space', 'text', '   \n'),
      block('tool', 'tool', 'tool output'),
    ]

    expect(hasTimelineAssistantCopyPayload(blocks, labels)).toBe(false)
    expect(buildTimelineAssistantCopyPayload(blocks, labels)).toBe('   \n')
  })
})
