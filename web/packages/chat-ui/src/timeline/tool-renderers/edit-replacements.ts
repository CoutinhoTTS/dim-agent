export interface EditReplacement {
  index: number
  oldText: string
  newText: string
  replaceAll: boolean
}

export interface EditReplacementSet {
  replacements: EditReplacement[]
  /** The call carries the pre-#1482 top-level oldText/newText pair. */
  legacy: boolean
  /** The call carries both shapes, which the App Server refuses. */
  conflict: boolean
  /** An `edits` field is present but not every entry is renderable. */
  malformed: boolean
}

const readString = (value: unknown): string | null =>
  typeof value === 'string' ? value : null

const topLevelOldText = (input: Record<string, unknown>): string | null =>
  readString(input.oldText) ?? readString(input.old_string)

const topLevelNewText = (input: Record<string, unknown>): string | null =>
  readString(input.newText) ?? readString(input.new_string)

const readReplacement = (value: unknown, index: number): EditReplacement | null => {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return null
  const entry = value as Record<string, unknown>
  const oldText = readString(entry.oldText)
  const newText = readString(entry.newText)
  if (oldText === null || newText === null) return null
  return {
    index,
    oldText,
    newText,
    replaceAll: entry.replaceAll === true,
  }
}

/**
 * Reads the replacements of an edit call. A call recorded before
 * arcships/dim#1482 carries the top-level oldText/newText pair; a call recorded
 * after it carries `edits`, which is the only shape the App Server applies in
 * order. When `edits` is present, it wins: never mix it with the top-level pair,
 * because the App Server refuses a call that carries both.
 */
export const readEditReplacements = (
  input: Record<string, unknown> | null | undefined,
): EditReplacementSet => {
  if (!input) return { replacements: [], legacy: false, conflict: false, malformed: false }

  const hasEdits = Object.prototype.hasOwnProperty.call(input, 'edits')
  const hasTopLevel = topLevelOldText(input) !== null || topLevelNewText(input) !== null

  if (!hasEdits) {
    const oldText = topLevelOldText(input)
    const newText = topLevelNewText(input)
    return {
      replacements: oldText === null || newText === null
        ? []
        : [{ index: 0, oldText, newText, replaceAll: input.replaceAll === true }],
      legacy: oldText !== null && newText !== null,
      conflict: false,
      malformed: false,
    }
  }

  const raw = input.edits
  if (!Array.isArray(raw)) {
    return { replacements: [], legacy: false, conflict: hasTopLevel, malformed: true }
  }

  const replacements = raw
    .flatMap((value, index) => {
      const replacement = readReplacement(value, index)
      return replacement === null ? [] : [replacement]
    })
    .map((replacement, index) => ({ ...replacement, index }))
  return {
    replacements,
    legacy: false,
    conflict: hasTopLevel,
    malformed: raw.length === 0 || replacements.length !== raw.length,
  }
}
