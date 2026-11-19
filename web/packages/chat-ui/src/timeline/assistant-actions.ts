import type { ChatUiTimelineBlock } from '../model'

export interface ChatUiTimelineCopyLabels {
  runFailedPlaceholder: string
  userInterrupt: string
}

export function hasTimelineAssistantText(content: string): boolean {
  return /\S/.test(content)
}

export function timelineAssistantBlockCopyText(
  block: ChatUiTimelineBlock,
  labels: ChatUiTimelineCopyLabels,
): string | undefined {
  if (block.kind === 'text' || block.kind === 'generated-image' || block.kind === 'plan-draft') {
    return block.content
  }
  if (block.kind === 'status') {
    const content = block.content.trim()
    if (content) {
      return content
    }
    if (block.statusType === 'error') {
      return labels.runFailedPlaceholder
    }
    if (block.statusType === 'cancelled') {
      return labels.userInterrupt
    }
  }
  return undefined
}

export function hasTimelineAssistantCopyPayload(
  blocks: readonly ChatUiTimelineBlock[],
  labels: ChatUiTimelineCopyLabels,
): boolean {
  return blocks.some((block) => {
    const content = timelineAssistantBlockCopyText(block, labels)
    return content !== undefined && hasTimelineAssistantText(content)
  })
}

export function buildTimelineAssistantCopyPayload(
  blocks: readonly ChatUiTimelineBlock[],
  labels: ChatUiTimelineCopyLabels,
): string {
  const parts: string[] = []
  for (const block of blocks) {
    const content = timelineAssistantBlockCopyText(block, labels)
    if (content !== undefined) {
      parts.push(content)
    }
  }
  return parts.join('\n\n')
}
