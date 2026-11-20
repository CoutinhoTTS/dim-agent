import type {
  ToolCallRenderer,
  TimelineToolDetailSection,
  TimelineToolTodoItem,
} from './types'
import { classifyTimelineToolCategory, getTimelineToolLabelKey } from '../tool-summary'
import {
  readArray,
  readParamsRecord,
  readStructuredRecord,
  truncateDetailLine,
  listSection,
  DETAIL_LINE_MAX,
  INPUT_PREVIEW_LINE_MAX,
} from './utils'
import { isTodoToolName, readTodoSnapshotItems } from '../todo-snapshot'

type TodoStatus = TimelineToolTodoItem['status']

const todoStatusMark = (status: TodoStatus): string => {
  switch (status) {
    case 'completed': return '[x]'
    case 'in_progress': return '[~]'
    case 'pending': return '[ ]'
  }
}

const todoStatusText = (status: TodoStatus): string =>
  status === 'in_progress' ? 'in progress' : status

const readTodoItems = (items: unknown[] | null | undefined): TimelineToolTodoItem[] => {
  if (!items) return []
  const result = readTodoSnapshotItems(items)
  return result.kind === 'snapshot' ? result.items : []
}

const todoSection = (
  items: unknown[] | null | undefined,
  maxLines = DETAIL_LINE_MAX,
): TimelineToolDetailSection | null => {
  const todos = readTodoItems(items)
  if (todos.length === 0) {
    return listSection('todos', items, maxLines)
  }
  const visibleTodos = todos.slice(0, maxLines)
  const lines = visibleTodos.map((todo, index) =>
    `${index + 1}. ${todoStatusMark(todo.status)} ${truncateDetailLine(todo.content)} - ${todoStatusText(todo.status)} - ${todo.priority}`,
  )
  if (todos.length > maxLines) {
    lines.push(`... omitted ${todos.length - maxLines} items`)
  }
  return {
    id: 'todos',
    kind: 'summary',
    lines,
    truncated: todos.length > maxLines,
    todos: visibleTodos,
  }
}

export const todosRenderer: ToolCallRenderer = {
  id: 'todos',

  match(toolName) {
    return isTodoToolName(toolName)
  },

  getLabelKey(toolName, params) {
    return getTimelineToolLabelKey(toolName, classifyTimelineToolCategory(toolName), params ?? undefined)
  },

  getPrimaryParameter(block) {
    const params = readParamsRecord(block)
    const result = readStructuredRecord(block.toolResult)
    const items = readArray(params, ['todos', 'items'])
      ?? readArray(result, ['todos', 'items'])
    const total = items?.length ?? 0
    if (total === 0) return null

    const todos = readTodoItems(items)
    const completed = todos.filter(t => t.status === 'completed').length
    const pending = total - completed

    if (completed === 0) {
      return { kind: 'i18n' as const, key: 'toolDisplay.todosCreated', params: { count: total } as Record<string, string | number> }
    }
    if (pending === 0) {
      return { kind: 'i18n' as const, key: 'toolDisplay.todosAllDone', params: { count: total } as Record<string, string | number> }
    }
    return { kind: 'i18n' as const, key: 'toolDisplay.todosPending', params: { pending, total } as Record<string, string | number> }
  },

  getDetailSections(block) {
    const params = readParamsRecord(block)
    const result = readStructuredRecord(block.toolResult)
    const items = readArray(params, ['todos', 'items'])
      ?? readArray(result, ['todos', 'items'])
    const section = todoSection(items, DETAIL_LINE_MAX)
    return section ? [section] : []
  },

  hasDetails(block) {
    return this.getDetailSections(block).length > 0
  },

  getInputPreview(params): string[] {
    const items = readArray(params, ['todos', 'items'])
    return todoSection(items, INPUT_PREVIEW_LINE_MAX)?.lines ?? []
  },
}
