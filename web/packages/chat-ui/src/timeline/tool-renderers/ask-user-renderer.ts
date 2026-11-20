import type { ToolCallRenderer, TimelineToolInlineParameter, TimelineToolDetailSection } from './types'
import type { ChatUiToolRenderBlock } from './types'
import { formatWorkDuration } from '../duration'
import { getTimelineToolLabelKey } from '../tool-summary'
import {
  formatAskUserAnswer,
  isAskUserDeclined,
  readAskUserAnswer,
  readAskUserAnswers,
  readAskUserOptionLabels,
} from './ask-user-answers'
import { normalizeToolName, readArray, readStructuredRecord, countFromArrayOrNumber } from './utils'

const ASK_USER_MATCHERS = ['askuserquestion', 'ask_user_question', 'ask_user']

const isAskUserTool = (toolName: string): boolean => {
  const normalized = normalizeToolName(toolName)
  return ASK_USER_MATCHERS.includes(normalized) || ASK_USER_MATCHERS.includes(toolName.toLowerCase())
}

const readQuestions = (params: Record<string, unknown> | null): Record<string, unknown>[] => {
  const raw = readArray(params, ['questions', 'items'])
  if (!raw) return []
  return raw.filter((q): q is Record<string, unknown> => q !== null && typeof q === 'object' && !Array.isArray(q))
}

/** Build collapsed answer summary. Returns null if no answers. */
function buildAnswerSummary(block: ChatUiToolRenderBlock): string | null {
  const params = block.toolParams as Record<string, unknown> | null
  const result = readStructuredRecord(block.toolResult)
  if (!result) return null

  const questions = readQuestions(params)
  const answers = readAskUserAnswers(result)
  if (Object.keys(answers).length === 0) return null

  const parts: string[] = []
  for (let i = 0; i < questions.length; i++) {
    const q = questions[i]!
    const header = typeof q.header === 'string' ? q.header : ''
    const answer = readAskUserAnswer(answers, q, i, header)
    if (answer === undefined || answer === null) continue

    const optionLabels = readAskUserOptionLabels(q)
    const formatted = formatAskUserAnswer(answer, optionLabels)
    if (!formatted) continue

    if (questions.length === 1) {
      parts.push(formatted)
    } else {
      const label = header || (typeof q.question === 'string' ? q.question.slice(0, 20) : `Q${i + 1}`)
      parts.push(`${label}: ${formatted}`)
    }
  }

  if (parts.length === 0) return null

  // Show first 2 parts + count for 3+
  if (parts.length > 2) {
    return `${parts.slice(0, 2).join('  ')}  +${parts.length - 2}`
  }
  return parts.join('  ')
}

/**
 * How long the user was given to answer, when the question ended by timing out.
 * Returns null for every other result shape.
 */
function readTimedOutWaitedMs(block: ChatUiToolRenderBlock): number | null {
  const result = readStructuredRecord(block.toolResult)
  if (!result || result.code !== 'ask_user_timed_out') return null
  return typeof result.waitedMs === 'number' ? result.waitedMs : 0
}

export const askUserRenderer: ToolCallRenderer = {
  id: 'ask-user',

  match(toolName, _params) {
    return isAskUserTool(toolName)
  },

  getLabelKey(toolName, _params) {
    return getTimelineToolLabelKey(toolName, 'unknown')
  },

  getPrimaryParameter(block): TimelineToolInlineParameter | null {
    const waitedMs = readTimedOutWaitedMs(block)
    if (waitedMs !== null) {
      return {
        kind: 'i18n',
        key: 'toolAskUser.timedOut',
        params: { duration: formatWorkDuration(waitedMs) },
      }
    }

    if (isAskUserDeclined(readStructuredRecord(block.toolResult))) {
      return { kind: 'i18n', key: 'toolAskUser.declined' }
    }

    // When answers exist, show answer summary instead of question count
    const summary = buildAnswerSummary(block)
    if (summary) {
      return { kind: 'text', text: summary }
    }

    // No answers yet (running) — show question count
    const params = block.toolParams as Record<string, unknown> | null
    const count = countFromArrayOrNumber(params, ['questions', 'items']) ?? 1
    return {
      kind: 'i18n',
      key: 'toolQuantity.questions',
      params: { count },
    }
  },

  getDetailSections(block): TimelineToolDetailSection[] {
    const params = block.toolParams as Record<string, unknown> | null
    const result = readStructuredRecord(block.toolResult)
    const questions = readQuestions(params)
    const answers = readAskUserAnswers(result)

    if (questions.length === 0) return []

    const lines: string[] = []
    for (let i = 0; i < questions.length; i++) {
      const q = questions[i]!
      const header = typeof q.header === 'string' ? q.header : ''
      const question = typeof q.question === 'string' ? q.question : ''

      if (i > 0) lines.push('')
      if (header) lines.push(header)
      if (question) lines.push(question)

      const answer = readAskUserAnswer(answers, q, i, header)
      if (answer !== undefined && answer !== null) {
        const optionLabels = readAskUserOptionLabels(q)
        lines.push(formatAskUserAnswer(answer, optionLabels))
      }
    }

    return [{
      id: 'questions',
      kind: 'summary',
      lines,
      truncated: false,
    }]
  },

  hasDetails(block): boolean {
    return this.getDetailSections(block).length > 0
  },
}
