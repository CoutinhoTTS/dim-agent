import type { ToolCallRenderer } from './types'
import { classifyTimelineToolCategory, getTimelineToolLabelKey } from '../tool-summary'
import {
  normalizeToolName,
  readString,
  readArray,
  readParamsRecord,
  readStructuredRecord,
  hostFromUrl,
  resultText,
  isJsonLikeText,
  previewLinesFromText,
  readablePairLines,
  listSection,
  textParam,
  DETAIL_LINE_MAX,
  INPUT_PREVIEW_LINE_MAX,
} from './utils'

const isWebSearch = (toolName: string | undefined): boolean => {
  const rawName = (toolName ?? '').trim().toLowerCase()
  const name = normalizeToolName(toolName)
  return rawName.includes('web_search') || rawName.includes('search_query') || name === 'websearch'
}

const isWebFetch = (toolName: string | undefined): boolean => {
  const rawName = (toolName ?? '').trim().toLowerCase()
  const name = normalizeToolName(toolName)
  return rawName.includes('web_fetch') || name === 'webfetch'
}

export const webRenderer: ToolCallRenderer = {
  id: 'web',

  match(toolName) {
    return isWebSearch(toolName) || isWebFetch(toolName)
  },

  getLabelKey(toolName, params) {
    return getTimelineToolLabelKey(toolName, classifyTimelineToolCategory(toolName), params ?? undefined)
  },

  getPrimaryParameter(block) {
    const params = readParamsRecord(block)

    if (isWebSearch(block.toolName)) {
      return textParam(readString(params, ['query', 'search_query']))
    }

    if (isWebFetch(block.toolName)) {
      const url = readString(params, ['url'])
      return textParam(url ? hostFromUrl(url) : null)
    }

    return null
  },

  getDetailSections(block) {
    const result = readStructuredRecord(block.toolResult)

    const resultItems = readArray(result, ['results', 'items'])
    const list = listSection('web-results', resultItems, DETAIL_LINE_MAX)
    if (list) {
      return [list]
    }

    const text = resultText(block)
    const preview = text && !isJsonLikeText(text)
      ? previewLinesFromText(text)
      : null
    return preview ? [preview] : []
  },

  hasDetails(block) {
    return this.getDetailSections(block).length > 0
  },

  getInputPreview(params): string[] {
    return readablePairLines(params, ['url', 'query', 'search_query'], INPUT_PREVIEW_LINE_MAX)
  },
}
