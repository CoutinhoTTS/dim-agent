import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App, type Component } from 'vue'
import {
  TimelineUserActions,
  type ChatUiTimelineUserActionIcons,
  type ChatUiTimelineUserActionLabels,
} from '../src'

const labels: ChatUiTimelineUserActionLabels = {
  withdraw: 'Withdraw',
  copy: 'Copy entire message',
  edit: 'Edit message',
  fork: 'Fork from here',
  branch: 'Branch from here',
}

const icon = (name: string): Component => defineComponent({
  name,
  setup: () => () => h('i', { 'data-testid': `icon-${name}` }),
})

const icons: ChatUiTimelineUserActionIcons = {
  withdraw: icon('withdraw'),
  copy: icon('copy'),
  check: icon('check'),
  edit: icon('edit'),
  fork: icon('fork'),
  branch: icon('branch'),
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

const Passthrough = defineComponent({
  name: 'StubPassthrough',
  inheritAttrs: false,
  setup: (_, { slots }) => () => slots.default?.(),
})

const TooltipContent = defineComponent({
  name: 'StubTooltipContent',
  setup: (_, { slots }) => () => h('span', { 'data-testid': 'tooltip-content' }, slots.default?.()),
})

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
  vi.restoreAllMocks()
})

type ActionsProps = {
  timeText?: string
  canWithdraw?: boolean
  canEdit?: boolean
  canFork?: boolean
  canBranch?: boolean
  branchDisabled?: boolean
  actionPending?: boolean
  copied?: boolean
}

async function mountActions(options: ActionsProps = {}) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const events = {
    copy: vi.fn(),
    edit: vi.fn(),
    withdraw: vi.fn(),
    fork: vi.fn(),
    branch: vi.fn(),
  }
  const app = createApp({
    render: () => h(TimelineUserActions, {
      timeText: options.timeText ?? '12:34',
      canWithdraw: options.canWithdraw ?? false,
      canEdit: options.canEdit ?? true,
      canFork: options.canFork ?? true,
      canBranch: options.canBranch ?? true,
      branchDisabled: options.branchDisabled ?? false,
      actionPending: options.actionPending ?? false,
      copied: options.copied ?? false,
      labels,
      icons,
      buttonComponent: Button,
      tooltipProviderComponent: Passthrough,
      tooltipComponent: Passthrough,
      tooltipTriggerComponent: Passthrough,
      tooltipContentComponent: TooltipContent,
      onCopy: events.copy,
      onEdit: events.edit,
      onWithdraw: events.withdraw,
      onFork: events.fork,
      onBranch: events.branch,
    }),
  })
  app.mount(host)
  mounted.push(app)
  await nextTick()
  return { host, events }
}

function actionOrder(host: HTMLElement): (string | null)[] {
  const actions = host.querySelector('[data-testid="timeline-user-actions"]')!
  return Array.from(actions.querySelectorAll<HTMLElement>('[data-testid="timeline-user-sent-time"], button[data-testid]'))
    .map(element => element.getAttribute('data-testid'))
}

describe('TimelineUserActions', () => {
  it('renders the time and actions in order with host labels', async () => {
    const { host } = await mountActions()
    expect(actionOrder(host)).toEqual([
      'timeline-user-sent-time',
      'timeline-user-copy',
      'timeline-user-edit',
      'timeline-user-fork',
      'timeline-user-branch',
    ])
    expect(host.querySelector('[data-testid="timeline-user-sent-time"]')?.textContent?.trim()).toBe('12:34')
    for (const label of [labels.copy, labels.edit, labels.fork, labels.branch]) {
      expect(host.textContent).toContain(label)
    }
  })

  it('keeps copy available and hides write actions when read-only', async () => {
    const { host } = await mountActions({ canEdit: false, canFork: false, canBranch: false })
    expect(actionOrder(host)).toEqual(['timeline-user-sent-time', 'timeline-user-copy'])
    expect(host.querySelector('[data-testid="timeline-user-withdraw"]')).toBeNull()
  })

  it('shows withdraw only when the host allows it', async () => {
    const { host } = await mountActions({ canWithdraw: true })
    expect(actionOrder(host)).toEqual([
      'timeline-user-sent-time',
      'timeline-user-withdraw',
      'timeline-user-copy',
      'timeline-user-edit',
      'timeline-user-fork',
      'timeline-user-branch',
    ])
    expect(host.textContent).toContain(labels.withdraw)
  })

  it('emits one callback per action', async () => {
    const { host, events } = await mountActions({ canWithdraw: true })
    for (const testId of ['withdraw', 'copy', 'edit', 'fork', 'branch']) {
      host.querySelector<HTMLButtonElement>(`[data-testid="timeline-user-${testId}"]`)!.click()
    }
    expect(events.withdraw).toHaveBeenCalledOnce()
    expect(events.copy).toHaveBeenCalledOnce()
    expect(events.edit).toHaveBeenCalledOnce()
    expect(events.fork).toHaveBeenCalledOnce()
    expect(events.branch).toHaveBeenCalledOnce()
  })

  it('swaps the copy icon for the success state', async () => {
    const { host } = await mountActions({ copied: true })
    expect(host.querySelector('[data-testid="icon-copy"]')).toBeNull()
    expect(host.querySelector('[data-testid="copy-success-icon"]')).not.toBeNull()
  })

  it('disables write actions while pending and branch when disabled', async () => {
    const { host, events } = await mountActions({ actionPending: true, branchDisabled: true })
    const edit = host.querySelector<HTMLButtonElement>('[data-testid="timeline-user-edit"]')!
    const fork = host.querySelector<HTMLButtonElement>('[data-testid="timeline-user-fork"]')!
    const branch = host.querySelector<HTMLButtonElement>('[data-testid="timeline-user-branch"]')!
    expect(edit.disabled).toBe(true)
    expect(fork.disabled).toBe(true)
    expect(branch.disabled).toBe(true)

    edit.click()
    fork.click()
    branch.click()
    expect(events.edit).not.toHaveBeenCalled()
    expect(events.fork).not.toHaveBeenCalled()
    expect(events.branch).not.toHaveBeenCalled()
  })

  it('does not emit branch while branchDisabled even without a native disabled button', async () => {
    const { host, events } = await mountActions({ branchDisabled: true })
    host.querySelector<HTMLButtonElement>('[data-testid="timeline-user-branch"]')!.click()
    expect(events.branch).not.toHaveBeenCalled()
  })
})
