import { afterEach, describe, expect, it } from 'vitest'
import { createApp, h, type App, type Component, type VNode } from 'vue'
import {
  TimelineDetailsRegion,
  ToolParameterPreview,
} from '../src'

const mounted: App[] = []

function mount(component: Component, props: Record<string, unknown>, slot?: () => VNode | string): HTMLElement {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(component, props, slot === undefined ? undefined : { default: slot }),
  })
  app.mount(host)
  mounted.push(app)
  return host
}

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

describe('TimelineDetailsRegion', () => {
  it('renders the collapsed grid wrapper and opens it through the class contract', () => {
    const host = mount(
      TimelineDetailsRegion,
      { collapsible: true, open: false },
      () => h('p', { 'data-testid': 'details-child' }, 'child'),
    )

    const region = host.querySelector<HTMLElement>('[data-testid="timeline-details-region"]')
    expect(region?.classList.contains('collapse-grid')).toBe(true)
    expect(region?.classList.contains('collapse-grid-open')).toBe(false)
    expect(host.querySelector('[data-testid="details-child"]')?.textContent).toBe('child')
  })

  it('adds the open class and renders the slot without a wrapper when not collapsible', () => {
    const openHost = mount(
      TimelineDetailsRegion,
      { collapsible: true, open: true },
      () => 'open child',
    )
    expect(openHost.querySelector('[data-testid="timeline-details-region"]')
      ?.classList.contains('collapse-grid-open')).toBe(true)

    const plainHost = mount(
      TimelineDetailsRegion,
      { collapsible: false, open: false },
      () => 'plain child',
    )
    expect(plainHost.querySelector('[data-testid="timeline-details-region"]')).toBeNull()
    expect(plainHost.textContent).toContain('plain child')
  })
})

describe('ToolParameterPreview', () => {
  it('renders one line per entry', () => {
    const host = mount(ToolParameterPreview, { lines: ['first', 'second'] })

    expect(host.querySelectorAll('.tool-parameter-preview__line')).toHaveLength(2)
    expect(host.textContent).toContain('first')
    expect(host.textContent).toContain('second')
  })

  it('renders nothing for an empty line list', () => {
    const host = mount(ToolParameterPreview, { lines: [] })

    expect(host.querySelector('[data-testid="tool-parameter-preview"]')).toBeNull()
  })
})
