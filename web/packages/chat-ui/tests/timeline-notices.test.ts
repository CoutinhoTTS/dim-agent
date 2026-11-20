import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App, type Component, type VNode } from 'vue'
import {
  TimelineErrorBoundary,
  TimelineRetryNotice,
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
  vi.restoreAllMocks()
})

describe('TimelineRetryNotice', () => {
  it('renders the retry countdown and emits retry from the shared button', async () => {
    const retry = vi.fn()
    const host = mount(TimelineRetryNotice, {
      errorText: 'Authentication required',
      label: 'Retry in 10s',
      retryTitle: 'Retry',
      canRetry: true,
      onRetry: retry,
    })

    expect(host.textContent).toContain('Authentication required')
    const button = host.querySelector('button')
    expect(button?.textContent).toBe('Retry in 10s')
    button?.click()
    await nextTick()
    expect(retry).toHaveBeenCalledTimes(1)
  })

  it('keeps the pending state disabled and uses the passive label without retry', () => {
    const host = mount(TimelineRetryNotice, {
      errorText: 'Authentication required',
      label: 'Retrying',
      retryTitle: 'Retry',
      canRetry: true,
      pending: true,
    })

    expect(host.querySelector('button')?.disabled).toBe(true)

    mounted.pop()?.unmount()
    const passive = mount(TimelineRetryNotice, {
      errorText: 'Authentication required',
      label: 'Retried',
      retryTitle: 'Retry',
      canRetry: false,
    })
    expect(passive.querySelector('button')).toBeNull()
    expect(passive.textContent).toContain('Retried')
  })
})

describe('TimelineErrorBoundary', () => {
  it('renders its slot until a child throws, then shows the host-provided fallback', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined)
    const host = document.createElement('div')
    document.body.appendChild(host)
    const Throwing = defineComponent({
      setup() {
        throw new Error('row failed')
      },
      render: () => null,
    })
    const app = createApp({
      render: () => h(
        TimelineErrorBoundary,
        { fallbackText: 'Row unavailable' },
        { default: () => h(Throwing) },
      ),
    })
    app.mount(host)
    mounted.push(app)
    await nextTick()

    expect(host.querySelector('[data-testid="timeline-row-render-error"]')?.textContent)
      .toContain('Row unavailable')
    expect(errorSpy).toHaveBeenCalled()
  })
})
