import type { Component } from 'vue'

export type ChatUiToolCallInlineStatus = 'queued' | 'running' | 'complete' | 'error'

export interface ChatUiToolCallInlineAgentInfo {
  agentName: string
  providerId?: string
  modelId?: string
  externalAgentType?: string
}

/**
 * Host-resolved display facts for a tool call summary line. The host keeps
 * label/i18n lookup and primary-parameter projection; the component only lays
 * the resolved text out.
 */
export interface ChatUiToolCallInlineTrace {
  rawToolName: string
  rendererId: string
  labelText: string
  parameterText: string
  parameterTitle?: string | null
}

export interface ChatUiToolCallInlineLabels {
  queued: string
  /** Label the host uses for a glob listing, so the icon can stay `files`. */
  listFiles: string
}

export type ChatUiToolCallInlineIconKey =
  | 'read'
  | 'search'
  | 'files'
  | 'file-search'
  | 'file-write'
  | 'file-edit'
  | 'terminal'
  | 'web'
  | 'browser'
  | 'cua'
  | 'ask-user'
  | 'todos'
  | 'skill'
  | 'wait'
  | 'agent'
  | 'mcp'
  | 'generic'

export interface ChatUiToolCallInlineIcon {
  component: Component
  props?: Record<string, unknown>
}

export type ChatUiToolCallInlineIcons = Partial<Record<ChatUiToolCallInlineIconKey, ChatUiToolCallInlineIcon>>

export function resolveToolCallInlineIconKey(
  trace: Pick<ChatUiToolCallInlineTrace, 'rendererId' | 'rawToolName'>,
  isGlobListing: boolean,
): ChatUiToolCallInlineIconKey | null {
  if (trace.rendererId === 'command') return 'terminal'
  if (trace.rendererId === 'search') {
    if (isGlobListing) return 'files'
    if (trace.rawToolName === 'grep' || trace.rawToolName === 'rg') return 'search'
    return 'file-search'
  }
  if (trace.rendererId === 'file') {
    if (trace.rawToolName === 'write' || trace.rawToolName === 'write_file') return 'file-write'
    if (trace.rawToolName === 'edit' || trace.rawToolName === 'edit_file' || trace.rawToolName === 'apply_patch') return 'file-edit'
    return 'read'
  }
  if (trace.rendererId === 'web') return 'web'
  if (trace.rendererId === 'browser') return 'browser'
  if (trace.rendererId === 'cua') return 'cua'
  if (trace.rendererId === 'ask-user') return 'ask-user'
  if (trace.rendererId === 'todos') return 'todos'
  if (trace.rendererId === 'skill') return 'skill'
  if (trace.rendererId === 'wait') return 'wait'
  if (trace.rendererId === 'agent') return 'agent'
  if (trace.rendererId === 'mcp') return 'mcp'
  if (trace.rendererId === 'generic') return 'generic'
  return null
}
