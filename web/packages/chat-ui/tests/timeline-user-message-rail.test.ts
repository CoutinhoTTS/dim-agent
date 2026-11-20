import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, type App } from 'vue'
import {
  TimelineUserMessageRail,
  type ChatUiRunChangedPathEntry,
  type ChatUiTimelineMessageDetail,
  type ChatUiTimelineUserMessageMarker,
  type ChatUiTimelineUserMessageRailLabels,
} from '../src'

const marker: ChatUiTimelineUserMessageMarker = {
  id: 'user-1',
  rowKey: 'row-1',
  visibleRowKeys: ['row-1'],
  userMessageId: 'user-1',
  preview: 'change files',
  previewLoaded: true,
  runId: 'run-1',
  turnEndSequence: 7,
}

const labels: ChatUiTimelineUserMessageRailLabels = {
  emptyChangedPaths: 'Nothing changed',
  timeline: count => `Timeline of ${count} messages`,
  jumpToMessage: preview => `Jump: ${preview}`,
  option: (index, count, preview) => `${index}/${count}: ${preview}`,
}

let app: App<Element> | undefined

afterEach(() => {
  app?.unmount()
  app = undefined
  document.body.replaceChildren()
  vi.restoreAllMocks()
})

interface MountOptions {
  markers?: readonly ChatUiTimelineUserMessageMarker[]
  loadedMarkers?: readonly ChatUiTimelineUserMessageMarker[]
  readMessageDetail?: (input: { sessionId: string; messageId: string }) => Promise<ChatUiTimelineMessageDetail | null | undefined>
  readRunChangedPaths?: (input: { sessionId: string; runId: string; throughSequence?: number }) => Promise<ChatUiRunChangedPathEntry[] | null>
  onMarkerClick?: (clicked: ChatUiTimelineUserMessageMarker) => void
}

async function mountRail(options: MountOptions = {}): Promise<HTMLElement> {
  const host = document.createElement('div')
  document.body.append(host)
  const markers = options.markers ?? [marker]
  const loadedMarkers = options.loadedMarkers ?? markers
  app = createApp({
    render: () => h(TimelineUserMessageRail, {
      sessionId: 'session-1',
      markers,
      loadedMarkers,
      labels,
      readMessageDetail: options.readMessageDetail,
      readRunChangedPaths: options.readRunChangedPaths,
      onMarkerClick: options.onMarkerClick,
    }),
  })
  app.mount(host)
  await nextTick()
  return host
}

describe('TimelineUserMessageRail', () => {
  it('renders the empty changed-path label from props', async () => {
    const readRunChangedPaths = vi.fn(async () => [])
    const host = await mountRail({ readRunChangedPaths })

    const markerWrap = host.querySelector<HTMLElement>('.timeline-user-message-rail__marker-wrap')!
    markerWrap.dispatchEvent(new Event('pointerenter'))

    await vi.waitFor(() => {
      expect(host.querySelector('.timeline-user-message-rail__empty')?.textContent?.trim())
        .toBe('Nothing changed')
    })
    expect(host.querySelector('.timeline-user-message-rail__chip')).toBeNull()
  })

  it('renders changed paths and the overflow count', async () => {
    const readRunChangedPaths = vi.fn(async (): Promise<ChatUiRunChangedPathEntry[]> => [
      { path: 'src/a.ts', changeType: 'modified' },
      { path: 'src/b.ts', changeType: 'created' },
      { path: 'src/c.ts', changeType: 'deleted' },
      { path: 'src/d.ts', changeType: 'modified' },
      { path: 'src/e.ts', changeType: 'unknown' },
    ])
    const host = await mountRail({ readRunChangedPaths })

    const markerWrap = host.querySelector<HTMLElement>('.timeline-user-message-rail__marker-wrap')!
    markerWrap.dispatchEvent(new Event('pointerenter'))

    await vi.waitFor(() => {
      expect(host.querySelectorAll('.timeline-user-message-rail__chip')).toHaveLength(4)
    })
    expect(host.querySelector('.timeline-user-message-rail__path')?.textContent?.trim()).toBe('a.ts')
    expect(host.querySelector('.timeline-user-message-rail__change-type')?.textContent?.trim()).toBe('modified')
    expect(host.querySelector('.timeline-user-message-rail__more')?.textContent?.trim()).toBe('+1')
    expect(readRunChangedPaths).toHaveBeenCalledWith({
      sessionId: 'session-1',
      runId: 'run-1',
      throughSequence: 7,
    })
  })

  it('loads a timeline message detail into the card preview', async () => {
    const readMessageDetail = vi.fn(async () => ({
      message: { content: '  first line\n second line  ' },
      assistantPreview: '**answer**',
    }))
    const unloadedMarker: ChatUiTimelineUserMessageMarker = {
      ...marker,
      runId: undefined,
      turnEndSequence: undefined,
      preview: 'User message 1',
      previewLoaded: false,
    }
    const host = await mountRail({
      markers: [unloadedMarker],
      readMessageDetail,
    })

    const markerWrap = host.querySelector<HTMLElement>('.timeline-user-message-rail__marker-wrap')!
    markerWrap.dispatchEvent(new Event('pointerenter'))

    await vi.waitFor(() => {
      expect(host.querySelector('.timeline-user-message-rail__preview')?.textContent?.trim())
        .toBe('first line second line')
    })
    expect(host.querySelector('.timeline-user-message-rail__assistant-preview')?.textContent?.trim())
      .toBe('answer')
    expect(readMessageDetail).toHaveBeenCalledWith({
      sessionId: 'session-1',
      messageId: 'user-1',
    })
  })

  it('emits the marker and keeps the jump label from props', async () => {
    const clicks: ChatUiTimelineUserMessageMarker[] = []
    const host = await mountRail({ onMarkerClick: clicked => clicks.push(clicked) })

    const button = host.querySelector<HTMLButtonElement>('.timeline-user-message-rail__marker')!
    expect(button.getAttribute('aria-label')).toBe('Jump: change files')

    button.click()
    await nextTick()

    expect(clicks).toEqual([marker])
  })

  it('uses the dense-rail labels for the listbox and active option', async () => {
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null)
    const denseMarkers: ChatUiTimelineUserMessageMarker[] = Array.from({ length: 104 }, (_, index) => ({
      id: `marker-${index}`,
      visibleRowKeys: [],
      userMessageId: `user-${index}`,
      preview: `message ${index}`,
      previewLoaded: true,
    }))
    const host = await mountRail({ markers: denseMarkers })

    const listbox = host.querySelector<HTMLElement>('.timeline-user-message-rail__dense-hitbox')!
    expect(listbox.getAttribute('aria-label')).toBe('Timeline of 104 messages')

    listbox.dispatchEvent(new FocusEvent('focus'))
    listbox.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }))
    await nextTick()

    expect(host.querySelector('[role="option"]')?.getAttribute('aria-label')).toBe('2/104: message 1')
  })
})
