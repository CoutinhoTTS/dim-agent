import type { Component } from 'vue'
import type { ChatUiTimelineSearchMatch } from '../timeline/search'
import type { ChatUiTimelineUserMessageMarker } from '../timeline/user-message-rail'

/**
 * Minimal row shapes the shared plain-timeline surface reads. Hosts keep their
 * richer row projections; rows are dispatched back to the injected row
 * component, so anything beyond identity and turn state is pass-through.
 */
export interface ChatUiPlainTimelineTurnRow {
  kind: 'turn'
  key: string
  turn: {
    id: string
    isRunning: boolean
    userMessage: { id: string } | null
  }
  runningStartedAt?: Date | null
}

export interface ChatUiPlainTimelineMessageRow {
  kind: 'message'
  key: string
}

export interface ChatUiPlainTimelineDisplayRow {
  kind: 'display'
  key: string
}

export interface ChatUiPlainTimelineRunningRow {
  kind: 'running'
  key: string
}

export type ChatUiPlainTimelineRow =
  | ChatUiPlainTimelineTurnRow
  | ChatUiPlainTimelineMessageRow
  | ChatUiPlainTimelineDisplayRow
  | ChatUiPlainTimelineRunningRow

/**
 * Rendered window around the scroll viewport. The host owns offset estimates,
 * realization, and incremental-window state; the surface only projects the
 * already-selected slice and its aggregate spacers.
 */
export interface ChatUiPlainTimelineSurfaceWindow {
  start: number
  end: number
  rows: readonly ChatUiPlainTimelineRow[]
  topSpacerPx: number
  bottomSpacerPx: number
  contentVisibilityEnabled: (row: ChatUiPlainTimelineRow) => boolean
  contentVisibilityStyle: (row: ChatUiPlainTimelineRow) => Record<string, string> | undefined
}

export interface ChatUiPlainTimelineTurnUiState {
  detailsExpanded: boolean
  detailsToggleVisible: boolean
}

/**
 * Per-row presentation state. The host resolves expansion, marker, and retry
 * inputs; the surface maps them onto the injected row component.
 */
export interface ChatUiPlainTimelineSurfaceRowState {
  latestTurnRowId: string | null
  turnUiStateById: Readonly<Record<string, ChatUiPlainTimelineTurnUiState>>
  blockExpandedById: Readonly<Record<string, boolean>>
  blockGroupExpandedById: Readonly<Record<string, boolean>>
  assistantUserMessageIdByRowKey: Readonly<Record<string, string | null>>
  userMessageById: ReadonlyMap<string, unknown>
  searchQuery: string
  activeSearchMatchId: string | null
  searchMatchesBySource: Readonly<Record<string, readonly ChatUiTimelineSearchMatch[]>>
  elapsedNowMs: number | undefined
  presentationActive: boolean
  readOnly: boolean
}

export interface ChatUiPlainTimelinePrependAnchorDiagnostic {
  outcome: string
  rowKey?: string
  capturedTopWithinViewportPx?: number
  restoredTopWithinViewportPx?: number
  durationMs: number
}

/**
 * Debug/introspection attributes on the timeline root. Hosts own these
 * measurements and trim diagnostics; the surface only renders them.
 */
export interface ChatUiPlainTimelineSurfaceDiagnostics {
  prependAnchor: ChatUiPlainTimelinePrependAnchorDiagnostic | null
  oldestRawMessageId: string | undefined
  trimState: string
  trimAnchorKey: string
  trimAtBottom: boolean
  trimRestoreState: string
  trimCapturedTop: number | null
  trimRestoredTop: number | null
  renderWindowStart: number
  renderWindowEnd: number
}

/**
 * Viewport-level view state. Nothing here is derived from scroll math by the
 * surface: the host decides empty/loading/restore/gap visibility.
 */
export interface ChatUiPlainTimelineSurfaceState {
  sessionId: string | null
  messageCount: number
  rowCount: number
  isRunning: boolean
  isLoadingOlder: boolean
  restoreCovered: boolean
  restoreCoverVisible: boolean
  restoreTransitionSuppressed: boolean
  sendLandingSettling: boolean
  layoutResizing: boolean
  canLoadOlder: boolean
  hasNewerGap: boolean
  showUserMessageRail: boolean
  userMessageRailHost: HTMLElement | null
  showScrollToBottom: boolean
}

export interface ChatUiPlainTimelineSurfaceMarkers {
  markers: readonly ChatUiTimelineUserMessageMarker[]
  loadedMarkers: readonly ChatUiTimelineUserMessageMarker[]
  rowDomSignature: string
}

export interface ChatUiPlainTimelineSurfaceActivity {
  visible: boolean
  text: string
}

export interface ChatUiPlainTimelineSurfaceLabels {
  emptySession: string
  loading: string
  newerHistoryGap: string
  returnToLatest: string
}

/**
 * Host-owned renderers. The surface owns layout and dispatch; the desktop
 * adapter keeps the row wrapper, its error boundary, the user-message rail,
 * icons, and the restore-cover brand mark.
 */
export interface ChatUiPlainTimelineSurfaceComponents {
  row: Component
  errorBoundary: Component
  userMessageRail: Component
  brandMark: Component
  chevronDown: Component
  chevronUp: Component
}

export interface ChatUiPlainTimelineSurfaceProps {
  state: ChatUiPlainTimelineSurfaceState
  window: ChatUiPlainTimelineSurfaceWindow
  rowState: ChatUiPlainTimelineSurfaceRowState
  diagnostics: ChatUiPlainTimelineSurfaceDiagnostics
  markers: ChatUiPlainTimelineSurfaceMarkers
  activity: ChatUiPlainTimelineSurfaceActivity
  labels: ChatUiPlainTimelineSurfaceLabels
  components: ChatUiPlainTimelineSurfaceComponents
}

/**
 * Message actions are forwarded verbatim. The surface never reads the message
 * body, so the host keeps its concrete message type and narrows at the emit
 * boundary.
 */
export interface ChatUiPlainTimelineMessageActionPayload {
  message: unknown
  content: string
  onSubmitted?: () => void
}

export interface ChatUiPlainTimelineSurfaceEmits {
  'load-latest': []
  'jump-to-bottom': []
  'marker-click': [marker: ChatUiTimelineUserMessageMarker]
  'rail-wheel': [event: WheelEvent]
  'toggle-details': [turnId: string]
  'toggle-block': [blockId: string]
  'toggle-block-group': [groupId: string]
  'edit-message': [payload: ChatUiPlainTimelineMessageActionPayload]
  'retry-message': [payload: ChatUiPlainTimelineMessageActionPayload]
  'fork-message': [message: unknown]
  'branch-message': [message: { id: string; role: 'user' }]
  'preview-file': [target: { path: string; startLine?: number }]
  'commit': [paths: string[]]
  'commit-and-push': [paths: string[]]
  'push': []
  'diff-file': [path: string, messageId: string]
  'open-agent': [target: { subagentSessionId: string; title: string }]
}
