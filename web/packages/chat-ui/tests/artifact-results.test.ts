import { describe, expect, it } from 'vitest'
import {
  extractCuaVisualResults,
  extractGeneratedImageResults,
  extractPlanDraftResult,
  type ChatUiTimelineBlock,
} from '../src'

function block(overrides: Partial<ChatUiTimelineBlock> = {}): ChatUiTimelineBlock {
  return {
    id: 'block-1',
    kind: 'tool',
    status: 'complete',
    content: '',
    ...overrides,
  }
}

describe('generated image results', () => {
  it('extracts browser screenshot blobs and deduplicates paths', () => {
    const results = extractGeneratedImageResults(block({
      toolName: 'browser',
      toolParams: { action: 'screenshot' },
      toolResult: {
        blobRefs: [{ path: '/tmp/shot.png', mimeType: 'image/png' }],
        structuredContent: {
          images: [{ path: '/tmp/shot.png', mimeType: 'image/png' }],
        },
      },
    }))
    expect(results).toEqual([{
      id: '/tmp/shot.png',
      path: '/tmp/shot.png',
      mimeType: 'image/png',
    }])
  })

  it('ignores non-screenshot browser calls', () => {
    expect(extractGeneratedImageResults(block({
      toolName: 'browser',
      toolParams: { action: 'navigate' },
      toolResult: { blobRefs: [{ path: '/tmp/shot.png', mimeType: 'image/png' }] },
    }))).toEqual([])
  })

  it('derives image mime types from markdown links', () => {
    const results = extractGeneratedImageResults(block({
      toolName: 'browser',
      toolParams: { action: 'screenshot' },
      content: '![shot](/tmp/shot.webp)',
    }))
    expect(results).toEqual([{
      id: '/tmp/shot.webp',
      path: '/tmp/shot.webp',
      mimeType: 'image/webp',
    }])
  })

  it('extracts CUA visual results for known tool names', () => {
    const results = extractCuaVisualResults(block({
      toolName: 'mcp__computer__get_window_state',
      toolResult: {
        structuredContent: {
          images: [{ path: '/tmp/window.png', mediaType: 'image/png', sizeBytes: 42, revisedPrompt: 'screen' }],
        },
      },
    }))
    expect(results).toEqual([{
      id: '/tmp/window.png',
      path: '/tmp/window.png',
      mimeType: 'image/png',
      sizeBytes: 42,
      revisedPrompt: 'screen',
    }])
    expect(extractCuaVisualResults(block({ toolName: 'browser' }))).toEqual([])
  })
})

describe('plan draft results', () => {
  it('reads a structured plan_read draft', () => {
    expect(extractPlanDraftResult(block({
      toolName: 'plan_read',
      toolResult: {
        structuredContent: {
          exists: true,
          content: 'draft',
          path: '/tmp/plan.md',
          mode: 'write',
        },
      },
    }))).toEqual({
      id: 'block-1:plan-draft',
      content: 'draft',
      path: '/tmp/plan.md',
      mode: 'write',
    })
  })

  it('ignores a missing plan and failed tool calls', () => {
    expect(extractPlanDraftResult(block({
      toolName: 'plan_read',
      content: 'No plan draft exists',
      toolResult: { structuredContent: { exists: false } },
    }))).toBeNull()
    expect(extractPlanDraftResult(block({
      toolName: 'plan_read',
      status: 'error',
      toolResult: { structuredContent: { exists: true, content: 'draft' } },
    }))).toBeNull()
  })

  it('reads a matching plan file write', () => {
    expect(extractPlanDraftResult(block({
      toolName: 'write',
      toolParams: { path: '/tmp/plan.md', content: 'draft' },
      toolResult: {
        structuredContent: {
          path: '/tmp/plan.md',
          revision: 'r1',
          bytesWritten: 5,
        },
      },
    }), '/tmp/plan.md')).toEqual({
      id: 'block-1:plan-draft',
      content: 'draft',
      path: '/tmp/plan.md',
      mode: 'write',
    })
  })

  it('reads a numbered plan file read and rejects other paths', () => {
    expect(extractPlanDraftResult(block({
      toolName: 'read',
      toolResult: {
        structuredContent: {
          path: '/tmp/plan.md',
          revision: 'r1',
          kind: 'text',
        },
        content: '1\tdraft\n2\tline',
      },
    }), '/tmp/plan.md')).toEqual({
      id: 'block-1:plan-draft',
      content: 'draft\nline',
      path: '/tmp/plan.md',
      mode: 'read',
    })

    expect(extractPlanDraftResult(block({
      toolName: 'read',
      toolResult: {
        structuredContent: {
          path: '/tmp/other.md',
          revision: 'r1',
          kind: 'text',
        },
        content: '1\tdraft',
      },
    }), '/tmp/plan.md')).toBeNull()
  })
})
