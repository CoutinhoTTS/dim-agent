import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, type App } from 'vue'
import {
  ToolCallInlineSummary,
  resolveToolCallInlineIconKey,
  type ChatUiToolCallInlineIcons,
  type ChatUiToolCallInlineLabels,
  type ChatUiToolCallInlineTrace,
} from '../src'

const labels: ChatUiToolCallInlineLabels = {
  queued: 'Queued',
  listFiles: 'List files',
}

function iconStub(key: string) {
  return {
    component: defineComponent({
      name: `Icon-${key}`,
      render: () => h('i', { 'data-icon': key }),
    }),
  }
}

const icons: ChatUiToolCallInlineIcons = {
  read: iconStub('read'),
  search: iconStub('search'),
  files: iconStub('files'),
  'file-search': iconStub('file-search'),
  'file-write': iconStub('file-write'),
  'file-edit': iconStub('file-edit'),
  terminal: iconStub('terminal'),
  web: iconStub('web'),
  browser: iconStub('browser'),
  cua: iconStub('cua'),
  'ask-user': iconStub('ask-user'),
  todos: iconStub('todos'),
  skill: iconStub('skill'),
  wait: iconStub('wait'),
  agent: iconStub('agent'),
  mcp: iconStub('mcp'),
  generic: iconStub('generic'),
}

const TooltipStub = defineComponent({
  name: 'TooltipStub',
  setup(_, { slots }) {
    return () => h('span', { 'data-testid': 'tooltip' }, slots.default?.())
  },
})

const TooltipTriggerStub = defineComponent({
  name: 'TooltipTriggerStub',
  inheritAttrs: false,
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
      'data-size': props.size,
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
    return () => h('i', { 'data-testid': 'favicon', 'data-hostname': props.hostname, 'data-size': props.size })
  },
})

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

function mountSummary(overrides: Record<string, unknown> = {}): HTMLElement {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(ToolCallInlineSummary, {
      trace: {
        rawToolName: 'read',
        rendererId: 'file',
        labelText: 'Read',
        parameterText: 'src/app.ts',
      } satisfies ChatUiToolCallInlineTrace,
      status: 'complete',
      labels,
      icons,
      fileIconUrl: 'data:image/svg+xml,file-icon',
      tooltipComponent: TooltipStub,
      tooltipTriggerComponent: TooltipTriggerStub,
      tooltipContentComponent: TooltipContentStub,
      providerIconComponent: ProviderIconStub,
      faviconComponent: FaviconStub,
      ...overrides,
    }),
  })
  app.mount(host)
  mounted.push(app)
  return host
}

function summaryIcon(host: HTMLElement): string | null {
  return host.querySelector<HTMLElement>('[data-testid="timeline-tool-icon"]')?.dataset.icon ?? null
}

function text(host: HTMLElement, testId: string): string | null {
  return host.querySelector<HTMLElement>(`[data-testid="${testId}"]`)?.textContent?.trim() ?? null
}

describe('ToolCallInlineSummary', () => {
  it('renders a file read summary with the file icon and resolved text', () => {
    const host = mountSummary()

    expect(host.querySelector('[data-testid="timeline-tool-summary"]')).not.toBeNull()
    expect(summaryIcon(host)).toBe('read')
    expect(text(host, 'timeline-tool-summary-label')).toBe('Read')
    expect(text(host, 'timeline-tool-primary-parameter')).toBe('src/app.ts')
    expect(host.querySelector<HTMLImageElement>('[data-testid="timeline-tool-file-icon"]')?.src)
      .toBe('data:image/svg+xml,file-icon')
    expect(host.querySelector('[data-testid="timeline-tool-summary"]')?.getAttribute('title'))
      .toBe('Read src/app.ts')
  })

  it('renders a command summary with the terminal icon', () => {
    const host = mountSummary({
      trace: {
        rawToolName: 'exec_command',
        rendererId: 'command',
        labelText: 'Ran',
        parameterText: 'pnpm test',
      } satisfies ChatUiToolCallInlineTrace,
      fileIconUrl: null,
    })

    expect(summaryIcon(host)).toBe('terminal')
    expect(text(host, 'timeline-tool-summary-label')).toBe('Ran')
    expect(text(host, 'timeline-tool-primary-parameter')).toBe('pnpm test')
    expect(host.querySelector('[data-testid="timeline-tool-file-icon"]')).toBeNull()
  })

  it('uses the files icon for glob listings and file-search for grep', () => {
    const listingHost = mountSummary({
      trace: {
        rawToolName: 'glob',
        rendererId: 'search',
        labelText: labels.listFiles,
        parameterText: 'src/**/*.ts',
      } satisfies ChatUiToolCallInlineTrace,
      fileIconUrl: null,
    })
    expect(summaryIcon(listingHost)).toBe('files')

    const grepHost = mountSummary({
      trace: {
        rawToolName: 'grep',
        rendererId: 'search',
        labelText: 'Search',
        parameterText: 'needle',
      } satisfies ChatUiToolCallInlineTrace,
      fileIconUrl: null,
    })
    expect(summaryIcon(grepHost)).toBe('search')
  })

  it('keeps the resolved parameter title in the root title without replacing the label', () => {
    const host = mountSummary({
      trace: {
        rawToolName: 'read',
        rendererId: 'file',
        labelText: 'Read',
        parameterText: 'src/app.ts',
        parameterTitle: 'App entry',
      } satisfies ChatUiToolCallInlineTrace,
      fileIconUrl: null,
    })

    expect(host.querySelector('[data-testid="timeline-tool-summary"]')?.getAttribute('title'))
      .toBe('Read App entry')
  })

  it('shimmers the summary while running and clears it when complete', () => {
    const running = mountSummary({ status: 'running' })
    expect(running.querySelector('[data-testid="timeline-tool-summary"]')?.classList.contains('shimmer-text'))
      .toBe(true)

    const complete = mountSummary({ status: 'complete' })
    expect(complete.querySelector('[data-testid="timeline-tool-summary"]')?.classList.contains('shimmer-text'))
      .toBe(false)
  })

  it('renders the queued label and favicon host', () => {
    const host = mountSummary({ status: 'queued', faviconHostname: 'example.com' })

    expect(text(host, 'timeline-tool-queued')).toBe('Queued')
    expect(host.querySelector<HTMLElement>('[data-testid="favicon"]')?.dataset.hostname).toBe('example.com')
  })

  it('renders the agent chip with provider icon and model tooltip', () => {
    const host = mountSummary({
      agentInfo: {
        agentName: 'Planner',
        providerId: 'openai',
        modelId: 'gpt-5',
        externalAgentType: 'codex',
      },
      agentModelLabel: 'OpenAI · GPT-5',
    })

    const chip = host.querySelector<HTMLElement>('[data-testid="timeline-tool-agent-chip"]')
    expect(chip?.textContent).toContain('Planner')
    const provider = host.querySelector<HTMLElement>('[data-testid="provider-icon"]')
    expect(provider?.dataset.providerId).toBe('openai')
    expect(provider?.dataset.modelId).toBe('gpt-5')
    expect(provider?.dataset.externalAgentType).toBe('codex')
    expect(provider?.dataset.size).toBe('14')
    expect(text(host, 'tooltip-content')).toBe('OpenAI · GPT-5')
  })

  it('shows the bare agent name in a popout window', () => {
    const host = mountSummary({
      agentInfo: { agentName: 'Planner' },
      agentModelLabel: 'OpenAI · GPT-5',
      isPopoutWindow: true,
    })

    expect(host.querySelector('[data-testid="timeline-tool-agent-chip"]')).toBeNull()
    expect(host.textContent).toContain('Planner')
  })

  it('renders an external agent type without an agent chip', () => {
    const host = mountSummary({ externalAgentType: 'claude-code' })

    expect(host.querySelector('[data-testid="timeline-tool-agent-chip"]')).toBeNull()
    expect(host.textContent).toContain('claude-code')
    expect(host.querySelector<HTMLElement>('[data-testid="provider-icon"]')?.dataset.externalAgentType)
      .toBe('claude-code')
  })

  it('emits agent-chip-click when the chip is clicked', () => {
    const onAgentChipClick = vi.fn()
    const host = mountSummary({
      agentInfo: { agentName: 'Planner' },
      onAgentChipClick,
    })

    host.querySelector<HTMLElement>('[data-testid="timeline-tool-agent-chip"]')?.click()
    expect(onAgentChipClick).toHaveBeenCalledTimes(1)
  })

  it('maps renderer ids to icon keys', () => {
    expect(resolveToolCallInlineIconKey({ rendererId: 'file', rawToolName: 'write' }, false)).toBe('file-write')
    expect(resolveToolCallInlineIconKey({ rendererId: 'file', rawToolName: 'apply_patch' }, false)).toBe('file-edit')
    expect(resolveToolCallInlineIconKey({ rendererId: 'search', rawToolName: 'glob' }, true)).toBe('files')
    expect(resolveToolCallInlineIconKey({ rendererId: 'search', rawToolName: 'glob' }, false)).toBe('file-search')
    expect(resolveToolCallInlineIconKey({ rendererId: 'wait', rawToolName: 'wait' }, false)).toBe('wait')
    expect(resolveToolCallInlineIconKey({ rendererId: 'unknown', rawToolName: 'x' }, false)).toBeNull()
  })
})
