/**
 * Parse a file href that may contain a line-number suffix.
 *
 * Supported formats:
 * - GitHub style: `path/to/file#L42`, `path/to/file#L42-L50`
 * - Colon style:  `path/to/file:42`, `path/to/file:42-50`, `path/to/file:12:3`
 *
 * Colon suffixes are only matched at the end of the value and after a word
 * character, so Windows drive letters and URL schemes are left alone. Invalid
 * line values still have their suffix stripped; the line fields are dropped so
 * the base path continues to resolve.
 */
export interface ParsedFileHref {
  path: string
  startLine?: number
  endLine?: number
}

const HASH_LINE_RE = /^(.*?)#L(\d+)(?:-L?(\d+))?$/
const COLON_LINE_RE = /^(.*\w):(\d+)(?:-(\d+))?$/
const COLON_LINE_COLUMN_RE = /^(.*\w):(\d+):(\d+)$/

function buildResult(path: string, startRaw: string, endRaw: string | undefined): ParsedFileHref {
  const start = Number(startRaw)
  const end = endRaw != null ? Number(endRaw) : undefined
  const validStart = start >= 1 ? start : undefined
  const validEnd = validStart != null && end != null && end >= start ? end : undefined
  return { path, startLine: validStart, endLine: validEnd }
}

export function parseFileHref(href: string): ParsedFileHref {
  const hashMatch = HASH_LINE_RE.exec(href)
  if (hashMatch) {
    return buildResult(hashMatch[1]!, hashMatch[2]!, hashMatch[3])
  }

  if (!href.includes('://')) {
    const columnMatch = COLON_LINE_COLUMN_RE.exec(href)
    if (columnMatch) {
      return buildResult(columnMatch[1]!, columnMatch[2]!, undefined)
    }

    const colonMatch = COLON_LINE_RE.exec(href)
    if (colonMatch) {
      return buildResult(colonMatch[1]!, colonMatch[2]!, colonMatch[3])
    }
  }

  return { path: href }
}

export function stripLineSuffix(href: string): string {
  return parseFileHref(href).path
}
