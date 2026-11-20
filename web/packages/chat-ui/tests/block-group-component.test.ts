import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, ref, type App, type Component, type Ref } from 'vue'
import {
  TimelineBlockGroup,
  type ChatUiBackgroundTaskNotice,
  type ChatUiTimelineBlockGroupBlock,
  type ChatUiTimelineBlockGroupIcons,
  type ChatUiTimelineBlockGroupLabels,
} from '../src'

const labels: ChatUiTimelineBlockGroupLabels = {
  backgroundTaskCommand: 'command',
  open: 'Open',
  separator: ' · ',
  workActionSeparator: ', ',
  backgroundTaskStatus: {
    completed: 'completed',
    failed: 'failed',
    cancelled: 'cancelled',
  },
}

const icon = (name: string): Component => defineComponent({
  name,
  setup: () => () => h('i', { 'data-testid': `icon-${name}` }),
})

const icons: ChatUiTimelineBlockGroupIcons = {
  workflow: icon('workflow'),
  chevron: icon('chevron'),
  bot: icon('bot'),
  terminal: icon('terminal'),
  layers: icon('layers'),
}

const Button = defineComponent({
  name: 'StubButton',
  inheritAttrs: false,
  props: { variant: String },
  emits: ['click'],
  setup: (props, { emit, slots, attrs }) => () => h('button', {
    ...attrs,
    onClick: (event: MouseEvent) => emit('click', event),
  }, slots.default?.()),
})

const passthrough = (name: string): Component => defineComponent({
  name,
  setup: (_, { slots }) => () => h('div', slots.default?.()),
})

const emptyComponent = (name: string): Component => defineComponent({
  name,
  setup: () => () => null,
})

const childBlock = (name: string): Component => defineComponent({
  name,
  inheritAttrs: false,
  setup: (_, { attrs }) => () => h('div', { ...attrs, 'data-testid': name }),
})

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
  vi.useRealTimers()
})

function block(
  id: string,
  kind: ChatUiTimelineBlockGroupBlock['kind'],
  overrides: Partial<ChatUiTimelineBlockGroupBlock> = {},
): ChatUiTimelineBlockGroupBlock {
  return {
    id,
    kind,
    status: 'complete',
    content: '',
    startedAt: null,
    endedAt: null,
    durationMs: null,
    ...overrides,
  }
}

async function mountGroup(options: {
  blocks: ChatUiTimelineBlockGroupBlock[] | Ref<ChatUiTimelineBlockGroupBlock[]>
  expanded?: boolean
  subagentTargets?: Record<string, { subagentSessionId: string; title: string }>
  readBackgroundTaskNotice?: (block: ChatUiTimelineBlockGroupBlock) => ChatUiBackgroundTaskNotice | null
  loadExecOutput?: (block: ChatUiTimelineBlockGroupBlock, outputPath: string) => Promise<string>
  transitionEnabled?: boolean
} = { blocks: [] }) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const events = {
    toggleBlock: vi.fn(),
    toggleGroup: vi.fn(),
    openAgent: vi.fn(),
    previewFile: vi.fn(),
  }
  const app = createApp({
    setup() {
      return () => h(TimelineBlockGroup, {
        groupId: 'group:one',
        blocks: Array.isArray(options.blocks) ? options.blocks : options.blocks.value,
        expanded: options.expanded ?? true,
        blockExpandedById: {},
        notificationTextByBlockId: { 'notice-1': 'Background task finished' },
        subagentTargetByNotificationBlockId: options.subagentTargets ?? {},
        toolCategoryByBlockId: {
          'read-1': 'read',
          'read-2': 'read',
          'command-1': 'command',
        },
        sessionId: 'session-1',
        mcpAppInlineAnchorsEnabled: false,
        transitionEnabled: options.transitionEnabled ?? false,
        labels,
        icons,
        buttonComponent: Button,
        tooltipComponent: passthrough('Tooltip'),
        tooltipTriggerComponent: passthrough('TooltipTrigger'),
        tooltipContentComponent: passthrough('TooltipContent'),
        thinkingItemComponent: childBlock('thinking-item'),
        toolCallItemComponent: childBlock('tool-call-item'),
        mcpAppInlineAnchorComponent: emptyComponent('mcp-anchor'),
        providerIconComponent: emptyComponent('provider-icon'),
        readBackgroundTaskNotice: options.readBackgroundTaskNotice ?? (() => null),
        getAgentModelLabel: () => 'Provider · Model',
        loadExecOutput: options.loadExecOutput,
        onToggleBlock: events.toggleBlock,
        onToggleBlockGroup: events.toggleGroup,
        onOpenAgent: events.openAgent,
        onPreviewFile: events.previewFile,
      }, {
        'summary-item': ({ item }: { item: { key: string; count: number } }) =>
          h('span', `${item.key}:${item.count}`),
      })
    },
  })
  mounted.push(app)
  app.mount(host)
  await nextTick()
  return { host, events }
}

function railChildren(host: HTMLElement): HTMLElement[] {
  const rail = host.querySelector('[data-testid="timeline-block-group-rail"]')!
  return [...rail.children] as HTMLElement[]
}

function summaryText(host: HTMLElement): string {
  return host.querySelector('[data-testid="timeline-block-group-summary"]')!.textContent!.trim()
}

describe('TimelineBlockGroup', () => {
  it('orders summary categories and exposes the toggle event', async () => {
    const { host, events } = await mountGroup({
      blocks: [
        block('thinking-1', 'thinking'),
        block('read-1', 'tool'),
        block('command-1', 'tool'),
        block('read-2', 'tool'),
        block('notice-1', 'notification'),
      ],
    })

    expect(summaryText(host)).toBe('thinking:1 · read:2, command:1 · notifications:1')
    expect(host.querySelector('[data-summary-category="read"]')).not.toBeNull()
    expect(host.querySelector('[data-summary-category="notifications"]')).not.toBeNull()

    host.querySelector<HTMLButtonElement>('[data-testid="timeline-block-group-toggle"]')!.click()
    expect(events.toggleGroup).toHaveBeenCalledWith('group:one')
  })

  it('renders a subagent notification and expands its result', async () => {
    const notice = (block: ChatUiTimelineBlockGroupBlock): ChatUiBackgroundTaskNotice | null =>
      block.id === 'notice-1'
        ? {
            taskId: 'task-1',
            status: 'completed',
            toolName: 'agent',
            agentName: 'Explorer',
            providerId: 'openai',
            modelId: 'gpt-5',
            summary: 'done',
          }
        : null
    const { host } = await mountGroup({
      blocks: [
        block('notice-1', 'notification', {
          content: '<notification><result>Found the answer</result></notification>',
        }),
      ],
      readBackgroundTaskNotice: notice,
    })

    expect(host.querySelector('[data-testid="timeline-notification-block"]')).not.toBeNull()
    expect(host.textContent).toContain('Explorer')
    expect(host.textContent).toContain('Background task finished')
    expect(host.textContent).toContain('Found the answer')
  })

  it('loads exec output through the host callback and toggles the block', async () => {
    const loadExecOutput = vi.fn(async () => 'build output')
    const { host, events } = await mountGroup({
      blocks: [
        block('exec-1', 'notification', {
          content: '',
        }),
      ],
      readBackgroundTaskNotice: block => block.id === 'exec-1'
        ? {
            taskId: 'task-2',
            status: 'completed',
            toolName: 'exec',
            label: 'npm test',
            outputPath: '/tmp/output.log',
          }
        : null,
      loadExecOutput,
    })

    const toggle = host.querySelector<HTMLButtonElement>('[data-testid="timeline-notification-block"] button')!
    toggle.click()
    await nextTick()
    await nextTick()

    expect(loadExecOutput).toHaveBeenCalledOnce()
    expect(host.textContent).toContain('build output')
    expect(events.toggleBlock).toHaveBeenCalledWith('exec-1')
  })

  it('does not replay the enter animation for the initial mounted rows', async () => {
    const { host } = await mountGroup({
      blocks: [block('thinking-1', 'thinking')],
      transitionEnabled: true,
    })

    // A single work row becomes a group while streaming. The rows already
    // exist, so mounting the rail must not fade/slide them in again.
    expect(railChildren(host)[0]!.style.transitionDelay).toBe('')
  })

  it('keeps the mounted thinking row when the run settles', async () => {
    const blocks = ref<ChatUiTimelineBlockGroupBlock[]>([
      block('thinking-1', 'thinking', { status: 'running', content: 'working' }),
      block('read-1', 'tool', { status: 'running' }),
    ])
    const { host } = await mountGroup({ blocks, transitionEnabled: true })
    const before = railChildren(host)[0]!

    blocks.value = blocks.value.map(block => ({ ...block, status: 'complete' as const }))
    await nextTick()

    expect(railChildren(host)[0]).toBe(before)
  })
})
