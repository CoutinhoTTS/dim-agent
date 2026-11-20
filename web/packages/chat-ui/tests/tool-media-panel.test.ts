import { afterEach, describe, expect, it } from 'vitest'
import { createApp, defineComponent, h, type App } from 'vue'
import { TimelineToolMediaPanel, type ChatUiToolMediaPanelBlock } from '../src'

const labels = {
  input: 'Input',
  output: 'Output',
}

const ImageCardStub = defineComponent({
  name: 'ImageCardStub',
  props: {
    images: { type: Array, required: true },
    compact: { type: Boolean, default: false },
    showActions: { type: Boolean, default: true },
  },
  setup(props) {
    return () => h('div', {
      'data-testid': 'image-card',
      'data-count': String((props.images as unknown[]).length),
      'data-compact': String(props.compact),
      'data-show-actions': String(props.showActions),
      'data-paths': (props.images as { path: string }[]).map(image => image.path).join(','),
    })
  },
})

const mounted: App[] = []

function mount(block: ChatUiToolMediaPanelBlock): HTMLElement {
  const host = document.createElement('div')
  const app = createApp({
    render: () => h(TimelineToolMediaPanel, {
      block,
      labels,
      imageCardComponent: ImageCardStub,
    }),
  })
  app.mount(host)
  mounted.push(app)
  return host
}

function block(overrides: Partial<ChatUiToolMediaPanelBlock> = {}): ChatUiToolMediaPanelBlock {
  return {
    content: '',
    ...overrides,
  }
}

function panel(host: HTMLElement): HTMLElement | null {
  return host.querySelector('[data-testid="timeline-tool-cua-panel"]')
}

function imageCard(host: HTMLElement): HTMLElement | null {
  return host.querySelector('[data-testid="image-card"]')
}

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.innerHTML = ''
})

describe('TimelineToolMediaPanel', () => {
  it('renders screenshot images as compact cards without actions or output', () => {
    const host = mount(block({
      toolName: 'cua-driver__get_window_state',
      toolParams: { window_id: 'window-1', x: 12 },
      toolResult: { images: [{ path: '/tmp/window.png', mimeType: 'image/png' }] },
    }))

    expect(panel(host)).not.toBeNull()
    const card = imageCard(host)
    expect(card?.getAttribute('data-count')).toBe('1')
    expect(card?.getAttribute('data-compact')).toBe('true')
    expect(card?.getAttribute('data-show-actions')).toBe('false')
    expect(card?.getAttribute('data-paths')).toBe('/tmp/window.png')
    expect(host.textContent).toContain('Input')
    expect(host.textContent).toContain('window_id: window-1')
    expect(host.textContent).toContain('x: 12')
    expect(host.textContent).not.toContain('Output')
  })

  it('renders expandable CUA output without an image card', () => {
    const host = mount(block({
      toolName: 'cua-driver__list_apps',
      toolResult: { content: 'Safari\nFinder' },
    }))

    expect(panel(host)).not.toBeNull()
    expect(imageCard(host)).toBeNull()
    expect(host.textContent).toContain('Output')
    expect(host.textContent).toContain('Safari')
    expect(host.textContent).toContain('Finder')
  })

  it('renders nothing for non-CUA tools', () => {
    const host = mount(block({
      toolName: 'read',
      toolParams: { path: 'src/app.ts' },
      toolResult: { images: [{ path: '/tmp/window.png', mimeType: 'image/png' }] },
    }))

    expect(panel(host)).toBeNull()
    expect(imageCard(host)).toBeNull()
    expect(host.textContent).toBe('')
  })

  it('projects only known input keys and skips nullish values', () => {
    const host = mount(block({
      toolName: 'cua-driver__page',
      toolParams: {
        name: 'Safari',
        window_id: 3,
        enabled: false,
        text: 'hello',
        x: { nested: true },
        extra: 'hidden',
        missing: undefined,
        nothing: null,
      },
      toolResult: { content: 'page body' },
    }))

    const input = host.querySelector('pre')
    expect(input?.textContent).toBe([
      'name: Safari',
      'window_id: 3',
      'x: {"nested":true}',
      'text: hello',
      'enabled: false',
    ].join('\n'))
    expect(host.textContent).not.toContain('extra')
    expect(host.textContent).not.toContain('hidden')
  })

  it('renders images and output together and caps thumbnails at three', () => {
    const host = mount(block({
      toolName: 'mcp__computer__zoom',
      toolParams: { window_id: 'window-1' },
      toolResult: {
        structuredContent: {
          images: [
            { path: '/tmp/1.png', mimeType: 'image/png' },
            { path: '/tmp/2.png', mimeType: 'image/png' },
            { path: '/tmp/3.png', mimeType: 'image/png' },
            { path: '/tmp/4.png', mimeType: 'image/png' },
          ],
        },
        content: 'page body',
      },
    }))

    expect(imageCard(host)?.getAttribute('data-count')).toBe('3')
    expect(host.textContent).toContain('+1')
    expect(host.textContent).toContain('window_id: window-1')
    expect(host.textContent).toContain('Output')
    expect(host.textContent).toContain('page body')
  })
})
