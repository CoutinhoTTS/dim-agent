import { afterEach, describe, expect, it } from 'vitest'
import { createApp, defineComponent, h, nextTick, type App, type Component } from 'vue'
import {
  TimelineUserContent,
  buildChatUiUserContentSegments,
  isChatUiLinkMentionSegment,
  type ChatUiTimelineUserContentIcons,
  type ChatUiTimelineUserContentOpenFilePayload,
  type ChatUiUserContentSegmentOptions,
} from '../src'

const normalizeAttachmentReference = (value: unknown): string => {
  const normalized = String(value ?? '').trim().replace(/\\/g, '/')
  if (!normalized || !/^file:\/\//i.test(normalized)) return normalized
  try {
    const parsed = new URL(normalized)
    const decoded = decodeURIComponent(parsed.pathname).replace(/\\/g, '/')
    if (!decoded) return normalized
    if (parsed.host) return `//${parsed.host}${decoded}`
    return /^\/[A-Za-z]:\//.test(decoded) ? decoded.slice(1) : decoded
  } catch {
    return normalized
  }
}

const baseOptions: ChatUiUserContentSegmentOptions = {
  normalizeAttachmentReference,
  resolveFileIconLookupPath: (absolutePath, label) => `${absolutePath}|${label}`,
  isPastedTextName: name => /^pasted-text-\d+\.txt$/.test(name),
}

const icon = (name: string): Component => defineComponent({
  name,
  setup: () => () => h('i', { 'data-testid': `icon-${name}` }),
})

const icons: ChatUiTimelineUserContentIcons = {
  skill: icon('skill'),
  commit: icon('commit'),
  genericLink: icon('generic-link'),
  linkIconRenderer: defineComponent({
    name: 'BrandIcon',
    props: { icon: { type: null, default: undefined } },
    setup: props => () => h('i', {
      'data-testid': 'icon-brand',
      'data-icon': String(props.icon),
    }),
  }),
  linkIcons: {
    github: 'github-data',
    gitlab: 'gitlab-data',
  },
}

const mounted: App[] = []

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount()
  document.body.replaceChildren()
})

function mountUserContent(
  props: { content: string } & Record<string, unknown>,
): { host: HTMLElement; events: ChatUiTimelineUserContentOpenFilePayload[] } {
  const events: ChatUiTimelineUserContentOpenFilePayload[] = []
  const host = document.createElement('div')
  document.body.appendChild(host)
  const app = createApp({
    render: () => h(TimelineUserContent, {
      icons,
      resolveFileIconLookupPath: baseOptions.resolveFileIconLookupPath,
      normalizeAttachmentReference: baseOptions.normalizeAttachmentReference,
      isPastedTextName: baseOptions.isPastedTextName,
      getFileIconUrl: (lookupPath: string) => `icon:${lookupPath}`,
      getLinkIconKind: (href: string) => (
        href.includes('github.com') ? 'github' : href.includes('gitlab.com') ? 'gitlab' : null
      ),
      onOpenFile: (payload: ChatUiTimelineUserContentOpenFilePayload) => events.push(payload),
      ...props,
    }),
  })
  app.mount(host)
  mounted.push(app)
  return { host, events }
}

describe('buildChatUiUserContentSegments', () => {
  it('returns no segments for empty text and one text segment otherwise', () => {
    expect(buildChatUiUserContentSegments('', baseOptions)).toEqual([])
    expect(buildChatUiUserContentSegments('hello', baseOptions)).toEqual([
      { type: 'text', key: 'text-0', text: 'hello', start: 0 },
    ])
  })

  it('splits known skill mentions and leaves unknown names as text', () => {
    expect(buildChatUiUserContentSegments('open $skill: alpha now', {
      ...baseOptions,
      knownSkillNames: new Set(['alpha']),
    })).toEqual([
      { type: 'text', key: 'text-0', text: 'open ', start: 0 },
      { type: 'skill', key: 'skill-5', name: 'alpha' },
      { type: 'text', key: 'text-18', text: ' now', start: 18 },
    ])

    expect(buildChatUiUserContentSegments('open $skill: unknown now', {
      ...baseOptions,
      knownSkillNames: new Set(['alpha']),
    })).toEqual([
      { type: 'text', key: 'text-0', text: 'open $skill: unknown now', start: 0 },
    ])
  })

  it('projects absolute file and directory mentions', () => {
    expect(buildChatUiUserContentSegments('see [@src/a.ts](/repo/src/a.ts) now', baseOptions)).toEqual([
      { type: 'text', key: 'text-0', text: 'see ', start: 0 },
      {
        type: 'file',
        key: 'file-4',
        label: 'a.ts',
        absolutePath: '/repo/src/a.ts',
        iconPath: '/repo/src/a.ts|a.ts',
        isDirectory: false,
        isPastedText: false,
      },
      { type: 'text', key: 'text-31', text: ' now', start: 31 },
    ])

    const [directory] = buildChatUiUserContentSegments('[folder/](/repo/folder/)', baseOptions)
    expect(directory).toMatchObject({
      type: 'file',
      label: 'folder/',
      absolutePath: '/repo/folder/',
      isDirectory: true,
    })
  })

  it('normalizes file URLs and marks pasted-text attachments', () => {
    const [segment] = buildChatUiUserContentSegments(
      '[pasted-text-1.txt](file:///tmp/pasted-text-1.txt)',
      {
        ...baseOptions,
        pastedTextAttachmentPaths: new Set(['/tmp/pasted-text-1.txt']),
      },
    )

    expect(segment).toMatchObject({
      type: 'file',
      label: 'pasted-text-1.txt',
      absolutePath: '/tmp/pasted-text-1.txt',
      isPastedText: true,
    })
  })

  it('recognizes commit refs and leaves older @shortHash labels working', () => {
    const [text, commit] = buildChatUiUserContentSegments(
      'fix [@abc1234](abc1234def567890)',
      baseOptions,
    )
    expect(text).toMatchObject({ type: 'text', text: 'fix ' })
    expect(commit).toMatchObject({
      type: 'commit',
      label: 'abc1234',
      hash: 'abc1234def567890',
    })
  })

  it('keeps links as link segments and only chips URL-labelled http links', () => {
    const segments = buildChatUiUserContentSegments(
      '[https://github.com/acme/repo](https://github.com/acme/repo) and [docs](https://example.com/docs)',
      baseOptions,
    )
    const links = segments.filter(segment => segment.type === 'link')
    expect(links).toHaveLength(2)
    expect(links[0]).toMatchObject({
      type: 'link',
      label: 'https://github.com/acme/repo',
      href: 'https://github.com/acme/repo',
    })
    expect(isChatUiLinkMentionSegment(links[0]!)).toBe(true)
    expect(isChatUiLinkMentionSegment(links[1]!)).toBe(false)
  })

  it('unescapes markdown labels and destinations', () => {
    const [segment] = buildChatUiUserContentSegments(
      '[file\\[1\\].ts](/repo/file\\(1\\).ts)',
      baseOptions,
    )
    expect(segment).toMatchObject({
      type: 'file',
      label: 'file[1].ts',
      absolutePath: '/repo/file\\(1).ts',
    })
  })
})

describe('TimelineUserContent', () => {
  it('renders text, skill, file, commit, and link segments in order', async () => {
    const content = [
      'Use $skill: alpha on [src/a.ts](/repo/src/a.ts)',
      'then [@abc1234](abc1234def567890) via',
      '[https://github.com/acme/repo](https://github.com/acme/repo)',
    ].join(' ')
    const { host } = mountUserContent({
      content,
      knownSkillNames: new Set(['alpha']),
    })
    await nextTick()

    const nodes = [...host.children]
    expect(nodes.map(node => node.className)).toEqual([
      '',
      'timeline-skill-mention',
      '',
      'timeline-file-mention',
      '',
      'timeline-commit-mention',
      '',
      'font-medium underline underline-offset-4 timeline-link-mention',
    ])
    expect(host.querySelector('[data-skill-name="alpha"]')?.textContent).toContain('alpha')
    expect(host.querySelector('.timeline-file-mention__label')?.textContent).toBe('a.ts')
    expect(host.querySelector('.timeline-commit-mention')?.getAttribute('title'))
      .toBe('abc1234def567890')
    expect(host.querySelector('[data-link-icon]')?.getAttribute('data-link-icon'))
      .toBe('github')
    expect(host.querySelector('[data-testid="icon-brand"]')?.getAttribute('data-icon'))
      .toBe('github-data')
  })

  it('emits the open-file payload and disables the pending mention', async () => {
    const content = '[src/a.ts](/repo/src/a.ts)'
    const { host, events } = mountUserContent({ content })
    await nextTick()

    const button = host.querySelector<HTMLButtonElement>('.timeline-file-mention')
    button?.click()
    expect(events).toEqual([{
      absolutePath: '/repo/src/a.ts',
      isDirectory: false,
      isPastedText: false,
      label: 'a.ts',
    }])

    const { host: pendingHost } = mountUserContent({
      content,
      openingFileMention: '/repo/src/a.ts',
    })
    await nextTick()
    expect(pendingHost.querySelector<HTMLButtonElement>('.timeline-file-mention')?.disabled)
      .toBe(true)
  })

  it('surfaces pasted-text file mentions with the pasted flag', async () => {
    const { host, events } = mountUserContent({
      content: '[pasted-text-2.txt](file:///tmp/pasted-text-2.txt)',
      pastedTextAttachmentPaths: new Set(['/tmp/pasted-text-2.txt']),
    })
    await nextTick()

    host.querySelector<HTMLButtonElement>('.timeline-file-mention')?.click()
    expect(events).toEqual([{
      absolutePath: '/tmp/pasted-text-2.txt',
      isDirectory: false,
      isPastedText: true,
      label: 'pasted-text-2.txt',
    }])
  })

  it('falls back to the generic link icon for URLs without a brand', async () => {
    const { host } = mountUserContent({
      content: '[https://example.com/docs](https://example.com/docs)',
    })
    await nextTick()

    expect(host.querySelector('[data-testid="icon-generic-link"]')).not.toBeNull()
    expect(host.querySelector('[data-link-icon]')?.getAttribute('data-link-icon'))
      .toBe('generic')
  })

  it('passes search matches and base offsets through the highlight component', async () => {
    const SearchHighlight = defineComponent({
      name: 'StubSearchHighlight',
      props: {
        text: { type: String, required: true },
        baseOffset: { type: Number, default: 0 },
        matches: { type: Array, default: () => [] },
      },
      setup: props => () => h('mark', {
        'data-search-text': props.text,
        'data-base-offset': String(props.baseOffset),
        'data-match-count': String(props.matches.length),
      }),
    })
    const { host } = mountUserContent({
      content: 'hello world',
      searchMatches: [{ id: 'm1', sourceMatchIndex: 0, startOffset: 6, endOffset: 11 }],
      searchHighlightComponent: SearchHighlight,
    })
    await nextTick()

    const mark = host.querySelector('mark')
    expect(mark?.getAttribute('data-search-text')).toBe('hello world')
    expect(mark?.getAttribute('data-base-offset')).toBe('0')
    expect(mark?.getAttribute('data-match-count')).toBe('1')
  })
})
