import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  createApp,
  defineComponent,
  h,
  ref,
  type App,
  type Component,
} from 'vue'
import {
  CHAT_UI_HOST,
  useChatUiHost,
  useOptionalChatUiHost,
  type ChatUiCapabilities,
  type ChatUiCommands,
  type ChatUiHost,
  type ChatUiSessionView,
  type ChatUiTimelineHost,
} from '../src'

const capabilities: ChatUiCapabilities = {
  interactive: true,
  terminal: true,
  browserPanel: false,
  filePreview: false,
  mcpApps: false,
  notifications: false,
  attachments: false,
}

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

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

function fixtureSession(): ChatUiSessionView {
  return {
    sessionId: 'session-1',
    status: 'ready',
    messages: [],
    timeline: [],
    running: false,
    stopping: false,
    pendingApprovals: [],
    pendingQuestions: [],
  }
}

const TurnOutputStub = defineComponent({
  name: 'TurnOutputStub',
  props: {
    sessionId: { type: String, default: '' },
  },
  setup(props) {
    return () => h('span', { 'data-testid': 'turn-output' }, `turn:${props.sessionId}`)
  },
})

interface FixtureTimeline {
  timeline: ChatUiTimelineHost
  notifySuccess: ReturnType<typeof vi.fn>
  canEdit: ReturnType<typeof vi.fn>
  canFork: ReturnType<typeof vi.fn>
  getAgentModelLabel: ReturnType<typeof vi.fn>
}

function fixtureTimeline(): FixtureTimeline {
  const notifySuccess = vi.fn()
  const notifyError = vi.fn()
  const canEdit = vi.fn((messageId: string) => messageId !== 'locked-message')
  const canFork = vi.fn((messageId: string) => messageId === 'user-1')
  const getAgentModelLabel = vi.fn((info: { providerId?: string; modelId?: string }) =>
    info.modelId ? `${info.providerId ?? 'provider'}/${info.modelId}` : null)
  const resolveLatestRun = vi.fn(() => ({ runId: 'run-1' }))

  return {
    notifySuccess,
    canEdit,
    canFork,
    getAgentModelLabel,
    timeline: {
      scope: {
        sessionId: ref('session-1'),
        branchId: ref('branch-1'),
        detailSessionId: ref('session-1'),
        restoreActive: ref(false),
      },
      actions: {
        canEdit,
        canFork,
        pending: ref(false),
      },
      hooks: {
        getAgentModelLabel,
        resolveLatestRun,
      },
      slots: {
        turnOutput: TurnOutputStub as Component,
      },
      effects: {
        notify: {
          success: notifySuccess,
          error: notifyError,
        },
      },
    },
  }
}

function fixtureHost(timeline?: ChatUiTimelineHost): ChatUiHost {
  return {
    session: ref(fixtureSession()),
    capabilities: ref(capabilities),
    commands: fixtureCommands(),
    i18n: {
      locale: ref('en'),
      t: key => key,
    },
    ...(timeline === undefined ? {} : { timeline }),
  }
}

const Fixture = defineComponent({
  name: 'TimelineHostFixture',
  setup() {
    const host = useChatUiHost()
    const timeline = host.timeline
    return () => {
      const scope = timeline?.scope
      const actions = timeline?.actions
      const hooks = timeline?.hooks
      const slots = timeline?.slots
      const effects = timeline?.effects
      const turnOutput = slots?.turnOutput
      const latestRun = hooks?.resolveLatestRun?.('session-1')

      return h('div', [
        h('span', {
          'data-testid': 'read-only',
          'data-value': actions?.canEdit('locked-message') ? 'no' : 'yes',
        }),
        h('span', {
          'data-testid': 'can-fork',
          'data-value': actions?.canFork('user-1') ? 'yes' : 'no',
        }),
        h('span', {
          'data-testid': 'branch',
          'data-value': scope?.branchId?.value ?? 'none',
        }),
        h('span', {
          'data-testid': 'restore-active',
          'data-value': scope?.restoreActive?.value ? 'yes' : 'no',
        }),
        h('span', {
          'data-testid': 'model-label',
          'data-value': hooks?.getAgentModelLabel?.({ providerId: 'openai', modelId: 'gpt-5' }) ?? 'none',
        }),
        h('span', {
          'data-testid': 'latest-run',
          'data-value': latestRun?.runId ?? 'none',
        }),
        turnOutput === undefined
          ? null
          : h(turnOutput, { sessionId: scope?.sessionId?.value ?? '' }),
        h('button', {
          'data-testid': 'notify',
          onClick: () => effects?.notify?.success('saved'),
        }),
      ])
    }
  },
})

function mountWithHost(host: ChatUiHost): HTMLElement {
  const container = document.createElement('div')
  document.body.append(container)
  const app = createApp({ render: () => h(Fixture) })
  app.provide(CHAT_UI_HOST, host)
  app.mount(container)
  mounted.push(app)
  return container
}

describe('timeline host fixture', () => {
  it('exposes scope, actions, hooks, slots, and effects through the host', () => {
    const fixture = fixtureTimeline()
    const container = mountWithHost(fixtureHost(fixture.timeline))
    const value = (testId: string): string | null =>
      container.querySelector(`[data-testid="${testId}"]`)?.getAttribute('data-value') ?? null

    expect(value('read-only')).toBe('yes')
    expect(value('can-fork')).toBe('yes')
    expect(value('branch')).toBe('branch-1')
    expect(value('restore-active')).toBe('no')
    expect(value('model-label')).toBe('openai/gpt-5')
    expect(value('latest-run')).toBe('run-1')
    expect(container.querySelector('[data-testid="turn-output"]')?.textContent).toBe('turn:session-1')

    expect(fixture.canEdit).toHaveBeenCalledWith('locked-message')
    expect(fixture.canFork).toHaveBeenCalledWith('user-1')
    expect(fixture.getAgentModelLabel).toHaveBeenCalledWith({ providerId: 'openai', modelId: 'gpt-5' })

    container.querySelector<HTMLButtonElement>('[data-testid="notify"]')?.click()
    expect(fixture.notifySuccess).toHaveBeenCalledWith('saved')
  })
})

describe('optional host lookup', () => {
  it('returns undefined without a provider', () => {
    const MissingFixture = defineComponent({
      setup() {
        const optional = useOptionalChatUiHost()
        return () => h('span', { 'data-missing': optional === undefined ? 'yes' : 'no' })
      },
    })

    const container = document.createElement('div')
    document.body.append(container)
    const app = createApp({ render: () => h(MissingFixture) })
    app.config.warnHandler = () => undefined
    app.mount(container)
    mounted.push(app)

    expect(container.querySelector('[data-missing]')?.getAttribute('data-missing')).toBe('yes')
  })
})
