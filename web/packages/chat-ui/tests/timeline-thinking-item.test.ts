import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, type App, type VNode } from 'vue'
import { TimelineThinkingItem } from '../src'

const mounted: App[] = []

function mount(
  props: Record<string, unknown>,
  slots?: Record<string, () => VNode | string>,
): HTMLElement {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(TimelineThinkingItem, props as never, slots),
  })
  app.mount(host)
  mounted.push(app)
  return host
}

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

describe('TimelineThinkingItem', () => {
  it('renders the collapsed running title, excerpt, and separator', () => {
    const host = mount({
      blockId: 'thinking-1',
      content: 'full reasoning',
      isRunning: true,
      expanded: false,
      titleText: 'Thinking · 2s',
      excerpt: 'latest tail',
      excerptTruncated: false,
    }, {
      icon: () => h('span', { 'data-testid': 'thinking-icon' }),
      chevron: () => h('span', { 'data-testid': 'thinking-chevron' }),
    })

    const button = host.querySelector('button')
    expect(button?.getAttribute('aria-label')).toBe('Thinking · 2s')
    expect(button?.getAttribute('aria-expanded')).toBe('false')
    expect(host.querySelector('.thinking-title')?.textContent?.trim()).toBe('Thinking · 2s')
    expect(host.querySelector('[data-testid="timeline-thinking-separator"]')).not.toBeNull()
    expect(host.querySelector('[data-testid="timeline-thinking-excerpt-text"]')?.textContent)
      .toBe('latest tail')
    expect(host.querySelector('[data-testid="timeline-thinking-quote"]')).toBeNull()
    expect(host.querySelector('[data-testid="thinking-icon"]')).not.toBeNull()
    expect(host.querySelector('[data-testid="thinking-chevron"]')).not.toBeNull()
  })

  it('mounts the expanded quote and emits toggle from the native button', async () => {
    const toggle = vi.fn()
    const host = mount({
      blockId: 'thinking-2',
      content: 'full reasoning',
      isRunning: false,
      expanded: true,
      titleText: 'Thought for 2s',
      excerpt: 'ignored while expanded',
      excerptTruncated: false,
      onToggle: toggle,
    })

    expect(host.querySelector('[data-testid="timeline-thinking-quote"]')?.textContent)
      .toBe('full reasoning')
    expect(host.querySelector('[data-testid="timeline-thinking-excerpt"]')).toBeNull()
    host.querySelector('button')?.click()
    await Promise.resolve()
    expect(toggle).toHaveBeenCalledTimes(1)
  })

  it('marks an excerpt whose source was longer than the rendered fragment', () => {
    const host = mount({
      blockId: 'thinking-3',
      content: 'full reasoning',
      isRunning: false,
      expanded: false,
      titleText: 'Thought for 2s',
      excerpt: 'tail',
      excerptTruncated: true,
    })

    expect(host.querySelector('[data-testid="timeline-thinking-excerpt"]')
      ?.classList.contains('has-source-gate')).toBe(true)
  })
})
