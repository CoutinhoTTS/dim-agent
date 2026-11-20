import type {
  ChatUiToolRenderBlock,
  ToolCallRenderer,
  TimelineToolInlineParameter,
  TimelineToolDetailSection,
} from './types'
import type { ChatUiToolLabelKey } from './keys'
import {
  inputPreviewLines,
  previewLinesFromRecord,
  previewLinesFromText,
  readablePairLines,
  readParamsRecord,
  readString,
  readStructuredRecord,
  resultText,
  textParam,
  toolSourceDetailLine,
  toolSourceLabelText,
} from './utils'

const MCP_PRIMARY_KEYS = ['query', 'q', 'path', 'url', 'name', 'id', 'command', 'input'] as const
const MCP_RESULT_KEYS = ['status', 'message', 'error', 'ok', 'count', 'items', 'total'] as const

const readMcpResultText = (block: ChatUiToolRenderBlock): string | null => {
  if (typeof block.toolResult === 'string' && block.toolResult.trim().length > 0) {
    return block.toolResult.trim()
  }
  return resultText(block)
}

export const mcpRenderer: ToolCallRenderer = {
  id: 'mcp',

  match(_toolName, _params, block) {
    return block?.toolSource?.kind === 'mcp'
  },

  getLabelKey(): ChatUiToolLabelKey {
    return 'timelineToolSummary.usedTool'
  },

  getLabelText(block) {
    return toolSourceLabelText(block)
  },

  getPrimaryParameter(block): TimelineToolInlineParameter | null {
    const params = readParamsRecord(block)
    return textParam(readString(params, MCP_PRIMARY_KEYS))
  },

  getDetailSections(block): TimelineToolDetailSection[] {
    const params = readParamsRecord(block)
    const result = readStructuredRecord(block.toolResult)
    const sections: TimelineToolDetailSection[] = []
    const sourceLine = toolSourceDetailLine(block)
    const summaryLines = [
      ...(sourceLine ? [sourceLine] : []),
      ...readablePairLines(result, MCP_RESULT_KEYS, 3),
    ]

    if (summaryLines.length > 0) {
      sections.push({
        id: 'mcp-summary',
        kind: 'summary',
        lines: summaryLines,
        truncated: false,
      })
    }

    const paramPreview = previewLinesFromRecord('mcp-params', params)
    if (paramPreview) sections.push(paramPreview)

    const text = readMcpResultText(block)
    const resultPreview = previewLinesFromText(text)
    if (resultPreview) {
      sections.push({
        ...resultPreview,
        id: 'mcp-result',
      })
    }

    return sections
  },

  hasDetails(block): boolean {
    return this.getDetailSections(block).length > 0
  },

  getInputPreview(params) {
    return inputPreviewLines(params)
  },
}
