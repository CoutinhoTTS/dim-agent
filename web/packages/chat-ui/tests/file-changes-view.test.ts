import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App, type Component } from 'vue'
import {
  TimelineFileChangesView,
  type ChatUiFileChangeIcons,
  type ChatUiFileChangeLabels,
  type ChatUiFileChangeRow,
} from '../src'

const labels: ChatUiFileChangeLabels = {
  filesChangedUndone: 'N files changed (undone)',
  fileChangesCount: count => `${count} files changed`,
  binary: 'binary',
  undo: 'Undo',
  reapply: 'Reapply',
  openWithDefault: 'Open with default app',
  revealInFolder: 'Show in folder',
  copyPath: 'Copy path',
  copyRelativePath: 'Copy relative path',
}

const icon = (name: string): Component => defineComponent({
  name,
  setup: () => () => h('i', { 'data-testid': `icon-${name}` }),
})

const icons: ChatUiFileChangeIcons = {
  chevronDown: icon('chevron-down'),
  chevronRight: icon('chevron-right'),
  rotateCcw: icon('rotate-ccw'),
  redo2: icon('redo-2'),
  loader2: icon('loader-2'),
  externalLink: icon('external-link'),
  folderOpen: icon('folder-open'),
  copy: icon('copy'),
}

const Button = defineComponent({
  name: 'StubButton',
  inheritAttrs: false,
  props: { variant: String, size: String, disabled: Boolean },
  emits: ['click'],
  setup: (props, { emit, slots, attrs }) => () => h('button', {
    ...attrs,
    disabled: props.disabled,
    onClick: (event: MouseEvent) => emit('click', event),
  }, slots.default?.()),
})

const ContextMenu = defineComponent({
  name: 'StubContextMenu',
  setup: (_, { slots }) => () => h('div', { 'data-testid': 'context-menu' }, slots.default?.()),
})
const ContextMenuTrigger = defineComponent({
  name: 'StubContextMenuTrigger',
  setup: (_, { slots }) => () => h('div', slots.default?.()),
})
const ContextMenuContent = defineComponent({
  name: 'StubContextMenuContent',
  setup: (_, { slots }) => () => h('div', slots.default?.()),
})
const ContextMenuItem = defineComponent({
  name: 'StubContextMenuItem',
  inheritAttrs: false,
  emits: ['select'],
  setup: (_, { emit, slots, attrs }) => () => h('button', {
    ...attrs,
    onClick: () => emit('select'),
  }, slots.default?.()),
})
const ContextMenuSeparator = defineComponent({
  name: 'StubContextMenuSeparator',
  setup: () => () => h('hr'),
})

function rows(): ChatUiFileChangeRow[] {
  return [
    { path: 'src/a.ts', absolutePath: '/repo/src/a.ts', scope: 'workspace', reportedOnly: false, statusKind: 'modified', insertions: 2, deletions: 1 },
    { path: 'src/b.ts', scope: 'workspace', reportedOnly: false, statusKind: 'added' },
    { path: 'outside.txt', absolutePath: '/tmp/outside.txt', scope: 'external', reportedOnly: false, statusKind: 'deleted', insertions: 0, deletions: 3 },
    { path: 'reported.txt', scope: 'workspace', reportedOnly: true },
  ]
}

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

async function mountView(options: Partial<{
  rows: ChatUiFileChangeRow[]
  summaryCount: number
  isUndone: boolean
  showUndo: boolean
  showReapply: boolean
  canUndo: boolean
  actionPending: boolean
  previewLoading: boolean
  localActionsAvailable: boolean
  diffDisabled: boolean
}> = {}) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const events = {
    expand: vi.fn(),
    diff: vi.fn(),
    undo: vi.fn(),
    reapply: vi.fn(),
    openFile: vi.fn(),
    showInFolder: vi.fn(),
    copyAbsolute: vi.fn(),
    copyRelative: vi.fn(),
  }
  const app = createApp({
    render: () => h(TimelineFileChangesView, {
      rows: options.rows ?? rows(),
      summaryCount: options.summaryCount ?? 4,
      isUndone: options.isUndone ?? false,
      showUndo: options.showUndo ?? true,
      showReapply: options.showReapply ?? false,
      canUndo: options.canUndo ?? true,
      actionPending: options.actionPending ?? false,
      previewLoading: options.previewLoading ?? false,
      localActionsAvailable: options.localActionsAvailable ?? true,
      diffDisabled: options.diffDisabled ?? false,
      labels,
      icons,
      buttonComponent: Button,
      contextMenuComponent: ContextMenu,
      contextMenuTriggerComponent: ContextMenuTrigger,
      contextMenuContentComponent: ContextMenuContent,
      contextMenuItemComponent: ContextMenuItem,
      contextMenuSeparatorComponent: ContextMenuSeparator,
      onExpand: events.expand,
      onDiff: events.diff,
      onUndo: events.undo,
      onReapply: events.reapply,
      'onOpen-file': events.openFile,
      'onShow-in-folder': events.showInFolder,
      'onCopy-absolute': events.copyAbsolute,
      'onCopy-relative': events.copyRelative,
    }),
  })
  app.mount(host)
  mounted.push(app)
  await nextTick()
  return { host, events }
}

describe('TimelineFileChangesView', () => {
  it('renders the summary, expands rows, and shows kind letters/stats', async () => {
    const { host, events } = await mountView()
    expect(host.textContent).toContain('4 files changed')
    expect(host.querySelector('[data-testid="timeline-file-changes-list"]')).toBeNull()

    host.querySelector<HTMLElement>('[data-testid="timeline-file-changes-toggle"]')!.click()
    await nextTick()
    expect(events.expand).toHaveBeenCalledWith(true)
    const list = host.querySelector('[data-testid="timeline-file-changes-list"]')
    expect(list).not.toBeNull()
    expect(host.querySelectorAll('[data-testid="timeline-file-changes-file"]')).toHaveLength(4)
    expect(host.querySelector('[data-testid="timeline-file-changes-status"]')?.textContent).toBe('M')
    expect(host.textContent).toContain('+2/-1')
    expect(host.textContent).toContain('binary')
  })

  it('emits diff only for eligible workspace rows', async () => {
    const { host, events } = await mountView()
    host.querySelector<HTMLElement>('[data-testid="timeline-file-changes-toggle"]')!.click()
    await nextTick()
    const fileButtons = [...host.querySelectorAll<HTMLElement>('[data-testid="timeline-file-changes-file"]')]
    fileButtons[0]!.click()
    expect(events.diff).toHaveBeenCalledWith(expect.objectContaining({ path: 'src/a.ts' }))

    fileButtons[1]!.click()
    expect(events.diff).toHaveBeenCalledTimes(2)
    fileButtons[2]!.click()
    fileButtons[3]!.click()
    expect(events.diff).toHaveBeenCalledTimes(2)
  })

  it('emits undo and reapply with gating', async () => {
    const undoView = await mountView({ canUndo: false })
    expect(undoView.host.querySelector<HTMLButtonElement>('[data-testid="timeline-file-changes-undo"]')?.disabled).toBe(true)

    const reapplyView = await mountView({ showUndo: false, showReapply: true, actionPending: true })
    const reapply = reapplyView.host.querySelector<HTMLButtonElement>('[data-testid="timeline-file-changes-reapply"]')
    expect(reapply?.disabled).toBe(true)
    reapplyView.host.querySelector<HTMLElement>('[data-testid="timeline-file-changes-toggle"]')!.click()
    await nextTick()
    expect(reapplyView.host.querySelector('[data-testid="icon-loader-2"]')).not.toBeNull()
  })

  it('emits context-menu actions for open, reveal, and path copies', async () => {
    const { host, events } = await mountView()
    host.querySelector<HTMLElement>('[data-testid="timeline-file-changes-toggle"]')!.click()
    await nextTick()
    host.querySelector<HTMLElement>('[data-testid="timeline-file-changes-context-open"]')!.click()
    expect(events.openFile).toHaveBeenCalledWith(expect.objectContaining({ path: 'src/a.ts' }))
    host.querySelector<HTMLElement>('[data-testid="timeline-file-changes-context-show-in-folder"]')!.click()
    expect(events.showInFolder).toHaveBeenCalled()
    host.querySelector<HTMLElement>('[data-testid="timeline-file-changes-context-copy-absolute"]')!.click()
    expect(events.copyAbsolute).toHaveBeenCalled()
    host.querySelector<HTMLElement>('[data-testid="timeline-file-changes-context-copy-relative"]')!.click()
    expect(events.copyRelative).toHaveBeenCalled()
  })

  it('uses the undone summary and omits the undo action when requested', async () => {
    const { host } = await mountView({ isUndone: true, summaryCount: 2, showUndo: false, showReapply: false })
    expect(host.textContent).toContain('N files changed (undone)')
    expect(host.querySelector('[data-testid="timeline-file-changes-undo"]')).toBeNull()
    expect(host.querySelector('[data-testid="timeline-file-changes-reapply"]')).toBeNull()
  })
})
