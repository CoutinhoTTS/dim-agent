import type { ChatUiToolLabelKey, TranslationParams } from './tool-renderers/keys'
import type {
  ChatUiToolRenderBlock,
  TimelineToolInlineParameter,
  TimelineToolTrace,
} from './tool-renderers/types'
import { createToolCallRegistry } from './tool-renderers'
import { ToolCallRendererRegistry } from './tool-renderers/registry'
import { asRecord } from './tool-renderers/utils'

const registry = createToolCallRegistry()

export const toolCallRegistry: ToolCallRendererRegistry = registry

export function resolveInlineParameterText(
  parameter: TimelineToolInlineParameter | null,
  t: (key: ChatUiToolLabelKey, params?: TranslationParams) => string,
): string {
  if (!parameter) return ''
  return parameter.kind === 'text'
    ? parameter.text
    : t(parameter.key, parameter.params)
}

export function buildTimelineToolTrace(block: ChatUiToolRenderBlock): TimelineToolTrace {
  return registry.buildTrace(block)
}

/**
 * Params-only readable input preview for the permission prompt. Routes to the
 * matching renderer's getInputPreview, falling back to a generic key/value preview.
 */
export function buildToolInputPreview(
  toolName: string | undefined,
  rawInput: unknown,
): string[] {
  return registry.buildInputPreview(toolName ?? '', asRecord(rawInput))
}

/** Semantic label key for a tool call, routed through the renderer registry. */
export function buildToolCallLabelKey(
  toolName: string | undefined,
  rawInput: unknown,
): ChatUiToolLabelKey {
  const params = asRecord(rawInput)
  return registry.find(toolName ?? '', params).getLabelKey(toolName ?? '', params)
}
