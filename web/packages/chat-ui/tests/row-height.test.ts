import { describe, expect, it } from 'vitest'
import {
  estimateOffscreenTimelineRowHeight,
  estimateTimelineRowHeight,
  estimateTimelineRowHeightForLayout,
  type ChatUiTimelineEstimateBlock,
  type ChatUiTimelineEstimateRow,
} from '../src'

function turnRow(overrides: {
  content?: string
  blocks?: readonly ChatUiTimelineEstimateBlock[]
  isRunning?: boolean
  runningStartedAt?: Date | null
} = {}): ChatUiTimelineEstimateRow {
  return {
    kind: 'turn',
    turn: {
      userMessage: { content: 'hello' },
      blocks: overrides.blocks ?? [],
      isRunning: overrides.isRunning ?? false,
    },
    runningStartedAt: overrides.runningStartedAt,
  }
}

describe('timeline row height estimates', () => {
  it('returns fixed heights for non-turn rows', () => {
    expect(estimateTimelineRowHeight(undefined)).toBe(180)
    expect(estimateTimelineRowHeight({ kind: 'running' })).toBe(32)
    expect(estimateTimelineRowHeight({ kind: 'display' })).toBe(48)
    expect(estimateTimelineRowHeight({ kind: 'message', message: { role: 'user' } })).toBe(48)
    expect(estimateTimelineRowHeight({ kind: 'message', message: { role: 'divider' } })).toBe(48)
    expect(estimateTimelineRowHeight({ kind: 'message', message: { role: 'assistant' } })).toBe(80)
  })

  it('reserves the empty-turn and running-empty-turn heights', () => {
    expect(estimateTimelineRowHeight(turnRow())).toBe(120)
    expect(estimateTimelineRowHeight(turnRow({ runningStartedAt: new Date() }))).toBe(128)
  })

  it('keeps off-screen text rows at the floor while expanded text grows', () => {
    const longText = 'word '.repeat(400)
    const row = turnRow({
      blocks: [{ kind: 'text', status: 'complete', content: longText }],
    })
    expect(estimateOffscreenTimelineRowHeight(row)).toBe(180)
    expect(estimateTimelineRowHeight(row)).toBeGreaterThan(500)
    expect(estimateTimelineRowHeightForLayout(row, 'static')).toBe(180)
    expect(estimateTimelineRowHeightForLayout(row, 'collapsed')).toBe(180)
    expect(estimateTimelineRowHeightForLayout(row, 'expanded')).toBe(estimateTimelineRowHeight(row))
  })

  it('scales generated-image height by image count or content lines', () => {
    const one = turnRow({
      blocks: [{ kind: 'generated-image', status: 'complete', content: 'image', generatedImages: [{}] }],
    })
    const two = turnRow({
      blocks: [{ kind: 'generated-image', status: 'complete', content: 'a\nb', generatedImages: [{}, {}] }],
    })
    const contentOnly = turnRow({
      blocks: [{ kind: 'generated-image', status: 'complete', content: 'a\nb\nc' }],
    })
    expect(estimateTimelineRowHeight(two)).toBe(estimateTimelineRowHeight(one) + 440)
    expect(estimateTimelineRowHeight(contentOnly)).toBe(estimateTimelineRowHeight(two) + 440)
  })

  it('counts wide characters as two columns and code fences as blocks', () => {
    const latin = turnRow({
      blocks: [{ kind: 'text', status: 'complete', content: 'a'.repeat(400) }],
    })
    const cjk = turnRow({
      blocks: [{ kind: 'text', status: 'complete', content: '中'.repeat(400) }],
    })
    expect(estimateTimelineRowHeight(cjk)).toBeGreaterThan(estimateTimelineRowHeight(latin))

    const plain = turnRow({
      blocks: [{
        kind: 'text',
        status: 'complete',
        content: Array.from({ length: 10 }, () => 'const x = 1').join('\n'),
      }],
    })
    const code = turnRow({
      blocks: [{
        kind: 'text',
        status: 'complete',
        content: '```ts\n' + Array.from({ length: 20 }, () => 'const x = 1').join('\n') + '\n```',
      }],
    })
    expect(estimateTimelineRowHeight(code)).toBeGreaterThan(estimateTimelineRowHeight(plain))
  })

  it('clamps very large rows to the maximum estimate', () => {
    const row = turnRow({
      blocks: [{ kind: 'generated-image', status: 'complete', content: '', generatedImages: Array.from({ length: 100 }, () => ({})) }],
    })
    expect(estimateTimelineRowHeight(row)).toBe(20000)
  })
})
