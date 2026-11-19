import { stripLineSuffix } from './file-link-parser'

export interface MarkdownFileReference {
  absolutePath: string
}

const UNIX_ABSOLUTE_PATH = /^\/[^/]/
const WINDOWS_DRIVE_ABSOLUTE_PATH = /^[A-Za-z]:[\\/].+/
const WINDOWS_UNC_ABSOLUTE_PATH = /^\\\\[^\\]+\\[^\\]+/
// POSIX-style Windows path that models sometimes emit: `/C:/Users/...` or `/C:\Users\...`.
const WINDOWS_SLASH_DRIVE_PATH = /^\/[A-Za-z]:[\\/]/
const AUTOLINKED_HTTP_URL = /^https?:\/\/(.+)$/i

export function decodePathText(value: string): string {
  try {
    return decodeURIComponent(value)
  } catch {
    return value
  }
}

function isWindowsRuntime(): boolean {
  const navigatorValue = globalThis.navigator
  const platform = navigatorValue?.platform ?? ''
  const userAgent = navigatorValue?.userAgent ?? ''
  return /win/i.test(platform) || /windows/i.test(userAgent)
}

export function isAbsoluteLocalPath(value: string): boolean {
  if (isWindowsRuntime()) {
    return WINDOWS_DRIVE_ABSOLUTE_PATH.test(value) || WINDOWS_UNC_ABSOLUTE_PATH.test(value)
  }
  return UNIX_ABSOLUTE_PATH.test(value)
}

/**
 * Strip the leading slash from a POSIX-style Windows drive path (`/C:/Users/...`),
 * matching how models emit absolute Windows paths. Windows-only: on other
 * platforms `/C:/...` is a valid Unix path and must stay untouched.
 */
export function normalizeWindowsDrivePath(value: string): string {
  if (isWindowsRuntime() && WINDOWS_SLASH_DRIVE_PATH.test(value)) {
    return value.slice(1)
  }
  return value
}

/**
 * Convert backslashes to forward slashes in Windows drive paths
 * (`C:\Users\...` → `C:/Users/...`) so every absolute path returned to
 * callers uses POSIX separators. UNC paths (`\\server\share`) keep their
 * backslashes. No-op outside Windows.
 */
function normalizeDrivePathSeparators(value: string): string {
  if (isWindowsRuntime() && /^[A-Za-z]:[\\/]/.test(value)) {
    return value.replace(/\\/g, '/')
  }
  return value
}

export function resolveAutolinkedWorkspaceCandidate({
  href,
  text,
  raw,
}: {
  href: string | null | undefined
  text: string | null | undefined
  raw: string | null | undefined
}): string | null {
  const hrefText = String(href ?? '').trim()
  const visibleText = String(text ?? '').trim()
  const rawText = String(raw ?? '').trim()
  const match = hrefText.match(AUTOLINKED_HTTP_URL)

  if (!match || !visibleText || rawText !== visibleText) {
    return null
  }

  const decodedHrefPath = decodePathText(match[1]).replace(/\/$/, '')
  if (decodedHrefPath !== visibleText) {
    return null
  }

  return visibleText
}

/**
 * Treat a path as a directory when its last segment has no file extension.
 * Purely textual — no disk check. `src/utils` → directory, `src/a.ts` → file.
 */
const FILE_EXTENSION_SUFFIX = /\.[A-Za-z0-9]{1,12}$/

export function pathLooksLikeDirectory(path: string): boolean {
  const clean = stripLineSuffix(path)
  const base = clean.replace(/[\\/]+$/, '').split(/[\\/]/).pop() ?? ''
  return !FILE_EXTENSION_SUFFIX.test(base)
}

const URL_SCHEME = /^[a-z][a-z0-9+.-]*:/i

/**
 * A Markdown link href pointing to a workspace-relative path (not a web URL,
 * anchor, or other URI scheme). Returns the normalized relative path, else null.
 * Absolute paths / file:// are handled by resolveMarkdownFileReference instead.
 */
export function resolveRelativeWorkspaceLink(href: string | null | undefined): string | null {
  const value = String(href ?? '').trim()
  if (!value || value.startsWith('#') || value.startsWith('//')) {
    return null
  }
  if (URL_SCHEME.test(value)) {
    return null
  }
  const decoded = decodePathText(value).replace(/\\/g, '/').replace(/^\.\//, '')
  return decoded || null
}

/**
 * Resolve a Markdown link href relative to the source file that contains it.
 * Returns the normalized path: the normalized absolute path for absolute
 * hrefs (including POSIX-style Windows paths like `/C:/Users/...`), a
 * `~`-prefixed home path as-is, or the workspace-relative path joined with the
 * source file's directory. Returns null when the href is not a local path.
 */
export function resolveRelativeWorkspaceLinkFromFile(
  href: string | null | undefined,
  sourceFilePath: string | null | undefined,
): string | null {
  // Absolute hrefs are checked before the relative-path branch: a Windows
  // drive href like `C:/Users/...` would otherwise be mistaken for a URL
  // scheme, and `/C:/Users/...` would be returned without normalization.
  // The `./` / `.\` prefix is stripped first so `./C:/Users/...` resolves
  // like the bare drive form instead of being joined to the source file
  // directory.
  const strippedPrefix = String(href ?? '').trim().replace(/^[.]\\/, './').replace(/^\.\//, '')
  const absoluteReference = resolveMarkdownFileReference(strippedPrefix)
  if (absoluteReference) {
    return absoluteReference.absolutePath
  }

  const relative = resolveRelativeWorkspaceLink(href)
  if (!relative) {
    return null
  }

  if (relative.startsWith('~')) {
    return relative
  }

  const sourcePath = String(sourceFilePath ?? '').trim().replace(/\\/g, '/')
  const sourceDir = sourcePath.includes('/') ? sourcePath.slice(0, sourcePath.lastIndexOf('/')) : ''
  if (!sourceDir) {
    return relative
  }
  const absolutePrefix = sourceDir.startsWith('//') ? '//' : sourceDir.startsWith('/') ? '/' : ''

  const parts: string[] = []
  for (const part of `${sourceDir}/${relative}`.split('/')) {
    if (!part || part === '.') continue
    if (part === '..') {
      const last = parts.at(-1)
      if (last && last !== '..') {
        parts.pop()
      } else {
        parts.push(part)
      }
      continue
    }
    parts.push(part)
  }
  return `${absolutePrefix}${parts.join('/')}`
}

export function resolveMarkdownFileReference(rawValue: string | null | undefined): MarkdownFileReference | null {
  const value = String(rawValue ?? '').trim()
  if (!value) {
    return null
  }

  if (value.startsWith('file://')) {
    try {
      const url = new URL(value)
      let pathname = decodePathText(url.pathname)
      if (/^\/[A-Za-z]:/.test(pathname)) {
        pathname = pathname.slice(1)
      }
      if (url.hostname) {
        // `file://server/share/...` is a UNC path. Keep it out of the plain
        // `//host/path` shape, which is a protocol-relative web URL, and use
        // the backslash form the rest of the resolver recognizes.
        return isWindowsRuntime()
          ? { absolutePath: `\\\\${url.hostname}${pathname.replace(/\//g, '\\')}` }
          : null
      }
      if (!isAbsoluteLocalPath(pathname)) {
        return null
      }
      return { absolutePath: pathname }
    } catch {
      const fallbackPath = decodePathText(value.replace(/^file:\/\//, ''))
      if (!isAbsoluteLocalPath(fallbackPath)) {
        return null
      }
      return { absolutePath: normalizeDrivePathSeparators(fallbackPath) }
    }
  }

  const decoded = decodePathText(value)
  const normalized = normalizeWindowsDrivePath(decoded)
  if (!isAbsoluteLocalPath(normalized)) {
    return null
  }

  return { absolutePath: normalizeDrivePathSeparators(normalized) }
}

/**
 * Join a workspace-relative path against the workspace root into a normalized
 * absolute path. Absolute inputs (POSIX, Windows drive / UNC) pass through
 * unchanged. Throws when no cwd is available and the input is not absolute.
 */
export function resolveWorkspaceFilePath(
  cwd: string | null | undefined,
  relativePath: string | null | undefined,
): string {
  const inputPath = String(relativePath ?? '').trim()
  if (isAbsoluteLocalPath(inputPath)) {
    return inputPath
  }
  const path = inputPath
    .replace(/^[\\/]+/g, '')
    .replace(/^\.\//, '')
    .replace(/\\/g, '/')
  const rawRoot = String(cwd ?? '').trim()
  if (!path || !rawRoot) {
    throw new Error('Cannot resolve workspace path without a cwd')
  }
  // Strip trailing separators, but keep the root itself when trimming would
  // empty it: POSIX root `/`, Windows drive root `C:\` and UNC root
  // `\\server\share` are all valid workspace cwds.
  const root = normalizeDrivePathSeparators(rawRoot.replace(/[\\/]+$/g, '') || rawRoot)
  return root === '/' ? `/${path}` : `${root}/${path}`
}
