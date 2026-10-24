import { appendFileSync } from 'node:fs'
import { definePlugin } from '@arcships/rutis'
import type { Services } from '../../npm/plugin.js'
import provides from './session-observer.json' with { type: 'json' }

// A Session row that observes its own Session: it reads the Session's
// identity, creation configuration and lifecycle by name and listens to
// the Session's channel from `apply` on.
export function observer(declaration: Record<string, Record<string, string>>) {
  return definePlugin({
    name: 'SessionObserverFixture',
    provides: declaration,
    inject: ['sessionIdentity', 'sessionConfig', 'sessionLifecycle'],
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
      const methods = Object.keys(declaration).find(name => name.startsWith('sessionMethods:'))!
      const namespace = methods.slice('sessionMethods:'.length)
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
      ctx.provide(methods, {
        async describe() {
          return ['status', 'through', 'arm', 'entered', 'release', 'throw', 'activated'].map(name => ({
            name: `${namespace}.${name}`, mutating: false, interruption: 'report', progress: false,
          }))
        },
        async call(call: { method: string; params: { sequence?: number; after?: number; branchId?: string } }) {
          const action = call.method.slice(namespace.length + 1)
          if (action === 'arm') { hold = true; entered = false }
          if (action === 'entered') await until(() => entered || closed)
          if (action === 'release') { hold = false; release?.() }
          if (action === 'throw') throwing = true
          if (action === 'activated') return { kind: 'value', value: await lifecycle.activated() }
          if (action === 'through') await until(() => closed || records.some(record =>
            record.branchId === (call.params.branchId ?? 'main') && record.event.sequence === call.params.sequence))
          return { kind: 'value', value: {
            binding, generation, pid: process.pid, closed,
            eligible: binding.config.interactive !== false && binding.config.collaborationRole === 'leader',
            records: records.filter(record => record.event.sequence > (call.params.after ?? 0)),
          } }
        },
      })
    },
  })
}

export default observer(provides)
