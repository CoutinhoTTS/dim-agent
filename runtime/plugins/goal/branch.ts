import { definePlugin } from '@arcships/rutis'
import type { SettingsResult } from '../../../npm/plugin.d.ts'
import { type AgentEvent, type InputMessage, Fold, OBJECTIVE_KEY, STATUS_KEY, decideInterrupt, decideTurnEnd, marker, writes } from './goal.ts'

type Dispatcher = {
  listen(name: string, keys: string[], handler: (event: AgentEvent) => void | Promise<void>): Promise<void>
  committed(): Promise<number | bigint>
  local(start: number | bigint, through: number | bigint): Promise<AgentEvent[]>
  history(through: number | bigint): Promise<AgentEvent[]>
}
type TreeCommands = { input(id: string, input: InputMessage): Promise<AgentEvent> }
type Settings = {
  read(request: { source: string; scope: 'user' }): Promise<SettingsResult<{ document: { maxContinuations: number } }>>
}

const KEYS = ['branch/start', 'input/message', 'input/consumed', 'input/deleted', 'turn/start', 'turn/end', 'stop/interrupt']

// GoalBranchPlugin (Goal.md §1, A.6): owns the branch's fold, decides
// continuation from consumed state only, and writes through owned tasks.
export default definePlugin({
  name: 'GoalBranch',
  provides: {},
  inject: ['dispatcher', 'timelineIdentity', 'treeCommands', 'settings'],
  async apply(ctx) {
    const dispatcher = ctx.use<Dispatcher>('dispatcher')
    const tree = ctx.use<TreeCommands>('treeCommands')
    const settings = ctx.use<Settings>('settings')
    const { branchId } = await ctx.use<{ get(): Promise<{ branchId: string }> }>('timelineIdentity').get()
    let stopped = false
    ctx.effect(() => { stopped = true })

    const fold = new Fold()
    let start: bigint | undefined
    // Write tasks in flight, keyed by the deciding sequence.
    const inFlight = new Set<string>()
    const task = (after: bigint, kind: 'end' | 'interrupt' | 'clear') => {
      const key = String(after)
      if (stopped || inFlight.has(key)) return
      inFlight.add(key)
      void (async () => {
        try {
          let input: InputMessage | undefined
          if (kind === 'end') {
            const read = await settings.read({ source: 'goal', scope: 'user' })
            // The async read can outlive its deciding turn or a newer goal change.
            if (stopped || fold.turnEnd?.sequence !== after) return
            if (!read.ok) {
              console.warn('Goal settings read failed:', read.error.code)
              return
            }
            const limit = read.value.document.maxContinuations
            const decision = decideTurnEnd(fold, limit)
            if (!decision) return
            input = decision.write === 'continuation'
              ? writes.continuation(branchId, decision.count, limit, after)
              : writes.pause(branchId, decision.reason, after)
          } else if (kind === 'clear') {
            if (!fold.isDecided(after) && fold.localBlocks === 0) input = writes.startClear(branchId, after)
          } else if (decideInterrupt(fold, after)) {
            input = writes.pause(branchId, 'interrupted', after)
          }
          if (!input) return
          await tree.input(branchId, input)
        } finally {
          inFlight.delete(key)
        }
      })()
    }
    const decideEnd = () => {
      const end = fold.turnEnd
      if (end && fold.running && !fold.isDecided(end.sequence)) task(end.sequence, 'end')
    }
    // The prefix before `branch/start` still has a live goal key (§3.4).
    const inheritedLive = async (at: bigint) => {
      const local = new Set((await dispatcher.local(at, at)).map(event => event.id))
      let live = false
      for (const event of await dispatcher.history(at)) {
        if (local.has(event.id) || event.type !== 'input' || event.name !== 'message') continue
        for (const raw of event.data.blocks ?? []) {
          if (marker(raw.text) && (raw.key === OBJECTIVE_KEY || raw.key === STATUS_KEY)) live = !raw.retire
        }
      }
      return live
    }
    const decideStart = (at: bigint) => {
      if (fold.isDecided(at) || fold.localBlocks > 0) return
      void inheritedLive(at).then(live => {
        if (live) task(at, 'clear')
      })
    }
    const consume = (event: AgentEvent, live: boolean) => {
      fold.apply(event)
      const sequence = BigInt(event.sequence)
      if (event.type === 'branch' && event.name === 'start') {
        start = sequence
        if (live) decideStart(sequence)
      } else if (!live) {
        return
      } else if (event.type === 'turn' && event.name === 'end') {
        decideEnd()
      } else if (event.type === 'stop' && event.name === 'interrupt' && event.data.scope === 'turn') {
        if (decideInterrupt(fold, sequence)) task(sequence, 'interrupt')
      } else if (event.type === 'input' && event.name === 'deleted') {
        decideEnd()
      }
    }

    // Subscribe, fix the cutoff, fold through it; live events at or below it
    // are already folded (A.6 Load).
    let cutoff = -1n
    let rebuilt!: () => void
    const ready = new Promise<void>(resolve => { rebuilt = resolve })
    await dispatcher.listen('Goal', KEYS, async event => {
      await ready
      if (BigInt(event.sequence) > cutoff) consume(event, true)
    })
    cutoff = BigInt(await dispatcher.committed())
    for (const event of await dispatcher.local(0, cutoff)) consume(event, false)
    rebuilt()
    // Decisions the previous generation owed are taken once.
    decideEnd()
    if (start !== undefined) decideStart(start)
  },
})
