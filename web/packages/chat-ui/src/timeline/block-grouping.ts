import type { ChatUiTimelineBlock, ChatUiTimelineBlockKind } from '../model/timeline'

export type ChatUiTimelineBlockGroup<TBlock> =
  | { kind: 'single'; block: TBlock }
  | { kind: 'group'; blocks: TBlock[]; groupId: string }

/**
 * Minimal block view the grouping predicate needs. Full timeline blocks and
 * the lightweight turn-structure block shapes both satisfy this contract, so
 * render grouping and expansion-key computation share one implementation.
 */
export interface ChatUiTimelineWorkBlockShape {
  id: string
  kind: ChatUiTimelineBlockKind
  toolSourceKind?: 'mcp'
  displayMode?: 'inline' | 'fullscreen'
  /** Empty running thinking placeholder; see `isEmptyTimelineThinking`. */
  isEmptyThinking?: boolean
}

const WORK_BLOCK_KINDS = new Set<ChatUiTimelineBlockKind>(['thinking', 'tool'])

/**
 * Thinking blocks may start as an empty placeholder while the provider has
 * not streamed any reasoning yet. They stay visible, but they carry no work
 * for grouping. Render grouping and lightweight expansion-key structures
 * classify the placeholder the same way.
 */
export function isEmptyTimelineThinking(
  block: Pick<ChatUiTimelineBlock, 'kind' | 'status' | 'content'>,
): boolean {
  return block.kind === 'thinking'
    && block.status === 'running'
    && block.content.trim().length === 0
}

/**
 * A work block (thinking/tool) is groupable unless it is an explicit inline
 * MCP App: inline apps keep their own visible 240px surface and stay single
 * group boundaries. Fullscreen MCP tools (displayMode missing or "fullscreen")
 * are ordinary work blocks.
 */
export function isGroupableWorkBlock(block: ChatUiTimelineWorkBlockShape): boolean {
  if (block.isEmptyThinking === true) {
    return false
  }
  if (!WORK_BLOCK_KINDS.has(block.kind)) {
    return false
  }
  if (block.toolSourceKind === 'mcp') {
    return block.displayMode !== 'inline'
  }
  return true
}

export interface ChatUiTimelineWorkGroupRun {
  groupId: string
  start: number
  end: number
}

/**
 * Scans groupable work blocks and returns the runs that contain at least
 * `minWorkBlocks` work blocks. Notification blocks inside a run are absorbed
 * into the group and hide with it. Text and other independent visual content
 * (status / generated-image / plan-draft) terminate a run and stay single.
 *
 * Every caller keeps the default `minWorkBlocks = 2` (a lone work block renders
 * as a single), and the grouping result stays a pure function of block data —
 * no isRunning-dependent group↔single reshape.
 *
 * `blockAt` keeps the scan allocation-free for both full block arrays and the
 * lightweight turn-structure arrays.
 */
export function scanTimelineWorkGroupRuns(
  length: number,
  blockAt: (index: number) => ChatUiTimelineWorkBlockShape,
  minWorkBlocks = 2,
): ChatUiTimelineWorkGroupRun[] {
  const runs: ChatUiTimelineWorkGroupRun[] = []
  let index = 0

  while (index < length) {
    if (!isGroupableWorkBlock(blockAt(index))) {
      index += 1
      continue
    }
    let end = index + 1
    let workCount = 1
    while (end < length) {
      const candidate = blockAt(end)
      // An empty thinking placeholder is transparent to grouping: it neither
      // counts as a work block nor terminates the run. It stays in the run
      // slice so it can render in place while it is still empty.
      if (candidate.kind === 'thinking' && candidate.isEmptyThinking === true) {
        end += 1
        continue
      }
      if (isGroupableWorkBlock(candidate)) {
        workCount += 1
        end += 1
        continue
      }
      if (candidate.kind === 'notification') {
        end += 1
        continue
      }
      break
    }
    if (workCount >= minWorkBlocks) {
      runs.push({
        groupId: `group:${blockAt(index).id}`,
        start: index,
        end,
      })
    }
    index = end
  }

  return runs
}

export function groupConsecutiveBlocks<TBlock>(
  blocks: TBlock[],
  options: {
    minWorkBlocks?: number
    shape: (block: TBlock) => ChatUiTimelineWorkBlockShape
  },
): ChatUiTimelineBlockGroup<TBlock>[] {
  const minWorkBlocks = options.minWorkBlocks ?? 2
  const groups: ChatUiTimelineBlockGroup<TBlock>[] = []
  const runs = scanTimelineWorkGroupRuns(blocks.length, index => options.shape(blocks[index]!), minWorkBlocks)
  let lastEnd = 0
  for (const run of runs) {
    for (let index = lastEnd; index < run.start; index += 1) {
      groups.push({ kind: 'single', block: blocks[index]! })
    }
    groups.push({
      kind: 'group',
      blocks: blocks.slice(run.start, run.end),
      groupId: run.groupId,
    })
    lastEnd = run.end
  }
  for (let index = lastEnd; index < blocks.length; index += 1) {
    groups.push({ kind: 'single', block: blocks[index]! })
  }
  return groups
}
