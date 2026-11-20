import type { ToolCallRenderer, TimelineToolInlineParameter, TimelineToolDetailSection } from './types'
import type { ChatUiToolRenderBlock } from './types'
import { classifyTimelineToolCategory, getTimelineToolLabelKey } from '../tool-summary'
import {
  highPriorityScalarKeys,
  readStructuredRecord,
  readParamsRecord,
  resultText,
  previewLinesFromText,
  previewLinesFromRecord,
  readablePairLines,
  toolSourceDetailLine,
} from './utils'

const genericResultText = (block: ChatUiToolRenderBlock): string | null => {
  if (typeof block.toolResult === 'string' && block.toolResult.trim().length > 0) {
    return block.toolResult.trim()
  }
  return resultText(block)
}

export const genericRenderer: ToolCallRenderer = {
  id: 'generic',

  match(_toolName, _params) {
    return true // always matches as fallback (must be registered last)
  },

  getLabelKey(toolName, params) {
    return getTimelineToolLabelKey(toolName, classifyTimelineToolCategory(toolName), params ?? undefined)
  },

  getPrimaryParameter(block): TimelineToolInlineParameter | null {
    const rawName = block.toolName?.trim()
    if (!rawName) return null
    // A mapped or category-recognized tool already has a localized label.
    // Repeating its wire name next to that label reintroduces English; keep the
    // identifier only for tools that would otherwise render as a bare generic
    // "used tool" row, so third-party tools stay diagnosable.
    const labelKey = getTimelineToolLabelKey(rawName, classifyTimelineToolCategory(rawName))
    if (labelKey !== 'timelineToolSummary.usedTool') return null
    return { kind: 'text', text: rawName }
  },

  getDetailSections(block): TimelineToolDetailSection[] {
    const params = readParamsRecord(block)
    const result = readStructuredRecord(block.toolResult)
    const sections: TimelineToolDetailSection[] = []
    const sourceLine = toolSourceDetailLine(block)

    const paramPreview = previewLinesFromRecord('params', params)
    if (paramPreview) sections.push(paramPreview)

    const pairs = [
      ...(sourceLine ? [sourceLine] : []),
      ...readablePairLines(params, highPriorityScalarKeys, 3),
      ...readablePairLines(result, ['status', 'message', 'error', 'ok', 'count', 'items', 'total'], 3),
    ]
    if (pairs.length > 0) {
      sections.push({
        id: 'generic-summary',
        kind: 'summary',
        lines: pairs,
        truncated: false,
      })
    }

    const text = genericResultText(block)
    if (text) {
      const preview = previewLinesFromText(text)
      if (preview) sections.push(preview)
    }

    return sections
  },

  hasDetails(block): boolean {
    return this.getDetailSections(block).length > 0
  },
}
