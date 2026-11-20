/**
 * Bounded tail fragment for a collapsed thinking row.
 *
 * The row is a one-line summary, so only the newest fragment is returned. The
 * scan inspects at most `max * 2` UTF-16 units before slicing by code point.
 */
export interface ThinkingFragment {
  text: string
  /** The source text was longer than the displayed fragment. */
  truncated: boolean
}

export function latestThinkingFragment(text: string, max = 220): ThinkingFragment {
  if (text === '' || max <= 0) {
    return { text: '', truncated: false }
  }

  let chunk = text.length > max * 2 ? text.slice(-max * 2) : text
  // A bounded slice can start on the low half of a surrogate pair.
  const first = chunk.charCodeAt(0)
  if (first >= 0xdc00 && first <= 0xdfff) {
    chunk = chunk.slice(1)
  }

  const chars = Array.from(chunk)
  return {
    text: chars.slice(-max).join('').replace(/\s+/g, ' ').trim(),
    truncated: text.length > max * 2 || chars.length > max,
  }
}
