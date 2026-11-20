import type { Component } from 'vue'
import type { ChatUiTimelineSearchMatch } from '../timeline/search'
import { toFileDisplayLabel } from '../utils/file-display-label'

/**
 * Link-icon kinds the user-content renderer can show. Hosts map each kind to
 * their own icon component; `null` from the resolver means the generic link.
 */
export type ChatUiUserContentLinkIconKind = 'github' | 'gitlab'

export interface ChatUiTimelineUserContentIcons {
  skill: Component
  commit: Component
  genericLink: Component
  /**
   * Renderer for one entry of `linkIcons` (for example Iconify's `Icon`).
   * The package stays independent of the host's icon library.
   */
  linkIconRenderer?: Component
  linkIcons?: Partial<Record<ChatUiUserContentLinkIconKind, unknown>>
}

export interface ChatUiTimelineUserContentOpenFilePayload {
  absolutePath: string
  isDirectory: boolean
  isPastedText: boolean
  label: string
}

/** File and attachment facts the desktop adapter keeps outside the package. */
export interface ChatUiUserContentSegmentOptions {
  knownSkillNames?: ReadonlySet<string>
  pastedTextAttachmentPaths?: ReadonlySet<string>
  normalizeAttachmentReference: (value: string) => string
  resolveFileIconLookupPath: (absolutePath: string, label: string, isDirectory: boolean) => string
  isPastedTextName: (name: string) => boolean
}

export type ChatUiUserContentSegment =
  | { type: 'text'; key: string; text: string; start: number }
  | { type: 'skill'; key: string; name: string }
  | {
    type: 'file'
    key: string
    label: string
    absolutePath: string
    iconPath: string
    isDirectory: boolean
    isPastedText: boolean
  }
  | { type: 'commit'; key: string; label: string; hash: string }
  | { type: 'link'; key: string; label: string; href: string }

/**
 * Mirrors the composer markdown link grammar: `[label](destination)`.
 * Kept in the package because the renderer must not import `@dimcode/shared`.
 */
export const USER_CONTENT_MARKDOWN_LINK_PATTERN =
  /\[((?:\\.|[^\]\\])*)\]\(((?:\\.|[^)\\\n])+?)\)/g

export const unescapeUserContentMarkdownLabel = (value: string): string =>
  value.replace(/\\([\\[\]])/g, '$1')

export const unescapeUserContentMarkdownDestination = (value: string): string =>
  value.replace(/\\([\\)])/g, '$1')

const COMMIT_HASH_PATTERN = /^[0-9a-f]{7,40}$/i

/**
 * The `$skill:` grammar is shared with the agent. The package carries the
 * matcher so a host only supplies the resolved skill-name set.
 */
const SKILL_MENTION_RE = /(^|[^\w$])\$skill:\s*(\S+)/gu

export interface UserContentSkillMentionRange {
  start: number
  end: number
  name: string
}

export function findUserContentSkillMentionRanges(
  text: string,
  skillNames?: ReadonlySet<string>,
): UserContentSkillMentionRange[] {
  const ranges: UserContentSkillMentionRange[] = []
  for (const match of text.matchAll(SKILL_MENTION_RE)) {
    const prefix = match[1] ?? ''
    const name = match[2]
    if (name === undefined) continue
    if (skillNames !== undefined && !skillNames.has(name)) continue
    const start = (match.index ?? 0) + prefix.length
    ranges.push({ start, end: start + match[0].length - prefix.length, name })
  }
  return ranges
}

// `\\server\share` UNC paths are absolute file references on Windows.
const isAbsoluteFilePath = (href: string): boolean =>
  /^\/(?!\/)/.test(href)
  || /^[A-Za-z]:[\\/]/.test(href)
  || /^\\\\[^\\]+\\/.test(href)

const isCommitMention = (label: string, href: string): boolean => {
  if (!COMMIT_HASH_PATTERN.test(href)) return false
  // Old messages may use [@shortHash](fullHash); strip leading @ for backward compat.
  const shortHash = label.startsWith('@') ? label.slice(1) : label
  return COMMIT_HASH_PATTERN.test(shortHash) && href.toLowerCase().startsWith(shortHash.toLowerCase())
}

const stripMentionPrefix = (label: string): string =>
  label.startsWith('@') ? label.slice(1) : label

const fileMentionLabel = (label: string): string => {
  const stripped = stripMentionPrefix(label)
  return toFileDisplayLabel(stripped, stripped.endsWith('/'))
}

function appendTextAndSkillSegments(
  segments: ChatUiUserContentSegment[],
  text: string,
  startOffset: number,
  knownSkillNames: ReadonlySet<string> | undefined,
): void {
  let lastIndex = 0

  for (const mention of findUserContentSkillMentionRanges(text, knownSkillNames)) {
    if (mention.start > lastIndex) {
      segments.push({
        type: 'text',
        key: `text-${startOffset + lastIndex}`,
        text: text.slice(lastIndex, mention.start),
        start: startOffset + lastIndex,
      })
    }

    segments.push({
      type: 'skill',
      key: `skill-${startOffset + mention.start}`,
      name: mention.name,
    })
    lastIndex = mention.end
  }

  if (lastIndex < text.length) {
    segments.push({
      type: 'text',
      key: `text-${startOffset + lastIndex}`,
      text: text.slice(lastIndex),
      start: startOffset + lastIndex,
    })
  }
}

/**
 * Split display text into text, skill, file, commit, and link segments. The
 * host supplies projected text plus its attachment/icon facts; the ordering,
 * keys, and markdown grammar stay package-owned so desktop and web cannot
 * drift.
 */
export function buildChatUiUserContentSegments(
  content: string,
  options: ChatUiUserContentSegmentOptions,
): ChatUiUserContentSegment[] {
  if (!content) {
    return []
  }

  const segments: ChatUiUserContentSegment[] = []
  let lastIndex = 0

  for (const match of content.matchAll(USER_CONTENT_MARKDOWN_LINK_PATTERN)) {
    const matched = match[0]
    const label = match[1]!
    const href = match[2]!
    const start = match.index ?? 0
    const end = start + matched.length

    if (start > lastIndex) {
      appendTextAndSkillSegments(segments, content.slice(lastIndex, start), lastIndex, options.knownSkillNames)
    }

    const decodedLabel = unescapeUserContentMarkdownLabel(label)
    const decodedHref = unescapeUserContentMarkdownDestination(href)
    if (/^file:\/\//i.test(decodedHref) || isAbsoluteFilePath(decodedHref)) {
      const displayLabel = fileMentionLabel(decodedLabel)
      const isDirectory = displayLabel.endsWith('/')
      const absolutePath = /^file:\/\//i.test(decodedHref)
        ? options.normalizeAttachmentReference(decodedHref)
        : decodedHref
      segments.push({
        type: 'file',
        key: `file-${start}`,
        label: displayLabel,
        absolutePath,
        iconPath: options.resolveFileIconLookupPath(absolutePath, displayLabel, isDirectory),
        isDirectory,
        isPastedText: options.isPastedTextName(decodedLabel)
          && options.pastedTextAttachmentPaths?.has(options.normalizeAttachmentReference(absolutePath)) === true,
      })
    } else if (isCommitMention(decodedLabel, decodedHref)) {
      segments.push({
        type: 'commit',
        key: `commit-${start}`,
        label: stripMentionPrefix(decodedLabel),
        hash: decodedHref,
      })
    } else {
      segments.push({
        type: 'link',
        key: `link-${start}`,
        label: decodedLabel,
        href: decodedHref,
      })
    }

    lastIndex = end
  }

  if (lastIndex < content.length) {
    appendTextAndSkillSegments(segments, content.slice(lastIndex), lastIndex, options.knownSkillNames)
  }

  if (segments.length === 0) {
    return [
      {
        type: 'text',
        key: 'text-0',
        text: content,
        start: 0,
      },
    ]
  }

  return segments
}

/**
 * A link chip only when the visible label is the URL itself and the URL parses.
 */
export function isChatUiLinkMentionSegment(
  segment: ChatUiUserContentSegment,
): segment is Extract<ChatUiUserContentSegment, { type: 'link' }> {
  if (segment.type !== 'link') return false
  if (segment.label !== segment.href) return false
  if (!/^https?:\/\//i.test(segment.href)) return false
  try {
    new URL(segment.href)
    return true
  } catch {
    return false
  }
}

export interface ChatUiTimelineUserContentProps {
  content: string
  searchMatches?: readonly ChatUiTimelineSearchMatch[]
  activeSearchMatchId?: string | null
  openingFileMention?: string | null
  knownSkillNames?: ReadonlySet<string>
  pastedTextAttachmentPaths?: ReadonlySet<string>
  icons: ChatUiTimelineUserContentIcons
  resolveFileIconLookupPath: ChatUiUserContentSegmentOptions['resolveFileIconLookupPath']
  getFileIconUrl: (lookupPath: string, isDirectory: boolean) => string
  normalizeAttachmentReference: ChatUiUserContentSegmentOptions['normalizeAttachmentReference']
  isPastedTextName: ChatUiUserContentSegmentOptions['isPastedTextName']
  getLinkIconKind: (href: string) => ChatUiUserContentLinkIconKind | null
  searchHighlightComponent?: Component
}
