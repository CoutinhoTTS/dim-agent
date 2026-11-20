import { afterEach, describe, expect, it } from 'vitest'
import { createApp, h, type App } from 'vue'
import { TimelineSkeleton } from '../src'

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

describe('TimelineSkeleton', () => {
  it('renders the centered skeleton host and brand slot', () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const app = createApp({
      render: () => h(TimelineSkeleton, null, {
        brand: () => h('span', { 'data-testid': 'brand' }, 'brand'),
      }),
    })
    app.mount(host)
    mounted.push(app)

    const skeleton = host.querySelector<HTMLElement>('[data-testid="timeline-skeleton"]')
    expect(skeleton).not.toBeNull()
    expect(skeleton?.classList.contains('timeline-skeleton')).toBe(true)
    expect(host.querySelector('[data-testid="brand"]')?.textContent).toBe('brand')
  })
})
