import { appendFileSync } from 'node:fs'
import { definePlugin } from '@arcships/rutis'
import type { Services, SessionMethods } from '../../npm/plugin.js'
import provides from './session-observer.json' with { type: 'json' }

// A Session row that observes its own Session: it reads the Session's
// identity, creation configuration and lifecycle by name and listens to
// the Session's channel from `apply` on. Its methods are `namespace`'s.
export function observer(namespace: string, declaration: Record<string, Record<string, string>>) {
  return definePlugin({
    name: 'SessionObserverFixture',
    provides: declaration,
    inject: ['sessionIdentity', 'sessionConfig', 'sessionLifecycle', 'sessionMethods'],
    async apply(ctx, options: { lifecycleFile?: string }) {
      const generation = crypto.randomUUID()
      const records: any[] = []
      const changed = new Set<() => void>()
      const identity = await ctx.use<Services['sessionIdentity']>('sessionIdentity').get()
      const config = await ctx.use<Services['sessionConfig']>('sessionConfig').get()
      const lifecycle = ctx.use<Services['sessionLifecycle']>('sessionLifecycle')
      const binding = { ...identity, config }
      let closed = false
      let hold = false
      let entered = false
      let throwing = false
      let release: (() => void) | undefined
      let work: Promise<void> | undefined
      let settled = false
      const notify = () => { for (const wake of changed) wake() }
      const until = async (ready: () => boolean) => {
        while (!ready()) await new Promise<void>(resolve => {
          const wake = () => { changed.delete(wake); resolve() }
          changed.add(wake)
        })
      }
      // Accepted work ends when the Session closes or this row unloads.
      let end!: () => void
      const ended = new Promise<void>(resolve => { end = resolve })
      const stop = () => { closed = true; release?.(); end(); notify() }
      void lifecycle.closing().then(stop)
      // Unload waits for running listeners: a held one lets go first.
      ctx.signal.addEventListener('abort', stop)
      ctx.effect(async () => {
        stop()
        await work
        if (options.lifecycleFile) appendFileSync(options.lifecycleFile, JSON.stringify({
          generation, sessionId: binding.sessionId, closed, settled,
          count: records.length,
        }) + '\n')
      })
      ctx.on(`session/${binding.sessionId}`, async (broadcast: any) => {
        if (broadcast.kind !== 'event' || closed) return
        const record = { sessionId: binding.sessionId, branchId: broadcast.branchId, event: broadcast.event }
        // Listeners start in emit order, so the record keeps it.
        work ??= ended.then(() => { settled = true })
        records.push(record); notify()
        if (hold && record.event.type === 'input' && record.event.name === 'applied') {
          entered = true; notify()
          await new Promise<void>(resolve => { release = resolve })
        }
        if (throwing) { throwing = false; throw new Error('fixture listener failure') }
      })
      await ctx.use<SessionMethods>('sessionMethods').register(
        namespace,
        ['status', 'through', 'arm', 'entered', 'release', 'throw', 'activated', 'hold_session', 'release_session'].map(name => ({
          name: `${namespace}.${name}`, mutating: false, interruption: 'report', progress: false,
        })),
        async (call: { method: string; params: { sequence?: number; after?: number; branchId?: string; id?: number } }) => {
          const action = call.method.slice(namespace.length + 1)
          if (action === 'arm') { hold = true; entered = false }
          if (action === 'entered') await until(() => entered || closed)
          if (action === 'release') { hold = false; release?.() }
          if (action === 'throw') throwing = true
          if (action === 'activated') return { kind: 'value', value: await lifecycle.activated() }
          if (action === 'hold_session') return { kind: 'value', value: await lifecycle.hold() }
          if (action === 'release_session') return { kind: 'value', value: await lifecycle.release(call.params.id!) }
          if (action === 'through') await until(() => closed || records.some(record =>
            record.branchId === (call.params.branchId ?? 'main') && record.event.sequence === call.params.sequence))
          return { kind: 'value', value: {
            binding, generation, pid: process.pid, closed,
            eligible: binding.config.interactive !== false && binding.config.collaborationRole === 'leader',
            records: records.filter(record => record.event.sequence > (call.params.after ?? 0)),
          } }
        },
      )
    },
  })
}

export default observer('session.observer', provides)
