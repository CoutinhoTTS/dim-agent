import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App, type Component } from 'vue'
import {
  TimelineArtifactsView,
  type ChatUiArtifactEntry,
  type ChatUiArtifactIcons,
  type ChatUiArtifactLabels,
} from '../src'

const labels: ChatUiArtifactLabels = {
  count: count => `${count} artifacts`,
  allArtifacts: count => `All ${count} artifacts`,
  deleted: 'Deleted',
  previewUnavailable: 'Preview unavailable',
  open: 'Open',
  download: 'Download',
  showLess: 'Show less',
  commitOrPush: 'Commit or push',
  commit: 'Commit',
  commitAndPush: 'Commit and push',
  push: 'Push',
  openWithDefault: 'Open with default app',
  revealInFolder: 'Show in folder',
}

const icon = (name: string): Component => defineComponent({
  name,
  setup: () => () => h('i', { 'data-testid': `icon-${name}` }),
})

const icons: ChatUiArtifactIcons = {
  chevronDown: icon('chevron-down'),
  chevronRight: icon('chevron-right'),
  cloudUpload: icon('cloud-upload'),
  download: icon('download'),
  externalLink: icon('external-link'),
  folderOpen: icon('folder-open'),
  gitCommit: icon('git-commit'),
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

const DropdownMenu = defineComponent({
  name: 'StubDropdownMenu',
  setup: (_, { slots }) => () => h('div', { 'data-testid': 'dropdown' }, slots.default?.()),
})
const DropdownMenuTrigger = defineComponent({
  name: 'StubDropdownMenuTrigger',
  setup: (_, { slots }) => () => h('div', slots.default?.()),
})
const DropdownMenuContent = defineComponent({
  name: 'StubDropdownMenuContent',
  setup: (_, { slots }) => () => h('div', slots.default?.()),
})
const DropdownMenuItem = defineComponent({
  name: 'StubDropdownMenuItem',
  inheritAttrs: false,
  props: { disabled: Boolean },
  emits: ['select'],
  setup: (props, { emit, slots, attrs }) => () => h('button', {
    ...attrs,
    disabled: props.disabled,
    onClick: () => emit('select'),
  }, slots.default?.()),
})

function entries(count: number): ChatUiArtifactEntry[] {
  return Array.from({ length: count }, (_, index) => ({
    path: `/tmp/file-${index + 1}.ts`,
    absolutePath: `/tmp/file-${index + 1}.ts`,
    exists: true,
    name: `file-${index + 1}.ts`,
  }))
}

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

async function mountView(options: {
  rows?: ChatUiArtifactEntry[]
  gitAvailable?: boolean
  localActionsAvailable?: boolean
  downloadAvailable?: boolean
} = {}) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const events = {
    open: vi.fn(),
    download: vi.fn(),
    openExternal: vi.fn(),
    showInFolder: vi.fn(),
    commit: vi.fn(),
    commitAndPush: vi.fn(),
    push: vi.fn(),
    contextmenu: vi.fn(),
  }
  const app = createApp({
    render: () => h(TimelineArtifactsView, {
      entries: options.rows ?? entries(4),
      labels,
      icons,
      fileIconUrl: (path: string) => `icon:${path}`,
      gitAvailable: options.gitAvailable ?? false,
      localActionsAvailable: options.localActionsAvailable ?? true,
      downloadAvailable: options.downloadAvailable ?? true,
      buttonComponent: Button,
      dropdownMenuComponent: DropdownMenu,
      dropdownMenuTriggerComponent: DropdownMenuTrigger,
      dropdownMenuContentComponent: DropdownMenuContent,
      dropdownMenuItemComponent: DropdownMenuItem,
      onOpen: events.open,
      onDownload: events.download,
      'onOpen-external': events.openExternal,
      'onShow-in-folder': events.showInFolder,
      onCommit: events.commit,
      'onCommit-and-push': events.commitAndPush,
      onPush: events.push,
      onContextmenu: events.contextmenu,
    }),
  })
  app.mount(host)
  mounted.push(app)
  await nextTick()
  return { host, events }
}

describe('TimelineArtifactsView', () => {
  it('previews three artifacts and expands the rest', async () => {
    const { host } = await mountView()
    expect(host.textContent).toContain('4 artifacts')
    expect(host.querySelectorAll('[data-testid="timeline-artifact-card"]')).toHaveLength(3)
    expect(host.querySelector('[data-testid="timeline-artifacts-toggle"]')?.textContent).toContain('All 4 artifacts')

    host.querySelector<HTMLElement>('[data-testid="timeline-artifacts-toggle"]')!.click()
    await nextTick()
    expect(host.querySelectorAll('[data-testid="timeline-artifact-card"]')).toHaveLength(4)
    expect(host.querySelector('[data-testid="timeline-artifacts-toggle"]')?.textContent).toContain('Show less')
  })

  it('emits open, download, and context actions with the entry', async () => {
    const { host, events } = await mountView()
    host.querySelector<HTMLElement>('[data-testid="timeline-artifact-open"]')!.click()
    expect(events.open).toHaveBeenCalledWith(expect.objectContaining({ path: '/tmp/file-1.ts' }), expect.any(MouseEvent))

    host.querySelector<HTMLElement>('[data-testid="timeline-artifact-download"]')!.click()
    expect(events.download).toHaveBeenCalled()

    host.querySelector<HTMLElement>('[data-testid="timeline-artifact-dropdown-open-external"]')!.click()
    expect(events.openExternal).toHaveBeenCalledWith(expect.objectContaining({ path: '/tmp/file-1.ts' }))

    host.querySelector<HTMLElement>('[data-testid="timeline-artifact-dropdown-show-in-folder"]')!.click()
    expect(events.showInFolder).toHaveBeenCalled()

    host.querySelector<HTMLElement>('[data-testid="timeline-artifact-card"]')!
      .dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }))
    expect(events.contextmenu).toHaveBeenCalled()
  })

  it('emits turn-scoped git actions', async () => {
    const { host, events } = await mountView({ gitAvailable: true })
    host.querySelector<HTMLElement>('[data-testid="timeline-artifacts-commit"]')!.click()
    expect(events.commit).toHaveBeenCalled()
    host.querySelector<HTMLElement>('[data-testid="timeline-artifacts-commit-and-push"]')!.click()
    expect(events.commitAndPush).toHaveBeenCalled()
    host.querySelector<HTMLElement>('[data-testid="timeline-artifacts-push"]')!.click()
    expect(events.push).toHaveBeenCalled()
  })

  it('renders deleted and non-previewable rows without open actions', async () => {
    const { host } = await mountView({
      rows: [
        { path: '/tmp/deleted.ts', exists: false, name: 'deleted.ts' },
        { path: '/tmp/binary.bin', exists: true, name: 'binary.bin', canOpen: false },
      ],
      localActionsAvailable: false,
      downloadAvailable: false,
    })
    expect(host.querySelector('[data-testid="timeline-artifact-deleted"]')?.textContent).toBe('Deleted')
    expect(host.textContent).toContain('Preview unavailable')
    expect(host.querySelector('[data-testid="timeline-artifact-open"]')).toBeNull()
  })
})
