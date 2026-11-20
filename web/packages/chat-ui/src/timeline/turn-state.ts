import type { ChatUiTimelineBlockKind } from '../model/timeline'

export const TIMELINE_HISTORY_COLLAPSE_COMPLETED_TURN_THRESHOLD = 4

export function resolveTimelineTurnExpansionId(turn: Pick<{ id: string }, 'id'>): string {
  return turn.id
}

export function resolveTimelineBlockExpansionId(block: Pick<{ id: string }, 'id'>): string {
  return block.id
}

export interface TimelineTurnDetailsCollapseContext {
  turnIndex: number
  turnCount: number
  completedTurnCount: number
  latestCompletedTurnIndex: number
  hasRunningTurn: boolean
}

export function shouldEnableTimelineTurnDetailsCollapse(
  turn: Pick<{ isRunning: boolean }, 'isRunning'>,
  context?: TimelineTurnDetailsCollapseContext,
): boolean {
  return !turn.isRunning
    && context !== undefined
    && context.completedTurnCount > TIMELINE_HISTORY_COLLAPSE_COMPLETED_TURN_THRESHOLD
    && (context.hasRunningTurn || context.turnIndex < context.latestCompletedTurnIndex)
}

export function shouldExpandTimelineTurnDetails(
  turn: Pick<{ isRunning: boolean }, 'isRunning'>,
  context?: TimelineTurnDetailsCollapseContext,
): boolean {
  return turn.isRunning || !shouldEnableTimelineTurnDetailsCollapse(turn, context)
}

const NON_WORK_BLOCK_KINDS = new Set<ChatUiTimelineBlockKind>([
  'media',
  'retry',
  'text',
  'status',
  'generated-image',
  'plan-draft',
])

export function isTimelineWorkBlock(block: Pick<{ kind: ChatUiTimelineBlockKind }, 'kind'>): boolean {
  return !NON_WORK_BLOCK_KINDS.has(block.kind)
}

export function shouldRenderTimelineTurnWorkDetails(
  turn: { blocks: readonly { kind: ChatUiTimelineBlockKind }[] },
): boolean {
  return turn.blocks.some(isTimelineWorkBlock)
}

type TimedBlock = { startedAt?: Date | null; endedAt?: Date | null }

export function calculateTimelineWorkDuration<TBlock extends TimedBlock & { kind: ChatUiTimelineBlockKind }>(
  blocks: readonly TBlock[],
): number | null {
  const workBlocks = blocks.filter(isTimelineWorkBlock)
  const timedWorkBlocks = workBlocks.filter(
    (block): block is TBlock & { startedAt: Date; endedAt: Date } =>
      block.startedAt !== null
      && block.startedAt !== undefined
      && block.endedAt !== null
      && block.endedAt !== undefined,
  )
  if (workBlocks.length === 0 || timedWorkBlocks.length !== workBlocks.length) {
    return null
  }

  const startedAt = Math.min(...timedWorkBlocks.map(block => block.startedAt.getTime()))
  const endedAt = Math.max(...timedWorkBlocks.map(block => block.endedAt.getTime()))
  return Math.max(0, endedAt - startedAt)
}
