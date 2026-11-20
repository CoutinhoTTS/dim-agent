// Incremental string hashing for timeline row cache keys.
//
// Streaming content is append-only, so callers can hash only the appended
// suffix instead of walking the full string on every chunk. The integer
// results match hashing the joined value.

export const HASH_BASE = 31

export function hashText(value: string): number {
  let hash = 0
  for (let index = 0; index < value.length; index += 1) {
    hash = Math.imul(hash, HASH_BASE) + value.charCodeAt(index) | 0
  }
  return hash
}

/**
 * `hashText(prefix + appended)` without materializing the concatenation.
 * Math.imul truncates to 32 bits, so this matches the per-character loop.
 */
export function hashTextAppend(prefixHash: number, appended: string): number {
  let pow = 1
  for (let index = 0; index < appended.length; index += 1) {
    pow = Math.imul(pow, HASH_BASE)
  }
  return Math.imul(prefixHash, pow) + hashText(appended) | 0
}

/**
 * `hashText(parts.join(separator))` without materializing the joined string.
 */
export function hashTextJoinedParts(parts: Array<string>, separator: string): number {
  let hash = 0
  for (let index = 0; index < parts.length; index += 1) {
    if (index > 0) {
      for (let sepIndex = 0; sepIndex < separator.length; sepIndex += 1) {
        hash = Math.imul(hash, HASH_BASE) + separator.charCodeAt(sepIndex) | 0
      }
    }
    const part = parts[index]!
    for (let partIndex = 0; partIndex < part.length; partIndex += 1) {
      hash = Math.imul(hash, HASH_BASE) + part.charCodeAt(partIndex) | 0
    }
  }
  return hash
}
