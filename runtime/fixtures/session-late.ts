import { definePlugin } from '@arcships/rutis'
import provides from './session-late.json' with { type: 'json' }

export default definePlugin({
  name: 'LateSessionTypeScriptFixture',
  inject: ['toolContribution:ts_echo'],
  provides,
  apply(ctx) {
    let stopped = false
    ctx.effect(() => { stopped = true })
    const alive = () => { if (stopped) throw new Error('provider unloaded') }
    ctx.provide('toolContribution:late', {
      declaration() { alive(); return { name: 'late', description: 'Late tool', inputSchema: { type: 'object' }, source: 'extension', risk: 'session' } },
      async execute() { alive(); return { content: 'late result' } },
    })
    ctx.provide('sessionMethods:session.late', {
      async describe() { alive(); return [{ name: 'session.late.status', mutating: false, interruption: 'report', progress: false }] },
      async call() { alive(); return { kind: 'value', value: 'active' } },
    })
  },
})
