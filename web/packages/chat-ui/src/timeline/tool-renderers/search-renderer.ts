import type { ToolCallRenderer, TimelineToolDetailSection } from './types'
import { classifyTimelineToolCategory, getTimelineToolLabelKey } from '../tool-summary'
import {
  normalizeToolName,
  readString,
  readArray,
  readParamsRecord,
  readStructuredRecord,
  readablePairLines,
  listSection,
  summarySection,
  textParam,
  DETAIL_LINE_MAX,
  INPUT_PREVIEW_LINE_MAX,
  queryKeys,
  pathKeys,
} from './utils'

const isGlobListing = (params: Record<string, unknown> | null): boolean => {
  if (!params) return false
  const pattern = readString(params, ['pattern'])
  return !pattern || pattern === '*'
}

export const searchRenderer: ToolCallRenderer = {
  id: 'search',

  match(toolName) {
    const name = normalizeToolName(toolName)
    const rawName = toolName.trim().toLowerCase()
    return (
      name === 'grep'
      || name === 'rg'
      || name === 'glob'
      || name === 'find'
      || name === 'list_files'
      || name === 'ls'
      || rawName.includes('search_files')
    )
  },

  getLabelKey(toolName, params) {
    const name = normalizeToolName(toolName)
    if (name === 'glob' && isGlobListing(params)) {
      return 'toolDisplay.listFiles'
    }
    return getTimelineToolLabelKey(toolName, classifyTimelineToolCategory(toolName), params ?? undefined)
  },

  getPrimaryParameter(block) {
    const params = readParamsRecord(block)
    const name = normalizeToolName(block.toolName)
    if (name === 'glob' && isGlobListing(params)) {
      return textParam(readString(params, pathKeys))
    }
    return textParam(readString(params, queryKeys) ?? readString(params, pathKeys))
  },

  getDetailSections(block) {
    const params = readParamsRecord(block)
    const result = readStructuredRecord(block.toolResult)

    const lines = readablePairLines(result, ['totalMatches', 'totalFiles', 'returnedMatches', 'numFiles', 'truncated'])
    const matchItems = readArray(result, ['matches', 'results', 'files'])
      ?? readArray(params, ['files', 'paths'])
    const matches = listSection('matches', matchItems, Math.max(DETAIL_LINE_MAX - lines.length, 2))
    const summary = summarySection('search-summary', lines)

    return [summary, matches].filter((s): s is TimelineToolDetailSection => Boolean(s))
  },

  hasDetails(block) {
    return this.getDetailSections(block).length > 0
  },

  getInputPreview(params): string[] {
    return readablePairLines(params, [...queryKeys, ...pathKeys], INPUT_PREVIEW_LINE_MAX)
  },
}
