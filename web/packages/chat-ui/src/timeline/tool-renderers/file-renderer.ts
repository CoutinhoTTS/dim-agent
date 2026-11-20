import type { ToolCallRenderer, TimelineToolInlineParameter, TimelineToolDetailSection } from './types'
import { classifyTimelineToolCategory, getTimelineToolLabelKey } from '../tool-summary'
import {
  readString,
  basenameFromPath,
  previewLinesFromNonEmptyText,
  pathKeys,
  INPUT_PREVIEW_LINE_MAX,
} from './utils'
import { readEditReplacements } from './edit-replacements'

const isFileTool = (toolName: string): boolean => {
  const category = classifyTimelineToolCategory(toolName)
  if (category === 'read' || category === 'edit') return true
  const lowerName = toolName.toLowerCase()
  return lowerName.includes('apply_patch')
}

// Session blob storage layout (packages/agent/src/session/blob.ts):
// sessions/<sessionId>/blobs/files/blob_<uuid>[.ext]. Read results over the
// inline budget are offloaded there; showing the raw blob filename in the
// collapsed row is meaningless to users, so such reads display a cached-
// content label instead.
export const SESSION_BLOB_PATH_RE = /(?:^|[/\\])sessions[/\\][^/\\]+[/\\]blobs[/\\]files[/\\]blob_[^/\\]+$/

export const isSessionBlobPath = (value: string | null | undefined): boolean =>
  value !== null && value !== undefined && value.trim().length > 0 && SESSION_BLOB_PATH_RE.test(value)

export const fileRenderer: ToolCallRenderer = {
  id: 'file',

  match(toolName, _params) {
    return isFileTool(toolName)
  },

  getLabelKey(toolName, params) {
    return getTimelineToolLabelKey(toolName, classifyTimelineToolCategory(toolName), params ?? undefined)
  },

  getPrimaryParameter(block): TimelineToolInlineParameter | null {
    const params = block.toolParams as Record<string, unknown> | null
    const target = block.toolTarget ?? readString(params, pathKeys)
    if (!target) return null

    const name = classifyTimelineToolCategory(block.toolName)
    if (name === 'read') {
      if (isSessionBlobPath(target)) {
        return { kind: 'i18n', key: 'toolDisplay.readCachedContent' }
      }
      const offset = typeof params?.offset === 'number' ? params.offset : undefined
      const limit = typeof params?.limit === 'number' ? params.limit : undefined
      if (offset !== undefined && limit !== undefined) {
        const end = offset + limit - 1
        return {
          kind: 'i18n',
          key: 'toolDisplay.readLineRange',
          params: { file: basenameFromPath(target), start: String(offset), end: String(end) },
        }
      }
      if (offset !== undefined) {
        return {
          kind: 'i18n',
          key: 'toolDisplay.readFromLine',
          params: { file: basenameFromPath(target), start: String(offset) },
        }
      }
    }

    return { kind: 'text', text: basenameFromPath(target) }
  },

  getDetailSections(block): TimelineToolDetailSection[] {
    const params = block.toolParams as Record<string, unknown> | null
    const target = block.toolTarget ?? readString(params, pathKeys)
    if (!target) return []
    return [{
      id: 'file',
      kind: 'summary',
      lines: [target],
      truncated: false,
    }]
  },

  hasDetails(_block): boolean {
    return true
  },

  getInputPreview(params): string[] {
    const path = readString(params, pathKeys)
    const lines: string[] = path ? [path] : []
    const edit = readEditReplacements(params)
    if (edit.legacy && edit.replacements.length === 1) {
      const [replacement] = edit.replacements
      lines.push(...previewLinesFromNonEmptyText(replacement.oldText, 12).lines.map(line => `- ${line}`))
      lines.push(...previewLinesFromNonEmptyText(replacement.newText, 12).lines.map(line => `+ ${line}`))
      return lines
    }
    if (edit.replacements.length > 0) {
      for (const replacement of edit.replacements) {
        if (lines.length >= INPUT_PREVIEW_LINE_MAX) break
        lines.push(...previewLinesFromNonEmptyText(replacement.oldText, 1).lines.map(line => `- ${line}`))
        if (lines.length >= INPUT_PREVIEW_LINE_MAX) break
        lines.push(...previewLinesFromNonEmptyText(replacement.newText, 1).lines.map(line => `+ ${line}`))
        if (replacement.replaceAll && lines.length < INPUT_PREVIEW_LINE_MAX) {
          lines.push('  replaceAll')
        }
      }
      return lines
    }
    const newText = readString(params, ['content', 'newText', 'new_string'])
    if (newText) {
      lines.push(...previewLinesFromNonEmptyText(newText, INPUT_PREVIEW_LINE_MAX - lines.length).lines)
    }
    return lines
  },
}
