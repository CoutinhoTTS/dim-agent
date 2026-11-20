import { afterEach, describe, expect, it, vi } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App, type Component } from 'vue'
import {
  ContextReferenceHoverContent,
  TimelineContextReferenceList,
  type ChatUiContextReferenceIcons,
  type ChatUiContextReferenceLabels,
  type ChatUiSentReferenceMetadata,
} from '../src'

const labels: ChatUiContextReferenceLabels = {
  selectedText: 'Selected text',
  userComment: 'Comment',
  saveAnnotation: 'Save annotation',
  editAnnotation: 'Edit annotation',
  removeAnnotation: 'Remove annotation',
}

const icon = (name: string): Component => defineComponent({
  name,
  setup: () => () => h('i', { 'data-testid': `icon-${name}` }),
})

const icons: ChatUiContextReferenceIcons = {
  source: {
    file: icon('file'),
    document: icon('document'),
    spreadsheet: icon('spreadsheet'),
    conversation: icon('conversation'),
    browser: icon('browser'),
  },
  check: icon('check'),
  pencil: icon('pencil'),
  remove: icon('remove'),
}

const HoverCard = defineComponent({
  name: 'StubHoverCard',
  setup: (_, { slots }) => () => h('div', { 'data-testid': 'hover-card' }, slots.default?.()),
})
const HoverCardTrigger = defineComponent({
  name: 'StubHoverCardTrigger',
  setup: (_, { slots }) => () => h('div', slots.default?.()),
})
const HoverCardContent = defineComponent({
  name: 'StubHoverCardContent',
  setup: (_, { slots }) => () => h('div', { 'data-testid': 'hover-card-content' }, slots.default?.()),
})

const references: ChatUiSentReferenceMetadata[] = [
  { source: 'file', text: 'src/a.ts', selectedText: 'const a', comment: 'note' },
  { source: 'document', text: 'spec.pdf' },
  { source: 'conversation', text: 'earlier answer' },
]

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

function mount(render: () => ReturnType<typeof h>): HTMLElement {
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({ render })
  app.mount(host)
  mounted.push(app)
  return host
}

describe('TimelineContextReferenceList', () => {
  it('renders chips with source icons and annotation labels', () => {
    const host = mount(() => h(TimelineContextReferenceList, {
      references,
      labels,
      icons,
      hoverCardComponent: HoverCard,
      hoverCardTriggerComponent: HoverCardTrigger,
      hoverCardContentComponent: HoverCardContent,
    }))
    expect(host.querySelectorAll('[data-testid^="timeline-context-reference-"]')).toHaveLength(3)
    expect(host.querySelector('[data-testid="timeline-context-reference-1"]')?.textContent).toContain('src/a.ts')
    expect(host.querySelector('[data-testid="timeline-context-reference-3"]')?.textContent).toContain('annotation')
    expect(host.querySelector('[data-testid="icon-file"]')).not.toBeNull()
    expect(host.querySelector('[data-testid="icon-conversation"]')).not.toBeNull()
    expect(host.querySelector('[data-testid="context-reference-selected-text"]')?.textContent).toContain('const a')
    expect(host.querySelector('[data-testid="context-reference-comment"]')?.textContent).toBe('note')
  })
})

describe('ContextReferenceHoverContent', () => {
  it('edits and saves a trimmed comment', async () => {
    const updateComment = vi.fn()
    const host = mount(() => h(ContextReferenceHoverContent, {
      position: 1,
      selectedText: 'const a',
      comment: 'note',
      editable: true,
      labels,
      icons,
      onUpdateComment: updateComment,
    }))
    const editButton = [...host.querySelectorAll('button')]
      .find(button => button.getAttribute('aria-label') === 'Edit annotation')
    expect(editButton).not.toBeUndefined()
    editButton!.click()
    await nextTick()

    const textarea = host.querySelector('textarea')
    expect(textarea).not.toBeNull()
    textarea!.value = '  updated  '
    textarea!.dispatchEvent(new Event('input'))
    textarea!.dispatchEvent(new Event('blur'))
    expect(updateComment).toHaveBeenCalledWith('updated')
  })

  it('cancels on Escape and removes the annotation', async () => {
    const updateComment = vi.fn()
    const remove = vi.fn()
    const host = mount(() => h(ContextReferenceHoverContent, {
      position: 1,
      selectedText: 'const a',
      comment: 'note',
      editable: true,
      labels,
      icons,
      onUpdateComment: updateComment,
      onRemove: remove,
    }))
    const editButton = [...host.querySelectorAll('button')]
      .find(button => button.getAttribute('aria-label') === 'Edit annotation')
    editButton!.click()
    await nextTick()
    const textarea = host.querySelector('textarea')!
    textarea.value = 'draft'
    textarea.dispatchEvent(new Event('input'))
    textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    await nextTick()
    expect(host.querySelector('textarea')).toBeNull()
    expect(updateComment).not.toHaveBeenCalled()

    ;[...host.querySelectorAll('button')]
      .find(button => button.getAttribute('aria-label') === 'Remove annotation')!
      .click()
    expect(remove).toHaveBeenCalled()
  })
})
