import { afterEach, describe, expect, it } from 'vitest'
import { createApp, h, nextTick, type App, type Component, type VNode } from 'vue'
import {
  TimelineSearchDomText,
  TimelineSearchHighlightedText,
  TimelineWindowTruncationDivider,
} from '../src'

const mounted: App[] = []

function mount(component: Component, props: Record<string, unknown>, slot?: () => VNode | string): HTMLElement {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(component, props, slot === undefined ? undefined : { default: slot }),
  })
  app.mount(host)
  mounted.push(app)
  return host
}

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

describe('TimelineSearchHighlightedText', () => {
  it('splits matching ranges and marks the active match', () => {
    const host = mount(TimelineSearchHighlightedText, {
      text: 'alpha beta alpha',
      matches: [
        { id: 'm1', sourceMatchIndex: 0, startOffset: 0, endOffset: 5 },
        { id: 'm2', sourceMatchIndex: 1, startOffset: 11, endOffset: 16 },
      ],
      activeMatchId: 'm2',
    })

    expect(host.textContent).toBe('alpha beta alpha')
    const matches = host.querySelectorAll('.timeline-search-match')
    expect(matches).toHaveLength(2)
    expect(matches[1]?.classList.contains('timeline-search-match-active')).toBe(true)
    expect(matches[1]?.getAttribute('data-timeline-search-match-id')).toBe('m2')
  })

  it('applies the base offset to nested searchable text', () => {
    const host = mount(TimelineSearchHighlightedText, {
      text: 'alpha',
      matches: [{ id: 'm1', sourceMatchIndex: 0, startOffset: 10, endOffset: 15 }],
      baseOffset: 10,
    })

    expect(host.querySelectorAll('.timeline-search-match')).toHaveLength(1)
  })
})

describe('TimelineSearchDomText', () => {
  it('wraps rendered text nodes and cleans highlights on unmount', async () => {
    const host = mount(
      TimelineSearchDomText,
      {
        query: 'alpha',
        matches: [{ id: 'm1', sourceMatchIndex: 0, startOffset: 0, endOffset: 5 }],
        activeMatchId: 'm1',
      },
      () => h('span', 'alpha beta'),
    )

    await nextTick()
    await nextTick()
    const highlight = host.querySelector('[data-timeline-search-generated="true"]')
    expect(highlight?.textContent).toBe('alpha')
    expect(highlight?.className).toContain('timeline-search-match-active')

    mounted.pop()?.unmount()
    expect(host.querySelector('[data-timeline-search-generated="true"]')).toBeNull()
  })
})

describe('TimelineWindowTruncationDivider', () => {
  it('renders the bounded-view notice and accepts host copy', () => {
    const host = mount(TimelineWindowTruncationDivider, { row: { key: 'row-1' } })
    expect(host.querySelector('[role="status"]')?.textContent)
      .toContain('Part of this response cannot be rendered safely')

    const custom = mount(TimelineWindowTruncationDivider, { text: 'View bounded' })
    expect(custom.querySelector('[role="status"]')?.textContent).toContain('View bounded')
  })
})
