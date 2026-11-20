/**
 * GitHub-style heading slug. `MarkdownHeadingNode` uses it for heading ids and
 * fragment navigation resolves against the same function, so do not approximate
 * the slug at click time.
 */
export function githubHeadingSlug(value: string): string {
  return value
    .normalize('NFKC')
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, '')
    .replace(/\s+/g, '-')
}
