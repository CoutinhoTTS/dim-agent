import { afterEach, describe, expect, it } from 'vitest'
import { createApp, h, type App } from 'vue'
import { FileChip, type ChatUiFileChipOpenPayload } from '../src'

const mounted: App[] = []

function mountFileChip(
  props: Record<string, unknown>,
  onOpen?: (payload: ChatUiFileChipOpenPayload) => void,
): HTMLElement {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(FileChip, {
      onOpen: (payload: ChatUiFileChipOpenPayload) => onOpen?.(payload),
      ...props,
    }),
  })
  app.mount(host)
  mounted.push(app)
  return host
}

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

describe('FileChip', () => {
  it('renders the host icon URL and the path contract attributes', () => {
    const host = mountFileChip({
      absolutePath: '/repo/src/app.ts',
      isDirectory: false,
      fileIconUrl: 'icon:/repo/src/app.ts',
    })

    const button = host.querySelector('button.file-chip')
    expect(button).not.toBeNull()
    expect(button?.getAttribute('title')).toBe('/repo/src/app.ts')
    expect(button?.getAttribute('data-file-absolute')).toBe('/repo/src/app.ts')
    expect(button?.getAttribute('data-file-relative')).toBeNull()
    expect(button?.getAttribute('data-file-directory')).toBeNull()
    expect(host.querySelector('.file-chip__icon')?.getAttribute('src')).toBe('icon:/repo/src/app.ts')
    expect(host.textContent).toContain('app.ts')
  })

  it('keeps the supplied label and derives a display label when absent', () => {
    const labelled = mountFileChip({
      relativeHref: 'src/components/App.vue',
      label: '@src/components/App.vue',
    })
    expect(labelled.querySelector('.file-chip__label')?.textContent).toBe('@src/components/App.vue')

    const stripped = mountFileChip({ relativeHref: '@src/components/App.vue' })
    expect(stripped.querySelector('.file-chip__label')?.textContent).toBe('App.vue')

    const derived = mountFileChip({ absolutePath: '/repo/src/app.ts' })
    expect(derived.querySelector('.file-chip__label')?.textContent).toBe('app.ts')
  })

  it('appends a trailing slash to a directory label', () => {
    const host = mountFileChip({
      relativeHref: 'src/components',
      isDirectory: true,
    })

    expect(host.querySelector('.file-chip__label')?.textContent).toBe('components/')
    expect(host.querySelector('button')?.getAttribute('data-file-directory')).toBe('true')
    expect(host.querySelector('button')?.hasAttribute('data-file-relative')).toBe(true)
  })

  it('emits the absolute path payload on click', () => {
    const events: ChatUiFileChipOpenPayload[] = []
    const host = mountFileChip(
      { absolutePath: '/repo/folder', isDirectory: true },
      payload => events.push(payload),
    )

    host.querySelector('button')?.click()

    expect(events).toEqual([
      {
        absolutePath: '/repo/folder',
        relativeHref: undefined,
        isDirectory: true,
        external: false,
      },
    ])
  })

  it('emits the relative href payload with an external modifier', () => {
    const events: ChatUiFileChipOpenPayload[] = []
    const host = mountFileChip(
      { relativeHref: '../generated/generated.ts' },
      payload => events.push(payload),
    )

    host.querySelector('button')?.dispatchEvent(new MouseEvent('click', {
      bubbles: true,
      cancelable: true,
      metaKey: true,
    }))

    expect(events).toEqual([
      {
        absolutePath: undefined,
        relativeHref: '../generated/generated.ts',
        isDirectory: false,
        external: true,
      },
    ])
  })

  it('disables the chip without a path or while the host is opening', () => {
    const events: ChatUiFileChipOpenPayload[] = []
    const empty = mountFileChip({}, payload => events.push(payload))
    expect(empty.querySelector('button')?.disabled).toBe(true)

    const opening = mountFileChip(
      { absolutePath: '/repo/app.ts', opening: true },
      payload => events.push(payload),
    )
    const button = opening.querySelector('button')
    expect(button?.disabled).toBe(true)
    expect(button?.classList.contains('file-chip--disabled')).toBe(true)
    button?.dispatchEvent(new MouseEvent('click', { bubbles: true }))

    expect(events).toEqual([])
  })
})
