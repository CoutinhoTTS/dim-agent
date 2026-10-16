import { definePlugin } from '@arcships/rutis'

// Experimental contracts for the C4 transport probe; not product services.
type SessionHost = { identity(): string; closed(): void }
type Notice =
  | { kind: 'event'; branchId: string; event: { sequence: number } }
  | { kind: 'fence'; token: number }
  | { kind: 'closed' }
type ToolCall = { id: string; name: string; input: { text?: string; hold?: boolean } }

export default definePlugin({
  name: 'C4SessionPart',
  inject: ['c4Session'],
  provides: {
    c4Tool: { declaration: 'sync', execute: 'async' },
    c4Probe: {
      sync: 'sync', async: 'async', status: 'sync', notice: 'async',
      fence: 'async', started: 'async',
    },
  },
  apply(ctx) {
    const session = ctx.use<SessionHost>('c4Session')
    const sessionId = session.identity()
    let stopped = false
    const notices: Notice[] = []
    const fences = new Set<number>()
    const waiting = new Map<number, () => void>()
    const pending = new Set<(reason: string) => void>()
    let started: (() => void) | undefined
    const alive = () => { if (stopped) throw new Error('session part unloaded') }
    ctx.effect(() => {
      stopped = true
      for (const finish of pending) finish('unloaded')
      for (const done of waiting.values()) done()
      started?.()
      session.closed()
    })
    ctx.provide('c4Tool', {
      declaration() {
        alive()
        return {
          name: 'spike', description: 'C4 transport probe',
          inputSchema: { type: 'object' }, source: 'extension', risk: 'session',
        }
      },
      async execute(call: ToolCall, cancel?: Promise<void>) {
        alive()
        if (call.input.hold && !cancel) throw new Error('held call requires cancellation')
        const text = call.input.hold
          ? await new Promise<string>(resolve => {
            const finish = (reason: string) => { pending.delete(finish); resolve(reason) }
            pending.add(finish)
            started?.()
            started = undefined
            cancel?.then(() => finish('cancelled'))
          })
          : `${sessionId}:${call.input.text}`
        return { content: text }
      },
    })
    ctx.provide('c4Probe', {
      sync(value: unknown) { alive(); return value },
      async async(value: unknown) { alive(); return value },
      status() { alive(); return { sessionId, pid: process.pid, node: process.version, notices } },
      async notice(record: Notice) {
        alive()
        if (record.kind === 'fence') {
          fences.add(record.token)
          waiting.get(record.token)?.()
          waiting.delete(record.token)
        } else {
          notices.push(record)
        }
        return null
      },
      async fence(token: number) {
        alive()
        if (!fences.has(token)) await new Promise<void>(resolve => waiting.set(token, resolve))
        alive()
        return null
      },
      async started() {
        alive()
        if (!pending.size) await new Promise<void>(resolve => { started = resolve })
        alive()
        return null
      },
    })
  },
})
