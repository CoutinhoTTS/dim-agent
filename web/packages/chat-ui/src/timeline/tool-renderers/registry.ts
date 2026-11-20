import type { ChatUiToolRenderBlock, ToolCallRenderer, TimelineToolTrace } from './types'
import { inputPreviewLines } from './utils'

/**
 * Registry of ToolCallRenderer instances.
 * Renderers are checked in registration order; the first match wins.
 * The last registered renderer should be the generic fallback.
 */
export class ToolCallRendererRegistry {
  private renderers: ToolCallRenderer[] = []

  register(renderer: ToolCallRenderer): void {
    this.renderers.push(renderer)
  }

  find(
    toolName: string,
    params: Record<string, unknown> | null,
    block?: ChatUiToolRenderBlock,
  ): ToolCallRenderer {
    for (const renderer of this.renderers) {
      if (renderer.match(toolName, params, block)) {
        return renderer
      }
    }
    throw new Error(
      `No renderer found for tool "${toolName}". Ensure a generic fallback renderer is registered last.`,
    )
  }

  buildTrace(block: ChatUiToolRenderBlock): TimelineToolTrace {
    const toolName = block.toolName ?? ''
    const params = block.toolParams ?? null
    const renderer = this.find(toolName, params, block)
    const labelKey = renderer.getLabelKey(toolName, params)
    const labelText = renderer.getLabelText?.(block) ?? undefined
    const detailSections = renderer.getDetailSections(block)

    return {
      rawToolName: toolName,
      toolLabelKey: labelKey,
      ...(labelText === undefined ? {} : { toolLabelText: labelText }),
      primaryParameter: renderer.getPrimaryParameter(block),
      detailLines: detailSections.flatMap(section => section.lines),
      detailSections,
      rendererId: renderer.id,
      rendererKind: renderer.id === 'generic' ? 'generic' : 'known',
    }
  }

  buildInputPreview(toolName: string, params: Record<string, unknown> | null): string[] {
    const renderer = this.find(toolName, params)
    const preview = renderer.getInputPreview?.(params)
    // Fall back to the generic preview when a renderer's curated keys miss the params.
    return preview && preview.length > 0 ? preview : inputPreviewLines(params)
  }
}
