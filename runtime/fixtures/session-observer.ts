import { appendFileSync } from 'node:fs'
import { definePlugin } from '@arcships/rutis'
import provides from './session-observer.json' with { type: 'json' }

export function observer(declaration: Record<string, Record<string, string>>) {
  return definePlugin({
    name: 'SessionObserverFixture',
    provides: declaration,
    apply(ctx, options: { badBind?: string; lifecycleFile?: string }) {
      const generation = crypto.randomUUID()
      const records: any[] = []
      const changed = new Set<() => void>()
      let binding: any
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
      const events = Object.keys(declaration).find(name => name.startsWith('sessionEvents:'))!
      const methods = Object.keys(declaration).find(name => name.startsWith('sessionMethods:'))!
      const namespace = methods.slice('sessionMethods:'.length)
      ctx.provide(events, {
        bind(value: any, closing: Promise<void>) {
          if (options.badBind === 'throw') throw new Error('fixture bind failure')
          if (options.badBind === 'value') return 1
          if (options.badBind === 'future') return Promise.resolve(null)
          binding = value
          closing.then(() => { closed = true; release?.(); notify() })
          // Unload waits for running listeners: a held one lets go first.
          ctx.signal.addEventListener('abort', () => { closed = true; release?.(); notify() })
          ctx.effect(async () => {
            closed = true
            release?.()
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
            work ??= closing.then(() => { settled = true })
            records.push(record); notify()
            if (hold && record.event.type === 'input' && record.event.name === 'applied') {
              entered = true; notify()
              await new Promise<void>(resolve => { release = resolve })
            }
            if (throwing) { throwing = false; throw new Error('fixture listener failure') }
          })
          return null
        },
      })
      ctx.provide(methods, {
        async describe() {
          return ['status', 'through', 'arm', 'entered', 'release', 'throw'].map(name => ({
            name: `${namespace}.${name}`, mutating: false, interruption: 'report', progress: false,
          }))
        },
        async call(call: { method: string; params: { sequence?: number; after?: number; branchId?: string } }) {
          const action = call.method.slice(namespace.length + 1)
          if (action === 'arm') { hold = true; entered = false }
          if (action === 'entered') await until(() => entered || closed)
          if (action === 'release') { hold = false; release?.() }
          if (action === 'throw') throwing = true
          if (action === 'through') await until(() => closed || records.some(record =>
            record.branchId === (call.params.branchId ?? 'main') && record.event.sequence === call.params.sequence))
          return { kind: 'value', value: {
            binding, generation, pid: process.pid, closed,
            eligible: binding?.config.interactive !== false && binding?.config.collaborationRole === 'leader',
            records: records.filter(record => record.event.sequence > (call.params.after ?? 0)),
          } }
        },
      })
    },
  })
}

export default observer(provides)
