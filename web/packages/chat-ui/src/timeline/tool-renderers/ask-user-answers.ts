export type AskUserAnswerMap = Record<string, string | string[]>

const text = (value: unknown): string | undefined => (
  typeof value === 'string' && value.trim() !== '' ? value : undefined
)

/**
 * Reads questions answers from either result shape:
 * - the built-in Desktop adapter returns `{ answers: { [questionId]: value } }`;
 * - the App Server `ask_user` tool returns
 *   `{ answers: [{ questionId, selected, custom? }] }`.
 */
export const readAskUserAnswers = (result: Record<string, unknown> | null): AskUserAnswerMap => {
  const answers = result?.answers
  if (Array.isArray(answers)) {
    const map: AskUserAnswerMap = {}
    for (const answer of answers) {
      if (answer === null || typeof answer !== 'object' || Array.isArray(answer)) continue
      const item = answer as Record<string, unknown>
      const questionId = text(item.questionId) ?? text(item.question_id)
      if (questionId === undefined) continue
      const selected = Array.isArray(item.selected)
        ? item.selected.map(text).filter((value): value is string => value !== undefined)
        : []
      const custom = text(item.custom)
      const values = [...selected, ...(custom === undefined ? [] : [custom])]
      if (values.length > 0) map[questionId] = values
    }
    return map
  }
  if (answers !== null && typeof answers === 'object') {
    return answers as AskUserAnswerMap
  }
  return {}
}

export const isAskUserDeclined = (result: Record<string, unknown> | null): boolean =>
  result?.declined === true

const readRawQuestionId = (question: Record<string, unknown>): string | undefined =>
  text(question.id) ?? text(question.questionId) ?? text(question.question_id)

/**
 * The App Server resolves a missing question id to its 1-based position and
 * returns answers under that id. The tool-call params echo the model input, so
 * an id-less question reaches the timeline without one.
 */
export const readAskUserQuestionId = (
  question: Record<string, unknown>,
  index: number,
): string => readRawQuestionId(question) ?? String(index + 1)

export const readAskUserAnswer = (
  answers: AskUserAnswerMap,
  question: Record<string, unknown>,
  index: number,
  header: string,
): string | string[] | undefined => {
  const byId = answers[readAskUserQuestionId(question, index)]
  if (byId !== undefined) return byId
  return header ? answers[header] : undefined
}

export const readAskUserOptionLabels = (question: Record<string, unknown>): Set<string> => {
  const rawOptions = question.options
  if (!Array.isArray(rawOptions)) return new Set()
  const labels = new Set<string>()
  for (const option of rawOptions) {
    if (option && typeof option === 'object' && typeof (option as Record<string, unknown>).label === 'string') {
      labels.add((option as Record<string, unknown>).label as string)
    }
  }
  return labels
}

export const formatAskUserAnswer = (
  value: string | string[],
  optionLabels: Set<string>,
): string => {
  const values = Array.isArray(value) ? value : [value]
  return values
    .map(v => {
      const s = String(v)
      return optionLabels.size > 0 && !optionLabels.has(s) ? `✎ ${s}` : s
    })
    .join(', ')
}
