/**
 * Normalize Windows absolute paths in Markdown link destinations before a
 * parser sees them. CommonMark treats backslash + ASCII punctuation as an
 * escape, so `[index.html](G:\Tangula\_lookbook\index.html)` otherwise loses a
 * separator. Only link destinations are touched; running text, inline code,
 * and fenced/indented code keep their original backslashes.
 */

const FENCE_PATTERN = /^(?:\s*(?:>\s*|[-+*]\s+|\d+[.)]\s+))*\s*(`{3,}|~{3,})(.*)$/
const INDENTED_CODE_PATTERN = /^(?:\s*>\s*)*(?: {4}|\t)/
const LINK_DESTINATION_PATTERN = /(\[[^\]\n]*\]\(\s*)(?:<([^>]*)>|((?:[^()\n]|\([^)\n]*\))+))/g
const TITLE_PATTERN = /\s+("[^"]*"|'[^']*'|\([^)]*\))$/

function normalizeLinkDestination(destination: string): string | null {
  if (!/^(?:[a-z]:|\\\\[\w.-]+)/i.test(destination)) {
    return null
  }
  if (!destination.includes('\\')) {
    return null
  }

  const isUnc = destination.startsWith('\\\\')
  const rest = isUnc ? destination.slice(2) : destination
  if (rest.includes('\\\\')) {
    return null
  }

  return isUnc
    ? encodeURIComponent(`\\\\${rest}`)
    : destination.replace(/\\/g, '/')
}

function normalizeLinkDestinations(text: string): string {
  return text.replace(
    LINK_DESTINATION_PATTERN,
    (match: string, prefix: string, angled: string | undefined, plain: string | undefined) => {
      if (angled !== undefined) {
        const normalized = normalizeLinkDestination(angled)
        return normalized === null ? match : `${prefix}<${normalized}>`
      }

      const raw = plain!
      const titleMatch = TITLE_PATTERN.exec(raw)
      const title = titleMatch ? raw.slice(titleMatch.index) : ''
      const path = titleMatch ? raw.slice(0, titleMatch.index) : raw
      const normalized = normalizeLinkDestination(path)
      if (normalized === null) {
        return match
      }
      return `${prefix}${/\s/.test(normalized) ? `<${normalized}>` : normalized}${title}`
    },
  )
}

function isEscaped(text: string, index: number): boolean {
  let backslashes = 0
  for (let cursor = index - 1; cursor >= 0 && text[cursor] === '\\'; cursor -= 1) {
    backslashes += 1
  }
  return backslashes % 2 === 1
}

function normalizeOutsideInlineCode(line: string): string {
  let result = ''
  let index = 0

  while (index < line.length) {
    let tick = line.indexOf('`', index)
    while (tick !== -1 && isEscaped(line, tick)) {
      result += normalizeLinkDestinations(line.slice(index, tick + 1))
      index = tick + 1
      tick = line.indexOf('`', index)
    }
    if (tick === -1) {
      result += normalizeLinkDestinations(line.slice(index))
      break
    }

    result += normalizeLinkDestinations(line.slice(index, tick))

    let runLength = 1
    while (line[tick + runLength] === '`') {
      runLength += 1
    }
    const fence = '`'.repeat(runLength)
    const close = line.indexOf(fence, tick + runLength)
    if (close === -1) {
      result += line.slice(tick)
      break
    }

    result += line.slice(tick, close + runLength)
    index = close + runLength
  }

  return result
}

export function normalizeMarkdownWindowsPaths(markdown: string): string {
  if (!markdown.includes('\\')) {
    return markdown
  }

  let fence: { marker: string; length: number } | null = null
  const normalizedLines = markdown.split('\n').map((line) => {
    const match = FENCE_PATTERN.exec(line)
    if (fence) {
      if (!INDENTED_CODE_PATTERN.test(line)
        && match
        && match[1]![0] === fence.marker
        && match[1]!.length >= fence.length
        && match[2]!.trim() === '') {
        fence = null
      }
      return line
    }
    if (INDENTED_CODE_PATTERN.test(line)) {
      return line
    }
    if (match) {
      fence = { marker: match[1]![0]!, length: match[1]!.length }
      return line
    }
    return normalizeOutsideInlineCode(line)
  })

  return normalizedLines.join('\n')
}
