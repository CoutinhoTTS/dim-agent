import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App, type Component } from 'vue'
import {
  PlainTimelineSurface,
  TimelineErrorBoundary,
  type ChatUiPlainTimelineRow,
  type ChatUiPlainTimelineSurfaceProps,
} from '../src'

const mounted: App[] = []
let emitted: Array<{ event: string; args: unknown[] }> = []
let renderedRowProps: Array<Record<string, unknown>> = []

const RowStub = defineComponent({
  name: 'RowStub',
  props: {
    row: { type: Object, required: true },
    sessionId: { type: String, default: null },
    isLatestTurn: { type: Boolean, default: false },
    timelineRunning: { type: Boolean, default: false },
    turnDetailsExpanded: { type: Boolean, default: false },
    turnDetailsToggleVisible: { type: Boolean, default: true },
    blockExpandedById: { type: Object, default: () => ({}) },
    blockGroupExpandedById: { type: Object, default: () => ({}) },
    assistantUserMessageId: { type: String, default: null },
    retryUserMessage: { type: Object, default: null },
    searchQuery: { type: String, default: '' },
    searchMatchesBySource: { type: Object, default: () => ({}) },
    activeSearchMatchId: { type: String, default: null },
    elapsedNowMs: { type: Number, default: undefined },
    markdownViewportPriority: { type: Boolean, default: true },
    readOnly: { type: Boolean, default: false },
  },
  emits: ['toggle-details', 'edit-message'],
  setup(props, { emit }) {
    return () => {
      const row = props.row as { key: string; kind: string }
      renderedRowProps.push({
        key: row.key,
        sessionId: props.sessionId,
        isLatestTurn: props.isLatestTurn,
        timelineRunning: props.timelineRunning,
        turnDetailsExpanded: props.turnDetailsExpanded,
        turnDetailsToggleVisible: props.turnDetailsToggleVisible,
        blockExpandedById: props.blockExpandedById,
        blockGroupExpandedById: props.blockGroupExpandedById,
        assistantUserMessageId: props.assistantUserMessageId,
        retryUserMessage: props.retryUserMessage,
        searchQuery: props.searchQuery,
        searchMatchesBySource: props.searchMatchesBySource,
        activeSearchMatchId: props.activeSearchMatchId,
        elapsedNowMs: props.elapsedNowMs,
        markdownViewportPriority: props.markdownViewportPriority,
        readOnly: props.readOnly,
      })
      return h('div', {
        'data-testid': 'row-stub',
        'data-row-key': row.key,
      }, [
        h('button', {
          'data-testid': `row-toggle-${row.key}`,
          onClick: () => emit('toggle-details', 'turn-1'),
        }),
        h('button', {
          'data-testid': `row-edit-${row.key}`,
          onClick: () => emit('edit-message', { message: { id: 'user-1' }, content: 'edited' }),
        }),
      ])
    }
  },
})

const ThrowingRow = defineComponent({
  name: 'ThrowingRow',
  setup() {
    return () => {
      throw new Error('row exploded')
    }
  },
})

const BoundaryStub = defineComponent({
  name: 'BoundaryStub',
  setup(_, { slots }) {
    return () => slots.default?.()
  },
})

const ErrorBoundaryHarness = defineComponent({
  name: 'ErrorBoundaryHarness',
  setup(_, { slots }) {
    return () => h(TimelineErrorBoundary, { fallbackText: 'Row failed' }, {
      default: slots.default,
    })
  },
})

const RailStub = defineComponent({
  name: 'RailStub',
  setup() {
    return () => h('div', { 'data-testid': 'rail-stub' })
  },
})

const BrandStub = defineComponent({
  name: 'BrandStub',
  setup() {
    return () => h('div', { 'data-testid': 'brand-stub' })
  },
})

const IconStub = defineComponent({
  name: 'IconStub',
  setup() {
    return () => h('i', { 'data-testid': 'icon-stub' })
  },
})

type SurfaceOverrides = Omit<
  Partial<ChatUiPlainTimelineSurfaceProps>,
  'state' | 'window' | 'rowState' | 'diagnostics' | 'markers' | 'activity' | 'labels' | 'components'
> & {
  state?: Partial<ChatUiPlainTimelineSurfaceProps['state']>
  window?: Partial<ChatUiPlainTimelineSurfaceProps['window']>
  rowState?: Partial<ChatUiPlainTimelineSurfaceProps['rowState']>
  diagnostics?: Partial<ChatUiPlainTimelineSurfaceProps['diagnostics']>
  markers?: Partial<ChatUiPlainTimelineSurfaceProps['markers']>
  activity?: Partial<ChatUiPlainTimelineSurfaceProps['activity']>
  labels?: Partial<ChatUiPlainTimelineSurfaceProps['labels']>
  components?: Partial<ChatUiPlainTimelineSurfaceProps['components']>
}

function makeProps(overrides: SurfaceOverrides = {}): ChatUiPlainTimelineSurfaceProps {
  const rows: ChatUiPlainTimelineRow[] = [
    { kind: 'turn', key: 'row-turn', turn: { id: 'turn-1', isRunning: true, userMessage: { id: 'user-1' } } },
    { kind: 'message', key: 'row-message' },
  ]
  const { state, window, rowState, diagnostics, markers, activity, labels, components, ...rest } = overrides
  return {
    ...rest,
    state: {
      sessionId: 'session-1',
      messageCount: 2,
      rowCount: rows.length,
      isRunning: true,
      isLoadingOlder: false,
      restoreCovered: false,
      restoreCoverVisible: false,
      restoreTransitionSuppressed: true,
      sendLandingSettling: false,
      layoutResizing: false,
      canLoadOlder: true,
      hasNewerGap: false,
      showUserMessageRail: false,
      userMessageRailHost: null,
      showScrollToBottom: true,
      ...state,
    },
    window: {
      start: 3,
      end: 5,
      rows,
      topSpacerPx: 0,
      bottomSpacerPx: 0,
      contentVisibilityEnabled: () => false,
      contentVisibilityStyle: () => undefined,
      ...window,
    },
    rowState: {
      latestTurnRowId: 'turn-1',
      turnUiStateById: {},
      blockExpandedById: {},
      blockGroupExpandedById: {},
      assistantUserMessageIdByRowKey: {},
      userMessageById: new Map(),
      searchQuery: '',
      activeSearchMatchId: null,
      searchMatchesBySource: {},
      elapsedNowMs: undefined,
      presentationActive: true,
      readOnly: false,
      ...rowState,
    },
    diagnostics: {
      prependAnchor: null,
      oldestRawMessageId: 'raw-oldest',
      trimState: 'idle',
      trimAnchorKey: '',
      trimAtBottom: false,
      trimRestoreState: 'idle',
      trimCapturedTop: null,
      trimRestoredTop: null,
      renderWindowStart: 3,
      renderWindowEnd: 5,
      ...diagnostics,
    },
    markers: {
      markers: [],
      loadedMarkers: [],
      rowDomSignature: '3:5',
      ...markers,
    },
    activity: {
      visible: false,
      text: '',
      ...activity,
    },
    labels: {
      emptySession: 'No messages yet',
      loading: 'Loading',
      newerHistoryGap: 'Newer turns are not loaded.',
      returnToLatest: 'Return to latest',
      ...labels,
    },
    components: {
      row: RowStub,
      errorBoundary: BoundaryStub,
      userMessageRail: RailStub,
      brandMark: BrandStub,
      chevronDown: IconStub,
      chevronUp: IconStub,
      ...components,
    },
  }
}

function mountSurface(props: ChatUiPlainTimelineSurfaceProps): { host: HTMLElement; app: App } {
  const host = document.createElement('div')
  document.body.append(host)
  const app = createApp(defineComponent({
    setup() {
      return () => h(PlainTimelineSurface as Component, {
        ...props,
        onLoadLatest: (...args: unknown[]) => emitted.push({ event: 'load-latest', args }),
        onJumpToBottom: (...args: unknown[]) => emitted.push({ event: 'jump-to-bottom', args }),
        onToggleDetails: (...args: unknown[]) => emitted.push({ event: 'toggle-details', args }),
        onEditMessage: (...args: unknown[]) => emitted.push({ event: 'edit-message', args }),
      })
    },
  }))
  app.mount(host)
  mounted.push(app)
  return { host, app }
}

beforeEach(() => {
  emitted = []
  renderedRowProps = []
})

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
  vi.restoreAllMocks()
})

describe('PlainTimelineSurface row dispatch', () => {
  it('projects the render window and per-row state onto the injected row component', async () => {
    const userMessage = { id: 'user-1', content: 'hello' }
    const props = makeProps({
      window: {
        rows: [
          { kind: 'turn', key: 'row-turn', turn: { id: 'turn-1', isRunning: true, userMessage: null } },
          { kind: 'message', key: 'row-message' },
        ],
        topSpacerPx: 120,
        bottomSpacerPx: 240,
        contentVisibilityEnabled: row => row.key === 'row-message',
        contentVisibilityStyle: row => row.key === 'row-message' ? { height: '42px' } : undefined,
      },
      rowState: {
        turnUiStateById: { 'turn-1': { detailsExpanded: true, detailsToggleVisible: false } },
        blockExpandedById: { 'block-1': true },
        blockGroupExpandedById: { 'group-1': true },
        assistantUserMessageIdByRowKey: { 'row-turn': 'user-1' },
        userMessageById: new Map([['user-1', userMessage]]),
        searchQuery: 'alpha',
        activeSearchMatchId: 'match-1',
        searchMatchesBySource: { 'message:user-1': [{ id: 'match-1', sourceMatchIndex: 0, startOffset: 0, endOffset: 5 }] },
        elapsedNowMs: 12_345,
        presentationActive: false,
        readOnly: true,
      },
    })
    const { host } = mountSurface(props)
    await nextTick()

    const shells = host.querySelectorAll('[data-timeline-row-key]')
    expect(Array.from(shells).map(shell => shell.getAttribute('data-timeline-row-key'))).toEqual(['row-turn', 'row-message'])
    expect(Array.from(shells).map(shell => shell.getAttribute('data-timeline-row-index'))).toEqual(['3', '4'])
    expect(shells[0]?.getAttribute('data-timeline-row-message-backed')).toBe('true')
    expect(shells[1]?.getAttribute('data-timeline-cv')).toBe('true')
    expect((shells[1] as HTMLElement).style.height).toBe('42px')

    const topSpacer = host.querySelector('[data-timeline-row-spacer="top"]') as HTMLElement
    const bottomSpacer = host.querySelector('[data-timeline-row-spacer="bottom"]') as HTMLElement
    expect(topSpacer.style.height).toBe('120px')
    expect(bottomSpacer.style.height).toBe('240px')

    expect(renderedRowProps).toHaveLength(2)
    const [turnRow, messageRow] = renderedRowProps
    expect(turnRow).toMatchObject({
      key: 'row-turn',
      sessionId: 'session-1',
      isLatestTurn: true,
      timelineRunning: true,
      turnDetailsExpanded: true,
      turnDetailsToggleVisible: false,
      blockExpandedById: { 'block-1': true },
      blockGroupExpandedById: { 'group-1': true },
      assistantUserMessageId: 'user-1',
      retryUserMessage: userMessage,
      searchQuery: 'alpha',
      activeSearchMatchId: 'match-1',
      elapsedNowMs: 12_345,
      markdownViewportPriority: false,
      readOnly: true,
    })
    expect(messageRow).toMatchObject({
      key: 'row-message',
      isLatestTurn: false,
      turnDetailsExpanded: false,
      turnDetailsToggleVisible: true,
      assistantUserMessageId: null,
      retryUserMessage: null,
      elapsedNowMs: undefined,
    })

    expect(host.querySelector('[data-testid="chat-timeline"]')?.getAttribute('data-render-window-start')).toBe('3')
    expect(host.querySelector('[data-testid="chat-timeline"]')?.getAttribute('data-timeline-trim-state')).toBe('idle')
  })

  it('forwards row and surface events to the host', async () => {
    const { host } = mountSurface(makeProps({
      state: { hasNewerGap: true, showScrollToBottom: true },
    }))
    await nextTick()

    ;(host.querySelector('[data-testid="row-toggle-row-turn"]') as HTMLButtonElement).click()
    ;(host.querySelector('[data-testid="row-edit-row-turn"]') as HTMLButtonElement).click()
    ;(host.querySelector('[data-testid="timeline-scroll-to-bottom"]') as HTMLButtonElement).click()
    ;(host.querySelector('[data-testid="timeline-newer-gap"]') as HTMLButtonElement).click()

    expect(emitted.map(item => item.event)).toEqual([
      'toggle-details',
      'edit-message',
      'jump-to-bottom',
      'load-latest',
    ])
    expect(emitted[0]?.args).toEqual(['turn-1'])
    expect(emitted[1]?.args).toEqual([{ message: { id: 'user-1' }, content: 'edited' }])
  })
})

describe('PlainTimelineSurface notices', () => {
  it('renders the empty notice only when the session has no messages and is not covered', async () => {
    const { host } = mountSurface(makeProps({
      state: { messageCount: 0, rowCount: 0, isRunning: false },
      window: { rows: [] },
    }))
    await nextTick()
    expect(host.textContent).toContain('No messages yet')
    expect(host.querySelectorAll('[data-timeline-row-key]')).toHaveLength(0)

    const covered = mountSurface(makeProps({
      state: { messageCount: 0, rowCount: 0, isRunning: false, restoreCovered: true, restoreCoverVisible: true },
      window: { rows: [] },
    }))
    await nextTick()
    expect(covered.host.textContent).not.toContain('No messages yet')
    expect(covered.host.querySelector('[data-testid="timeline-restore-cover"]')).not.toBeNull()
    expect(covered.host.querySelector('[data-testid="brand-stub"]')).not.toBeNull()
  })

  it('renders the older-history loading bar while keeping existing rows mounted', async () => {
    const { host } = mountSurface(makeProps({ state: { isLoadingOlder: true } }))
    await nextTick()
    const loading = host.querySelector('.timeline-history-loading')
    expect(loading).not.toBeNull()
    expect(loading?.textContent).toContain('Loading')
    expect(host.querySelectorAll('[data-testid="row-stub"]')).toHaveLength(2)
  })

  it('keeps a throwing row behind the injected error boundary', async () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
    const { host } = mountSurface(makeProps({
      window: { rows: [{ kind: 'message', key: 'row-bad' }] },
      components: { row: ThrowingRow, errorBoundary: ErrorBoundaryHarness },
    }))
    await nextTick()

    const fallback = host.querySelector('[data-testid="timeline-row-render-error"]')
    expect(fallback?.textContent).toContain('Row failed')
    expect(host.querySelector('[data-testid="row-stub"]')).toBeNull()
    expect(consoleError).toHaveBeenCalled()
  })

  it('forwards search state even when the filtered window has no rows', async () => {
    const { host } = mountSurface(makeProps({
      state: { messageCount: 0, rowCount: 0, isRunning: false },
      window: { rows: [] },
      rowState: {
        searchQuery: 'needle',
        activeSearchMatchId: 'match-9',
        searchMatchesBySource: { 'message:gone': [{ id: 'match-9', sourceMatchIndex: 0, startOffset: 0, endOffset: 6 }] },
      },
    }))
    await nextTick()

    expect(host.textContent).toContain('No messages yet')
    expect(host.querySelectorAll('[data-testid="row-stub"]')).toHaveLength(0)
    expect(host.querySelector('[data-timeline-row-count]')?.getAttribute('data-timeline-row-count')).toBe('0')
  })
})
