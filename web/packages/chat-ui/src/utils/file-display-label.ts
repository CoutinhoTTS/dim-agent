/**
 * Derive the compact label used by file chips: strip a leading `@`, drop the
 * directory prefix, and keep a trailing slash for directories.
 */
export function toFileDisplayLabel(value: string, isDirectory = false): string {
  const stripped = value.startsWith('@') ? value.slice(1) : value
  const trimmed = stripped.trim()
  const withoutTrailingSlash = trimmed.replace(/[\\/]+$/u, '')
  const label = withoutTrailingSlash.split(/[\\/]/u).pop() || withoutTrailingSlash || trimmed
  return isDirectory && label && !label.endsWith('/') ? `${label}/` : label
}
