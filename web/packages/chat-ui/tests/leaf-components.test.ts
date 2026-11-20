import { afterEach, describe, expect, it } from 'vitest'
import { createApp, h, nextTick, type App, type Component } from 'vue'
import {
  MarkdownHeadingNode,
  TimelineUserBubble,
  ToolCodeBlock,
} from '../src'

const mounted: App[] = []

function mount(component: Component, props: Record<string, unknown>, slot?: string): HTMLElement {
  const host = document.createElement('div')
  const app = createApp({
    render: () => h(
      component,
      props,
      slot === undefined ? undefined : { default: () => slot },
    ),
  })
  app.mount(host)
  mounted.push(app)
  return host
}

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.innerHTML = ''
})

describe('TimelineUserBubble', () => {
  it('keeps the timeline bubble contract and slot content', () => {
    const host = mount(TimelineUserBubble, { messageId: 'message-1' }, 'Hello')
    const bubble = host.querySelector('.timeline-user-bubble')

    expect(bubble).not.toBeNull()
    expect(bubble?.getAttribute('data-timeline-user-message-visible-id')).toBe('message-1')
    expect(bubble?.textContent).toContain('Hello')
  })
})

describe('ToolCodeBlock', () => {
  it('renders the unavailable patch state', async () => {
    const host = mount(ToolCodeBlock, {
      fileName: 'src/app.ts',
      patch: 'not a patch',
      patchUnavailableText: 'Patch unavailable',
      theme: { dark: 'dark', light: 'light' },
      themeType: 'light',
      codeFontSize: 14,
    })
    await nextTick()

    expect(host.querySelector('[data-testid="tool-code-block"]')).not.toBeNull()
    expect(host.textContent).toContain('Patch unavailable')
  })
})

describe('MarkdownHeadingNode', () => {
  it('assigns the GitHub slug to a heading without an explicit id', () => {
    const host = mount(MarkdownHeadingNode, {
      node: {
        type: 'heading',
        level: 2,
        text: 'Hello World',
        children: [],
        raw: '## Hello World',
      },
    })

    expect(host.querySelector('h2')?.getAttribute('id')).toBe('hello-world')
  })
})
