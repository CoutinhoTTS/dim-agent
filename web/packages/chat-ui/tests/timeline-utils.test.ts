import { describe, expect, it } from 'vitest'
import {
  classifyArtifact,
  createTimelineUserMessageMarkers,
  deriveTimelineUserMessageMarkers,
  hashText,
  hashTextAppend,
  hashTextJoinedParts,
  latestThinkingFragment,
  normalizeTimelineSearchQuery,
  resolveMessageRunId,
  resolveTurnRunId,
  sanitizeTimelineSearchInput,
  shouldShowTimelineUserMessageRail,
  timelineBlockSearchSourceId,
  timelineMessageSearchSourceId,
  timelineWindowOverBudget,
  toAssistantPreview,
  truncateUserMessagePreview,
} from '../src'

describe('timeline hashing', () => {
  it('matches the full-string hash for appended and joined text', () => {
    const prefix = 'assistant answer'
    const appended = ' continues'
    expect(hashTextAppend(hashText(prefix), appended)).toBe(hashText(prefix + appended))
    expect(hashTextJoinedParts(['a', 'b', 'c'], '\n')).toBe(hashText('a\nb\nc'))
  })
})

describe('timeline thinking excerpt', () => {
  it('returns a bounded tail and reports truncation', () => {
    expect(latestThinkingFragment('  first line\nsecond line  ')).toEqual({
      text: 'first line second line',
      truncated: false,
    })
    expect(latestThinkingFragment('a'.repeat(221))).toMatchObject({ truncated: true })
    expect(latestThinkingFragment('a'.repeat(220))).toMatchObject({ truncated: false })
  })
})

describe('timeline window budget', () => {
  it('checks message count and estimated bytes', () => {
    expect(timelineWindowOverBudget([{ content: 'x' }], 1024, 0)).toBe(true)
    expect(timelineWindowOverBudget([{ content: 'x'.repeat(40) }], 10)).toBe(true)
    expect(timelineWindowOverBudget([{ content: 'x' }], 1024, 10)).toBe(false)
  })
})

describe('turn artifacts', () => {
  it('classifies deliverable extensions and resolves run ids', () => {
    expect(classifyArtifact('/tmp/report.md')).toBe('markdown')
    expect(classifyArtifact('C:\\tmp\\chart.png')).toBe('image')
    expect(classifyArtifact('/tmp/source.ts')).toBeUndefined()
    expect(resolveMessageRunId({ id: 'm1', metadata: { runId: ' run-1 ' } })).toBe('run-1')
    expect(resolveTurnRunId({ userMessage: { id: 'm1', metadata: { runId: 'run-2' } } })).toBe('run-2')
    expect(resolveTurnRunId({ userMessage: null })).toBeUndefined()
  })
})

describe('timeline search helpers', () => {
  it('normalizes query text and exposes stable source ids', () => {
    expect(sanitizeTimelineSearchInput('a\r\nb\nc')).toBe('abc')
    expect(normalizeTimelineSearchQuery('  hello  ')).toBe('hello')
    expect(timelineMessageSearchSourceId('m1')).toBe('message:m1')
    expect(timelineBlockSearchSourceId('b1')).toBe('block:b1')
  })
})

describe('timeline user message rail', () => {
  it('derives previews, run ids, and turn-end sequences', () => {
    const markers = deriveTimelineUserMessageMarkers([
      {
        kind: 'turn',
        key: 'turn-1',
        turn: {
          userMessage: { id: 'u1', content: 'Hello **world**' },
          blocks: [{ kind: 'text', content: '# Answer' }],
          forkAfterSequence: 7,
        },
      },
    ])

    expect(markers).toHaveLength(1)
    expect(markers[0]).toMatchObject({
      id: 'u1',
      rowKey: 'turn-1',
      preview: 'Hello **world**',
      assistantPreview: 'Answer',
      turnEndSequence: 7,
    })
    expect(toAssistantPreview('## Title\n\nBody')).toBe('Title Body')
    expect(truncateUserMessagePreview('a'.repeat(181))).toHaveLength(180)
    expect(shouldShowTimelineUserMessageRail(markers)).toBe(false)
    expect(createTimelineUserMessageMarkers(['u1', 'u2'])[1]?.preview).toBe('User message 2')
  })
})
