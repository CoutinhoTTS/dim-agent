import type { ToolCallRenderer, TimelineToolInlineParameter, TimelineToolDetailSection } from './types'
import {
  readString,
  readStructuredRecord,
  readParamsRecord,
  normalizeToolName,
  readablePairLines,
  truncateNonEmptyToolPreviewText,
  INPUT_PREVIEW_LINE_MAX,
} from './utils'

export const skillRenderer: ToolCallRenderer = {
  id: 'skill',

  match(toolName, _params) {
    return normalizeToolName(toolName) === 'skill'
  },

  getLabelKey(_toolName, _params) {
    return 'toolDisplay.useSkill'
  },

  getPrimaryParameter(block): TimelineToolInlineParameter | null {
    const params = readParamsRecord(block)
    const skillName = readString(params, ['skill_name'])
    if (!skillName) return null
    return {
      kind: 'text',
      text: truncateNonEmptyToolPreviewText(skillName),
    }
  },

  getDetailSections(block): TimelineToolDetailSection[] {
    const result = readStructuredRecord(block.toolResult)
    if (!result) return []

    const description = typeof result.description === 'string' ? result.description.trim() : null
    if (!description) return []

    return [{
      id: 'skill-description',
      kind: 'preview',
      lines: [description],
      truncated: false,
    }]
  },

  hasDetails(block): boolean {
    return this.getDetailSections(block).length > 0
  },

  getInputPreview(params): string[] {
    return readablePairLines(params, ['skill_name', 'args', 'input'], INPUT_PREVIEW_LINE_MAX)
  },
}
