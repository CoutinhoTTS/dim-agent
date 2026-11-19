import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, ref, type App } from 'vue'
import {
  CHAT_UI_HOST,
  TimelineRow,
  type ChatUiHost,
  type ChatUiTimelineHost,
  type ChatUiTimelineRow,
  type ChatUiTimelineRowBlock,
  type ChatUiTimelineRowMessage,
  type ChatUiTurnTerminalState,
} from '../src'

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

function message(
  id: string,
  overrides: Partial<ChatUiTimelineRowMessage> = {},
): ChatUiTimelineRowMessage {
  return {
    id,
    role: 'user',
    content: `message ${id}`,
    status: 'complete',
    createdAt: new Date('2026-08-29T00:00:00Z'),
    updatedAt: new Date('2026-08-29T00:00:00Z'),
    ...overrides,
  }
}

function block(
  id: string,
  overrides: Partial<ChatUiTimelineRowBlock> = {},
): ChatUiTimelineRowBlock {
  return {
    id,
    kind: 'text',
    status: 'complete',
    content: 'Done.',
    startedAt: null,
    endedAt: null,
    durationMs: null,
    ...overrides,
  }
}

function turnRow(overrides: Partial<Extract<ChatUiTimelineRow, { kind: 'turn' }>['turn']> = {}): Extract<ChatUiTimelineRow, { kind: 'turn' }> {
  const now = new Date('2026-08-29T00:00:00Z')
  return {
    kind: 'turn',
    key: 'turn-1',
    turn: {
      id: 'turn-1',
      userMessage: message('user-1'),
      messageIds: ['user-1', 'assistant-1'],
      forkAnchorMessageId: 'assistant-1',
      forkAfterSequence: 7,
      blocks: [block('block-1')],
      model: 'test-model',
      canShowWorkDuration: false,
      workDurationMs: null,
      turnStartedAt: now,
      turnEndedAt: now,
      isRunning: false,
      ...overrides,
    },
  }
}

const UserMessage = defineComponent({
  name: 'StubUserMessage',
  inheritAttrs: false,
  props: {
    message: Object,
    readOnly: Boolean,
    canBranch: Boolean,
    branchDisabled: Boolean,
  },
  emits: ['edit-message', 'branch-message', 'fork-message'],
  setup: (props, { emit, attrs }) => () => h('div', {
    ...attrs,
    'data-testid': 'user-message',
    'data-message-id': (props.message as ChatUiTimelineRowMessage | null)?.id,
    'data-can-branch': String(props.canBranch),
    'data-branch-disabled': String(props.branchDisabled),
    'data-read-only': String(props.readOnly),
  }, [
    h('button', {
      'data-testid': 'user-branch',
      onClick: () => emit('branch-message'),
    }),
    h('button', {
      'data-testid': 'user-fork',
      onClick: () => emit('fork-message'),
    }),
  ]),
})

const AssistantMessage = defineComponent({
  name: 'StubAssistantMessage',
  inheritAttrs: false,
  props: {
    blocks: { type: Array, default: () => [] },
    terminal: { type: Object, default: null },
    isRunning: Boolean,
    readOnly: Boolean,
    mcpAppInlineAnchorsEnabled: Boolean,
    notificationTextByBlockId: { type: Object, default: () => ({}) },
    subagentTargetByNotificationBlockId: { type: Object, default: () => ({}) },
  },
  emits: ['toggle-block'],
  setup: (props, { emit, attrs }) => () => h('div', {
    ...attrs,
    'data-testid': 'assistant-message',
    'data-block-count': String((props.blocks as unknown[]).length),
    'data-terminal-status': (props.terminal as ChatUiTurnTerminalState | null)?.status,
    'data-running': String(props.isRunning),
    'data-read-only': String(props.readOnly),
    'data-mcp-anchors': String(props.mcpAppInlineAnchorsEnabled),
    'data-notice-texts': JSON.stringify(props.notificationTextByBlockId),
    'data-subagent-targets': JSON.stringify(props.subagentTargetByNotificationBlockId),
  }, [
    h('button', {
      'data-testid': 'assistant-toggle-block',
      onClick: () => emit('toggle-block', 'block-1'),
    }),
  ]),
})

const Artifacts = defineComponent({
  name: 'StubArtifacts',
  inheritAttrs: false,
  setup: (_, { attrs }) => () => h('div', { ...attrs, 'data-testid': 'artifacts' }),
})

const FileChanges = defineComponent({
  name: 'StubFileChanges',
  inheritAttrs: false,
  setup: (_, { attrs }) => () => h('div', { ...attrs, 'data-testid': 'file-changes' }),
})

const AssistantActions = defineComponent({
  name: 'StubAssistantActions',
  inheritAttrs: false,
  props: {
    canRetry: Boolean,
    retryDisabled: Boolean,
    forkMessageId: { type: String, default: null },
    forkDisabled: Boolean,
  },
  emits: ['fork-message', 'retry-message'],
  setup: (props, { emit, attrs }) => () => h('div', {
    ...attrs,
    'data-testid': 'assistant-actions',
    'data-can-retry': String(props.canRetry),
    'data-retry-disabled': String(props.retryDisabled),
    'data-fork-message-id': props.forkMessageId,
    'data-fork-disabled': String(props.forkDisabled),
  }, [
    h('button', {
      'data-testid': 'assistant-retry',
      onClick: () => emit('retry-message'),
    }),
    h('button', {
      'data-testid': 'assistant-fork',
      onClick: () => emit('fork-message'),
    }),
  ]),
})

const CompressionDivider = defineComponent({
  name: 'StubCompressionDivider',
  inheritAttrs: false,
  props: {
    row: { type: Object, default: null },
    message: { type: Object, default: null },
  },
  setup: (props, { attrs }) => () => h('div', {
    ...attrs,
    'data-testid': 'compression-divider',
    'data-row-kind': (props.row as { kind?: string } | null)?.kind,
    'data-message-id': (props.message as ChatUiTimelineRowMessage | null)?.id,
  }),
})

function mountRow(
  row: ChatUiTimelineRow,
  props: Record<string, unknown> = {},
  options: {
    timeline?: ChatUiTimelineHost
  } = {},
): { host: HTMLElement; events: Record<string, ReturnType<typeof vi.fn>> } {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const events = {
    toggleDetails: vi.fn(),
    toggleBlock: vi.fn(),
    toggleBlockGroup: vi.fn(),
    editMessage: vi.fn(),
    retryMessage: vi.fn(),
    forkMessage: vi.fn(),
    branchMessage: vi.fn(),
    previewFile: vi.fn(),
    openAgent: vi.fn(),
  }
  const app = createApp({
    setup() {
      return () => h(TimelineRow, {
        row,
        sessionId: 'session-1',
        turnDetailsExpanded: true,
        turnDetailsToggleVisible: true,
        blockExpandedById: {},
        blockGroupExpandedById: {},
        userMessageComponent: UserMessage,
        assistantMessageComponent: AssistantMessage,
        artifactsComponent: Artifacts,
        fileChangesComponent: FileChanges,
        assistantActionsComponent: AssistantActions,
        compressionDividerComponent: CompressionDivider,
        onToggleDetails: events.toggleDetails,
        onToggleBlock: events.toggleBlock,
        onToggleBlockGroup: events.toggleBlockGroup,
        onEditMessage: events.editMessage,
        onRetryMessage: events.retryMessage,
        onForkMessage: events.forkMessage,
        onBranchMessage: events.branchMessage,
        onPreviewFile: events.previewFile,
        onOpenAgent: events.openAgent,
        ...props,
      })
    },
  })
  if (options.timeline) {
    app.provide(CHAT_UI_HOST, { timeline: options.timeline } as ChatUiHost)
  }
  app.mount(host)
  mounted.push(app)
  return { host, events }
}

describe('TimelineRow dispatch', () => {
  it('renders the host user, assistant, and actions renderers for a turn row', async () => {
    const { host } = mountRow(turnRow())
    await nextTick()

    expect(host.querySelector('[data-testid="user-message"]')?.getAttribute('data-message-id')).toBe('user-1')
    expect(host.querySelector('[data-testid="assistant-message"]')?.getAttribute('data-block-count')).toBe('1')
    expect(host.querySelector('[data-testid="assistant-actions"]')).not.toBeNull()
    expect(host.querySelector('[data-testid="timeline-footer-region"]')?.classList.contains('collapse-grid-open')).toBe(true)
  })

  it('forwards block toggles to the row owner', async () => {
    const { host, events } = mountRow(turnRow())
    await nextTick()

    host.querySelector<HTMLButtonElement>('[data-testid="assistant-toggle-block"]')!.click()
    expect(events.toggleBlock).toHaveBeenCalledWith('block-1')
  })

  it('routes display and divider message rows to the compression divider', async () => {
    const { host } = mountRow({
      kind: 'display',
      key: 'display-1',
      row: { kind: 'context_compression' },
    })
    await nextTick()
    expect(host.querySelector('[data-testid="compression-divider"]')?.getAttribute('data-row-kind')).toBe('context_compression')

    const { host: messageHost } = mountRow({
      kind: 'message',
      key: 'message-1',
      message: message('divider-1', { role: 'divider', content: '' }),
    })
    await nextTick()
    expect(messageHost.querySelector('[data-testid="compression-divider"]')?.getAttribute('data-message-id')).toBe('divider-1')
  })

  it('routes window truncation display rows to the shared divider', async () => {
    const { host } = mountRow({
      kind: 'display',
      key: 'display-1',
      row: { kind: 'window_truncation' },
    })
    await nextTick()

    expect(host.querySelector('.timeline-window-truncation')).not.toBeNull()
    expect(host.querySelector('[data-testid="compression-divider"]')).toBeNull()
  })
})

describe('TimelineRow retry, fork, and branch gating', () => {
  it('gates retry on a retryable, non-empty user message', async () => {
    const { host } = mountRow(turnRow(), {
      isRetryableUserMessage: () => false,
    })
    await nextTick()
    expect(host.querySelector('[data-testid="assistant-actions"]')?.getAttribute('data-can-retry')).toBe('false')

    const { host: normalHost } = mountRow(turnRow())
    await nextTick()
    expect(normalHost.querySelector('[data-testid="assistant-actions"]')?.getAttribute('data-can-retry')).toBe('true')
  })

  it('hides retry while the turn is running', async () => {
    const { host } = mountRow(turnRow({ isRunning: true }), { timelineRunning: true })
    await nextTick()
    expect(host.querySelector('[data-testid="assistant-actions"]')?.getAttribute('data-can-retry')).toBe('false')
  })

  it('gates the assistant fork on both the anchor and the turn-end sequence', async () => {
    const { host, events } = mountRow(turnRow())
    await nextTick()
    expect(host.querySelector('[data-testid="assistant-actions"]')?.getAttribute('data-fork-message-id')).toBe('assistant-1')

    host.querySelector<HTMLButtonElement>('[data-testid="assistant-fork"]')!.click()
    expect(events.forkMessage).toHaveBeenCalledWith({
      id: 'assistant-1',
      role: 'assistant',
      content: undefined,
      sequence: 7,
    })

    const { host: noSequenceHost } = mountRow(turnRow({ forkAfterSequence: null }))
    await nextTick()
    expect(noSequenceHost.querySelector('[data-testid="assistant-actions"]')?.getAttribute('data-fork-message-id')).toBeNull()
  })

  it('emits the user fork with its order-key sequence', async () => {
    const { host, events } = mountRow(turnRow({
      userMessage: message('user-1', { orderKey: '00000000000000000007' }),
    }))
    await nextTick()

    host.querySelector<HTMLButtonElement>('[data-testid="user-fork"]')!.click()
    expect(events.forkMessage).toHaveBeenCalledWith(expect.objectContaining({
      id: 'user-1',
      role: 'user',
      sequence: 7,
    }))
  })

  it('gates branch on rest-point user messages and read-only rows', async () => {
    const { host, events } = mountRow(turnRow())
    await nextTick()
    const user = host.querySelector('[data-testid="user-message"]')
    expect(user?.getAttribute('data-can-branch')).toBe('true')
    expect(user?.getAttribute('data-branch-disabled')).toBe('false')

    host.querySelector<HTMLButtonElement>('[data-testid="user-branch"]')!.click()
    expect(events.branchMessage).toHaveBeenCalledWith({ id: 'user-1', role: 'user' })

    const { host: blockedHost } = mountRow(turnRow(), { canBranchFromUserMessage: () => false })
    await nextTick()
    expect(blockedHost.querySelector('[data-testid="user-message"]')?.getAttribute('data-can-branch')).toBe('false')
  })

  it('disables branch while the latest turn is running', async () => {
    const { host } = mountRow(turnRow({ isRunning: true }), { isLatestTurn: true, timelineRunning: true })
    await nextTick()
    expect(host.querySelector('[data-testid="user-message"]')?.getAttribute('data-branch-disabled')).toBe('true')
  })

  it('disables actions while host timeline actions are pending', async () => {
    const timeline: ChatUiTimelineHost = {
      actions: {
        canEdit: () => true,
        canFork: () => true,
        pending: ref(true),
      },
    }
    const { host } = mountRow(turnRow({ isRunning: true }), { timelineRunning: true }, { timeline })
    await nextTick()
    expect(host.querySelector('[data-testid="assistant-actions"]')?.getAttribute('data-fork-disabled')).toBe('true')
    expect(host.querySelector('[data-testid="user-message"]')?.getAttribute('data-branch-disabled')).toBe('true')
  })
})

describe('TimelineRow notices and fallback rendering', () => {
  it('renders the host system-notice text instead of the raw message content', async () => {
    const { host } = mountRow({
      kind: 'message',
      key: 'notice-1',
      message: message('notice-1', { role: 'user', content: 'raw notification' }),
    }, {
      resolveSystemNoticeText: (source: ChatUiTimelineRowMessage) => source.id === 'notice-1' ? 'Background task completed' : undefined,
    })
    await nextTick()

    expect(host.textContent).toContain('Background task completed')
    expect(host.textContent).not.toContain('raw notification')
  })

  it('falls back to plain message content when the host reports no notice', async () => {
    const { host } = mountRow({
      kind: 'message',
      key: 'plain-1',
      message: message('plain-1', { role: 'user', content: 'plain body' }),
    })
    await nextTick()

    expect(host.textContent).toContain('plain body')
  })

  it('renders a fallback terminal through the assistant renderer', async () => {
    const terminal: ChatUiTurnTerminalState = {
      status: 'failed',
      reason: 'provider_error',
      sourceMessageId: 'terminal-1',
    }
    const { host } = mountRow({
      kind: 'message',
      key: 'terminal-1',
      message: message('terminal-1', { role: 'user', content: 'raw terminal record' }),
    }, {
      readTurnTerminal: (source: ChatUiTimelineRowMessage) => source.id === 'terminal-1' ? terminal : undefined,
    })
    await nextTick()

    expect(host.querySelector('[data-testid="assistant-message"]')?.getAttribute('data-terminal-status')).toBe('failed')
    expect(host.textContent).not.toContain('raw terminal record')
  })

  it('passes notification text and subagent targets to the assistant renderer', async () => {
    const notification = message('notice-1', { role: 'user', content: '<notification />' })
    const { host } = mountRow(turnRow({
      blocks: [block('notice-block', { kind: 'notification', notificationMessage: notification })],
    }), {
      resolveSystemNoticeText: (source: ChatUiTimelineRowMessage) => source.id === 'notice-1' ? 'Task failed' : undefined,
      readSubagentTarget: (source: ChatUiTimelineRowBlock) => source.id === 'notice-block'
        ? { subagentSessionId: 'child-1', title: 'Scout' }
        : null,
    })
    await nextTick()

    const assistant = host.querySelector('[data-testid="assistant-message"]')
    expect(assistant?.getAttribute('data-notice-texts')).toBe(JSON.stringify({ 'notice-block': 'Task failed' }))
    expect(assistant?.getAttribute('data-subagent-targets')).toBe(JSON.stringify({ 'notice-block': { subagentSessionId: 'child-1', title: 'Scout' } }))
  })
})

describe('TimelineRow latest-turn surfaces and MCP gating', () => {
  it('renders artifacts and file changes for the latest turn with a run id', async () => {
    const { host } = mountRow(turnRow(), { isLatestTurn: true, turnRunId: 'run-1', fileChangesVisible: true })
    await nextTick()

    expect(host.querySelector('[data-testid="artifacts"]')).not.toBeNull()
    expect(host.querySelector('[data-testid="file-changes"]')).not.toBeNull()
  })

  it('hides latest-turn surfaces when the file-changes capability is off or the row is read-only', async () => {
    const { host } = mountRow(turnRow(), { isLatestTurn: true, turnRunId: 'run-1', fileChangesVisible: false })
    await nextTick()
    expect(host.querySelector('[data-testid="artifacts"]')).not.toBeNull()
    expect(host.querySelector('[data-testid="file-changes"]')).toBeNull()

    const { host: readOnlyHost } = mountRow(turnRow(), {
      isLatestTurn: true,
      turnRunId: 'run-1',
      fileChangesVisible: true,
      readOnly: true,
    })
    await nextTick()
    expect(readOnlyHost.querySelector('[data-testid="artifacts"]')).toBeNull()
    expect(readOnlyHost.querySelector('[data-testid="file-changes"]')).toBeNull()
  })

  it('keeps the MCP inline anchor flag enabled by default and forces it off for read-only rows', async () => {
    const { host } = mountRow(turnRow())
    await nextTick()
    expect(host.querySelector('[data-testid="assistant-message"]')?.getAttribute('data-mcp-anchors')).toBe('true')

    const { host: readOnlyHost } = mountRow(turnRow(), { readOnly: true })
    await nextTick()
    expect(readOnlyHost.querySelector('[data-testid="assistant-message"]')?.getAttribute('data-mcp-anchors')).toBe('false')
  })
})
