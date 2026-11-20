import type { ChatUiToolRenderBlock, TimelineToolDetailSection } from './types'
import { normalizeToolName } from '../tool-name'

// --- Constants ---

export const INLINE_TEXT_MAX = 80
export const INLINE_COMMAND_MAX = 96
export const DETAIL_LINE_MAX = 6
export const DETAIL_LINE_CHAR_MAX = 180
export const PREVIEW_CHAR_MAX = 12_000
/** Line cap for the permission-prompt input preview, which renders inside a scrollable block. */
export const INPUT_PREVIEW_LINE_MAX = 40

// --- Key lists ---

export const commandKeys = ['command', 'cmd', 'commandPreview', 'shellCommand', 'script'] as const
export const commandArgvKeys = ['command', 'commandArgv', 'argv'] as const
export const pathKeys = ['path', 'filePath', 'file_path', 'file', 'target'] as const
export const queryKeys = ['query', 'search_query', 'pattern'] as const

// --- Normalization ---

export { normalizeToolName }

export const compactWhitespace = (text: string): string => text.replace(/\s+/g, ' ').trim()

export const basenameFromPath = (path: string): string => {
  const segments = path.replace(/\\/g, '/').split('/').filter(Boolean)
  return segments.at(-1) ?? path
}

export const hostFromUrl = (url: string): string => {
  try {
    return new URL(url).hostname
  } catch {
    return url
  }
}

export const toolSourceLabelText = (block: ChatUiToolRenderBlock): string | null => {
  const source = block.toolSource
  if (source?.kind === 'mcp' && source.mcpServerId && source.mcpToolName) {
    return `MCP · ${source.mcpServerId}/${source.mcpToolName}`
  }
  return null
}

export const toolSourceDetailLine = (block: ChatUiToolRenderBlock): string | null => {
  const source = block.toolSource
  if (!source) return null
  if (source.kind === 'mcp' && source.mcpServerId && source.mcpToolName) {
    return `source: ${source.mcpServerId}/${source.mcpToolName}`
  }
  return null
}

// --- Type guards ---

export const asRecord = (value: unknown): Record<string, unknown> | null => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null
  }
  return value as Record<string, unknown>
}

export const isJsonLikeText = (text: string): boolean => {
  const trimmed = text.trim()
  if (trimmed.length > 1000) return false
  return trimmed.startsWith('{') || trimmed.startsWith('[')
}

export const parseMaybeJson = (value: string | undefined | null): unknown => {
  const text = value?.trim()
  if (!text || (!text.startsWith('{') && !text.startsWith('['))) {
    return null
  }
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

// --- Read helpers ---

export const readString = (
  record: Record<string, unknown> | null | undefined,
  keys: readonly string[],
): string | null => {
  if (!record) return null
  for (const key of keys) {
    const val = record[key]
    if (typeof val === 'string' && val.trim().length > 0) {
      return val.trim()
    }
  }
  return null
}

export const readArray = (
  record: Record<string, unknown> | null | undefined,
  keys: readonly string[],
): unknown[] | null => {
  if (!record) return null
  for (const key of keys) {
    const val = record[key]
    if (Array.isArray(val) && val.length > 0) {
      return val
    }
  }
  return null
}

// --- Text / detail helpers ---

export const truncateDetailLine = (line: string): string => {
  if (line.length <= DETAIL_LINE_CHAR_MAX) return line
  return line.slice(0, DETAIL_LINE_CHAR_MAX) + '...'
}

/**
 * `truncateToolPreviewText` for a caller-verified non-blank string (e.g. a
 * `readString` result). Compacting a non-blank string never empties it, so the
 * result is always a string.
 */
export const truncateNonEmptyToolPreviewText = (text: string, maxLength = INLINE_TEXT_MAX): string => {
  const compact = compactWhitespace(text.trim())
  if (compact.length <= maxLength) return compact
  return compact.slice(0, maxLength - 1) + '…'
}

export const truncateToolPreviewText = (text: string | null | undefined, maxLength = INLINE_TEXT_MAX): string | null => {
  if (!text?.trim()) return null
  return truncateNonEmptyToolPreviewText(text, maxLength)
}

export const countFromArrayOrNumber = (
  record: Record<string, unknown> | null | undefined,
  keys: readonly string[],
): number | null => {
  if (!record) return null
  for (const key of keys) {
    const val = record[key]
    if (Array.isArray(val)) return val.length
    if (typeof val === 'number' && val > 0) return val
  }
  return null
}

export const readablePairLines = (
  record: Record<string, unknown> | null | undefined,
  keys: readonly string[],
  maxLines?: number,
): string[] => {
  if (!record) return []
  const lines: string[] = []
  for (const key of keys) {
    const val = record[key]
    if (val === undefined || val === null) continue
    if (maxLines !== undefined && lines.length >= maxLines) break
    const text = typeof val === 'object' ? JSON.stringify(val) : String(val)
    const compact = compactWhitespace(text)
    if (compact.length > 0) {
      lines.push(`${key}: ${compact}`)
    }
  }
  return lines
}

export const resultText = (block: ChatUiToolRenderBlock): string | null => {
  const result = asRecord(block.toolResult)
  if (!result) return null
  const text = readString(result, ['content', 'output', 'text'])
  if (text) return text
  return null
}

// --- Section builders ---

export const summarySection = (id: string, lines: string[]): TimelineToolDetailSection | null => {
  if (lines.length === 0) return null
  return {
    id,
    kind: 'summary',
    lines: lines.slice(0, DETAIL_LINE_MAX),
    truncated: lines.length > DETAIL_LINE_MAX,
  }
}

/**
 * `previewLinesFromText` for a caller-verified non-blank string (e.g. a
 * `readString` result). A non-blank input always yields at least one line, so
 * the result is always a section.
 */
export const previewLinesFromNonEmptyText = (
  text: string,
  maxLines = DETAIL_LINE_MAX,
): TimelineToolDetailSection => {
  const clippedText = text.length > PREVIEW_CHAR_MAX ? text.slice(0, PREVIEW_CHAR_MAX) : text
  const rawLines = clippedText
    .split(/\r?\n/)
    .map(line => truncateDetailLine(line.trimEnd()))
    .filter(line => line.trim().length > 0)

  const truncatedByChars = text.length > PREVIEW_CHAR_MAX
  const truncatedByLines = rawLines.length > maxLines
  const lines = rawLines.slice(0, maxLines)

  if (truncatedByLines) {
    lines.push(`... omitted ${rawLines.length - maxLines} lines`)
  } else if (truncatedByChars) {
    lines.push(`... omitted ${text.length - PREVIEW_CHAR_MAX} chars`)
  }

  return {
    id: 'preview',
    kind: 'preview',
    lines,
    truncated: truncatedByChars || truncatedByLines,
  }
}

export const previewLinesFromText = (
  text: string | null | undefined,
  maxLines = DETAIL_LINE_MAX,
): TimelineToolDetailSection | null => {
  if (!text?.trim()) return null
  return previewLinesFromNonEmptyText(text, maxLines)
}

export const previewLinesFromRecord = (
  id: string,
  record: Record<string, unknown> | null,
  maxLines = DETAIL_LINE_MAX,
): TimelineToolDetailSection | null => {
  if (!record || Object.keys(record).length === 0) return null
  const preview = previewLinesFromText(JSON.stringify(record, null, 2), maxLines)
  return preview ? { ...preview, id } : null
}

export const listSection = (
  id: string,
  items: unknown[] | null | undefined,
  maxLines: number,
): TimelineToolDetailSection | null => {
  if (!items || items.length === 0) return null
  const lines = items.slice(0, maxLines).map(item => {
    if (typeof item === 'string') return truncateDetailLine(item)
    if (item && typeof item === 'object') {
      const record = item as Record<string, unknown>
      return truncateDetailLine(lineFromRecord(record))
    }
    return truncateDetailLine(String(item))
  })
  return {
    id,
    kind: 'summary',
    lines,
    truncated: items.length > maxLines,
  }
}

export const highPriorityScalarKeys = [
  'path', 'filePath', 'file_path', 'file', 'url', 'query',
  'search_query', 'pattern', 'command', 'cmd',
  'description', 'goal', 'task', 'title', 'name', 'id',
] as const

export const lineFromRecord = (record: Record<string, unknown>): string => {
  const scalarKeys = [
    'title', 'name', 'path', 'filePath', 'file', 'url',
    'question', 'text', 'content', 'description', 'goal', 'task', 'header',
  ]
  const primary = readString(record, scalarKeys)
  if (primary) return primary

  const pairs = Object.entries(record)
    .filter(([, v]) => v !== undefined && v !== null)
    .map(([k, v]) => {
      const valueStr = typeof v === 'object' ? JSON.stringify(v) : String(v)
      return `${k} ${compactWhitespace(valueStr)}`
    })
    .filter(s => s.trim().length > 0)
    .slice(0, 4)
  return pairs.join(', ')
}

export const readCommandText = (
  ...records: Array<Record<string, unknown> | null | undefined>
): string | null => {
  for (const record of records) {
    const command = readString(record, commandKeys)
    if (command) return command

    const argv = readArray(record, commandArgvKeys)
      ?.map(value => (typeof value === 'string' || typeof value === 'number') ? String(value).trim() : '')
      .filter(Boolean)
    if (argv?.length) {
      return argv.join(' ')
    }
  }
  return null
}

export const readStructuredRecord = (value: unknown): Record<string, unknown> | null => {
  const record = asRecord(value)
  if (!record) return null
  return asRecord(record.structuredContent)
    ?? asRecord(record.output)
    ?? record
}

export const readParamsRecord = (block: ChatUiToolRenderBlock): Record<string, unknown> | null =>
  asRecord(block.toolParams) ?? asRecord(parseMaybeJson(block.inputText))

/**
 * Generic params-only input preview used by the permission prompt when a renderer
 * does not override getInputPreview. Renders a raw command as-is, otherwise lists
 * `key: value` lines (nested values inlined as JSON), falling back to pretty JSON only
 * when no readable pairs exist. Covers wait / MCP / third-party tools.
 */
export const inputPreviewLines = (
  params: Record<string, unknown> | null,
  maxLines = INPUT_PREVIEW_LINE_MAX,
): string[] => {
  if (!params || Object.keys(params).length === 0) return []
  const command = readCommandText(params)
  if (command) {
    return previewLinesFromNonEmptyText(command, maxLines).lines
  }
  const pairs = readablePairLines(params, Object.keys(params), maxLines)
  if (pairs.length > 0) {
    return pairs
  }
  return previewLinesFromNonEmptyText(JSON.stringify(params, null, 2), maxLines).lines
}

export const textParam = (
  text: string | null | undefined,
  maxLength = INLINE_TEXT_MAX,
): { kind: 'text'; text: string; title?: string } | null => {
  if (!text?.trim()) return null
  const trimmed = text.trim()
  const shortened = truncateNonEmptyToolPreviewText(trimmed, maxLength)
  return {
    kind: 'text',
    text: shortened,
    title: shortened === trimmed ? undefined : trimmed,
  }
}
