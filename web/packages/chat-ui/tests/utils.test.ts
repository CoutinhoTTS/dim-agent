import { afterEach, describe, expect, it } from 'vitest'
import {
  ensureTrailingNewline,
  formatTimelineTimestamp,
  getSelectedTextInElement,
  githubHeadingSlug,
  normalizeMarkdownWindowsPaths,
  parseFileHref,
  stripLineSuffix,
  stripToolCodeLineNumbers,
  toFileDisplayLabel,
} from '../src'

afterEach(() => {
  window.getSelection()?.removeAllRanges()
  document.body.replaceChildren()
})

describe('file link parsing', () => {
  it('parses hash and colon line suffixes', () => {
    expect(parseFileHref('src/app.ts#L42')).toEqual({ path: 'src/app.ts', startLine: 42, endLine: undefined })
    expect(parseFileHref('src/app.ts:12')).toEqual({ path: 'src/app.ts', startLine: 12, endLine: undefined })
    expect(parseFileHref('src/app.ts:12-20')).toEqual({ path: 'src/app.ts', startLine: 12, endLine: 20 })
    expect(parseFileHref('src/app.vue:12:3')).toEqual({ path: 'src/app.vue', startLine: 12, endLine: undefined })
  })

  it('does not parse URL or drive-letter suffixes', () => {
    expect(parseFileHref('https://example.com/app.ts:42')).toEqual({ path: 'https://example.com/app.ts:42' })
    expect(parseFileHref('C:\\work\\app.ts')).toEqual({ path: 'C:\\work\\app.ts' })
    expect(stripLineSuffix('src/app.ts:42')).toBe('src/app.ts')
  })

  it('derives basename display labels', () => {
    expect(toFileDisplayLabel('@src/components/App.vue')).toBe('App.vue')
    expect(toFileDisplayLabel('src/components/', true)).toBe('components/')
  })
})

describe('markdown helpers', () => {
  it('normalizes Windows links outside code spans', () => {
    expect(normalizeMarkdownWindowsPaths('[app](C:\\work\\app.ts)')).toContain('C:/work/app.ts')
    expect(normalizeMarkdownWindowsPaths('`C:\\work\\app.ts`')).toBe('`C:\\work\\app.ts`')
  })

  it('creates GitHub-style heading slugs', () => {
    expect(githubHeadingSlug('一、结论与 PR 序列')).toBe('一结论与-pr-序列')
    expect(githubHeadingSlug('Hello World')).toBe('hello-world')
  })
})

describe('selection copy', () => {
  it('returns selection text only when it intersects the target element', () => {
    const target = document.createElement('div')
    target.textContent = 'inside'
    const other = document.createElement('div')
    other.textContent = 'outside'
    document.body.append(target, other)

    const selection = window.getSelection()!
    const targetRange = document.createRange()
    targetRange.selectNodeContents(target)
    selection.addRange(targetRange)
    expect(getSelectedTextInElement(target)).toBe('inside')

    selection.removeAllRanges()
    const otherRange = document.createRange()
    otherRange.selectNodeContents(other)
    selection.addRange(otherRange)
    expect(getSelectedTextInElement(target)).toBe('')
  })
})

describe('tool code helpers', () => {
  it('keeps a trailing newline and strips read prefixes', () => {
    expect(ensureTrailingNewline('a')).toBe('a\n')
    expect(ensureTrailingNewline('a\n')).toBe('a\n')
    expect(stripToolCodeLineNumbers('1\ta\n2\tb\nFile is larger')).toBe('a\nb')
  })
})

describe('timeline timestamps', () => {
  it('formats the time tier with the requested locale', () => {
    const at = new Date('2026-01-02T10:00:00')
    const now = new Date('2026-01-02T12:00:00')
    expect(formatTimelineTimestamp(at, 'en-US', now)).toContain('10:00')
    expect(formatTimelineTimestamp(at, 'en-US', now, { timeOnly: true })).toBe('10:00')
  })
})
