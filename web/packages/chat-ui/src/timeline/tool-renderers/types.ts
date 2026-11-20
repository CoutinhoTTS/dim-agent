import type { ChatUiToolLabelKey, TranslationParams } from './keys'

export type { ChatUiToolLabelKey, ChatUiToolTranslate, TranslationParams } from './keys'

/**
 * Minimal structural view of a tool block. Desktop `TimelineBlock` and the
 * package `ChatUiTimelineBlock` are both assignable to it.
 */
export interface ChatUiToolRenderBlock {
  id?: string
  kind?: string
  status?: string
  content?: string
  toolName?: string
  toolParams?: Record<string, unknown> | null
  toolResult?: unknown
  toolSource?: {
    kind?: string
    mcpServerId?: string
    mcpToolName?: string
  } | null
  toolTarget?: string | null
  backgroundTaskNames?: Record<string, string>
  inputText?: string
  toolInputText?: string
}

export type TimelineToolInlineParameter =
  | {
      kind: 'text'
      text: string
      title?: string
    }
  | {
      kind: 'i18n'
      key: ChatUiToolLabelKey
      params?: TranslationParams
      title?: string
    }

export interface TimelineToolTodoItem {
  id: string
  content: string
  status: TimelineToolTodoStatus
  priority: TimelineToolTodoPriority
}

export type TimelineToolTodoStatus = 'pending' | 'in_progress' | 'completed'
export type TimelineToolTodoPriority = 'high' | 'medium' | 'low'

export interface TimelineToolDetailSection {
  id: string
  kind: 'summary' | 'preview'
  lines: string[]
  truncated: boolean
  todos?: TimelineToolTodoItem[]
}

export interface TimelineToolTrace {
  rawToolName: string
  toolLabelKey: ChatUiToolLabelKey
  toolLabelText?: string
  primaryParameter: TimelineToolInlineParameter | null
  detailLines: string[]
  detailSections: TimelineToolDetailSection[]
  rendererId: string
  rendererKind: 'known' | 'generic'
}

/**
 * A ToolCallRenderer handles the display of one or more tool types
 * in the timeline. Each renderer is self-describing: it decides whether
 * it matches a given tool, and provides the label, primary parameter,
 * and detail sections for rendering.
 */
export interface ToolCallRenderer {
  /** Unique renderer identifier */
  readonly id: string
  /** Whether this renderer handles the given tool */
  match(toolName: string, params: Record<string, unknown> | null, block?: ChatUiToolRenderBlock): boolean
  /** Semantic label key for the tool summary line */
  getLabelKey(toolName: string, params: Record<string, unknown> | null): ChatUiToolLabelKey
  /** Optional dynamic label text. Use for runtime identities such as MCP server/tool names. */
  getLabelText?(block: ChatUiToolRenderBlock): string | null
  /** Primary parameter for the collapsed inline summary (null = no parameter) */
  getPrimaryParameter(block: ChatUiToolRenderBlock): TimelineToolInlineParameter | null
  /** Detail sections shown when expanded */
  getDetailSections(block: ChatUiToolRenderBlock): TimelineToolDetailSection[]
  /** Whether this tool has expandable details (false = no chevron) */
  hasDetails(block: ChatUiToolRenderBlock): boolean
  /**
   * Readable input preview from params only, for the permission prompt.
   * No running/result dependency — the tool has not executed yet.
   * Optional: the registry falls back to a generic key/value preview when absent.
   */
  getInputPreview?(params: Record<string, unknown> | null): string[]
}
