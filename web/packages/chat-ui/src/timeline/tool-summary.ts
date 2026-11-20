import type { ChatUiToolLabelKey } from './tool-renderers/keys'
import { normalizeToolName } from './tool-name'

export type TimelineToolSummaryCategory =
  | 'read'
  | 'edit'
  | 'command'
  | 'search'
  | 'webSearch'
  | 'webFetch'
  | 'subagent'
  | 'browser'
  | 'computerUse'
  | 'wait'
  | 'skill'
  | 'unknown'

export interface TimelineToolTargetSummary {
  label: string
  title: string
}

export interface TimelineToolSummary {
  category: TimelineToolSummaryCategory
  target: TimelineToolTargetSummary | null
  hasDetails: boolean
}

export interface TimelineWorkActionSummary {
  actions: TimelineToolSummaryCategory[]
  hasMore: boolean
}

/** Minimal structural view used by the category/summary helpers. */
export interface ChatUiToolSummaryBlock {
  kind?: string
  toolName?: string
  toolParams?: Record<string, unknown> | null
  toolResult?: unknown
  toolTarget?: string | null
  toolSummary?: TimelineToolSummary
  content?: string
  inputText?: string
}

const TOOL_DISPLAY_NAMES: Record<string, ChatUiToolLabelKey> = {
  // builtin SDK tools
  read: 'toolDisplay.readFile',
  write: 'toolDisplay.writeFile',
  edit: 'toolDisplay.editFile',
  exec: 'toolDisplay.runCommand',
  // builtin aliases (kept for backward compatibility)
  cat: 'toolDisplay.readFile',
  read_file: 'toolDisplay.readFile',
  write_file: 'toolDisplay.writeFile',
  edit_file: 'toolDisplay.editFile',
  exec_command: 'toolDisplay.runCommand',
  run_command: 'toolDisplay.runCommand',
  bash: 'toolDisplay.runCommand',
  // search tools
  list_files: 'toolDisplay.listFiles',
  grep: 'toolDisplay.grep',
  rg: 'toolDisplay.grep',
  glob: 'toolDisplay.glob',
  // web tools
  WebSearch: 'toolDisplay.webSearch',
  WebFetch: 'toolDisplay.webFetch',
  web_search: 'toolDisplay.webSearch',
  web_fetch: 'toolDisplay.webFetch',
  // App Server tools
  ask_user: 'toolDisplay.askUser',
  exec_stop: 'toolDisplay.stopTask',
  recall: 'toolDisplay.recall',
  settings: 'toolDisplay.readSettings',
  plugin: 'toolDisplay.managePlugins',
  codemode: 'toolDisplay.runCode',
  branch: 'toolDisplay.manageBranches',
  schedule: 'toolDisplay.manageSchedules',
  wakeup: 'toolDisplay.setWakeup',
  events: 'toolDisplay.manageEvents',
  goal_update: 'toolDisplay.updateGoal',
  agent: 'toolDisplay.agent',
  member_approval_allow: 'toolDisplay.approveMemberAction',
  member_approval_reject: 'toolDisplay.rejectMemberAction',
  // desktop adapter tools
  AskUserQuestion: 'toolDisplay.askUser',
  ask_user_question: 'toolDisplay.askUser',
  askuserquestion: 'toolDisplay.askUser',
  TodoWrite: 'toolDisplay.updateTasks',
  todo_write: 'toolDisplay.updateTasks',
  todowrite: 'toolDisplay.updateTasks',
  // plugin tools
  skill: 'toolDisplay.useSkill',
  generate_image: 'toolDisplay.generateImage',
  browser: 'toolDisplay.browser',
  // wait tool
  wait: 'toolDisplay.wait',
}

const CATEGORY_TOOL_LABEL_KEYS: Record<TimelineToolSummaryCategory, ChatUiToolLabelKey> = {
  read: 'toolDisplay.readFile',
  edit: 'toolDisplay.editFile',
  command: 'toolDisplay.runCommand',
  search: 'toolDisplay.grep',
  webSearch: 'toolDisplay.webSearch',
  webFetch: 'toolDisplay.webFetch',
  subagent: 'toolDisplay.createAgent',
  browser: 'toolDisplay.browser',
  computerUse: 'toolDisplay.computerUse',
  wait: 'toolDisplay.wait',
  skill: 'toolDisplay.useSkill',
  unknown: 'timelineToolSummary.usedTool',
}

export function getToolDisplayNameKey(toolName: string | undefined): ChatUiToolLabelKey | null {
  const rawName = toolName?.trim()
  if (!rawName) {
    return null
  }

  return TOOL_DISPLAY_NAMES[rawName]
    ?? TOOL_DISPLAY_NAMES[rawName.toLowerCase()]
    ?? null
}

export function getTimelineToolLabelKey(
  rawToolName: string | undefined,
  category: TimelineToolSummaryCategory,
  toolParams?: Record<string, unknown>,
): ChatUiToolLabelKey {
  const name = (rawToolName ?? '').trim().toLowerCase()
  if (name === 'agent' && toolParams) {
    const action = typeof toolParams.action === 'string' ? toolParams.action.trim().toLowerCase() : ''
    if (action === 'create') return 'toolDisplay.createAgent'
    if (action === 'list') return 'toolDisplay.listAgents'
    if (action === 'cancel') return 'toolDisplay.cancelAgent'
    if (action === 'query_provider_model') return 'toolDisplay.queryProviderModel'
    if (action === 'query_external_model') return 'toolDisplay.queryExternalModel'
  }
  return getToolDisplayNameKey(rawToolName) ?? CATEGORY_TOOL_LABEL_KEYS[category]
}

interface TimelineToolSummaryPhraseKeys {
  complete: ChatUiToolLabelKey
  completeTarget?: ChatUiToolLabelKey
}

const TOOL_SUMMARY_PHRASE_KEYS: Record<TimelineToolSummaryCategory, TimelineToolSummaryPhraseKeys> = {
  read: {
    complete: 'timelineToolSummary.readFiles',
    completeTarget: 'timelineToolSummary.readTarget',
  },
  edit: {
    complete: 'timelineToolSummary.editedCode',
    completeTarget: 'timelineToolSummary.editedTarget',
  },
  command: {
    complete: 'timelineToolSummary.ranCommand',
  },
  search: {
    complete: 'timelineToolSummary.searchedFiles',
  },
  webSearch: {
    complete: 'timelineToolSummary.searchedWeb',
  },
  webFetch: {
    complete: 'timelineToolSummary.fetchedWebPage',
  },
  subagent: {
    complete: 'toolDisplay.createAgent',
  },
  browser: {
    complete: 'toolDisplay.browser',
  },
  computerUse: {
    complete: 'toolDisplay.computerUse',
  },
  wait: {
    complete: 'toolDisplay.wait',
  },
  skill: {
    complete: 'timelineToolSummary.usedSkill',
  },
  unknown: {
    complete: 'timelineToolSummary.usedTool',
  },
}

const WORK_ACTION_KEYS: Record<TimelineToolSummaryCategory, ChatUiToolLabelKey> = {
  read: 'timelineToolAction.read',
  edit: 'timelineToolAction.edit',
  command: 'timelineToolAction.command',
  search: 'timelineToolAction.search',
  webSearch: 'timelineToolAction.webSearch',
  webFetch: 'timelineToolAction.webFetch',
  subagent: 'toolDisplay.createAgent',
  browser: 'toolDisplay.browser',
  computerUse: 'toolDisplay.computerUse',
  wait: 'toolDisplay.wait',
  skill: 'timelineToolAction.skill',
  unknown: 'timelineToolAction.unknown',
}

const truncate = (text: string, maxLen: number): string =>
  text.length > maxLen ? text.slice(0, maxLen) + '...' : text

const asRecord = (value: unknown): Record<string, unknown> | null => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null
  }
  return value as Record<string, unknown>
}

export function classifyTimelineToolCategory(
  toolName: string | undefined,
  _toolParams?: Record<string, unknown>,
  _blockKind: string = 'tool',
): TimelineToolSummaryCategory {
  const rawName = (toolName ?? '').trim().toLowerCase()
  const name = normalizeToolName(toolName)

  if (
    rawName.includes('web_search')
    || rawName.includes('search_query')
    || name === 'websearch'
  ) {
    return 'webSearch'
  }

  if (rawName.includes('web_fetch') || name === 'webfetch') {
    return 'webFetch'
  }

  if (
    name === 'read'
    || name === 'read_file'
    || name === 'open'
    || name === 'cat'
    || name === 'view'
  ) {
    return 'read'
  }

  if (
    name === 'write'
    || name === 'write_file'
    || name === 'edit'
    || name === 'edit_file'
    || name === 'apply_patch'
    || name === 'replace'
    || name === 'update'
    || name === 'delete'
  ) {
    return 'edit'
  }

  if (
    name === 'grep'
    || name === 'rg'
    || name === 'glob'
    || name === 'find'
    || name === 'list_files'
    || name === 'ls'
    || rawName.includes('search_files')
  ) {
    return 'search'
  }

  if (
    name === 'bash'
    || name === 'shell'
    || name === 'terminal'
    || name === 'exec'
    || name === 'run_command'
    || name === 'exec_command'
  ) {
    return 'command'
  }

  if (
    name === 'agent'
    || name === 'subagent'
    || rawName.includes('subagent')
  ) {
    return 'subagent'
  }

  if (name === 'browser') {
    return 'browser'
  }

  if (name === 'wait') {
    return 'wait'
  }

  if (rawName.startsWith('cua-driver__') || rawName === 'cua-driver') {
    return 'computerUse'
  }

  // plugin skill invocation — aggregated separately from generic tools
  if (name === 'skill') {
    return 'skill'
  }

  return 'unknown'
}

const basenameFromTarget = (target: string): string | null => {
  const trimmed = target.trim()
  if (!trimmed) {
    return null
  }

  if (/^https?:\/\//i.test(trimmed)) {
    try {
      const url = new URL(trimmed)
      return url.hostname || trimmed
    } catch {
      return trimmed
    }
  }

  const withoutTrailingSlash = trimmed.replace(/[\\/]+$/, '')
  const segments = withoutTrailingSlash.split(/[\\/]/).filter(Boolean)
  return segments.at(-1) ?? withoutTrailingSlash
}

export function summarizeTimelineToolTarget(
  target: string | null | undefined,
): TimelineToolTargetSummary | null {
  if (!target) {
    return null
  }

  const label = basenameFromTarget(target)
  return label ? { label, title: target.trim() } : null
}

const shouldShowTargetForCategory = (category: TimelineToolSummaryCategory): boolean =>
  category === 'read' || category === 'edit'

const hasMeaningfulValue = (value: unknown): boolean => {
  if (value === null || value === undefined) {
    return false
  }
  if (typeof value === 'string') {
    return value.trim().length > 0
  }
  if (Array.isArray(value)) {
    return value.length > 0
  }
  if (typeof value === 'object') {
    return Object.keys(value).length > 0
  }
  return true
}

const readToolTarget = (value: unknown): string | null => {
  if (Array.isArray(value)) {
    for (const item of value) {
      const nestedTarget = readToolTarget(item)
      if (nestedTarget) {
        return nestedTarget
      }
    }
    return null
  }

  const record = asRecord(value)
  if (!record) {
    return null
  }

  const candidate = record.path
    ?? record.filePath
    ?? record.file
    ?? record.target
    ?? record.source
    ?? record.cwd
    ?? record.workingDirectory

  if (typeof candidate === 'string' && candidate.trim().length > 0) {
    return candidate.trim()
  }

  for (const nestedKey of ['args', 'arguments', 'input', 'params', 'toolParams', 'tool_params']) {
    const nestedTarget = readToolTarget(record[nestedKey])
    if (nestedTarget) {
      return nestedTarget
    }
  }

  return null
}

const parseStructuredContent = (content: string): unknown => {
  const trimmed = content.trim()
  if (!trimmed || (!trimmed.startsWith('{') && !trimmed.startsWith('['))) {
    return null
  }

  try {
    return JSON.parse(trimmed)
  } catch {
    return null
  }
}

function extractByToolName(toolName: string, params: Record<string, unknown>): string | null {
  const name = toolName.toLowerCase()

  if (name === 'bash' || name === 'shell' || name === 'terminal' || name === 'exec' || name === 'run_command' || name === 'exec_command') {
    const command = params.command
    if (typeof command === 'string' && command.trim().length > 0) {
      return truncate(command.trim(), 80)
    }
    if (Array.isArray(command)) {
      const commandText = command
        .map(value => typeof value === 'string' || typeof value === 'number' ? String(value).trim() : '')
        .filter(Boolean)
        .join(' ')
      return commandText ? truncate(commandText, 80) : null
    }
    return null
  }

  if (name === 'grep' || name === 'rg') {
    const pattern = params.pattern
    const path = params.path
    if (typeof pattern === 'string' && pattern.trim().length > 0) {
      const base = `"${pattern.trim()}"`
      if (typeof path === 'string' && path.trim().length > 0) {
        return `${base} in ${path.trim()}`
      }
      return base
    }
    return null
  }

  if (name === 'read' || name === 'read_file' || name === 'write' || name === 'write_file' || name === 'edit' || name === 'edit_file') {
    const filePath = params.file_path ?? params.filePath ?? params.path
    if (typeof filePath === 'string' && filePath.trim().length > 0) {
      return filePath.trim()
    }
    return null
  }

  if (name === 'glob' || name === 'list_files') {
    const pattern = params.pattern
    const path = params.path
    if (typeof pattern === 'string' && pattern.trim().length > 0) {
      if (typeof path === 'string' && path.trim().length > 0) {
        return `${pattern.trim()} in ${path.trim()}`
      }
      return pattern.trim()
    }
    return null
  }

  if (name === 'agent') {
    const action = typeof params.action === 'string' ? params.action.trim().toLowerCase() : ''
    if (action === 'create') {
      const prompt = params.prompt
      if (typeof prompt === 'string' && prompt.trim().length > 0) {
        return truncate(prompt.trim(), 80)
      }
    }
    if (action === 'list' || action === 'cancel' || action === 'query_provider_model' || action === 'query_external_model') {
      return null
    }
    const description = params.description ?? params.prompt
    if (typeof description === 'string' && description.trim().length > 0) {
      return truncate(description.trim(), 80)
    }
    return null
  }

  if (name === 'web_search' || name === 'websearch') {
    const query = params.query ?? params.search_query
    if (typeof query === 'string' && query.trim().length > 0) {
      return query.trim()
    }
    return null
  }

  if (name === 'web_fetch' || name === 'webfetch') {
    const url = params.url
    if (typeof url === 'string' && url.trim().length > 0) {
      return url.trim()
    }
    return null
  }

  return null
}

export interface ChatUiToolTargetSource {
  toolParams?: Record<string, unknown> | null
  metadata?: unknown
  content?: string
}

export function extractToolTarget(
  source: ChatUiToolTargetSource,
  toolName?: string,
): string | null {
  // Try tool-aware extraction first when toolName is provided
  if (toolName && source.toolParams && typeof source.toolParams === 'object' && !Array.isArray(source.toolParams)) {
    const toolAwareTarget = extractByToolName(toolName, source.toolParams)
    if (toolAwareTarget) {
      return toolAwareTarget
    }
  }

  const directTarget = readToolTarget(source.toolParams ?? null)
  if (directTarget) {
    return directTarget
  }

  const metadataTarget = 'metadata' in source
    ? readToolTarget(source.metadata ?? null)
    : null
  if (metadataTarget) {
    return metadataTarget
  }

  const structuredTarget = 'content' in source
    ? readToolTarget(parseStructuredContent(source.content ?? ''))
    : null
  if (structuredTarget) {
    return structuredTarget
  }

  return null
}

const buildTimelineToolSummary = (
  block: ChatUiToolSummaryBlock,
): TimelineToolSummary => {
  const category = classifyTimelineToolCategory(block.toolName, block.toolParams ?? undefined, block.kind)
  const target = shouldShowTargetForCategory(category)
    ? summarizeTimelineToolTarget(block.toolTarget)
    : null

  return {
    category,
    target,
    hasDetails: hasMeaningfulValue(block.inputText)
      || hasMeaningfulValue(block.toolParams)
      || hasMeaningfulValue(block.toolResult)
      || hasMeaningfulValue(block.content),
  }
}

export function getTimelineToolSummary(
  block: ChatUiToolSummaryBlock,
): TimelineToolSummary {
  return block.toolSummary ?? buildTimelineToolSummary(block)
}

export function getTimelineToolSummaryPhraseKey(
  category: TimelineToolSummaryCategory,
  _status: string | undefined,
  hasTarget: boolean,
): ChatUiToolLabelKey {
  const keys = TOOL_SUMMARY_PHRASE_KEYS[category]
  return hasTarget && keys.completeTarget ? keys.completeTarget : keys.complete
}

export function getTimelineWorkActionLabelKey(
  category: TimelineToolSummaryCategory,
): ChatUiToolLabelKey {
  return WORK_ACTION_KEYS[category]
}

export function getTimelineWorkActionSummary(
  blocks: readonly ChatUiToolSummaryBlock[],
  maxActions = 3,
): TimelineWorkActionSummary {
  const actions: TimelineToolSummaryCategory[] = []
  for (const block of blocks) {
    if (block.kind !== 'tool') {
      continue
    }

    const category = getTimelineToolSummary(block).category
    if (!actions.includes(category)) {
      actions.push(category)
    }
  }

  return {
    actions: actions.slice(0, maxActions),
    hasMore: actions.length > maxActions,
  }
}
