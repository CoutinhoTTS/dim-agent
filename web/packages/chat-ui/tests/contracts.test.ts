import { describe, expect, it, vi } from 'vitest'
import { createSSRApp, defineComponent, h, ref } from 'vue'
import { renderToString } from '@vue/server-renderer'
import {
  CHAT_UI_HOST,
  CHAT_UI_MESSAGE_KEYS,
  chatUiEn,
  useChatUiHost,
  useOptionalChatUiHost,
  type ChatUiCapabilities,
  type ChatUiCommands,
  type ChatUiHost,
  type ChatUiI18n,
  type ChatUiSessionView,
  type ChatUiTimelineHost,
} from '../src'

const capabilities: ChatUiCapabilities = {
  interactive: true,
  terminal: false,
  browserPanel: false,
  filePreview: false,
  mcpApps: false,
  notifications: false,
  attachments: false,
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

function fixtureHost(): ChatUiHost {
  const session: ChatUiSessionView = {
    sessionId: 'session-1',
    status: 'ready',
    messages: [
      {
        id: 'message-1',
        role: 'user',
        content: 'Hello',
        status: 'complete',
        createdAt: new Date(0),
        updatedAt: new Date(0),
      },
    ],
    timeline: [],
    running: false,
    stopping: false,
    pendingApprovals: [],
    pendingQuestions: [],
  }
  return {
    session: ref(session),
    capabilities: ref(capabilities),
    commands: fixtureCommands(),
    i18n: {
      locale: ref('en'),
      t: (key) => chatUiEn[key],
    },
  }
}

describe('ChatUiHost', () => {
  it('renders a provider-backed fixture through injection', async () => {
    const host = fixtureHost()
    const Fixture = defineComponent({
      setup() {
        const injected = useChatUiHost()
        return () => h(
          'div',
          { 'data-session': injected.session.value?.sessionId },
          injected.i18n.t('chatUi.assistant'),
        )
      },
    })

    const app = createSSRApp({ render: () => h(Fixture) })
    app.provide(CHAT_UI_HOST, host)

    await expect(renderToString(app)).resolves.toContain('data-session="session-1"')
  })

  it('fails fast when the host provider is missing', async () => {
    const Fixture = defineComponent({
      setup() {
        useChatUiHost()
        return () => h('div')
      },
    })

    const app = createSSRApp({ render: () => h(Fixture) })
    app.config.warnHandler = () => undefined

    await expect(renderToString(app)).rejects.toThrow('Missing ChatUiHost provider')
  })

  it('offers an optional lookup and an additive timeline surface', async () => {
    const timeline: ChatUiTimelineHost = {
      scope: {
        sessionId: ref('session-1'),
        branchId: ref(null),
        detailSessionId: ref('session-1'),
        restoreActive: ref(false),
      },
      actions: {
        canEdit: () => true,
        canFork: () => true,
        pending: ref(false),
      },
      hooks: {
        getAgentModelLabel: info => info.modelId ?? null,
        resolveLatestRun: () => ({ runId: 'run-1' }),
      },
      effects: {
        notify: {
          success: () => undefined,
          error: () => undefined,
        },
      },
    }
    const host: ChatUiHost = { ...fixtureHost(), timeline }

    const MissingFixture = defineComponent({
      setup() {
        const optional = useOptionalChatUiHost()
        return () => h('div', { 'data-missing': optional === undefined ? 'yes' : 'no' })
      },
    })
    const missingApp = createSSRApp({ render: () => h(MissingFixture) })
    await expect(renderToString(missingApp)).resolves.toContain('data-missing="yes"')

    const Fixture = defineComponent({
      setup() {
        const injected = useOptionalChatUiHost()
        return () => h('div', {
          'data-scope': injected?.timeline?.scope?.branchId?.value ?? 'none',
          'data-label': injected?.timeline?.hooks?.getAgentModelLabel?.({ modelId: 'gpt-5' }) ?? 'none',
        })
      },
    })
    const app = createSSRApp({ render: () => h(Fixture) })
    app.provide(CHAT_UI_HOST, host)

    await expect(renderToString(app)).resolves.toContain('data-scope="none"')
    await expect(renderToString(app)).resolves.toContain('data-label="gpt-5"')
  })
})

const TIMELINE_MESSAGE_KEYS = [
  'chatUi.timeline.emptySession',
  'chatUi.timeline.loading',
  'chatUi.timeline.newerHistoryGap',
  'chatUi.timeline.returnToLatest',
  'chatUi.timeline.activity',
  'chatUi.timeline.workStopped',
  'chatUi.timeline.runningFailed',
  'chatUi.timeline.runFailedPlaceholder',
  'chatUi.timeline.notice.userInterrupt',
  'chatUi.timeline.notice.backgroundTaskCommand',
  'chatUi.timeline.notice.backgroundTaskStatus.completed',
  'chatUi.timeline.notice.backgroundTaskStatus.failed',
  'chatUi.timeline.notice.backgroundTaskStatus.cancelled',
  'chatUi.timeline.open',
  'chatUi.timeline.copy',
  'chatUi.timeline.copyMessage',
  'chatUi.timeline.copyEntireMessage',
  'chatUi.timeline.copySuccess',
  'chatUi.timeline.copyErrorMessage',
  'chatUi.timeline.notice.runTerminalErrorDetails.title',
  'chatUi.timeline.retry',
  'chatUi.timeline.forkFromHere',
  'chatUi.timeline.branchFromHere',
] as const satisfies readonly (typeof CHAT_UI_MESSAGE_KEYS)[number][]

describe('chat UI messages', () => {
  it('keeps the key list unique and fully translated in English', () => {
    expect(new Set(CHAT_UI_MESSAGE_KEYS).size).toBe(CHAT_UI_MESSAGE_KEYS.length)
    expect(Object.keys(chatUiEn).sort()).toEqual([...CHAT_UI_MESSAGE_KEYS].sort())
    for (const key of CHAT_UI_MESSAGE_KEYS) {
      expect(chatUiEn[key].trim().length).toBeGreaterThan(0)
    }
  })

  it('includes the additive timeline keys', () => {
    for (const key of TIMELINE_MESSAGE_KEYS) {
      expect(CHAT_UI_MESSAGE_KEYS).toContain(key)
    }
  })

  it('resolves every timeline key through a fixture ChatUiI18n', async () => {
    const i18n: ChatUiI18n = {
      locale: ref('en'),
      t: (key) => chatUiEn[key],
    }
    expect(i18n.locale.value).toBe('en')
    for (const key of TIMELINE_MESSAGE_KEYS) {
      expect(i18n.t(key)).toBe(chatUiEn[key])
      expect(i18n.t(key).trim().length).toBeGreaterThan(0)
    }

    const Fixture = defineComponent({
      setup() {
        const injected = useChatUiHost()
        return () => h('div', { 'data-label': injected.i18n.t('chatUi.timeline.emptySession') })
      },
    })
    const app = createSSRApp({ render: () => h(Fixture) })
    app.provide(CHAT_UI_HOST, { ...fixtureHost(), i18n })

    await expect(renderToString(app)).resolves.toContain('data-label="This session has no messages yet"')
  })
})
