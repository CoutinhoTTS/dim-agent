import type { ToolCallRenderer, TimelineToolInlineParameter, TimelineToolDetailSection } from './types'
import { classifyTimelineToolCategory, getTimelineToolLabelKey } from '../tool-summary'
import {
  readCommandText,
  readStructuredRecord,
  readParamsRecord,
  resultText,
  summarySection,
  previewLinesFromText,
  previewLinesFromNonEmptyText,
  DETAIL_LINE_MAX,
  INPUT_PREVIEW_LINE_MAX,
} from './utils'

export const commandRenderer: ToolCallRenderer = {
  id: 'command',

  match(toolName, _params) {
    return classifyTimelineToolCategory(toolName) === 'command'
  },

  getLabelKey(toolName, params) {
    const isBackground = typeof params?.runInBackground === 'boolean' && params.runInBackground
    if (isBackground) return 'toolDisplay.runCommandBackground'
    return getTimelineToolLabelKey(toolName, classifyTimelineToolCategory(toolName), params ?? undefined)
  },

  getPrimaryParameter(block): TimelineToolInlineParameter | null {
    const params = readParamsRecord(block)
    const result = readStructuredRecord(block.toolResult)
    const command = readCommandText(params, result)
    if (!command) return null
    return { kind: 'text', text: command.split(/\s+/)[0] }
  },

  getDetailSections(block): TimelineToolDetailSection[] {
    const params = readParamsRecord(block)
    const result = readStructuredRecord(block.toolResult)
    const command = readCommandText(params, result)
    const output = resultText(block)

    const sections: TimelineToolDetailSection[] = []
    if (command) {
      const cmd = summarySection('command', [command])
      if (cmd) sections.push(cmd)
    }
    if (output) {
      const maxLines = command ? DETAIL_LINE_MAX - 1 : DETAIL_LINE_MAX
      const preview = previewLinesFromText(output, maxLines)
      if (preview) sections.push(preview)
    }
    return sections
  },

  hasDetails(_block): boolean {
    return true
  },

  getInputPreview(params): string[] {
    const command = readCommandText(params)
    return command ? previewLinesFromNonEmptyText(command, INPUT_PREVIEW_LINE_MAX).lines : []
  },
}
