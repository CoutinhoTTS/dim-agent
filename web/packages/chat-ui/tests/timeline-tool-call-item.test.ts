import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, ref, type App, type Ref } from 'vue'
import {
  TimelineToolCallItem,
  type ChatUiTimelineToolCallBlock,
  type ChatUiTimelineToolCallIcons,
  type ChatUiTimelineToolCallLabels,
} from '../src'

const labels: ChatUiTimelineToolCallLabels = {
  expandMessage: 'Show more',
  collapse: 'Show less',
  loading: 'Loading',
  retry: 'Retry',
  askUserTimedOut: duration => `timed out ${duration}`,
  askUserDeclined: 'Declined',
}

const ButtonStub = defineComponent({
  name: 'ButtonStub',
  setup(_, { slots, attrs }) {
    return () => h('button', attrs, slots.default?.())
  },
})

const InlineSummaryStub = defineComponent({
  name: 'InlineSummaryStub',
  props: {
    trace: { type: Object, required: true },
    status: { type: String, required: true },
    filePath: { type: String, default: null },
    faviconHostname: { type: String, default: null },
    agentInfo: { type: Object, default: null },
    externalAgentType: { type: String, default: null },
  },
  emits: ['agent-chip-click'],
  setup(props, { emit }) {
    return () => h('span', {
      'data-testid': 'timeline-tool-summary',
      'data-renderer-id': (props.trace as { rendererId: string }).rendererId,
      'data-raw-tool': (props.trace as { rawToolName: string }).rawToolName,
      'data-primary': JSON.stringify((props.trace as { primaryParameter: unknown }).primaryParameter),
      'data-file-path': props.filePath ?? '',
      'data-favicon-host': props.faviconHostname ?? '',
      'data-external-agent': props.externalAgentType ?? '',
    }, [
      h('span', { 'data-testid': 'timeline-tool-summary-label' }, (props.trace as { toolLabelKey: string }).toolLabelKey),
      props.agentInfo === null
        ? null
        : h('button', {
            'data-testid': 'agent-chip',
            onClick: () => emit('agent-chip-click'),
          }, (props.agentInfo as { agentName: string }).agentName),
    ])
  },
})

const MediaPanelStub = defineComponent({
  name: 'MediaPanelStub',
  props: { block: { type: Object, required: true } },
  setup(props) {
    return () => h('div', {
      'data-testid': 'media-panel',
      'data-block-id': (props.block as { id: string }).id,
    })
  },
})

const EditReplacementListStub = defineComponent({
  name: 'EditReplacementListStub',
  props: {
    replacements: { type: Array, required: true },
    fileName: { type: String, required: true },
    showIndexes: { type: Boolean, default: true },
  },
  setup(props) {
    return () => h('div', { 'data-testid': 'edit-replacement-list' },
      (props.replacements as { oldText: string; newText: string }[]).map(replacement =>
        h('span', `${replacement.oldText}->${replacement.newText}`)))
  },
})

const ToolCodeBlockStub = defineComponent({
  name: 'ToolCodeBlockStub',
  props: {
    fileName: { type: String, required: true },
    code: { type: String, default: undefined },
    oldCode: { type: String, default: undefined },
    newCode: { type: String, default: undefined },
    showLineNumbers: { type: Boolean, default: true },
    startLine: { type: Number, default: undefined },
    stripLineNumbers: { type: Boolean, default: false },
  },
  setup(props) {
    return () => h('div', {
      'data-testid': 'tool-code-block',
      'data-code': props.code ?? '',
      'data-old-code': props.oldCode ?? '',
      'data-new-code': props.newCode ?? '',
      'data-start-line': String(props.startLine ?? ''),
    })
  },
})

const FaviconStub = defineComponent({
  name: 'FaviconStub',
  props: {
    hostname: { type: String, required: true },
    size: { type: Number, required: true },
  },
  setup(props) {
    return () => h('i', { 'data-testid': 'favicon', 'data-hostname': props.hostname })
  },
})

const ProviderIconStub = defineComponent({
  name: 'ProviderIconStub',
  props: {
    providerId: { type: String, default: undefined },
    modelId: { type: String, default: undefined },
    externalAgentType: { type: String, default: undefined },
    size: { type: Number, default: undefined },
  },
  setup(props) {
    return () => h('i', {
      'data-testid': 'provider-icon',
      'data-provider-id': props.providerId,
      'data-model-id': props.modelId,
      'data-external-agent-type': props.externalAgentType,
    })
  },
})

const AnimatedNumberStub = defineComponent({
  name: 'AnimatedNumberStub',
  props: {
    value: { type: Number, required: true },
    prefix: { type: String, default: '' },
    durationMs: { type: Number, required: true },
  },
  setup(props) {
    return () => h('span', { 'data-testid': 'animated-number' }, `${props.prefix}${props.value}`)
  },
})

const TooltipStub = defineComponent({
  name: 'TooltipStub',
  setup(_, { slots }) {
    return () => h('span', { 'data-testid': 'tooltip' }, slots.default?.())
  },
})
const TooltipTriggerStub = defineComponent({
  name: 'TooltipTriggerStub',
  setup(_, { slots, attrs }) {
    return () => h('span', { ...attrs, 'data-testid': 'tooltip-trigger' }, slots.default?.())
  },
})
const TooltipContentStub = defineComponent({
  name: 'TooltipContentStub',
  setup(_, { slots }) {
    return () => h('span', { 'data-testid': 'tooltip-content' }, slots.default?.())
  },
})

function iconStub(name: string) {
  return defineComponent({
    name: `Icon-${name}`,
    setup() {
      return () => h('i', { 'data-icon': name })
    },
  })
}

const icons: ChatUiTimelineToolCallIcons = {
  chevron: iconStub('chevron'),
  todoCompleted: iconStub('todo-completed'),
  todoInProgress: iconStub('todo-in-progress'),
  todoPending: iconStub('todo-pending'),
}

interface MountOptions {
  expanded?: boolean
  sessionId?: string | null
  hasToolMedia?: (callId: string) => boolean
  getBackgroundTaskName?: (sessionId: string, taskId: string) => string | undefined
  getAgentModelLabel?: (info: { agentName: string }) => string | null
  loadFullOutput?: (request: { messageId: string }) => Promise<{ message?: { content?: string } } | null> | null
}

interface Mounted {
  host: HTMLElement
  expanded: Ref<boolean>
  events: {
    toggle: number
    previewFile: { path: string; startLine?: number }[]
    openAgent: { subagentSessionId: string; title: string }[]
  }
}

const mounted: App[] = []

function mount(
  block: ChatUiTimelineToolCallBlock,
  options: MountOptions = {},
): Mounted {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const expanded = ref(options.expanded ?? false)
  const events: Mounted['events'] = { toggle: 0, previewFile: [], openAgent: [] }
  const app = createApp({
    setup() {
      return () => h(TimelineToolCallItem, {
        block,
        expanded: expanded.value,
        sessionId: options.sessionId === undefined ? 'session-1' : options.sessionId,
        labels,
        icons,
        buttonComponent: ButtonStub,
        inlineSummaryComponent: InlineSummaryStub,
        toolResultComponent: MediaPanelStub,
        mediaPanelComponent: MediaPanelStub,
        editReplacementListComponent: EditReplacementListStub,
        toolCodeBlockComponent: ToolCodeBlockStub,
        faviconComponent: FaviconStub,
        providerIconComponent: ProviderIconStub,
        animatedNumberComponent: AnimatedNumberStub,
        tooltipComponent: TooltipStub,
        tooltipTriggerComponent: TooltipTriggerStub,
        tooltipContentComponent: TooltipContentStub,
        getAgentModelLabel: options.getAgentModelLabel ?? (() => 'Model label'),
        ...(options.hasToolMedia === undefined ? {} : { hasToolMedia: options.hasToolMedia }),
        ...(options.getBackgroundTaskName === undefined ? {} : { getBackgroundTaskName: options.getBackgroundTaskName }),
        ...(options.loadFullOutput === undefined ? {} : { loadFullOutput: options.loadFullOutput }),
        onToggle: () => { events.toggle += 1 },
        onPreviewFile: (target: { path: string; startLine?: number }) => { events.previewFile.push(target) },
        onOpenAgent: (target: { subagentSessionId: string; title: string }) => { events.openAgent.push(target) },
      })
    },
  })
  app.mount(host)
  mounted.push(app)
  return { host, expanded, events }
}

function block(overrides: Partial<ChatUiTimelineToolCallBlock> = {}): ChatUiTimelineToolCallBlock {
  return {
    id: 'block-1',
    kind: 'tool',
    status: 'complete',
    content: '',
    startedAt: null,
    endedAt: null,
    durationMs: null,
    ...overrides,
  }
}

function byTestId(host: HTMLElement, testId: string): HTMLElement | null {
  return host.querySelector(`[data-testid="${testId}"]`)
}

function byTestIdAll(host: HTMLElement, testId: string): HTMLElement[] {
  return Array.from(host.querySelectorAll<HTMLElement>(`[data-testid="${testId}"]`))
}

function click(element: HTMLElement | null): void {
  expect(element).not.toBeNull()
  element!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
}

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

describe('TimelineToolCallItem header', () => {
  it('renders the summary row, block metadata, and toggles expansion from the row', async () => {
    const { host, events } = mount(block({
      toolName: 'exec',
      toolParams: { command: 'pnpm build', cwd: '/repo' },
      toolResult: { content: 'done' },
    }))
    await nextTick()

    const root = byTestId(host, 'timeline-tool-block')!
    expect(root.dataset.blockId).toBe('block-1')
    expect(root.dataset.toolStatus).toBe('complete')
    expect(root.className).toContain('mb-3')
    expect(byTestId(host, 'timeline-tool-summary')?.dataset.rendererId).toBe('command')
    expect(byTestId(host, 'timeline-tool-summary-label')?.textContent).toBe('toolDisplay.runCommand')
    expect(byTestId(host, 'timeline-tool-chevron')).not.toBeNull()

    click(host.querySelector('button'))
    await nextTick()
    expect(events.toggle).toBe(1)
  })

  it('renders diff stats for writes and edits', async () => {
    const write = mount(block({
      toolName: 'write',
      toolParams: { path: '/repo/a.ts', content: 'one\ntwo\nthree' },
    }))
    await nextTick()

    expect(byTestId(write.host, 'timeline-tool-diff-stats')?.textContent).toBe('+3')

    const edit = mount(block({
      toolName: 'edit',
      toolParams: { path: '/repo/a.ts', oldText: 'old', newText: 'one\ntwo' },
    }))
    await nextTick()

    const stats = byTestId(edit.host, 'timeline-tool-diff-stats')!
    expect(stats.textContent).toBe('+2-1')
    expect(stats.querySelectorAll('[data-testid="animated-number"]')).toHaveLength(2)
  })

  it('stages web-search favicons and rolls the overflow count', async () => {
    const { host } = mount(block({
      toolName: 'web_search',
      toolParams: { query: 'dim' },
      toolResult: {
        structuredContent: {
          results: [
            { title: 'A', url: 'https://a.example.com/1', snippet: 'a' },
            { title: 'B', url: 'https://b.example.com/2', snippet: 'b' },
            { title: 'C', url: 'https://c.example.com/3', snippet: 'c' },
            { title: 'D', url: 'https://d.example.com/4', snippet: 'd' },
          ],
        },
      },
    }))
    await nextTick()

    const preview = byTestId(host, 'timeline-tool-search-favicons')!
    expect(preview.querySelectorAll('[data-testid="favicon"]')).toHaveLength(3)
    expect(Array.from(preview.querySelectorAll<HTMLElement>('[data-testid="favicon"]'))
      .map(favicon => favicon.dataset.hostname)).toEqual(['a.example.com', 'b.example.com', 'c.example.com'])
    expect(preview.querySelector('[data-testid="animated-number"]')?.textContent).toBe('+1')
  })

  it('shows the child count and keeps child rows collapsed until expanded', async () => {
    const parentBlock = block({
      id: 'parent',
      toolName: 'codemode',
      childCalls: [
        block({ id: 'child-1', toolName: 'read', toolCallId: 'child-1' }),
        block({ id: 'child-2', toolName: 'grep', status: 'running' }),
      ],
    })
    const collapsed = mount(parentBlock)
    await nextTick()

    expect(byTestId(collapsed.host, 'timeline-tool-child-count')?.textContent).toBe('2')
    expect(byTestId(collapsed.host, 'timeline-tool-child-calls')).toBeNull()
    expect(byTestId(collapsed.host, 'timeline-tool-chevron')).not.toBeNull()
  })

  it('renders the child call list when expanded', async () => {
    const { host } = mount(block({
      id: 'parent',
      toolName: 'codemode',
      childCalls: [
        block({ id: 'child-1', toolName: 'read', toolCallId: 'child-1' }),
        block({ id: 'child-2', toolName: 'grep', status: 'running' }),
      ],
    }), { expanded: true })
    await nextTick()

    const rows = byTestIdAll(host, 'timeline-tool-child-call')
    expect(rows).toHaveLength(2)
    expect(rows.map(row => row.dataset.toolStatus)).toEqual(['complete', 'running'])
    expect(rows[0]!.querySelector<HTMLElement>('[data-testid="timeline-tool-summary"]')?.dataset.rawTool).toBe('read')
  })

  it('previews multi-replacement edits in the collapsed row', async () => {
    const { host } = mount(block({
      toolName: 'edit',
      toolParams: {
        path: '/repo/a.ts',
        edits: [
          { oldText: 'old-one', newText: 'new-one' },
          { oldText: 'old-two', newText: 'new-two', replaceAll: true },
        ],
      },
    }))
    await nextTick()

    const preview = byTestId(host, 'timeline-tool-edit-preview')!
    expect(preview.textContent).toContain('- old-one')
    expect(preview.textContent).toContain('+ new-one')
    expect(preview.textContent).toContain('- old-two')
    expect(preview.textContent).toContain('+ new-two')
    expect(preview.textContent).toContain('replaceAll')
  })
})

describe('TimelineToolCallItem exec and search panels', () => {
  it('renders the exec input, cwd, output, and failure metadata', async () => {
    const { host } = mount(block({
      toolName: 'exec',
      toolParams: { command: 'pnpm build', cwd: '/repo' },
      toolResult: { content: 'boom', isError: true, structuredContent: { exitCode: 2, signal: 'SIGTERM' } },
    }), { expanded: true })
    await nextTick()

    const panel = byTestId(host, 'timeline-tool-exec-panel')!
    expect(panel.textContent).toContain('$ pnpm build')
    expect(panel.textContent).toContain('cwd: /repo')
    expect(panel.textContent).toContain('boom')
    expect(panel.textContent).toContain('exit code: 2')
    expect(panel.textContent).toContain('signal: SIGTERM')
  })

  it('renders the search pattern, path, and output', async () => {
    const { host } = mount(block({
      toolName: 'grep',
      toolParams: { pattern: 'needle', path: '/repo/src' },
      toolResult: { content: 'src/a.ts:1:needle' },
    }), { expanded: true })
    await nextTick()

    const panel = byTestId(host, 'timeline-tool-search-panel')!
    expect(panel.textContent).toContain('pattern: needle')
    expect(panel.textContent).toContain('path: /repo/src')
    expect(panel.textContent).toContain('src/a.ts:1:needle')
  })

  it('renders a glob listing with the path as the primary input', async () => {
    const { host } = mount(block({
      toolName: 'glob',
      toolParams: { pattern: '*', path: '/repo/src' },
      toolResult: { content: 'src/a.ts' },
    }), { expanded: true })
    await nextTick()

    expect(byTestId(host, 'timeline-tool-search-panel')?.textContent).toContain('path: /repo/src')
  })
})

describe('TimelineToolCallItem file panel', () => {
  it('opens the right-panel preview for workspace reads instead of expanding', async () => {
    const { host, events } = mount(block({
      toolName: 'read',
      toolParams: { path: '/repo/a.ts', offset: 10, limit: 5 },
      toolResult: { content: 'line10' },
    }))
    await nextTick()

    expect(byTestId(host, 'timeline-tool-chevron')).toBeNull()
    click(host.querySelector('button'))
    await nextTick()

    expect(events.previewFile).toEqual([{ path: '/repo/a.ts', startLine: 10 }])
    expect(events.toggle).toBe(0)
  })

  it('keeps session blob reads inline and renders the code block', async () => {
    const path = 'sessions/session-1/blobs/files/blob_abc'
    const { host, events } = mount(block({
      toolName: 'read',
      toolTarget: path,
      toolParams: { path },
      toolResult: { content: '1\tconst value = 1' },
    }), { expanded: true })
    await nextTick()

    const panel = byTestId(host, 'timeline-tool-file-panel')!
    expect(panel).not.toBeNull()
    const code = byTestId(host, 'tool-code-block')!
    expect(code.dataset.code).toBe('1\tconst value = 1')
    expect(code.dataset.startLine).toBe('1')
    expect(byTestId(host, 'timeline-tool-file-path-open')).toBeNull()

    click(host.querySelector('button'))
    await nextTick()
    expect(events.toggle).toBe(1)
    expect(events.previewFile).toEqual([])
  })

  it('opens the edited file from the sticky path', async () => {
    const { host, events } = mount(block({
      toolName: 'edit',
      toolParams: { path: '/repo/a.ts', oldText: 'old', newText: 'new' },
    }), { expanded: true })
    await nextTick()

    const path = byTestId(host, 'timeline-tool-file-path-open')!
    expect(path.textContent).toBe('/repo/a.ts')
    expect(path.className).toContain('cursor-pointer')
    expect(path.className).toContain('hover:underline')
    expect(byTestId(host, 'tool-code-block')?.dataset.oldCode).toBe('old')
    expect(byTestId(host, 'tool-code-block')?.dataset.newCode).toBe('new')

    click(path)
    await nextTick()
    expect(events.previewFile).toEqual([{ path: '/repo/a.ts' }])
  })

  it('renders every replacement of a multi-replacement edit', async () => {
    const { host } = mount(block({
      toolName: 'edit',
      toolParams: {
        path: '/repo/a.ts',
        edits: [
          { oldText: 'old-one', newText: 'new-one' },
          { oldText: 'old-two', newText: 'new-two', replaceAll: true },
        ],
      },
    }), { expanded: true })
    await nextTick()

    expect(byTestId(host, 'edit-replacement-list')?.textContent).toContain('old-one->new-one')
    expect(byTestId(host, 'edit-replacement-list')?.textContent).toContain('old-two->new-two')
    expect(byTestId(host, 'tool-code-block')).toBeNull()
  })

  it('renders write content and result errors', async () => {
    const { host } = mount(block({
      toolName: 'write',
      toolParams: { path: '/repo/a.ts', content: 'next' },
      toolResult: { content: 'permission denied', isError: true },
    }), { expanded: true })
    await nextTick()

    expect(byTestId(host, 'timeline-tool-file-path-open')).toBeNull()
    expect(byTestId(host, 'timeline-tool-file-panel')?.textContent).toContain('/repo/a.ts')
    expect(byTestId(host, 'tool-code-block')?.dataset.code).toBe('next')
    expect(byTestId(host, 'timeline-tool-file-panel')?.textContent).toContain('permission denied')
  })
})

describe('TimelineToolCallItem web panels', () => {
  it('renders search results with favicons and dates', async () => {
    const { host } = mount(block({
      toolName: 'web_search',
      toolParams: { query: 'dim' },
      toolResult: {
        structuredContent: {
          results: [{
            title: 'Dim docs',
            url: 'https://docs.example.com/guide',
            snippet: 'A guide',
            datePublished: '2026-01-02T00:00:00Z',
          }],
        },
      },
    }), { expanded: true })
    await nextTick()

    const panel = byTestId(host, 'timeline-tool-web-panel')!
    expect(panel.textContent).toContain('Dim docs')
    expect(panel.textContent).toContain('docs.example.com')
    expect(panel.textContent).toContain('2026-01-02')
    expect(panel.textContent).toContain('A guide')
    expect(panel.querySelector('[data-testid="favicon"]')?.getAttribute('data-hostname')).toBe('docs.example.com')
  })

  it('renders fetched content and the truncation range', async () => {
    const { host } = mount(block({
      toolName: 'web_fetch',
      toolParams: { url: 'https://docs.example.com/guide' },
      toolResult: {
        content: 'page body',
        structuredContent: { truncated: true, contentLength: 2048, returnedLength: 1024 },
      },
    }), { expanded: true })
    await nextTick()

    const panel = byTestId(host, 'timeline-tool-web-panel')!
    expect(panel.textContent).toContain('https://docs.example.com/guide')
    expect(panel.textContent).toContain('page body')
  })

  it('renders a fetch error and the warning marker on the row', async () => {
    const { host } = mount(block({
      toolName: 'web_fetch',
      toolParams: { url: 'https://docs.example.com/missing' },
      toolResult: { content: 'not found', isError: true },
    }), { expanded: true })
    await nextTick()

    expect(byTestId(host, 'timeline-tool-web-error')).not.toBeNull()
    expect(byTestId(host, 'timeline-tool-web-panel')?.textContent).toContain('not found')
  })
})

describe('TimelineToolCallItem ask-user panel', () => {
  const questions = {
    questions: [
      { id: 'choice', header: 'Choice', question: 'Pick one', options: [{ label: 'A' }, { label: 'B' }] },
    ],
  }

  it('renders questions and formatted answers', async () => {
    const { host } = mount(block({
      toolName: 'ask_user',
      toolParams: questions,
      toolResult: {
        structuredContent: {
          declined: false,
          answers: [{ questionId: 'choice', selected: ['A'], custom: 'typed answer' }],
        },
      },
    }), { expanded: true })
    await nextTick()

    const panel = byTestId(host, 'timeline-tool-ask-user-panel')!
    expect(panel.textContent).toContain('Choice')
    expect(panel.textContent).toContain('Pick one')
    expect(panel.textContent).toContain('A, ✎ typed answer')
    expect(panel.textContent).not.toContain('timed out')
  })

  it('renders the timeout copy with the waited duration', async () => {
    const { host } = mount(block({
      toolName: 'ask_user',
      toolParams: questions,
      toolResult: { structuredContent: { code: 'ask_user_timed_out', waitedMs: 300_000 } },
    }), { expanded: true })
    await nextTick()

    expect(byTestId(host, 'timeline-tool-ask-user-timed-out')?.textContent).toBe('timed out 5m 0s')
  })

  it('renders the declined copy', async () => {
    const { host } = mount(block({
      toolName: 'ask_user',
      toolParams: questions,
      toolResult: { structuredContent: { declined: true, answers: [] } },
    }), { expanded: true })
    await nextTick()

    expect(byTestId(host, 'timeline-tool-ask-user-declined')?.textContent).toBe('Declined')
  })
})

describe('TimelineToolCallItem todos, skill, and raw panels', () => {
  it('renders a valid todo snapshot with status icons and priority dots', async () => {
    const { host } = mount(block({
      toolName: 'todo_write',
      toolParams: {
        todos: [
          { id: '1', content: 'Ship it', status: 'completed', priority: 'high' },
          { id: '2', content: 'Review it', status: 'in_progress', priority: 'medium' },
          { id: '3', content: 'Wait', status: 'pending', priority: 'low' },
        ],
      },
    }), { expanded: true })
    await nextTick()

    const panel = byTestId(host, 'timeline-tool-todos-panel')!
    expect(panel.textContent).toContain('Ship it')
    expect(panel.textContent).toContain('Review it')
    expect(panel.textContent).toContain('Wait')
    expect(panel.querySelector('[data-icon="todo-completed"]')).not.toBeNull()
    expect(panel.querySelector('[data-icon="todo-in-progress"]')).not.toBeNull()
    expect(panel.querySelector('[data-icon="todo-pending"]')).not.toBeNull()
  })

  it('renders an invalid todo snapshot as a plain line list', async () => {
    const { host } = mount(block({
      toolName: 'todo_write',
      toolParams: { todos: [{ foo: 'bar' }] },
    }), { expanded: true })
    await nextTick()

    expect(byTestId(host, 'timeline-tool-todos-panel')?.textContent).toContain('foo')
  })

  it('renders skill descriptions', async () => {
    const { host } = mount(block({
      toolName: 'skill',
      toolParams: { skill_name: 'deploy' },
      toolResult: { structuredContent: { description: 'Deploys the app' } },
    }), { expanded: true })
    await nextTick()

    expect(byTestId(host, 'timeline-tool-skill-panel')?.textContent).toContain('Deploys the app')
  })

  it('falls back to the raw input/output panel', async () => {
    const { host } = mount(block({
      toolName: 'mystery_tool',
      toolParams: { alpha: 1 },
      toolResult: { content: 'raw output' },
    }), { expanded: true })
    await nextTick()

    const panel = byTestId(host, 'timeline-tool-raw-panel')!
    expect(panel.textContent).toContain('"alpha": 1')
    expect(panel.textContent).toContain('raw output')
  })
})

describe('TimelineToolCallItem agent panel', () => {
  it('runs a created agent prompt and opens the agent tab from the chip', async () => {
    const { host, events } = mount(block({
      toolName: 'agent',
      toolParams: { action: 'create', agentName: 'Reviewer', prompt: 'Review the diff' },
      toolResult: { structuredContent: { childSessionId: 'child-session' } },
    }), { expanded: true })
    await nextTick()

    expect(byTestId(host, 'timeline-tool-agent-panel')?.textContent).toContain('Review the diff')
    const chip = byTestId(host, 'agent-chip')
    expect(chip?.textContent).toBe('Reviewer')
    click(chip)
    await nextTick()

    expect(events.openAgent).toEqual([{ subagentSessionId: 'child-session', title: 'Reviewer' }])
  })

  it('lists agents with their provider/model metadata', async () => {
    const { host } = mount(block({
      toolName: 'agent',
      toolParams: { action: 'list' },
      toolResult: {
        structuredContent: {
          agents: [{
            taskId: 'task-1',
            agentName: 'Reviewer',
            taskTitle: 'Review it',
            status: 'running',
            providerId: 'anthropic',
            modelId: 'claude',
          }],
        },
      },
    }), { expanded: true })
    await nextTick()

    const panel = byTestId(host, 'timeline-tool-agent-panel')!
    expect(panel.textContent).toContain('Reviewer')
    expect(panel.textContent).toContain('Review it')
    expect(panel.textContent).toContain('running')
    expect(panel.querySelector('[data-testid="provider-icon"]')?.getAttribute('data-model-id')).toBe('claude')
    expect(byTestId(host, 'tooltip-content')?.textContent).toBe('Model label')
  })

  it('renders provider models and external models', async () => {
    const provider = mount(block({
      toolName: 'agent',
      toolParams: { action: 'query_provider_model' },
      toolResult: { structuredContent: { models: [{ providerId: 'anthropic', modelId: 'claude' }] } },
    }), { expanded: true })
    await nextTick()
    expect(byTestId(provider.host, 'timeline-tool-agent-panel')?.textContent).toContain('claude')

    const external = mount(block({
      toolName: 'agent',
      toolParams: { action: 'query_external_model', agentType: 'claude-code' },
      toolResult: { structuredContent: { models: ['opus', 'sonnet'], default: 'opus' } },
    }), { expanded: true })
    await nextTick()
    const panel = byTestId(external.host, 'timeline-tool-agent-panel')!
    expect(panel.textContent).toContain('claude-code')
    expect(panel.textContent).toContain('opus')
    expect(panel.textContent).toContain('(default)')
  })
})

describe('TimelineToolCallItem browser and CUA panels', () => {
  it('renders browser input and output for expandable actions', async () => {
    const { host } = mount(block({
      toolName: 'browser',
      toolParams: { action: 'snapshot', interactiveOnly: true },
      toolResult: { content: 'snapshot body' },
    }), { expanded: true })
    await nextTick()

    const panel = byTestId(host, 'timeline-tool-browser-panel')!
    expect(panel.textContent).toContain('interactiveOnly: true')
    expect(panel.textContent).toContain('snapshot body')
  })

  it('renders CUA output through the injected media panel', async () => {
    const { host } = mount(block({
      id: 'cua-block',
      toolName: 'cua-driver__list_apps',
      toolResult: { content: 'Safari' },
    }), { expanded: true })
    await nextTick()

    expect(byTestId(host, 'media-panel')?.dataset.blockId).toBe('cua-block')
  })

  it('prefers the host tool-result slot when the host reports media', async () => {
    const { host } = mount(block({
      id: 'exec-block',
      toolCallId: 'call-media',
      toolName: 'exec',
      toolParams: { command: 'screenshot' },
      toolResult: { content: 'ok' },
    }), { expanded: true, hasToolMedia: callId => callId === 'call-media' })
    await nextTick()

    expect(byTestId(host, 'media-panel')?.dataset.blockId).toBe('exec-block')
    expect(byTestId(host, 'timeline-tool-exec-panel')).toBeNull()
  })
})

describe('TimelineToolCallItem wait and full output', () => {
  it('resolves the wait task name through the host callback', async () => {
    const getBackgroundTaskName = vi.fn(() => 'Renderer build')
    const { host } = mount(block({
      toolName: 'wait',
      toolParams: { taskId: 'task-bg-1' },
    }), { getBackgroundTaskName })
    await nextTick()

    expect(getBackgroundTaskName).toHaveBeenCalledWith('session-1', 'task-bg-1')
    expect(byTestId(host, 'timeline-tool-summary')?.dataset.primary).toContain('Renderer build')
    expect(byTestId(host, 'timeline-tool-chevron')).toBeNull()
  })

  it('loads full output, retries a failure, and drops the payload on collapse', async () => {
    const loadFullOutput = vi.fn()
      .mockResolvedValueOnce({ message: { content: 'full output' } })
      .mockRejectedValueOnce(new Error('detail unavailable'))
      .mockResolvedValueOnce({ message: { content: 'second full output' } })
    const { host, expanded } = mount(block({
      toolCallId: 'call-1',
      toolName: 'read',
      toolResult: { content: 'preview', dimTruncated: { bytes: 4096 } },
    }), { expanded: true, loadFullOutput })
    await nextTick()

    const toggle = byTestId(host, 'timeline-tool-toggle-full-output')!
    expect(toggle.textContent?.trim()).toBe('Show more')
    click(toggle)
    await nextTick()
    await nextTick()

    expect(loadFullOutput).toHaveBeenCalledWith({ messageId: 'call-1' })
    expect(byTestId(host, 'timeline-tool-full-output')?.textContent).toBe('full output')

    click(byTestId(host, 'timeline-tool-toggle-full-output'))
    await nextTick()
    expect(byTestId(host, 'timeline-tool-full-output')).toBeNull()

    click(byTestId(host, 'timeline-tool-toggle-full-output'))
    await nextTick()
    await nextTick()
    expect(byTestId(host, 'timeline-tool-full-output')).toBeNull()
    expect(byTestId(host, 'timeline-tool-retry-full-output')?.textContent?.trim()).toBe('Retry')

    click(byTestId(host, 'timeline-tool-retry-full-output'))
    await nextTick()
    await nextTick()
    expect(byTestId(host, 'timeline-tool-full-output')?.textContent).toBe('second full output')

    expanded.value = false
    await nextTick()
    expect(byTestId(host, 'timeline-tool-full-output')).toBeNull()
    expect(byTestId(host, 'timeline-tool-toggle-full-output')?.textContent?.trim()).toBe('Show more')
  })

  it('does not request full output without a host transport', async () => {
    const { host } = mount(block({
      toolName: 'read',
      toolResult: { content: 'preview', dimTruncated: { bytes: 4096 } },
    }), { expanded: true })
    await nextTick()

    click(byTestId(host, 'timeline-tool-toggle-full-output'))
    await nextTick()
    expect(byTestId(host, 'timeline-tool-full-output')).toBeNull()
    expect(byTestId(host, 'timeline-tool-retry-full-output')).toBeNull()
  })
})
