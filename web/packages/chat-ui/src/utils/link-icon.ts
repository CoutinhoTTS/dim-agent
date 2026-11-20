/**
 * Recognized link icon kinds. `null` means no dedicated icon (generic link).
 *
 * Public provider hosts get a brand icon. Self-hosted GitLab instances also
 * qualify when `gitlab` is the exact first DNS label; broader hostname matching
 * would risk classifying unrelated sites as GitLab.
 *
 * The package owns only the classification. Hosts supply the icon data and the
 * renderer component, so the package does not depend on a brand icon set.
 */
export type LinkIconKind = 'github' | 'gitlab'

/**
 * Classify an absolute URL into a brand icon kind.
 * Returns `null` for non-http(s) URLs or unrecognized hosts.
 */
export function getLinkIconKind(href: string): LinkIconKind | null {
  let host: string
  try {
    const url = new URL(href)
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null
    host = url.hostname.toLowerCase()
  } catch {
    return null
  }

  if (host === 'github.com' || host.endsWith('.github.com')) return 'github'
  if (host === 'gitlab.com' || host.endsWith('.gitlab.com') || host.startsWith('gitlab.')) return 'gitlab'
  return null
}
