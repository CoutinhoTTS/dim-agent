import type { TimelineToolTodoItem } from './tool-renderers/types'
import { asRecord, normalizeToolName, parseMaybeJson } from './tool-renderers/utils'

export type TodoSnapshotReadResult =
  | { kind: 'missing' }
  | { kind: 'invalid'; message: string }
  | { kind: 'snapshot'; items: TimelineToolTodoItem[] }

const nestedSnapshotKeys = ['structuredContent', 'output', 'content', 'input'] as const

export function isTodoToolName(toolName: string | undefined): boolean {
  const rawName = (toolName ?? '').trim().toLowerCase()
  const name = normalizeToolName(toolName)
  return rawName === 'todowrite'
    || rawName === 'todo_write'
    || name === 'todowrite'
    || name === 'todo_write'
}

export function readTodoSnapshotValue(value: unknown, depth = 0): TodoSnapshotReadResult {
  if (depth > nestedSnapshotKeys.length) {
    return { kind: 'missing' }
  }
  const parsedJson = typeof value === 'string' ? parseMaybeJson(value) : null
  const record = asRecord(value) ?? asRecord(parsedJson)
  if (!record) {
    return { kind: 'missing' }
  }

  const declared = Object.hasOwn(record, 'todos')
    ? record.todos
    : Object.hasOwn(record, 'items')
      ? record.items
      : undefined
  if (declared !== undefined) {
    return readTodoSnapshotItems(declared)
  }

  for (const key of nestedSnapshotKeys) {
    const nested = readTodoSnapshotValue(record[key], depth + 1)
    if (nested.kind !== 'missing') {
      return nested
    }
  }
  return { kind: 'missing' }
}

export function readTodoSnapshotItems(value: unknown): TodoSnapshotReadResult {
  if (!Array.isArray(value)) {
    return { kind: 'invalid', message: 'todos/items must be an array' }
  }

  const items: TimelineToolTodoItem[] = []
  const ids = new Set<string>()
  for (let index = 0; index < value.length; index += 1) {
    const record = asRecord(value[index])
    if (!record) {
      return { kind: 'invalid', message: `todos[${index}] must be an object` }
    }
    const id = readRequiredString(record.id)
    const content = readRequiredString(record.content)
    const status = record.status
    const priority = record.priority
    if (!id || !content || !isTodoStatus(status) || !isTodoPriority(priority)) {
      return { kind: 'invalid', message: `todos[${index}] has an invalid contract` }
    }
    if (ids.has(id)) {
      return { kind: 'invalid', message: `todo id "${id}" must be unique` }
    }
    ids.add(id)
    items.push({ id, content, status, priority })
  }
  return { kind: 'snapshot', items }
}

function readRequiredString(value: unknown): string | null {
  return typeof value === 'string' && value.trim().length > 0 ? value : null
}

function isTodoStatus(value: unknown): value is TimelineToolTodoItem['status'] {
  return value === 'pending' || value === 'in_progress' || value === 'completed'
}

function isTodoPriority(value: unknown): value is TimelineToolTodoItem['priority'] {
  return value === 'high' || value === 'medium' || value === 'low'
}
