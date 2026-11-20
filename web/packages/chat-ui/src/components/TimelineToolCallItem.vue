<script setup lang="ts">
import { inject, computed, ref, watch, type Component } from 'vue'
import { useOptionalChatUiHost } from '../contracts'
import { CHAT_UI_TIMELINE_RESTORE_ACTIVE } from '../presentation'
import { buildTimelineToolTrace, buildToolInputPreview } from '../timeline/tool-trace'
import { extractCuaVisualResults } from '../timeline/generated-image-results'
import { classifyTimelineToolCategory } from '../timeline/tool-summary'
import { formatWorkDuration } from '../timeline/duration'
import {
  asRecord,
  listSection,
  readArray,
  readCommandText,
  readString,
  readStructuredRecord,
  INPUT_PREVIEW_LINE_MAX,
} from '../timeline/tool-renderers/utils'
import { readTodoSnapshotItems } from '../timeline/todo-snapshot'
import type { TimelineToolTodoItem } from '../timeline/tool-renderers/types'
import { isSessionBlobPath } from '../timeline/tool-renderers/file-renderer'
import { readEditReplacements } from '../timeline/tool-renderers/edit-replacements'
import {
  formatAskUserAnswer,
  isAskUserDeclined,
  readAskUserAnswer,
  readAskUserAnswers,
  readAskUserOptionLabels,
} from '../timeline/tool-renderers/ask-user-answers'
import type {
  ChatUiTimelineToolCallBlock,
  ChatUiTimelineToolCallFullOutputDetail,
  ChatUiTimelineToolCallFullOutputRequest,
  ChatUiTimelineToolCallIcons,
  ChatUiTimelineToolCallLabels,
  ChatUiTimelineToolCallOpenAgentPayload,
  ChatUiTimelineToolCallAgentModelInfo,
  ChatUiTimelineToolResultTruncation,
} from './tool-call-item'

defineOptions({
  name: 'TimelineToolCallItem',
})

const props = withDefaults(defineProps<{
  block: ChatUiTimelineToolCallBlock
  expanded: boolean
  sessionId: string | null
  labels: ChatUiTimelineToolCallLabels
  icons: ChatUiTimelineToolCallIcons
  buttonComponent: Component
  inlineSummaryComponent: Component
  toolResultComponent?: Component
  mediaPanelComponent: Component
  editReplacementListComponent: Component
  toolCodeBlockComponent: Component
  faviconComponent: Component
  providerIconComponent: Component
  animatedNumberComponent: Component
  tooltipComponent: Component
  tooltipTriggerComponent: Component
  tooltipContentComponent: Component
  hasToolMedia?: (callId: string) => boolean
  getAgentModelLabel?: (info: ChatUiTimelineToolCallAgentModelInfo) => string | null
  getBackgroundTaskName?: (sessionId: string, taskId: string) => string | undefined
  loadFullOutput?: (request: ChatUiTimelineToolCallFullOutputRequest) => Promise<ChatUiTimelineToolCallFullOutputDetail> | null
}>(), {
  toolResultComponent: undefined,
  hasToolMedia: undefined,
  getAgentModelLabel: undefined,
  getBackgroundTaskName: undefined,
  loadFullOutput: undefined,
})

const emit = defineEmits<{
  toggle: []
  'preview-file': [target: { path: string; startLine?: number }]
  'open-agent': [target: ChatUiTimelineToolCallOpenAgentPayload]
}>()

const timelineHost = useOptionalChatUiHost()?.timeline
const legacyRestoreTransitionSuppressed = inject(CHAT_UI_TIMELINE_RESTORE_ACTIVE, null)
const restoreTransitionSuppressed = timelineHost?.scope?.restoreActive
  ?? legacyRestoreTransitionSuppressed
// Restore mounts whole turns behind the restore cover. Appear transitions
// there are invisible and only add BaseTransition work to the restore's first
// frames; the component's update transitions stay untouched.
const appearTransitionsEnabled = computed(() => !(restoreTransitionSuppressed?.value ?? false))

const expanded = computed(() => props.expanded)
const childCount = computed(() => props.block.childCalls?.length ?? 0)
const isWait = computed(() => (props.block.toolName ?? '').trim().toLowerCase() === 'wait')
const resolvedBlock = computed<ChatUiTimelineToolCallBlock>(() => {
  const sessionId = props.sessionId
  if (!isWait.value || !sessionId || props.getBackgroundTaskName === undefined) return props.block
  const taskId = readString(asRecord(props.block.toolParams), ['taskId'])
    ?? readString(asRecord(props.block.toolParams), ['task_id'])
  if (!taskId || props.block.backgroundTaskNames?.[taskId]?.trim()) return props.block
  const label = props.getBackgroundTaskName(sessionId, taskId)
  if (label === undefined) return props.block
  return {
    ...props.block,
    backgroundTaskNames: { ...props.block.backgroundTaskNames, [taskId]: label },
  }
})
const trace = computed(() => buildTimelineToolTrace(resolvedBlock.value))
const cuaImages = computed(() => extractCuaVisualResults(props.block))
const visibleCuaImages = computed(() => cuaImages.value.slice(0, 3))
const hiddenCuaImageCount = computed(() => Math.max(0, cuaImages.value.length - visibleCuaImages.value.length))
const isCommand = computed(() => trace.value.rendererId === 'command')
const isSearch = computed(() => trace.value.rendererId === 'search')
const isFile = computed(() => trace.value.rendererId === 'file')
const isWeb = computed(() => trace.value.rendererId === 'web')
const isAskUser = computed(() => trace.value.rendererId === 'ask-user')
const isTodos = computed(() => trace.value.rendererId === 'todos')
const isSkill = computed(() => trace.value.rendererId === 'skill')
const isAgent = computed(() => trace.value.rendererId === 'agent')
const isBrowser = computed(() => trace.value.rendererId === 'browser')
const isCua = computed(() => trace.value.rendererId === 'cua')
const useRawPanel = computed(() => !isWait.value && !isBrowser.value && !isCua.value)
const truncatedToolResult = computed<ChatUiTimelineToolResultTruncation | null>(() => {
  const result = asRecord(props.block.toolResult)
  const truncation = asRecord(result?.dimTruncated)
  return truncation !== null && typeof truncation.bytes === 'number'
    ? truncation as unknown as ChatUiTimelineToolResultTruncation
    : null
})
const fullOutputText = ref<string | null>(null)
const fullOutputLoading = ref(false)
const fullOutputError = ref(false)
let fullOutputRequest = 0

// Keep-mounted latch: once expanded, panel content stays mounted so the
// collapse transition always runs over real content. Without it, collapse
// unmounts the panel in the same frame the grid starts closing, which both
// skips the height animation and briefly mounts unrelated fallback content.
const panelMounted = ref(props.expanded)
watch(expanded, (value) => {
  if (value) panelMounted.value = true
  else {
    // The full payload is read only while this panel is open. Drop it on
    // collapse so the bounded preview is the only retained copy, and invalidate
    // a request that is still in flight.
    fullOutputRequest += 1
    fullOutputText.value = null
    fullOutputLoading.value = false
    fullOutputError.value = false
  }
})

async function toggleFullOutput(): Promise<void> {
  if (fullOutputText.value !== null) {
    fullOutputText.value = null
    return
  }
  if (props.loadFullOutput === undefined) return
  let pending: ReturnType<NonNullable<typeof props.loadFullOutput>>
  try {
    pending = props.loadFullOutput({ messageId: props.block.toolCallId ?? props.block.id })
  } catch {
    fullOutputError.value = true
    return
  }
  if (pending === null) return
  const request = ++fullOutputRequest
  fullOutputLoading.value = true
  fullOutputError.value = false
  try {
    const detail = await pending
    if (request !== fullOutputRequest || !expanded.value) return
    const content = detail?.message?.content
    const fallback = content !== undefined && content !== ''
      ? content
      : formatOutput(detail?.message?.toolResult)
    if (fallback === '') {
      fullOutputError.value = true
      return
    }
    fullOutputText.value = fallback
  } catch {
    if (request === fullOutputRequest) fullOutputError.value = true
  } finally {
    if (request === fullOutputRequest) fullOutputLoading.value = false
  }
}

// Agent chip info for collapsed state
const agentInfo = computed(() => {
  if (!isAgent.value) return null
  const params = asRecord(props.block.toolParams)
  const result = asRecord(props.block.toolResult)
  const structured = asRecord(result?.structuredContent)
  const action = readString(params, ['action'])

  // agentName: prefer params, fall back to structuredContent (cancel result carries it)
  const agentName = readString(params, ['agentName']) ?? readString(structured, ['agentName'])
  if (!agentName) return null

  // external agents carry agentType in params; cancel result may carry it in structuredContent
  let externalAgentType: string | undefined
  if (action === 'create_external') {
    externalAgentType = readString(params, ['agentType']) ?? undefined
  }
  if (!externalAgentType) {
    externalAgentType = readString(structured, ['externalAgentType']) ?? undefined
  }

  // Prefer providerId/modelId from params (explicitly specified);
  // fall back to structuredContent (resolved selection, e.g. inherited default model)
  let providerId = readString(params, ['providerId']) ?? undefined
  let modelId = readString(params, ['modelId']) ?? undefined
  if (!providerId) providerId = readString(structured, ['selectedProviderId']) ?? undefined
  if (!modelId) modelId = readString(structured, ['selectedModelId']) ?? undefined

  return { agentName, providerId, modelId, externalAgentType }
})

// For query_external_model: show agentType icon in collapsed state
const queryExternalAgentType = computed(() => {
  if (!isAgent.value) return null
  const params = asRecord(props.block.toolParams)
  const action = readString(params, ['action'])
  if (action !== 'query_external_model') return null
  return readString(params, ['agentType']) ?? null
})

// Child session id for opening the agent tab
const agentChildSessionId = computed(() => {
  if (!isAgent.value) return null
  const result = asRecord(props.block.toolResult)
  const structured = asRecord(result?.structuredContent)
  return readString(structured, ['childSessionId'])
})

// --- agent panel ---
interface AgentListItem { taskId: string; agentName: string; taskTitle: string; status: string; providerId?: string; modelId?: string; externalAgentType?: string }
interface AgentPanelData {
  action: 'create' | 'list' | 'cancel' | 'query_provider_model' | 'query_external_model' | ''
  prompt?: string
  agents?: AgentListItem[]
  models?: Array<{ providerId: string; modelId: string; semanticText?: string }>
  externalModels?: { agentType: string; models: string[]; source?: string; defaultModel?: string }
}

const agentPanel = computed<AgentPanelData | null>(() => {
  if (!isAgent.value) return null
  const params = asRecord(props.block.toolParams)
  const result = asRecord(props.block.toolResult)
  const structured = asRecord(result?.structuredContent)
  const action = readString(params, ['action']) ?? ''

  if (action === 'create') {
    const prompt = readString(params, ['prompt'])
    return { action, prompt: prompt ?? undefined }
  }
  if (action === 'list') {
    const rawAgents = structured?.agents
    const agents: AgentListItem[] = Array.isArray(rawAgents)
      ? rawAgents.filter((a): a is Record<string, unknown> => a !== null && typeof a === 'object')
          .map(a => ({
            taskId: String(a.taskId ?? ''),
            agentName: String(a.agentName ?? ''),
            taskTitle: String(a.taskTitle ?? ''),
            status: String(a.status ?? ''),
            ...(typeof a.providerId === 'string' ? { providerId: a.providerId } : {}),
            ...(typeof a.modelId === 'string' ? { modelId: a.modelId } : {}),
            ...(typeof a.externalAgentType === 'string' ? { externalAgentType: a.externalAgentType } : {}),
          }))
      : []
    return { action, agents }
  }
  if (action === 'query_provider_model') {
    const rawModels = structured?.models
    const models: Array<{ providerId: string; modelId: string; semanticText?: string }> = Array.isArray(rawModels)
      ? rawModels.filter((m): m is Record<string, unknown> => m !== null && typeof m === 'object')
          .map(m => ({
            providerId: String(m.providerId ?? ''),
            modelId: String(m.modelId ?? ''),
            ...(typeof m.semanticText === 'string' ? { semanticText: m.semanticText } : {}),
          }))
          .filter(m => m.providerId || m.modelId)
      : []
    return { action, models }
  }
  if (action === 'query_external_model') {
    const agentType = readString(params, ['agentType']) ?? ''
    const rawModels = structured?.models
    const models: string[] = Array.isArray(rawModels)
      ? rawModels.filter((m): m is string => typeof m === 'string')
      : []
    const source = readString(structured, ['source']) ?? undefined
    const defaultModel = readString(structured, ['default']) ?? undefined
    return { action, externalModels: { agentType, models, source, defaultModel } }
  }
  // cancel: no detail
  return null
})

// Diff stats for write/edit in collapsed state
const diffStats = computed<{ additions: number; deletions: number } | null>(() => {
  if (!isFile.value) return null
  const panel = filePanel.value
  if (panel.view.kind === 'edit') {
    const oldLines = panel.view.replacements.flatMap(replacement => replacement.oldText.split(/\r?\n/))
      .filter(line => line.length > 0)
    const newLines = panel.view.replacements.flatMap(replacement => replacement.newText.split(/\r?\n/))
      .filter(line => line.length > 0)
    return { additions: newLines.length, deletions: oldLines.length }
  }
  if (panel.view.kind === 'content') {
    const toolName = (props.block.toolName ?? '').trim().toLowerCase()
    const isWrite = toolName === 'write' || toolName === 'write_file'
    if (isWrite) {
      const lines = panel.view.text.split(/\r?\n/).filter(l => l.length > 0)
      return { additions: lines.length, deletions: 0 }
    }
  }
  return null
})
const filePath = computed(() => {
  if (!isFile.value) return null
  const params = asRecord(props.block.toolParams)
  return props.block.toolTarget ?? readString(params, ['path', 'filePath', 'file_path']) ?? null
})

// Read tools open the file in the right-panel preview on click instead of
// expanding an inline panel (workspace files only; session blob offloads stay
// inline since they are not user-facing workspace paths).
const isReadTool = computed(() => {
  if (!isFile.value) return false
  const toolName = (props.block.toolName ?? '').trim().toLowerCase()
  if (classifyTimelineToolCategory(toolName) !== 'read') return false
  const path = filePath.value
  return path !== null && !isSessionBlobPath(path)
})

// --- web panel ---
interface WebSearchResult { title: string; url: string; snippet: string; rank?: number; siteName?: string; datePublished?: string }
interface WebPanelData {
  mode: 'search' | 'fetch'
  query?: string
  hostname?: string
  url?: string
  results?: WebSearchResult[]
  fetchContent?: string
  fetchRange?: string
  isError: boolean
  errorMessage?: string
}

const webPanel = computed<WebPanelData | null>(() => {
  if (!isWeb.value) return null
  const toolName = (props.block.toolName ?? '').trim().toLowerCase()
  const isSearch = toolName.includes('web_search') || toolName.includes('search_query') || toolName === 'websearch'
  const params = asRecord(props.block.toolParams)
  const result = asRecord(props.block.toolResult)
  const structured = asRecord(result?.structuredContent)
  const isError = result?.isError === true

  if (isSearch) {
    const query = readString(params, ['query', 'search_query']) ?? ''
    const rawResults = structured?.results
    const results: WebSearchResult[] = Array.isArray(rawResults)
      ? rawResults.filter((r): r is Record<string, unknown> => r !== null && typeof r === 'object')
          .map(r => ({
            title: String(r.title ?? ''),
            url: String(r.url ?? ''),
            snippet: String(r.snippet ?? ''),
            ...(typeof r.rank === 'number' ? { rank: r.rank } : {}),
            ...(typeof r.siteName === 'string' ? { siteName: r.siteName } : {}),
            ...(typeof r.datePublished === 'string' ? { datePublished: r.datePublished } : {}),
          }))
          .filter(r => r.title || r.url)
      : []
    return { mode: 'search', query, results, isError }
  }

  // web_fetch
  const url = readString(params, ['url']) ?? ''
  const hostname = url ? hostnameFromUrl(url) : null
  const content = typeof result?.content === 'string' ? result.content : ''
  const startIndex = typeof structured?.startIndex === 'number' ? structured.startIndex : 0
  const contentLength = typeof structured?.contentLength === 'number' ? structured.contentLength : 0
  const returnedLength = typeof structured?.returnedLength === 'number' ? structured.returnedLength : content.length
  const truncated = structured?.truncated === true

  let fetchRange = ''
  if (truncated && contentLength > 0) {
    fetchRange = `${formatBytes(returnedLength)}/${formatBytes(contentLength)}`
  }

  return {
    mode: 'fetch',
    hostname: hostname ?? undefined,
    url,
    fetchContent: content,
    fetchRange,
    isError,
    errorMessage: isError ? content : undefined,
  }
})

// Favicon hostnames for search collapsed preview (first 3)
const searchFaviconHosts = computed<string[]>(() => {
  if (!webPanel.value || webPanel.value.mode !== 'search') return []
  return webPanel.value.results?.slice(0, 3).map(r => hostnameFromUrl(r.url)).filter((h): h is string => h !== null) ?? []
})
const searchResultCount = computed(() => {
  if (!webPanel.value || webPanel.value.mode !== 'search') return 0
  return webPanel.value.results?.length ?? 0
})

// §9.2 web search: favicon batch enters one by one, 80ms between hosts
// (same data-enter-index stagger pattern as block group children).
const FAVICON_STAGGER_STEP_MS = 80

function setFaviconStaggerDelay(element: Element) {
  const index = Number((element as HTMLElement).dataset.enterIndex ?? 0)
  ;(element as HTMLElement).style.transitionDelay = `${index * FAVICON_STAGGER_STEP_MS}ms`
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n}`
  if (n < 1024 * 1024) return `${Math.round(n / 1024)}K`
  return `${Math.round(n / (1024 * 1024))}M`
}

function hostnameFromUrl(url: string): string | null {
  try {
    return new URL(url).hostname
  } catch {
    return null
  }
}

// --- ask-user panel ---
interface AskUserQA {
  header: string
  question: string
  answer: string | null
}

const askUserPanel = computed<{
  items: AskUserQA[]
  timedOutDuration: string | null
  declined: boolean
} | null>(() => {
  if (!isAskUser.value) return null
  const params = asRecord(props.block.toolParams)
  const structured = readStructuredRecord(props.block.toolResult)
  const answers = readAskUserAnswers(structured)
  // A question nobody answered: the row and the panel say so instead of looking
  // like a question that is still waiting.
  const timedOutDuration = structured?.code === 'ask_user_timed_out'
    ? formatWorkDuration(typeof structured.waitedMs === 'number' ? structured.waitedMs : 0)
    : null
  const declined = isAskUserDeclined(structured)

  const rawQuestions = params?.questions
  const questions: Record<string, unknown>[] = Array.isArray(rawQuestions)
    ? rawQuestions.filter((q): q is Record<string, unknown> => q !== null && typeof q === 'object')
    : []

  const items: AskUserQA[] = questions.map((q, i) => {
    const header = typeof q.header === 'string' ? q.header : ''
    const question = typeof q.question === 'string' ? q.question : ''
    const rawAnswer = readAskUserAnswer(answers, q, i, header)
    const answer = rawAnswer === undefined || rawAnswer === null
      ? null
      : formatAskUserAnswer(rawAnswer, readAskUserOptionLabels(q))

    return { header, question, answer }
  })

  return { items, timedOutDuration, declined }
})

// --- todos panel ---
// Parsed through the validated todo-snapshot reader. Valid snapshots render as
// todo rows; an invalid snapshot renders as a generic raw list — the same
// fallback the registry's todos renderer uses — instead of fabricated
// pending/medium defaults.
type TodosPanelData =
  | { kind: 'snapshot'; items: TimelineToolTodoItem[] }
  | { kind: 'invalid'; lines: string[] }

const todosPanel = computed<TodosPanelData | null>(() => {
  if (!isTodos.value) return null
  const params = asRecord(props.block.toolParams)
  const result = asRecord(props.block.toolResult)
  const structured = asRecord(result?.structuredContent)
  const rawItems = readArray(params, ['todos', 'items'])
    ?? readArray(structured, ['todos', 'items'])
    ?? readArray(result, ['todos'])
  if (rawItems === null) return null
  const verdict = readTodoSnapshotItems(rawItems)
  if (verdict.kind === 'invalid') {
    const section = listSection('todos', rawItems, INPUT_PREVIEW_LINE_MAX)
    return section === null ? null : { kind: 'invalid', lines: section.lines }
  }
  if (verdict.kind === 'missing') return null
  return verdict.items.length > 0 ? { kind: 'snapshot', items: verdict.items } : null
})

const priorityColor: Record<string, string> = {
  high: 'bg-orange-500',
  medium: 'bg-amber-300',
  low: 'bg-muted-foreground/40',
}

// --- skill panel ---
const skillData = computed<{ name: string; description: string | null } | null>(() => {
  if (!isSkill.value) return null
  const params = asRecord(props.block.toolParams)
  const name = readString(params, ['skill_name']) ?? ''
  const result = asRecord(props.block.toolResult)
  const structured = asRecord(result?.structuredContent)
  const description = typeof structured?.description === 'string' ? structured.description.trim() : null
  return { name, description: description && description.length > 0 ? description : null }
})

// --- browser panel ---
// Expandable browser actions: navigate, snapshot, console, errors, eval, cdp.
// reload/click/fill/press/scroll/wait output is a one-liner already conveyed by
// the collapsed row; screenshot has a companion image block.
const BROWSER_EXPANDABLE_ACTIONS = new Set(['navigate', 'snapshot', 'console', 'errors', 'eval', 'cdp'])

const browserPanel = computed<{ inputLines: string[]; output: string } | null>(() => {
  if (!isBrowser.value) return null
  const params = asRecord(props.block.toolParams)
  if (!params) return null
  const action = typeof params.action === 'string' ? params.action.trim().toLowerCase() : ''
  if (!BROWSER_EXPANDABLE_ACTIONS.has(action)) return null

  // Input: show key params (exclude action itself) as readable key: value lines.
  const inputKeys = ['url', 'expression', 'method', 'params', 'selector', 'interactiveOnly', 'compact', 'clear', 'fullPage'] as const
  const inputLines: string[] = []
  for (const key of inputKeys) {
    const val = params[key]
    if (val === undefined || val === null) continue
    inputLines.push(`${key}: ${typeof val === 'string' ? val : JSON.stringify(val)}`)
  }

  const output = formatOutput(props.block.toolResult)
  return { inputLines, output: output || '' }
})

// --- cua panel ---
// Screenshot actions: get_window_state, zoom, health_report produce images.
// Info/query actions: list_apps, list_windows, get_accessibility_tree,
// get_screen_size, get_cursor_position, page — expandable with input/output.
// Others (click, type_text, etc.) output a one-liner — not expandable.
const CUA_SCREENSHOT_ACTIONS = new Set(['get_window_state', 'zoom', 'health_report'])
const CUA_EXPANDABLE_ACTIONS = new Set([
  'list_apps', 'list_windows', 'get_accessibility_tree',
  'get_screen_size', 'get_cursor_position', 'page',
])

const cuaPanel = computed<{ inputLines: string[]; output: string; hasImages: boolean } | null>(() => {
  if (!isCua.value) return null
  const toolName = (props.block.toolName ?? '').trim().toLowerCase()
  // Strip cua-driver__ prefix
  const action = toolName.startsWith('cua-driver__')
    ? toolName.slice('cua-driver__'.length)
    : toolName
  const hasImages = cuaImages.value.length > 0
  const isScreenshot = CUA_SCREENSHOT_ACTIONS.has(action)
  const isExpandable = CUA_EXPANDABLE_ACTIONS.has(action)

  if (!isScreenshot && !isExpandable && !hasImages) return null

  const params = asRecord(props.block.toolParams)
  const inputKeys = [
    'name', 'bundle_id', 'pid', 'window_id', 'element_token', 'element_index',
    'x', 'y', 'text', 'value', 'key', 'keys', 'direction', 'amount',
    'session', 'enabled', 'action', 'selector', 'css_selector', 'javascript',
    'url', 'from_x', 'from_y', 'to_x', 'to_y', 'x1', 'y1', 'x2', 'y2',
  ] as const
  const inputLines: string[] = []
  if (params) {
    for (const key of inputKeys) {
      const val = params[key]
      if (val === undefined || val === null) continue
      inputLines.push(`${key}: ${typeof val === 'string' ? val : JSON.stringify(val)}`)
    }
  }

  const output = isScreenshot ? '' : formatOutput(props.block.toolResult)
  return { inputLines, output: output || '', hasImages }
})

// --- generic raw panel ---
const rawInput = computed(() => props.block.toolInputText ?? formatRaw(props.block.toolParams))
const rawOutput = computed(() => formatOutput(props.block.toolResult))

// --- exec panel ---
const execInput = computed(() => buildExecInput(props.block))
const execOutput = computed(() => buildExecOutput(props.block))

// --- search panel ---
const searchInput = computed(() => buildSearchInput(props.block))
const searchOutput = computed(() => formatOutput(props.block.toolResult))

// --- file panel ---
const filePanel = computed(() => buildFilePanel(props.block))

// Collapsed edit calls show the same params-only preview the renderer registry
// produces for the permission prompt. Multi-replacement calls would otherwise
// collapse to just the path plus approximate line counts.
const collapsedEditPreview = computed(() => {
  if (!isFile.value || expanded.value) return []
  const params = asRecord(props.block.toolParams)
  const edit = readEditReplacements(params)
  if (edit.replacements.length === 0) return []
  if (edit.replacements.length === 1 && !edit.replacements[0]!.replaceAll) return []
  return buildToolInputPreview(props.block.toolName, params).slice(1, 9)
})
const collapsedEditPreviewMore = computed(() => {
  if (collapsedEditPreview.value.length === 0) return false
  const params = asRecord(props.block.toolParams)
  return buildToolInputPreview(props.block.toolName, params).length > collapsedEditPreview.value.length + 1
})
// Sticky path line of the file panel; the edit variant is clickable and adds
// the affordances that signal it.
const FILE_PANEL_PATH_CLASS = 'sticky top-0 z-10 truncate bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground'
const FILE_PANEL_PATH_OPEN_CLASS = `${FILE_PANEL_PATH_CLASS} block w-full cursor-pointer text-left hover:underline`
const hasDetails = computed(() => {
  if (childCount.value > 0) return true
  if (truncatedToolResult.value !== null) return true
  if (props.hasToolMedia?.(props.block.toolCallId ?? props.block.id) === true) return true
  if (isCommand.value) {
    return execInput.value.command.length > 0 || execOutput.value.text.length > 0
  }
  if (isSearch.value) {
    return searchInput.value.primary.length > 0 || searchOutput.value.length > 0
  }
  if (isFile.value) {
    if (isReadTool.value) return false
    return filePanel.value.title.length > 0 || filePanel.value.errorText.length > 0
  }
  if (isWeb.value) {
    if (!webPanel.value) return false
    if (webPanel.value.mode === 'search') return (webPanel.value.results?.length ?? 0) > 0
    return (webPanel.value.fetchContent?.length ?? 0) > 0 || webPanel.value.isError
  }
  if (isAskUser.value) {
    return (askUserPanel.value?.items.length ?? 0) > 0
  }
  if (isTodos.value) {
    return todosPanel.value !== null
  }
  if (isSkill.value) {
    return (skillData.value?.description?.length ?? 0) > 0
  }
  if (isAgent.value) {
    const panel = agentPanel.value
    if (panel) {
      if (panel.action === 'create') return (panel.prompt?.length ?? 0) > 0
      if (panel.action === 'list') return (panel.agents?.length ?? 0) > 0
      if (panel.action === 'query_provider_model') return (panel.models?.length ?? 0) > 0
      if (panel.action === 'query_external_model') return (panel.externalModels?.models.length ?? 0) > 0 || !!panel.externalModels?.defaultModel
    }
    // cancel and unknown actions fall through to rawPanel
  }
  if (isBrowser.value) {
    return browserPanel.value !== null && (browserPanel.value.inputLines.length > 0 || browserPanel.value.output.length > 0)
  }
  if (isCua.value) {
    return cuaPanel.value !== null
  }
  if (useRawPanel.value) {
    return rawInput.value.length > 0 || rawOutput.value.length > 0
  }
  // isWait: waitRenderer has no details by contract (collapsed row carries all info).
  return false
})
const getAgentModelLabel = (info: ChatUiTimelineToolCallAgentModelInfo): string | null =>
  props.getAgentModelLabel?.(info) ?? null

function openAgentTab() {
  if (!agentChildSessionId.value || !agentInfo.value) return
  emit('open-agent', {
    subagentSessionId: agentChildSessionId.value,
    title: agentInfo.value.agentName,
  })
}

// File panel: code/diff data for ToolCodeBlock. Edit calls that carry more than
// one replacement render through EditReplacementList instead.
const editReplacements = computed(() => {
  if (!isFile.value) return null
  const panel = filePanel.value
  if (panel.view.kind !== 'edit') return null
  if (panel.view.legacy && panel.view.replacements.length === 1) return null
  return panel.view.replacements
})

const fileCode = computed(() => {
  if (!isFile.value) return null
  const panel = filePanel.value
  const toolName = (props.block.toolName ?? '').trim().toLowerCase()
  const isRead = toolName === 'read' || toolName === 'read_file' || toolName === 'cat' || toolName === 'view' || toolName === 'open'
  if (panel.view.kind === 'content') {
    const startLine = isRead ? extractStartLine(panel.view.text) : undefined
    return { mode: 'content' as const, text: panel.view.text, title: panel.title, showLineNumbers: true, startLine, stripLineNumbers: isRead }
  }
  if (panel.view.kind === 'edit' && panel.view.legacy && panel.view.replacements.length === 1) {
    const [replacement] = panel.view.replacements
    return { mode: 'diff' as const, oldText: replacement.oldText, newText: replacement.newText, title: panel.title, showLineNumbers: false, startLine: undefined, stripLineNumbers: false }
  }
  return null
})

function extractStartLine(text: string): number | undefined {
  const match = /^(\d+)\t/.exec(text)
  return match ? Number(match[1]) : undefined
}

function toggleExpanded() {
  if (!hasDetails.value) return
  emit('toggle')
}

function handleToolCallClick() {
  if (isReadTool.value) {
    const path = filePath.value
    // The collapsed row summarizes the read range from `offset`; opening the
    // preview on that same line lands the click where the tool actually looked.
    const offset = asRecord(props.block.toolParams)?.offset
    if (path) emit('preview-file', { path, startLine: typeof offset === 'number' ? offset : undefined })
    return
  }
  toggleExpanded()
}

// The expanded edit panel's sticky path line opens the same right-panel preview
// the read row does, so an edited file is one click away from its full content.
function handleFilePathClick() {
  const path = filePath.value
  if (!path) return
  emit('preview-file', { path })
}

function formatRaw(value: unknown): string {
  if (value === null || value === undefined) return ''
  if (typeof value === 'string') return value
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

function formatOutput(value: unknown): string {
  if (value === null || value === undefined) return ''
  if (typeof value === 'string') return value
  const record = asRecord(value)
  if (record) {
    const content = record.content
    if (typeof content === 'string' && content.trim().length > 0) return content
  }
  return formatRaw(value)
}

function buildExecInput(block: ChatUiTimelineToolCallBlock): { command: string; cwd: string | null } {
  const params = asRecord(block.toolParams)
  const command = readCommandText(params)
  const cwd = readString(params, ['cwd'])
  return { command: command ?? '', cwd }
}

function buildExecOutput(block: ChatUiTimelineToolCallBlock): { text: string; exitCode: number | null; signal: string | null; isError: boolean } {
  const record = asRecord(block.toolResult)
  const structured = asRecord(record?.structuredContent)

  let text: string
  if (record) {
    const content = record.content
    text = typeof content === 'string' ? content : ''
  } else if (typeof block.toolResult === 'string') {
    text = block.toolResult
  } else {
    text = ''
  }

  const exitCode = structured && typeof structured.exitCode === 'number' ? structured.exitCode : null
  const signal = readString(structured, ['signal'])
  const isError = record?.isError === true || exitCode !== null && exitCode !== 0

  return { text, exitCode, signal, isError }
}

function buildSearchInput(block: ChatUiTimelineToolCallBlock): { primary: string; primaryLabel: string; secondary: string | null } {
  const params = asRecord(block.toolParams)
  if (!params) return { primary: '', primaryLabel: 'pattern', secondary: null }

  const pattern = readString(params, ['pattern'])
  const path = readString(params, ['path', 'filePath', 'file_path'])
  const query = readString(params, ['query', 'search_query'])

  // Glob listing: pattern is * or missing → path is primary
  const isListing = !pattern || pattern === '*'
  if (isListing) {
    return { primary: path ?? '', primaryLabel: 'path', secondary: null }
  }

  // Grep / glob search: pattern (or query) is primary, path is secondary
  const primary = pattern ?? query ?? ''
  return { primary, primaryLabel: pattern ? 'pattern' : 'query', secondary: path }
}

type FilePanelView =
  | { kind: 'content'; text: string }
  | { kind: 'edit'; replacements: ReturnType<typeof readEditReplacements>['replacements']; legacy: boolean }
  | { kind: 'empty' }

function buildFilePanel(block: ChatUiTimelineToolCallBlock): {
  title: string
  view: FilePanelView
  errorText: string
  isEdit: boolean
} {
  const params = asRecord(block.toolParams)
  const title = block.toolTarget
    ?? readString(params, ['path', 'filePath', 'file_path'])
    ?? ''
  const toolName = (block.toolName ?? '').trim().toLowerCase()
  const isEdit = toolName === 'edit' || toolName === 'edit_file' || toolName === 'apply_patch'
  const isWrite = toolName === 'write' || toolName === 'write_file'

  let view: FilePanelView = { kind: 'empty' }

  if (isEdit) {
    const replacements = readEditReplacements(params)
    if (replacements.replacements.length > 0) {
      view = { kind: 'edit', replacements: replacements.replacements, legacy: replacements.legacy }
    }
  } else if (isWrite) {
    const content = readString(params, ['content'])
    if (content) {
      view = { kind: 'content', text: content }
    }
  } else {
    // read: content comes from output
    const output = formatOutput(block.toolResult)
    if (output) {
      view = { kind: 'content', text: output }
    }
  }

  // Error text from output
  let errorText = ''
  const result = asRecord(block.toolResult)
  if (result?.isError === true) {
    errorText = formatOutput(block.toolResult)
  }

  return { title, view, errorText, isEdit }
}
</script>

<template>
  <div
    class="mb-3 outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background"
    data-testid="timeline-tool-block"
    :data-block-id="block.id"
    :data-tool-status="block.status"
    tabindex="0"
  >
    <div class="flex min-w-0 items-center gap-1">
      <component
        :is="buttonComponent"
        variant="ghost"
        class="group/tool-call !p-0 h-auto min-w-0 flex-1 justify-start gap-1 rounded-none text-base font-normal shadow-none transition-colors hover:bg-transparent dark:hover:bg-transparent"
        :class="{ 'cursor-default': !hasDetails && !isReadTool }"
        @click="handleToolCallClick"
      >
        <component
          :is="inlineSummaryComponent"
          class="min-w-0"
          :trace="trace"
          :status="block.status"
          :file-path="filePath"
          :favicon-hostname="webPanel?.mode === 'fetch' ? webPanel.hostname : null"
          :agent-info="agentInfo"
          :external-agent-type="queryExternalAgentType"
          @agent-chip-click="openAgentTab"
        />
        <div
          v-if="diffStats"
          class="flex shrink-0 items-center gap-0.5 text-base tabular-nums"
          data-testid="timeline-tool-diff-stats"
        >
          <component :is="animatedNumberComponent" :value="diffStats.additions" prefix="+" :duration-ms="220" class="text-activity-success" />
          <component :is="animatedNumberComponent" v-if="diffStats.deletions > 0" :value="diffStats.deletions" prefix="-" :duration-ms="220" class="text-destructive" />
        </div>
        <!-- web search: favicon batch preview（§9.2：逐条 fade-in stagger 80ms，+N roll） -->
        <div
          v-if="isWeb && webPanel?.mode === 'search' && searchFaviconHosts.length > 0"
          class="flex shrink-0 items-center gap-0.5"
          data-testid="timeline-tool-search-favicons"
        >
          <TransitionGroup
            name="param-swap"
            tag="div"
            class="flex shrink-0 items-center gap-0.5"
            :appear="appearTransitionsEnabled"
            @before-enter="setFaviconStaggerDelay"
          >
            <component
              :is="faviconComponent"
              v-for="(host, index) in searchFaviconHosts"
              :key="host"
              :data-enter-index="index"
              :hostname="host"
              :size="16"
            />
          </TransitionGroup>
          <Transition name="param-swap" :appear="appearTransitionsEnabled">
            <component
              :is="animatedNumberComponent"
              v-if="searchResultCount > 3"
              :value="searchResultCount - 3"
              prefix="+"
              :duration-ms="220"
              class="text-xs text-muted-foreground"
            />
          </Transition>
        </div>
        <span
          v-if="isWeb && webPanel?.isError"
          class="shrink-0 text-xs text-activity-warning"
          data-testid="timeline-tool-web-error"
        >✗</span>
        <span
          v-if="childCount > 0"
          class="shrink-0 rounded-full bg-muted px-1.5 text-[10px] tabular-nums text-muted-foreground"
          data-testid="timeline-tool-child-count"
        >{{ childCount }}</span>
        <component
          :is="icons.chevron"
          v-if="hasDetails"
          :class="{ 'rotate-90': expanded }"
          class="h-4 w-4 shrink-0 cursor-pointer text-muted-foreground transition-[color,transform] group-hover/tool-call:text-foreground"
          data-testid="timeline-tool-chevron"
          @click.stop="toggleExpanded"
        />
      </component>
    </div>

    <div
      v-if="collapsedEditPreview.length > 0"
      class="ml-5 mt-0.5 space-y-0.5 overflow-hidden font-mono text-[11px] leading-4 text-muted-foreground"
      data-testid="timeline-tool-edit-preview"
    >
      <div
        v-for="(line, index) in collapsedEditPreview"
        :key="index"
        class="truncate"
        :class="line.startsWith('+') ? 'text-activity-success' : line.startsWith('-') ? 'text-destructive' : ''"
      >{{ line }}</div>
      <div v-if="collapsedEditPreviewMore" class="text-muted-foreground/60">…</div>
    </div>

    <div class="collapse-grid" :class="{ 'collapse-grid-open': expanded && hasDetails }">
      <div class="collapse-grid-inner">
        <div class="py-1">
          <div
            v-if="panelMounted && childCount > 0"
            class="mb-2 ml-2 space-y-1 border-l border-border pl-2"
            data-testid="timeline-tool-child-calls"
          >
            <div
              v-for="child in block.childCalls"
              :key="child.id"
              class="flex min-w-0 items-center gap-1.5"
              data-testid="timeline-tool-child-call"
              :data-tool-status="child.status"
            >
              <span
                class="h-1.5 w-1.5 shrink-0 rounded-full"
                :class="child.status === 'error'
                  ? 'bg-destructive'
                  : child.status === 'running' || child.status === 'queued'
                    ? 'bg-activity-warning'
                    : 'bg-muted-foreground'"
              />
              <component
                :is="inlineSummaryComponent"
                class="min-w-0"
                :trace="buildTimelineToolTrace(child)"
                :status="child.status"
              />
            </div>
          </div>
          <component v-if="panelMounted && toolResultComponent && props.hasToolMedia?.(block.toolCallId ?? block.id) === true" :is="toolResultComponent" :block="block" />
          <div
            v-else-if="isCommand && panelMounted && block.toolParams"
            class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card"
            data-testid="timeline-tool-exec-panel"
          >
            <div class="sticky top-0 z-10 bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground">输入</div>
            <div class="px-3 pb-2 font-mono text-xs leading-5">
              <div class="text-foreground/90"><span class="text-muted-foreground">$</span> {{ execInput.command }}</div>
              <div v-if="execInput.cwd" class="text-muted-foreground/70">  cwd: {{ execInput.cwd }}</div>
            </div>
            <div v-if="execOutput.text.length > 0" class="border-t border-border"></div>
            <template v-if="execOutput.text.length > 0">
              <div class="sticky top-0 z-10 bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground">输出</div>
              <div v-if="execOutput.isError" class="px-3 pt-1 font-mono text-xs text-muted-foreground">
                exit code: {{ execOutput.exitCode }}<span v-if="execOutput.signal"> · signal: {{ execOutput.signal }}</span>
              </div>
              <pre class="px-3 pb-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ execOutput.text }}</pre>
            </template>
          </div>
          <div
            v-else-if="isSearch && panelMounted"
            class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card"
            data-testid="timeline-tool-search-panel"
          >
            <template v-if="searchInput.primary">
              <div class="sticky top-0 z-10 bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground">输入</div>
              <div class="px-3 pb-2 font-mono text-xs leading-5">
                <div class="text-foreground/90">{{ searchInput.primaryLabel }}: {{ searchInput.primary }}</div>
                <div v-if="searchInput.secondary" class="text-muted-foreground/70">  path: {{ searchInput.secondary }}</div>
              </div>
            </template>
            <div v-if="searchInput.primary && searchOutput" class="border-t border-border"></div>
            <template v-if="searchOutput">
              <div class="sticky top-0 z-10 bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground">输出</div>
              <pre class="px-3 pb-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ searchOutput }}</pre>
            </template>
          </div>
          <div
            v-else-if="isFile && panelMounted"
            class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card"
            data-testid="timeline-tool-file-panel"
          >
            <button
              v-if="filePanel.isEdit && filePanel.title"
              type="button"
              :class="FILE_PANEL_PATH_OPEN_CLASS"
              data-testid="timeline-tool-file-path-open"
              @click="handleFilePathClick"
            >{{ filePanel.title }}</button>
            <div v-else :class="FILE_PANEL_PATH_CLASS">{{ filePanel.title }}</div>
            <component
              :is="editReplacementListComponent"
              v-if="editReplacements"
              :replacements="editReplacements"
              :file-name="filePanel.title"
              :show-indexes="editReplacements.length > 1"
            />
            <component
              :is="toolCodeBlockComponent"
              v-else-if="fileCode"
              :file-name="fileCode.title"
              :code="fileCode.mode === 'content' ? fileCode.text : undefined"
              :old-code="fileCode.mode === 'diff' ? fileCode.oldText : undefined"
              :new-code="fileCode.mode === 'diff' ? fileCode.newText : undefined"
              :show-line-numbers="fileCode.showLineNumbers"
              :start-line="fileCode.startLine"
              :strip-line-numbers="fileCode.stripLineNumbers"
            />
            <template v-if="filePanel.errorText">
              <div class="border-t border-border"></div>
              <div class="sticky top-0 z-10 bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground">输出</div>
              <pre class="px-3 pb-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ filePanel.errorText }}</pre>
            </template>
          </div>
          <!-- web panel -->
          <div
            v-else-if="isWeb && panelMounted && webPanel"
            class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card"
            data-testid="timeline-tool-web-panel"
          >
            <!-- search results -->
            <template v-if="webPanel.mode === 'search'">
              <div
                v-for="result in webPanel.results"
                :key="result.url"
                class="border-b border-border/50 px-3 py-2 first:border-t-0"
              >
                <div class="flex items-center gap-1.5">
                  <component
                    :is="faviconComponent"
                    v-if="hostnameFromUrl(result.url)"
                    :hostname="hostnameFromUrl(result.url)!"
                    :size="16"
                  />
                  <a
                    :href="result.url"
                    target="_blank"
                    rel="noopener"
                    class="min-w-0 truncate text-xs font-medium text-foreground/90 hover:underline"
                    @click.stop
                  >{{ result.title }}</a>
                </div>
                <div class="mt-0.5 truncate text-[11px] text-muted-foreground/60">
                  {{ hostnameFromUrl(result.url) }}<span v-if="result.datePublished"> · {{ result.datePublished.slice(0, 10) }}</span>
                </div>
                <p v-if="result.snippet" class="mt-1 text-xs text-muted-foreground/80 line-clamp-2">{{ result.snippet }}</p>
              </div>
            </template>
            <!-- fetch content -->
            <template v-else>
              <div class="sticky top-0 z-10 flex items-center gap-1.5 bg-card px-3 pt-2 pb-1">
                <component
                  :is="faviconComponent"
                  v-if="webPanel.hostname"
                  :hostname="webPanel.hostname"
                  :size="16"
                />
                <a
                  v-if="webPanel.url"
                  :href="webPanel.url"
                  target="_blank"
                  rel="noopener"
                  class="min-w-0 truncate text-[11px] text-muted-foreground hover:underline"
                  @click.stop
                >{{ webPanel.url }}</a>
              </div>
              <div v-if="webPanel.isError" class="px-3 py-2 text-xs text-activity-warning">
                {{ webPanel.errorMessage }}
              </div>
              <pre v-else class="px-3 py-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ webPanel.fetchContent }}</pre>
            </template>
          </div>
          <!-- ask-user panel -->
          <div
            v-else-if="isAskUser && panelMounted && askUserPanel"
            class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card"
            data-testid="timeline-tool-ask-user-panel"
          >
            <div
              v-for="(qa, idx) in askUserPanel.items"
              :key="idx"
              class="px-3 py-2"
              :class="{ 'border-t border-border/50': idx > 0 }"
            >
              <div v-if="qa.header" class="text-[11px] font-medium text-muted-foreground">{{ qa.header }}</div>
              <div v-if="qa.question" class="text-xs text-foreground/80">{{ qa.question }}</div>
              <div
                v-if="qa.answer"
                class="mt-1 text-xs font-medium text-foreground"
              >{{ qa.answer }}</div>
            </div>
            <div
              v-if="askUserPanel.timedOutDuration"
              class="border-t border-border/50 px-3 py-2 text-xs text-muted-foreground"
              data-testid="timeline-tool-ask-user-timed-out"
            >
              {{ labels.askUserTimedOut(askUserPanel.timedOutDuration) }}
            </div>
            <div
              v-if="askUserPanel.declined"
              class="border-t border-border/50 px-3 py-2 text-xs text-muted-foreground"
              data-testid="timeline-tool-ask-user-declined"
            >
              {{ labels.askUserDeclined }}
            </div>
          </div>
          <!-- todos panel -->
          <div
            v-else-if="isTodos && panelMounted && todosPanel"
            class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card"
            data-testid="timeline-tool-todos-panel"
          >
            <template v-if="todosPanel.kind === 'snapshot'">
              <div
                v-for="todo in todosPanel.items"
                :key="todo.id"
                class="flex items-center gap-2 border-b border-border/50 px-3 py-1.5 last:border-b-0"
              >
                <component
                  :is="icons.todoCompleted"
                  v-if="todo.status === 'completed'"
                  class="h-3.5 w-3.5 shrink-0 text-activity-success"
                />
                <component
                  :is="icons.todoInProgress"
                  v-else-if="todo.status === 'in_progress'"
                  class="h-3.5 w-3.5 shrink-0 text-primary"
                />
                <component
                  :is="icons.todoPending"
                  v-else
                  class="h-3.5 w-3.5 shrink-0 text-muted-foreground"
                />
                <span
                  class="min-w-0 flex-1 truncate text-xs"
                  :class="todo.status === 'completed' ? 'text-muted-foreground line-through' : 'text-foreground/80'"
                >{{ todo.content }}</span>
                <span
                  class="h-2 w-2 shrink-0 rounded-full"
                  :class="priorityColor[todo.priority]"
                  :title="todo.priority"
                />
              </div>
            </template>
            <template v-else>
              <div
                v-for="(line, index) in todosPanel.lines"
                :key="index"
                class="border-b border-border/50 px-3 py-1.5 text-xs text-foreground/80 last:border-b-0"
              >{{ line }}</div>
            </template>
          </div>
          <!-- skill panel -->
          <div
            v-else-if="isSkill && panelMounted && skillData?.description"
            class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card"
            data-testid="timeline-tool-skill-panel"
          >
            <div class="px-3 py-2 text-xs leading-5 text-foreground/80">{{ skillData.description }}</div>
          </div>
          <!-- agent panel -->
          <div
            v-else-if="isAgent && panelMounted && agentPanel && hasDetails"
            class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card"
            data-testid="timeline-tool-agent-panel"
          >
            <!-- create: prompt only -->
            <template v-if="agentPanel.action === 'create' && agentPanel.prompt">
              <pre class="px-3 py-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ agentPanel.prompt }}</pre>
            </template>
            <!-- list: agent rows -->
            <template v-else-if="agentPanel.action === 'list' && agentPanel.agents?.length">
              <div
                v-for="agent in agentPanel.agents"
                :key="agent.taskId"
                class="flex items-center gap-2 border-b border-border/50 px-3 py-2 first:border-t-0"
              >
                <component :is="tooltipComponent">
                  <component :is="tooltipTriggerComponent" as-child>
                    <span class="inline-flex shrink-0 items-center">
                      <component
                        :is="providerIconComponent"
                        :provider-id="agent.providerId"
                        :model-id="agent.modelId"
                        :external-agent-type="agent.externalAgentType"
                        :size="14"
                      />
                    </span>
                  </component>
                  <component :is="tooltipContentComponent" v-if="getAgentModelLabel(agent)">
                    {{ getAgentModelLabel(agent) }}
                  </component>
                </component>
                <span class="text-xs font-medium text-foreground/90">{{ agent.agentName }}</span>
                <span class="min-w-0 truncate text-xs text-muted-foreground/70">{{ agent.taskTitle }}</span>
                <span class="ml-auto shrink-0 text-xs text-muted-foreground/60">{{ agent.status }}</span>
              </div>
            </template>
            <!-- query_provider_model: model rows -->
            <template v-else-if="agentPanel.action === 'query_provider_model' && agentPanel.models?.length">
              <div
                v-for="model in agentPanel.models"
                :key="`${model.providerId}/${model.modelId}`"
                class="flex items-center gap-2 border-b border-border/50 px-3 py-2 first:border-t-0"
              >
                <component
                  :is="providerIconComponent"
                  :provider-id="model.providerId"
                  :model-id="model.modelId"
                  :size="14"
                />
                <span class="min-w-0 truncate font-mono text-xs text-foreground/80">{{ model.modelId }}</span>
              </div>
            </template>
            <!-- query_external_model: external agent model list -->
            <template v-else-if="agentPanel.action === 'query_external_model' && agentPanel.externalModels">
              <div class="px-3 py-2">
                <div class="mb-1 flex items-center gap-2">
                  <component
                    :is="providerIconComponent"
                    :external-agent-type="agentPanel.externalModels.agentType"
                    :size="14"
                  />
                  <span class="text-xs font-medium text-foreground/90">{{ agentPanel.externalModels.agentType }}</span>
                </div>
                <div
                  v-for="model in agentPanel.externalModels.models"
                  :key="model"
                  class="flex items-center gap-2 py-0.5"
                >
                  <span class="font-mono text-xs text-foreground/80">{{ model }}</span>
                  <span v-if="model === agentPanel.externalModels.defaultModel" class="text-xs text-muted-foreground/60">(default)</span>
                </div>
                <div v-if="agentPanel.externalModels.defaultModel && !agentPanel.externalModels.models.includes(agentPanel.externalModels.defaultModel)" class="flex items-center gap-2 py-0.5">
                  <span class="font-mono text-xs text-foreground/80">{{ agentPanel.externalModels.defaultModel }}</span>
                  <span class="text-xs text-muted-foreground/60">(default)</span>
                </div>
              </div>
            </template>
          </div>
          <!-- browser panel -->
          <div
            v-else-if="isBrowser && panelMounted && browserPanel"
            class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card"
            data-testid="timeline-tool-browser-panel"
          >
            <template v-if="browserPanel.inputLines.length > 0">
              <div class="sticky top-0 z-10 bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground">输入</div>
              <pre class="px-3 pb-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ browserPanel.inputLines.join('\n') }}</pre>
            </template>
            <div v-if="browserPanel.inputLines.length > 0 && browserPanel.output" class="border-t border-border"></div>
            <template v-if="browserPanel.output">
              <div class="sticky top-0 z-10 bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground">输出</div>
              <pre class="px-3 pb-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ browserPanel.output }}</pre>
            </template>
          </div>
          <!-- cua panel -->
          <component
            :is="mediaPanelComponent"
            v-else-if="isCua && panelMounted && cuaPanel"
            :block="block"
          />
          <div
            v-else-if="useRawPanel && panelMounted"
            class="mt-1 max-h-[240px] overflow-auto rounded-md border border-border bg-card"
            data-testid="timeline-tool-raw-panel"
          >
            <template v-if="rawInput">
              <div class="sticky top-0 z-10 bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground">输入</div>
              <pre class="px-3 pb-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ rawInput }}</pre>
            </template>
            <div v-if="rawInput && rawOutput" class="border-t border-border"></div>
            <template v-if="rawOutput">
              <div class="sticky top-0 z-10 bg-card px-3 pt-2 pb-1 text-[11px] text-muted-foreground">输出</div>
              <pre class="px-3 pb-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words">{{ rawOutput }}</pre>
            </template>
          </div>
          <div
            v-if="panelMounted && truncatedToolResult"
            class="mt-1 border-t border-border px-3 py-2"
            data-testid="timeline-tool-truncated"
          >
            <div class="flex items-center gap-2 text-[11px] text-muted-foreground">
              <span class="min-w-0 flex-1">…</span>
              <component
                :is="buttonComponent"
                variant="ghost"
                size="sm"
                class="h-6 shrink-0 px-2"
                :disabled="fullOutputLoading"
                data-testid="timeline-tool-toggle-full-output"
                @click.stop="toggleFullOutput"
              >
                {{ fullOutputText === null ? labels.expandMessage : labels.collapse }}
              </component>
              <span v-if="fullOutputLoading">{{ labels.loading }}</span>
              <component
                :is="buttonComponent"
                v-else-if="fullOutputError"
                variant="ghost"
                size="sm"
                class="h-6 shrink-0 px-2"
                data-testid="timeline-tool-retry-full-output"
                @click.stop="toggleFullOutput"
              >{{ labels.retry }}</component>
            </div>
            <pre
              v-if="fullOutputText"
              class="mt-2 max-h-[360px] overflow-auto rounded-md border border-border bg-card px-3 py-2 font-mono text-xs leading-5 text-foreground/80 whitespace-pre-wrap break-words"
              data-testid="timeline-tool-full-output"
            >{{ fullOutputText }}</pre>
          </div>

        </div>
      </div>
    </div>

  </div>
</template>

<style scoped>
.shimmer-text {
  background: linear-gradient(
    90deg,
    currentColor 0%,
    currentColor 40%,
    var(--activity-shimmer-highlight) 50%,
    currentColor 60%,
    currentColor 100%
  );
  background-size: 200% 100%;
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: shimmer 2s steps(48, end) infinite;
}

@keyframes shimmer {
  0% { background-position: 100% 0; }
  100% { background-position: -100% 0; }
}

.collapse-grid {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows 300ms cubic-bezier(0.16, 1, 0.3, 1);
}

.collapse-grid-open {
  grid-template-rows: 1fr;
}

.collapse-grid-inner {
  overflow: hidden;
  min-height: 0;
  opacity: 0;
  transition: opacity 300ms ease-in;
}

.collapse-grid-open .collapse-grid-inner {
  opacity: 1;
}
</style>
