import { normalizeToolName } from './tool-name'
import type { ChatUiToolResultBlock } from './tool-result-block'

export interface PlanDraftResult {
  id: string
  content: string
  path?: string
  mode?: string
}

const PLAN_DRAFT_TOOL_NAMES = new Set(['plan_read', 'plan_write'])

const asRecord = (value: unknown): Record<string, unknown> | null => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null
  }
  return value as Record<string, unknown>
}

const readString = (value: unknown): string | undefined =>
  typeof value === 'string' && value.trim().length > 0 ? value.trim() : undefined

const readContent = (value: unknown): string | undefined =>
  typeof value === 'string' && value.trim().length > 0 ? value : undefined

const readDraftFromStructuredContent = (value: unknown): Omit<PlanDraftResult, 'id'> | null => {
  const record = asRecord(value)
  if (!record) {
    return null
  }

  const content = readContent(record.content)
  const exists = record.exists === true
  if (!exists || !content) {
    return null
  }

  const path = readString(record.path)
  const mode = readString(record.mode)
  return {
    content,
    ...(path ? { path } : null),
    ...(mode ? { mode } : null),
  }
}

const readDraftFromToolResult = (
  value: unknown,
  toolName: string,
): Omit<PlanDraftResult, 'id'> | null => {
  const record = asRecord(value)
  if (!record) {
    if (toolName !== 'plan_read') {
      return null
    }
    const content = readContent(value)
    return content && !content.toLowerCase().startsWith('no plan draft')
      ? { content }
      : null
  }

  const structuredDraft = readDraftFromStructuredContent(record.structuredContent)
  if (structuredDraft) {
    return structuredDraft
  }

  if (toolName !== 'plan_read') {
    return null
  }

  const content = readContent(record.content)
  return content && !content.toLowerCase().startsWith('no plan draft')
    ? {
        content,
        ...(readString(record.path) ? { path: readString(record.path) } : null),
      }
    : null
}

const readDraftFromFileTool = (
  block: ChatUiToolResultBlock,
  planDraftPath: string | undefined,
): Omit<PlanDraftResult, 'id'> | null => {
  if (block.toolName !== 'write' && block.toolName !== 'read') return null
  const result = asRecord(block.toolResult)
  const metadata = asRecord(result?.structuredContent)
  const path = typeof metadata?.path === 'string' && metadata.path.length > 0
    ? metadata.path
    : undefined
  if (!path || !planDraftPath || result?.isError === true || typeof metadata?.revision !== 'string') return null
  if (path.replace(/\\/g, '/') !== planDraftPath.replace(/\\/g, '/')) return null

  let content: string | undefined
  if (block.toolName === 'write') {
    if (block.toolParams?.path !== path || typeof metadata.bytesWritten !== 'number') return null
    content = readContent(block.toolParams.content)
  } else {
    if (metadata.kind !== 'text' || typeof result?.content !== 'string') return null
    const lines = result.content.split('\n')
    // Partial, truncated and offloaded reads do not contain a complete 1-based sequence.
    if (!lines.every((line, index) => line.startsWith(`${index + 1}\t`))) return null
    content = readContent(lines.map((line) => line.slice(line.indexOf('\t') + 1)).join('\n'))
  }
  return content === undefined ? null : { content, path, mode: block.toolName }
}

export const extractPlanDraftResult = (
  block: ChatUiToolResultBlock,
  planDraftPath?: string,
): PlanDraftResult | null => {
  if (block.status !== 'complete' || asRecord(block.toolResult)?.isError === true) return null
  const fileDraft = readDraftFromFileTool(block, planDraftPath)
  if (fileDraft) return { id: `${block.id}:plan-draft`, ...fileDraft }

  const toolName = normalizeToolName(block.toolName)
  if (!PLAN_DRAFT_TOOL_NAMES.has(toolName)) {
    return null
  }

  const draft = readDraftFromToolResult(block.toolResult, toolName)
    ?? readDraftFromToolResult(block.content, toolName)
  if (!draft) {
    return null
  }

  return {
    id: `${block.id}:plan-draft`,
    ...draft,
  }
}
