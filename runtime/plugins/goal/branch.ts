import { definePlugin } from '@arcships/rutis'
import { type AgentEvent, type Decision, type InputMessage, Fold, OBJECTIVE_KEY, STATUS_KEY, decideInterrupt, decideTurnEnd, isOpening, marker, writes } from './goal.ts'

type Dispatcher = {
  listen(name: string, keys: string[], handler: (event: AgentEvent) => void | Promise<void>): Promise<void>
  committed(): Promise<number | bigint>
  local(start: number | bigint, through: number | bigint): Promise<AgentEvent[]>
  history(through: number | bigint): Promise<AgentEvent[]>
}
type TreeCommands = { input(id: string, input: InputMessage): Promise<AgentEvent> }
type Settings = {
  read(request: { source: string; scope: 'user' }): Promise<{ ok: boolean; value?: { document: { maxContinuations: number } } }>
}

const KEYS = ['branch/start', 'session/forked', 'input/message', 'input/consumed', 'input/deleted', 'turn/start', 'turn/end', 'stop/interrupt']

// GoalBranchPlugin (Goal.md §1, A.6): owns the branch's fold, decides
// continuation from consumed state only, and writes through owned tasks.
export default definePlugin({
  name: 'GoalBranch',
  provides: {},
  inject: ['dispatcher', 'timelineIdentity', 'treeCommands', 'settings', 'branchLifecycle'],
  async apply(ctx) {
    const dispatcher = ctx.use<Dispatcher>('dispatcher')
    const tree = ctx.use<TreeCommands>('treeCommands')
    const settings = ctx.use<Settings>('settings')
    const { branchId } = await ctx.use<{ get(): Promise<{ branchId: string }> }>('timelineIdentity').get()
    const lifecycle = ctx.use<{ activated(): Promise<boolean>; closing(): Promise<void> }>('branchLifecycle')
    let stopped = false
    ctx.effect(() => { stopped = true })
    void lifecycle.closing().then(() => { stopped = true })

    // Each decision uses the latest document read (§1).
    let limit = 20
    const readLimit = async () => {
      const read = await settings.read({ source: 'goal', scope: 'user' })
      if (read.ok && read.value) limit = read.value.document.maxContinuations
    }
    await readLimit()
    ctx.on('settings.changed', (change: { source: string }) => {
      if (change.source === 'goal') void readLimit()
    })

    const fold = new Fold()
    let start: bigint | undefined
    // Write tasks in flight, keyed by the deciding sequence.
    const inFlight = new Set<string>()
    const task = (after: bigint, recheck: () => Decision | 'clear' | undefined) => {
      const key = String(after)
      if (inFlight.has(key)) return
      inFlight.add(key)
      void (async () => {
        try {
          // Write tasks are background work: they wait for the branch to be
          // published (a decision taken while loading would otherwise write
          // to a branch that does not accept input yet), and end on closing.
          if (!(await lifecycle.activated())) return
          // Recheck over the fold as consumed now; later facts precede the
          // write and are resolved in sequence by their consumers.
          const decision = recheck()
          if (stopped || !decision) return
          const input = decision === 'clear'
            ? writes.startClear(branchId, after)
            : decision.write === 'continuation'
              ? writes.continuation(branchId, decision.count, limit, after)
              : writes.pause(branchId, decision.reason, after)
          await tree.input(branchId, input)
        } finally {
          inFlight.delete(key)
        }
      })()
    }
    const decideEnd = () => {
      const end = fold.turnEnd
      if (end && decideTurnEnd(fold, limit)) task(end.sequence, () => decideTurnEnd(fold, limit))
    }
    // The prefix before the opening (inherited or copied) still has a live
    // goal key (§3.4, A11).
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
        if (live) task(at, () => (fold.isDecided(at) || fold.localBlocks > 0 ? undefined : 'clear'))
      })
    }
    const consume = (event: AgentEvent, live: boolean) => {
      fold.apply(event)
      const sequence = BigInt(event.sequence)
      if (isOpening(event)) {
        start = sequence
        if (live) decideStart(sequence)
      } else if (!live) {
        return
      } else if (event.type === 'turn' && event.name === 'end') {
        decideEnd()
      } else if (event.type === 'stop' && event.name === 'interrupt' && event.data.scope === 'turn') {
        if (decideInterrupt(fold, sequence)) task(sequence, () => decideInterrupt(fold, sequence))
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
