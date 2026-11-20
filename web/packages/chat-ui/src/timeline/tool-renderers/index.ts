export type {
  ChatUiToolLabelKey,
  ChatUiToolTranslate,
  ChatUiToolRenderBlock,
  ToolCallRenderer,
  TimelineToolInlineParameter,
  TimelineToolDetailSection,
  TimelineToolTodoItem,
  TimelineToolTrace,
  TimelineToolTodoPriority,
  TimelineToolTodoStatus,
  TranslationParams,
} from './types'
export { ToolCallRendererRegistry } from './registry'
export * from './utils'
export * from './edit-replacements'
export * from './ask-user-answers'
export * from './file-renderer'
export * from './command-renderer'
export * from './search-renderer'
export * from './web-renderer'
export * from './todos-renderer'
export * from './ask-user-renderer'
export * from './agent-renderer'
export * from './wait-renderer'
export * from './skill-renderer'
export * from './browser-renderer'
export * from './cua-renderer'
export * from './mcp-renderer'
export * from './generic-renderer'

import { ToolCallRendererRegistry } from './registry'
import { fileRenderer } from './file-renderer'
import { commandRenderer } from './command-renderer'
import { searchRenderer } from './search-renderer'
import { webRenderer } from './web-renderer'
import { todosRenderer } from './todos-renderer'
import { askUserRenderer } from './ask-user-renderer'
import { agentRenderer } from './agent-renderer'
import { waitRenderer } from './wait-renderer'
import { skillRenderer } from './skill-renderer'
import { browserRenderer } from './browser-renderer'
import { cuaRenderer } from './cua-renderer'
import { mcpRenderer } from './mcp-renderer'
import { genericRenderer } from './generic-renderer'

/**
 * Create and populate the toolcall renderer registry.
 * Order matters: specific renderers first, generic fallback last.
 */
export function createToolCallRegistry(): ToolCallRendererRegistry {
  const registry = new ToolCallRendererRegistry()
  registry.register(mcpRenderer)
  registry.register(todosRenderer)
  registry.register(askUserRenderer)
  registry.register(agentRenderer)
  registry.register(waitRenderer)
  registry.register(fileRenderer)
  registry.register(commandRenderer)
  registry.register(searchRenderer)
  registry.register(webRenderer)
  registry.register(skillRenderer)
  registry.register(browserRenderer)
  registry.register(cuaRenderer)
  registry.register(genericRenderer) // must be last
  return registry
}
