import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  createApp,
  defineComponent,
  h,
  nextTick,
  ref,
  type App,
  type Component,
} from 'vue'
import {
  CHAT_UI_HOST,
  useChatUiHost,
  type ChatUiCapabilities,
  type ChatUiCommands,
  type ChatUiHost,
  type ChatUiMessage,
  type ChatUiSessionView,
  type ChatUiTimelineHost,
} from '../src'

/**
 * Executable adapter recipe for a non-desktop host (#1837). It intentionally
 * produces every optional ChatUiHost/ChatUiTimelineHost field so contract
 * additions fail here before the web adapter picks them up.
 */

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

function fixtureMessage(overrides: Partial<ChatUiMessage> = {}): ChatUiMessage {
  return {
    id: 'message-1',
    role: 'user',
    content: 'Hello',
    status: 'complete',
    createdAt: new Date('2026-01-01T00:00:00Z'),
    updatedAt: new Date('2026-01-01T00:00:00Z'),
    ...overrides,
  }
}

function fixtureSession(): ChatUiSessionView {
  const message = fixtureMessage()
  return {
    sessionId: 'session-1',
    title: 'Fixture session',
    status: 'ready',
    messages: [message],
    timeline: [],
    running: false,
    stopping: false,
    pendingApprovals: [],
    pendingQuestions: [],
  }
}

function fixtureCapabilities(): ChatUiCapabilities {
  return {
    interactive: true,
    terminal: false,
    browserPanel: false,
    filePreview: false,
    mcpApps: false,
    notifications: false,
    attachments: false,
  }
}

function fixtureCommands(): ChatUiCommands {
  return {
    send: vi.fn(async () => undefined),
    stop: vi.fn(async () => undefined),
    retry: vi.fn(async () => undefined),
    fork: vi.fn(async () => undefined),
    branch: vi.fn(async () => undefined),
    edit: vi.fn(async () => undefined),
    approve: vi.fn(async () => undefined),
    answer: vi.fn(async () => undefined),
    openFile: vi.fn(),
    copy: vi.fn(async () => undefined),
  }
}

function slot(name: string): Component {
  return defineComponent({
    name: `Slot-${name}`,
    setup() {
      return () => h('span', { 'data-slot': name }, name)
    },
  })
}

function fixtureTimeline(): ChatUiTimelineHost {
  return {
    scope: {
      sessionId: ref('session-1'),
      branchId: ref('branch-1'),
      detailSessionId: ref('detail-session-1'),
      restoreActive: ref(false),
    },
    actions: {
      canEdit: vi.fn(() => true),
      canFork: vi.fn(() => true),
      pending: ref(false),
    },
    slots: {
      userContent: slot('userContent'),
      turnOutput: slot('turnOutput'),
      markdown: slot('markdown'),
      media: slot('media'),
      toolResult: slot('toolResult'),
    },
    hooks: {
      hasUserContent: vi.fn(() => true),
      hasToolMedia: vi.fn(() => true),
      systemNoticeText: vi.fn(() => 'system notice'),
      isSystemNoticeMessage: vi.fn(() => true),
      isNonRestPointInput: vi.fn(() => false),
      getAgentModelLabel: vi.fn(() => 'openai/gpt-5'),
      resolveLatestRun: vi.fn(() => ({ runId: 'run-1' })),
      getSubagentDurationMs: vi.fn(() => 42),
    },
    io: {
      readTextAttachment: vi.fn(async () => 'attachment text'),
      readTimelineMessageDetail: vi.fn(async () => ({
        message: { content: 'detail text' },
        assistantPreview: 'assistant preview',
      })),
      readRunChangedPaths: vi.fn(async () => [
        { path: 'src/a.ts', changeType: 'modified' as const },
      ]),
    },
    effects: {
      openPlan: vi.fn(),
      notify: {
        success: vi.fn(),
        error: vi.fn(),
      },
      visibility: {
        fileChanges: ref(true),
      },
      perf: {
        start: vi.fn(() => 1),
        finish: vi.fn(),
        checkpoint: vi.fn(),
      },
    },
  }
}

function fixtureHost(timeline: ChatUiTimelineHost): ChatUiHost {
  return {
    session: ref(fixtureSession()),
    capabilities: ref(fixtureCapabilities()),
    commands: fixtureCommands(),
    i18n: {
      locale: ref('en'),
      t: key => `translated:${key}`,
    },
    timeline,
  }
}

const HostProbe = defineComponent({
  name: 'HostProbe',
  setup() {
    const host = useChatUiHost()
    const timeline = host.timeline
    const message = fixtureMessage()
    const hooks = timeline?.hooks
    const slots = timeline?.slots
    const actions = timeline?.actions
    const scope = timeline?.scope

    void host.commands.send({ text: 'hello' })
    void host.commands.stop()
    void host.commands.retry('message-1')
    void host.commands.fork('message-1')
    void host.commands.branch('message-1')
    void host.commands.edit('message-1', 'edited')
    void host.commands.approve('request-1', 'allow')
    void host.commands.answer('request-1', [{ questionId: 'q1', answer: 'a1' }])
    host.commands.openFile('src/a.ts', 3)
    void host.commands.copy('copied')

    return () => {
      const visibleSlots: Array<[string, Component | undefined]> = [
        ['userContent', slots?.userContent],
        ['turnOutput', slots?.turnOutput],
        ['markdown', slots?.markdown],
        ['media', slots?.media],
        ['toolResult', slots?.toolResult],
      ]

      return h('div', [
        h('span', {
          'data-testid': 'translated',
        }, host.i18n.t('chatUi.assistant')),
        h('span', {
          'data-testid': 'scope',
        }, [scope?.sessionId?.value, scope?.branchId?.value, scope?.detailSessionId?.value].join('|')),
        h('span', {
          'data-testid': 'can-edit',
        }, String(actions?.canEdit('message-1'))),
        h('span', {
          'data-testid': 'can-fork',
        }, String(actions?.canFork('message-1'))),
        h('span', {
          'data-testid': 'has-content',
        }, String(hooks?.hasUserContent?.('message-1'))),
        h('span', {
          'data-testid': 'has-media',
        }, String(hooks?.hasToolMedia?.('call-1'))),
        h('span', {
          'data-testid': 'notice',
        }, hooks?.systemNoticeText?.(message) ?? ''),
        h('span', {
          'data-testid': 'is-notice',
        }, String(hooks?.isSystemNoticeMessage?.(message))),
        h('span', {
          'data-testid': 'non-rest',
        }, String(hooks?.isNonRestPointInput?.(message))),
        h('span', {
          'data-testid': 'model',
        }, hooks?.getAgentModelLabel?.({ providerId: 'openai', modelId: 'gpt-5' }) ?? ''),
        h('span', {
          'data-testid': 'run',
        }, hooks?.resolveLatestRun?.('session-1')?.runId ?? ''),
        h('span', {
          'data-testid': 'subagent-duration',
        }, String(hooks?.getSubagentDurationMs?.('session-1', 'task-1'))),
        ...visibleSlots
          .filter((entry): entry is [string, Component] => entry[1] !== undefined)
          .map(([name, component]) => h(component, { 'data-testid': `slot-${name}` })),
      ])
    }
  },
})

function mountProbe(host: ChatUiHost): HTMLElement {
  const container = document.createElement('div')
  document.body.append(container)
  const app = createApp({ render: () => h(HostProbe) })
  app.provide(CHAT_UI_HOST, host)
  app.mount(container)
  mounted.push(app)
  return container
}

describe('non-desktop ChatUiHost adapter', () => {
  it('produces every host and timeline field without desktop imports', async () => {
    const timeline = fixtureTimeline()
    const host = fixtureHost(timeline)
    const container = mountProbe(host)
    await nextTick()

    const text = (testId: string): string | null =>
      container.querySelector(`[data-testid="${testId}"]`)?.textContent ?? null

    expect(text('translated')).toBe('translated:chatUi.assistant')
    expect(text('scope')).toBe('session-1|branch-1|detail-session-1')
    expect(text('can-edit')).toBe('true')
    expect(text('can-fork')).toBe('true')
    expect(text('has-content')).toBe('true')
    expect(text('has-media')).toBe('true')
    expect(text('notice')).toBe('system notice')
    expect(text('is-notice')).toBe('true')
    expect(text('non-rest')).toBe('false')
    expect(text('model')).toBe('openai/gpt-5')
    expect(text('run')).toBe('run-1')
    expect(text('subagent-duration')).toBe('42')
    expect(container.querySelectorAll('[data-slot]')).toHaveLength(5)

    await expect(timeline.io?.readTextAttachment?.('session-1', 'uploads/a.txt'))
      .resolves.toBe('attachment text')
    await expect(timeline.io?.readTimelineMessageDetail?.({
      sessionId: 'session-1',
      messageId: 'message-1',
      branchId: 'branch-1',
    })).resolves.toEqual({
      message: { content: 'detail text' },
      assistantPreview: 'assistant preview',
    })
    await expect(timeline.io?.readRunChangedPaths?.({
      sessionId: 'session-1',
      runId: 'run-1',
      throughSequence: 7,
    })).resolves.toEqual([{ path: 'src/a.ts', changeType: 'modified' }])

    timeline.effects?.openPlan?.()
    timeline.effects?.notify?.success('saved')
    timeline.effects?.notify?.error('failed')
    timeline.effects?.perf?.start()
    timeline.effects?.perf?.finish('timeline', 1, { rows: 1 })
    timeline.effects?.perf?.checkpoint('frame', { rows: 1 })

    expect(host.commands.send).toHaveBeenCalledWith({ text: 'hello' })
    expect(host.commands.answer).toHaveBeenCalledWith('request-1', [
      { questionId: 'q1', answer: 'a1' },
    ])
    expect(host.commands.openFile).toHaveBeenCalledWith('src/a.ts', 3)
    expect(timeline.effects?.openPlan).toHaveBeenCalled()
    expect(timeline.effects?.notify?.success).toHaveBeenCalledWith('saved')
    expect(timeline.effects?.notify?.error).toHaveBeenCalledWith('failed')
    expect(timeline.effects?.perf?.finish).toHaveBeenCalledWith('timeline', 1, { rows: 1 })
  })

  it('keeps a read-only host interactive=false while commands stay callable', async () => {
    const timeline = fixtureTimeline()
    const host = fixtureHost(timeline)
    host.capabilities.value.interactive = false
    mountProbe(host)
    await nextTick()

    expect(host.capabilities.value.interactive).toBe(false)
    expect(host.commands.send).toHaveBeenCalled()
    expect(host.timeline?.actions?.pending.value).toBe(false)
  })
})
