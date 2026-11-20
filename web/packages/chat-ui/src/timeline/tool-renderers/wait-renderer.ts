import type { ToolCallRenderer, TimelineToolInlineParameter, TimelineToolDetailSection } from './types'
import type { ChatUiToolRenderBlock } from './types'
import { formatWorkDuration } from '../duration'
import {
  normalizeToolName,
  readStructuredRecord,
  readParamsRecord,
  readString,
  readablePairLines,
  INPUT_PREVIEW_LINE_MAX,
} from './utils'

const isWaitTool = (toolName: string): boolean => normalizeToolName(toolName) === 'wait'

/**
 * Resolve a background task's display name from the gateway-joined map.
 * Returns undefined when no name is available — callers show a generic phrase
 * rather than a raw task id, which is meaningless to users.
 */
const resolveTaskName = (block: ChatUiToolRenderBlock, taskId: string | undefined): string | undefined => {
  if (!taskId) return undefined
  const names = block.backgroundTaskNames
  const resolved = names && typeof names[taskId] === 'string' ? names[taskId].trim() : ''
  return resolved || undefined
}

const readWaitedMs = (result: Record<string, unknown> | null): number =>
  result && typeof result.waitedMs === 'number' ? result.waitedMs : 0

/**
 * Renders the `wait` tool. The collapsed row carries the whole story (which
 * task was waited on and how it ended), so there is no expandable detail.
 */
export const waitRenderer: ToolCallRenderer = {
  id: 'wait',

  match(toolName) {
    return isWaitTool(toolName)
  },

  getLabelKey(_toolName, _params) {
    return 'toolDisplay.wait'
  },

  getPrimaryParameter(block): TimelineToolInlineParameter | null {
    const result = readStructuredRecord(block.toolResult)
    const reason = readString(result, ['reason'])

    if (reason === 'notification') {
      return { kind: 'i18n', key: 'toolWait.ended' }
    }
    if (reason === 'timeout') {
      return { kind: 'i18n', key: 'toolWait.timedOut', params: { duration: formatWorkDuration(readWaitedMs(result)) } }
    }
    if (reason === 'user_steer' || reason === 'interrupted') {
      return { kind: 'i18n', key: 'toolWait.stopped' }
    }

    // Running state (no result yet).
    const name = resolveTaskName(block, readString(readParamsRecord(block), ['taskId']) ?? undefined)
    return name
      ? { kind: 'i18n', key: 'toolWait.waitingFor', params: { name } }
      : { kind: 'i18n', key: 'toolWait.waitingAny' }
  },

  getDetailSections(): TimelineToolDetailSection[] {
    return []
  },

  hasDetails() {
    return false
  },

  getInputPreview(params) {
    return readablePairLines(params, ['taskId', 'timeoutMs'], INPUT_PREVIEW_LINE_MAX)
  },
}
