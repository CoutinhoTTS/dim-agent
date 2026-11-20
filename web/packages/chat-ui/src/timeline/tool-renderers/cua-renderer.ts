import type { ToolCallRenderer, TimelineToolDetailSection, TimelineToolInlineParameter } from './types'
import type { ChatUiToolLabelKey } from './keys'
import {
  normalizeToolName,
  previewLinesFromText,
  readParamsRecord,
  readString,
  readStructuredRecord,
  readablePairLines,
  resultText,
  summarySection,
  truncateToolPreviewText,
  INPUT_PREVIEW_LINE_MAX,
} from './utils'

const CUA_TOOL_KEYS: Record<string, ChatUiToolLabelKey> = {
  list_apps: 'toolCua.listApps',
  list_windows: 'toolCua.listWindows',
  get_accessibility_tree: 'toolCua.readDesktop',
  get_window_state: 'toolCua.readWindow',
  get_screen_size: 'toolCua.getScreenSize',
  get_cursor_position: 'toolCua.getCursorPosition',
  zoom: 'toolCua.zoomWindow',
  launch_app: 'toolCua.launchApp',
  kill_app: 'toolCua.killApp',
  bring_to_front: 'toolCua.bringToFront',
  click: 'toolCua.click',
  double_click: 'toolCua.doubleClick',
  right_click: 'toolCua.rightClick',
  drag: 'toolCua.drag',
  scroll: 'toolCua.scroll',
  type_text: 'toolCua.typeText',
  press_key: 'toolCua.pressKey',
  hotkey: 'toolCua.hotkey',
  set_value: 'toolCua.setValue',
  page: 'toolCua.page',
  move_cursor: 'toolCua.moveCursor',
  start_session: 'toolCua.startSession',
  end_session: 'toolCua.endSession',
  set_agent_cursor_enabled: 'toolCua.setAgentCursorEnabled',
  set_agent_cursor_motion: 'toolCua.setAgentCursorMotion',
  set_agent_cursor_style: 'toolCua.setAgentCursorStyle',
  get_agent_cursor_state: 'toolCua.getAgentCursorState',
  check_permissions: 'toolCua.checkPermissions',
  health_report: 'toolCua.healthReport',
  get_config: 'toolCua.getConfig',
  set_config: 'toolCua.setConfig',
  check_for_update: 'toolCua.checkForUpdate',
  start_recording: 'toolCua.startRecording',
  stop_recording: 'toolCua.stopRecording',
  get_recording_state: 'toolCua.getRecordingState',
  replay_trajectory: 'toolCua.replayTrajectory',
  install_ffmpeg: 'toolCua.installFfmpeg',
}

const CUA_PAGE_ACTION_KEYS: Record<string, ChatUiToolLabelKey> = {
  get_text: 'toolCua.pageGetText',
  query_dom: 'toolCua.pageQueryDom',
  click_element: 'toolCua.pageClickElement',
  execute_javascript: 'toolCua.pageExecuteJavascript',
  enable_javascript_apple_events: 'toolCua.pageEnableJavascriptAppleEvents',
}

const readScalar = (
  record: Record<string, unknown> | null | undefined,
  keys: readonly string[],
): string | null => {
  if (!record) return null
  for (const key of keys) {
    const value = record[key]
    if ((typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') && String(value).trim()) {
      return String(value).trim()
    }
  }
  return null
}

const elementTarget = (params: Record<string, unknown> | null): string | null => {
  const token = readScalar(params, ['element_token'])
  if (token) return truncateToolPreviewText(token, 24)
  const index = readScalar(params, ['element_index'])
  if (index) return `#${index}`
  const x = readScalar(params, ['x'])
  const y = readScalar(params, ['y'])
  return x && y ? `${x}, ${y}` : null
}

const regionTarget = (params: Record<string, unknown> | null): string | null => {
  const values = ['x1', 'y1', 'x2', 'y2'].map(key => readScalar(params, [key]))
  return values.every(Boolean) ? `${values[0]},${values[1]} -> ${values[2]},${values[3]}` : null
}

const countTarget = (record: Record<string, unknown> | null, keys: readonly string[]): string | null => {
  if (!record) return null
  for (const key of keys) {
    const value = record[key]
    if (Array.isArray(value)) return String(value.length)
    if (typeof value === 'number' && Number.isFinite(value)) return String(value)
  }
  return null
}

const buildTarget = (
  name: string,
  params: Record<string, unknown> | null,
  result: Record<string, unknown> | null,
): string | null => {
  if (name === 'page') {
    return readScalar(params, ['selector', 'css_selector', 'url'])
      ?? truncateToolPreviewText(readString(params, ['javascript']), 80)
  }
  if ([
    'click',
    'double_click',
    'right_click',
    'set_value',
    'type_text',
    'press_key',
    'scroll',
  ].includes(name)) {
    return elementTarget(params)
      ?? truncateToolPreviewText(
        readScalar(params, ['text', 'value', 'key', 'direction', 'amount']),
        80,
      )
  }
  if (name === 'drag') {
    const values = ['from_x', 'from_y', 'to_x', 'to_y'].map(key => readScalar(params, [key]))
    return values.every(Boolean) ? `${values[0]},${values[1]} -> ${values[2]},${values[3]}` : null
  }
  if (name === 'zoom') {
    return regionTarget(params) ?? readScalar(params, ['window_id', 'pid'])
  }
  if (name === 'get_window_state') {
    return [
      readScalar(result, ['title', 'app_name']) ?? readScalar(params, ['window_id', 'pid']),
      countTarget(result, ['elements']),
    ].filter(Boolean).join(' · ') || null
  }
  if (name === 'get_screen_size') {
    const width = readScalar(result, ['width'])
    const height = readScalar(result, ['height'])
    return width && height ? `${width} x ${height}` : null
  }
  if (name === 'get_cursor_position' || name === 'move_cursor') {
    const x = readScalar(result, ['x']) ?? readScalar(params, ['x'])
    const y = readScalar(result, ['y']) ?? readScalar(params, ['y'])
    return x && y ? `${x}, ${y}` : null
  }
  if (name === 'hotkey') {
    const keys = params?.keys ?? params?.modifier
    return Array.isArray(keys) ? keys.map(String).join('+') : null
  }
  return readScalar(params, ['name', 'bundle_id', 'title', 'app_name', 'window_id', 'pid', 'key', 'direction', 'amount', 'session', 'cursor_id', 'enabled', 'output_dir', 'dir', 'last_video_path', 'confirm'])
    ?? readScalar(result, ['overall', 'status', 'message', 'current_version', 'latest_version'])
}

const readActionKey = (name: string, params: Record<string, unknown> | null): ChatUiToolLabelKey => {
  if (name === 'page') {
    const action = readScalar(params, ['action'])?.toLowerCase()
    if (action && CUA_PAGE_ACTION_KEYS[action]) return CUA_PAGE_ACTION_KEYS[action]
  }
  return CUA_TOOL_KEYS[name] ?? 'toolDisplay.computerUse'
}

const summaryLines = (
  params: Record<string, unknown> | null,
  result: Record<string, unknown> | null,
): string[] => [
  ...readablePairLines(params, [
    'name', 'bundle_id', 'pid', 'window_id', 'element_token', 'element_index', 'x', 'y',
    'text', 'key', 'direction', 'amount', 'session', 'enabled', 'action',
    'selector', 'css_selector', 'output_dir', 'dir',
  ], 6),
  ...readablePairLines(result, ['overall', 'status', 'message', 'error', 'window_id', 'pid'], 4),
  ...(countTarget(result, ['elements']) ? [`elements: ${countTarget(result, ['elements'])}`] : []),
]

export const cuaRenderer: ToolCallRenderer = {
  id: 'cua',

  match(toolName) {
    const rawName = toolName.trim().toLowerCase()
    const name = normalizeToolName(toolName)
    return rawName.startsWith('cua-driver__') || CUA_TOOL_KEYS[name] !== undefined
  },

  getLabelKey(): ChatUiToolLabelKey {
    return 'toolDisplay.computerUse'
  },

  getPrimaryParameter(block): TimelineToolInlineParameter | null {
    const params = readParamsRecord(block)
    const result = readStructuredRecord(block.toolResult)
    const name = normalizeToolName(block.toolName)
    const target = buildTarget(name, params, result)
    return {
      kind: 'i18n',
      key: readActionKey(name, params),
      params: { target: target ? ` · ${target}` : '' },
    }
  },

  getDetailSections(block): TimelineToolDetailSection[] {
    const params = readParamsRecord(block)
    const result = readStructuredRecord(block.toolResult)
    const summary = summarySection('cua-summary', summaryLines(params, result))
    const preview = previewLinesFromText(resultText(block))
    return [summary, preview].filter((section): section is TimelineToolDetailSection => Boolean(section))
  },

  hasDetails(block): boolean {
    return this.getDetailSections(block).length > 0
  },

  getInputPreview(params): string[] {
    return readablePairLines(params, Object.keys(params ?? {}), INPUT_PREVIEW_LINE_MAX)
  },
}
