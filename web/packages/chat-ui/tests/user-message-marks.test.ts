import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, h, nextTick, type App } from 'vue'
import { TimelineUserMarks, type ChatUiTimelineUserMark } from '../src'

const marks: ChatUiTimelineUserMark[] = [
  {
    num: 1,
    selector: '#save-button',
    textLabel: 'Save button',
    notes: ['Align it with the title', 'Use the primary tone'],
    interactive: true,
  },
  {
    num: 2,
    selector: '.sidebar',
    notes: [],
  },
]

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
  vi.restoreAllMocks()
})

async function mountMarks(input: readonly ChatUiTimelineUserMark[], parentClick = vi.fn()) {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const emitted: ChatUiTimelineUserMark[] = []
  const app = createApp({
    render: () => h('div', { onClick: parentClick }, [
      h(TimelineUserMarks, {
        marks: input,
        onMarkClick: (mark: ChatUiTimelineUserMark) => emitted.push(mark),
      }),
    ]),
  })
  app.mount(host)
  mounted.push(app)
  await nextTick()
  return { host, emitted, parentClick }
}

describe('TimelineUserMarks', () => {
  it('renders marks in order with their selector, label, and notes', async () => {
    const { host } = await mountMarks(marks)
    const cards = Array.from(host.querySelectorAll<HTMLElement>('[data-testid^="timeline-user-mark-"]'))
    expect(cards.map(card => card.getAttribute('data-testid'))).toEqual([
      'timeline-user-mark-1',
      'timeline-user-mark-2',
    ])
    expect(cards[0]?.textContent).toContain('mark 1')
    expect(cards[0]?.textContent).toContain('#save-button')
    expect(cards[0]?.textContent).toContain('Save button')
    expect(cards[0]?.textContent).toContain('Align it with the title')
    expect(cards[0]?.textContent).toContain('Use the primary tone')
    expect(cards[1]?.textContent).not.toContain('Save button')
    expect(cards[1]?.querySelector('.whitespace-pre-wrap')).toBeNull()
  })

  it('only marks interactive annotations as clickable', async () => {
    const { host } = await mountMarks(marks)
    expect(host.querySelector('[data-testid="timeline-user-mark-1"]')?.className).toContain('cursor-pointer')
    expect(host.querySelector('[data-testid="timeline-user-mark-2"]')?.className).not.toContain('cursor-pointer')
  })

  it('emits the clicked mark and stops the click from reaching the host', async () => {
    const { host, emitted, parentClick } = await mountMarks(marks)
    host.querySelector<HTMLElement>('[data-testid="timeline-user-mark-2"]')!.click()
    expect(emitted).toEqual([marks[1]])
    expect(parentClick).not.toHaveBeenCalled()
  })

  it('renders empty when there are no marks', async () => {
    const { host } = await mountMarks([])
    expect(host.querySelector('[data-testid^="timeline-user-mark-"]')).toBeNull()
    expect(host.textContent?.trim()).toBe('')
  })
})
