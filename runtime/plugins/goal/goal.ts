// Goal's language-neutral contract (docs/design/plugins/Goal.md): the change
// and continuation inputs, and the fold of a branch's local events.

export const OBJECTIVE_KEY = 'goal:objective'
export const STATUS_KEY = 'goal:status'

export type RawBlock = { type: 'raw'; text: string; key?: string; retire?: boolean }
export type InputMessage = {
  submission_id?: string
  user?: { content: unknown[] }
  blocks?: RawBlock[]
  mode?: 'queue' | 'steer' | 'append'
}
export type AgentEvent = {
  id: string
  sequence: number | bigint
  type: string
  name: string
  data: Record<string, any>
}

export type Status = 'active' | 'paused' | 'completed' | 'blocked'
export type PauseReason = 'user' | 'interrupted' | 'limit' | 'turn_failed'

const REASONS: Record<Exclude<PauseReason, 'user'>, string> = {
  interrupted: 'the user interrupted the turn',
  limit: 'the automatic continuation limit was reached',
  turn_failed: 'the last turn failed',
}

/** Escapes text placed inside a marker body or attribute (A.5, §7). */
export function escape(text: string): string {
  return text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;')
}

function block(attrs: Record<string, string | number>, body: string[], key?: string, retire = false): RawBlock {
  const marker = Object.entries(attrs).map(([name, value]) => `${name}="${escape(String(value))}"`).join(' ')
  const raw: RawBlock = { type: 'raw', text: `<dim-goal ${marker}>\n${body.join('\n')}\n</dim-goal>` }
  if (key) raw.key = key
  if (retire) raw.retire = true
  return raw
}

function input(blocks: RawBlock[], mode: 'queue' | 'append'): InputMessage {
  return { submission_id: `goal:${crypto.randomUUID()}`, blocks, mode }
}

function activeStatus(branch: string): RawBlock {
  return block({ kind: 'status', status: 'active', branch }, [
    `The goal for branch ${branch} is active. Keep working toward it across turns. When it is fully achieved, call goal_update with status "completed". If you cannot make progress without the user, call goal_update with status "blocked" and explain why.`,
  ], STATUS_KEY)
}

function cleared(branch: string, attrs: Record<string, string>, text: string): InputMessage {
  return input([
    block({ kind: 'goal', op: 'clear', ...attrs, branch }, [text], OBJECTIVE_KEY, true),
    block({ kind: 'status', status: 'cleared', branch }, [`The goal for branch ${branch} has no status.`], STATUS_KEY, true),
  ], 'append')
}

/** The change inputs (A.5); each is one `input/message`. */
export const changes = {
  set: (branch: string, objective: string) => input([
    block({ kind: 'goal', op: 'set', branch }, [
      'Objective:', escape(objective), '',
      `This is the user's standing goal for branch ${branch}. If you are working on a delegated sub-task started from branch ${branch}, treat this goal as background and do not call goal_update.`,
    ], OBJECTIVE_KEY),
    activeStatus(branch),
  ], 'queue'),
  edit: (branch: string, objective: string) => input([
    block({ kind: 'goal', op: 'edit', branch }, [
      'Objective:', escape(objective), '',
      `The user set the objective of the goal for branch ${branch}. This does not change its status.`,
    ], OBJECTIVE_KEY),
  ], 'append'),
  pause: (branch: string) => input([
    block({ kind: 'goal', op: 'pause', reason: 'user', branch }, [
      `The user set the goal status for branch ${branch} to paused. Do not continue working toward a goal for branch ${branch} on your own; respond only to the user's latest input.`,
    ], STATUS_KEY),
  ], 'append'),
  resume: (branch: string) => input([
    block({ kind: 'goal', op: 'resume', branch }, [
      `The user set the goal status for branch ${branch} to active. If branch ${branch} has a goal objective, keep working toward it across turns; when it is completed or blocked, call goal_update.`,
    ], STATUS_KEY),
  ], 'queue'),
  clear: (branch: string) => cleared(branch, {}, 'The previous goal was cleared by the user. Disregard it.'),
  report: (branch: string, status: 'completed' | 'blocked', summary: string) => input([
    block({ kind: 'goal', op: status, branch }, [
      status === 'completed' ? 'Result:' : 'Blocker:', escape(summary), '',
      `goal_update set the goal status for branch ${branch} to ${status}. Do not continue working toward a goal for branch ${branch} on your own.`,
    ], STATUS_KEY),
  ], 'append'),
}

/** Goal's own writes, each marked with the sequence that decided it. */
export const writes = {
  continuation: (branch: string, count: number, limit: number, after: bigint) => input([
    block({ kind: 'continuation', count, limit, after: String(after), branch }, [
      `Continue working toward the active goal for branch ${branch}. If it is completed or blocked, call goal_update. If you are working on a delegated sub-task started from branch ${branch}, do not call goal_update.`,
    ]),
  ], 'queue'),
  pause: (branch: string, reason: Exclude<PauseReason, 'user'>, after: bigint) => input([
    block({ kind: 'goal', op: 'pause', reason, after: String(after), branch }, [
      `The goal is paused (${REASONS[reason]}). Do not continue working toward it on your own; respond only to the user's latest input.`,
    ], STATUS_KEY),
  ], 'append'),
  startClear: (branch: string, after: bigint) => cleared(branch, { reason: 'new_branch', after: String(after) },
    `Any goal above belongs to another branch, not to branch ${branch}. Disregard it; branch ${branch} has no goal.`),
}

/** The attributes of a block's leading `<dim-goal ...>` marker, if any. */
export function marker(text: string): Record<string, string> | undefined {
  const match = /^<dim-goal ([^>]*)>/.exec(text)
  if (!match) return undefined
  const attrs: Record<string, string> = {}
  for (const [, name, value] of match[1].matchAll(/(\w+)="([^"]*)"/g)) attrs[name] = value
  return attrs
}

const seq = (event: AgentEvent) => BigInt(event.sequence)

/** The status a live `goal:status` block sets. */
function statusOf(attrs: Record<string, string>, text: string): { status: Status; reason?: string; summary?: string } {
  if (attrs.kind === 'status') return { status: 'active' }
  switch (attrs.op) {
    case 'pause': return { status: 'paused', reason: attrs.reason }
    case 'resume': return { status: 'active' }
    default: {
      const body = text.split('\n')
      return { status: attrs.op as Status, summary: body[2] }
    }
  }
}

/** One branch's goal state: the fold of its consumed local events (§2). */
export class Fold {
  objective: string | undefined
  status: { status: Status; reason?: string; summary?: string } | undefined
  /** Sequence of the latest change that set `active` (`set` or `resume`). */
  active = -1n
  lastOperator = -1n
  turnStart = -1n
  turnEnd: { sequence: bigint; status: string } | undefined
  /** Sequences of local goal blocks; any means the branch has its own goal history. */
  localBlocks = 0
  private interrupts: bigint[] = []
  private continuations: bigint[] = []
  private decided = new Set<string>()
  private driving = new Map<string, bigint>()
  private events: AgentEvent[] = []
  private deleted = new Set<string>()

  /** Applies one consumed local event; `input/deleted` refolds without the input. */
  apply(event: AgentEvent): void {
    if (event.type === 'input' && event.name === 'deleted') {
      this.deleted.add(event.data.input_id)
      this.events.push(event)
      this.refold()
      return
    }
    this.events.push(event)
    this.step(event)
  }

  private refold(): void {
    const events = this.events
    Object.assign(this, new Fold())
    this.events = events
    this.deleted = new Set(events.filter(e => e.type === 'input' && e.name === 'deleted').map(e => e.data.input_id))
    for (const event of events) this.step(event)
  }

  private step(event: AgentEvent): void {
    const sequence = seq(event)
    const key = `${event.type}/${event.name}`
    switch (key) {
      case 'input/message': {
        if (this.deleted.has(event.id)) return
        const data = event.data as InputMessage
        const blocks = data.blocks ?? []
        const mode = data.mode ?? (data.user ? 'queue' : 'append')
        if (mode !== 'append') this.driving.set(event.id, sequence)
        const first = blocks[0] && marker(blocks[0].text)
        if (data.user && !(blocks[0]?.text.startsWith('<dim-'))) this.lastOperator = sequence
        if (first?.after) this.decided.add(first.after)
        if (first?.kind === 'continuation') this.continuations.push(sequence)
        for (const raw of blocks) {
          const attrs = marker(raw.text)
          if (!attrs || (raw.key !== OBJECTIVE_KEY && raw.key !== STATUS_KEY)) continue
          this.localBlocks++
          if (raw.key === OBJECTIVE_KEY) {
            this.objective = raw.retire ? undefined : raw.text.split('\n')[2]
          } else if (raw.retire) {
            this.status = undefined
          } else {
            this.status = statusOf(attrs, raw.text)
            if (this.status.status === 'active' && (attrs.op === 'resume' || attrs.kind === 'status')) {
              this.active = sequence
            }
          }
        }
        return
      }
      case 'input/consumed':
        this.driving.delete(event.data.input_id)
        return
      case 'turn/start':
        this.turnStart = sequence
        return
      case 'turn/end':
        this.turnEnd = { sequence, status: event.data.status }
        return
      case 'stop/interrupt':
        if (event.data.scope === 'turn') this.interrupts.push(sequence)
        return
    }
  }

  /** A goal exists and is active (C1). */
  get running(): boolean {
    return this.objective !== undefined && this.status?.status === 'active'
  }
  get interrupted(): boolean {
    return this.interrupts.some(sequence => sequence > this.active)
  }
  get count(): number {
    const since = this.active > this.lastOperator ? this.active : this.lastOperator
    return this.continuations.filter(sequence => sequence > since).length
  }
  get pendingDriving(): boolean {
    return [...this.driving.keys()].some(id => !this.deleted.has(id))
  }
  get turnOpen(): boolean {
    return this.turnStart > (this.turnEnd?.sequence ?? -1n)
  }
  isDecided(sequence: bigint): boolean {
    return this.decided.has(String(sequence))
  }
}

export type Decision =
  | { write: 'continuation'; count: number }
  | { write: 'pause'; reason: Exclude<PauseReason, 'user'> }
  | undefined

/** The decision for the latest `turn/end` (A.4), over the fold as consumed. */
export function decideTurnEnd(fold: Fold, limit: number): Decision {
  const end = fold.turnEnd
  if (!end || fold.isDecided(end.sequence) || !fold.running) return undefined
  if (fold.interrupted) return { write: 'pause', reason: 'interrupted' }
  if (end.status === 'failed') return { write: 'pause', reason: 'turn_failed' }
  if (end.status !== 'completed' || fold.turnOpen || fold.pendingDriving) return undefined
  if (fold.count >= limit) return { write: 'pause', reason: 'limit' }
  return { write: 'continuation', count: fold.count + 1 }
}

/** The decision for a `stop/interrupt {scope: turn}` at `sequence`. */
export function decideInterrupt(fold: Fold, sequence: bigint): Decision {
  if (fold.isDecided(sequence) || !fold.running || fold.active >= sequence) return undefined
  return { write: 'pause', reason: 'interrupted' }
}
