import { afterEach, describe, expect, it } from 'vitest'
import { createApp, defineComponent, h, type App } from 'vue'
import { TimelineTextItem } from '../src'
import type { ChatUiTimelineBlock } from '../src'

const mounted: App[] = []

const MarkdownStub = defineComponent({
  props: {
    content: { type: String, default: '' },
    streaming: { type: Boolean, default: false },
    viewportPriority: { type: Boolean, default: false },
  },
  setup(props) {
    return () => h('div', {
      'data-testid': 'markdown-stub',
      'data-content': props.content,
      'data-streaming': String(props.streaming),
      'data-viewport-priority': String(props.viewportPriority),
    })
  },
})

function mount(block: ChatUiTimelineBlock): HTMLElement {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(TimelineTextItem, {
      block,
      sessionId: 'session-1',
      searchQuery: '',
      searchMatches: [],
      activeSearchMatchId: null,
      markdownViewportPriority: true,
      markdownComponent: MarkdownStub,
      isDark: true,
      codeBlockLightTheme: 'light',
      codeBlockDarkTheme: 'dark',
      codeBlockProps: { theme: 'dark' },
      codeBlockOptions: { fontSize: 12 },
    }),
  })
  app.mount(host)
  mounted.push(app)
  return host
}

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

describe('TimelineTextItem', () => {
  it('keeps the text block attributes and forwards the streaming markdown profile', () => {
    const host = mount({
      id: 'block-1',
      kind: 'text',
      status: 'running',
      content: 'G:\\Tangula\\_lookbook\\README.md',
    })

    const block = host.querySelector<HTMLElement>('[data-testid="timeline-text-block"]')
    expect(block?.dataset.blockId).toBe('block-1')
    expect(block?.dataset.scrollFollowContent).toBe('true')
    expect(block?.dataset.messageStatus).toBe('streaming')

    const markdown = host.querySelector<HTMLElement>('[data-testid="markdown-stub"]')
    expect(markdown?.dataset.content).toBe('G:\\Tangula\\_lookbook\\README.md')
    expect(markdown?.dataset.streaming).toBe('true')
    expect(markdown?.dataset.viewportPriority).toBe('true')
  })

  it('does not mark a completed text block as streaming', () => {
    const host = mount({
      id: 'block-2',
      kind: 'text',
      status: 'complete',
      content: 'Done',
    })

    const block = host.querySelector<HTMLElement>('[data-testid="timeline-text-block"]')
    expect(block?.dataset.scrollFollowContent).toBeUndefined()
    expect(block?.dataset.messageStatus).toBeUndefined()
    expect(host.querySelector<HTMLElement>('[data-testid="markdown-stub"]')?.dataset.streaming)
      .toBe('false')
  })
})
