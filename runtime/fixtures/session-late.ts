import { definePlugin } from '@arcships/rutis'
import provides from './session-late.json' with { type: 'json' }
import type { ToolRegistry } from '../../npm/plugin.js'

export default definePlugin({
  name: 'LateSessionTypeScriptFixture',
  // Pending until the test provides the gate row's name in its Session.
  inject: ['sessionMethods:session.typescript', 'toolRegistry'],
  provides,
  async apply(ctx) {
    let stopped = false
    ctx.effect(() => { stopped = true })
    const alive = () => { if (stopped) throw new Error('provider unloaded') }
    await ctx.use<ToolRegistry>('toolRegistry').register(
      { name: 'late', description: 'Late tool', inputSchema: { type: 'object' }, source: 'extension', risk: 'session' },
      async () => { alive(); return { content: 'late result' } },
    )
    ctx.provide('sessionMethods:session.late', {
      async describe() { alive(); return [{ name: 'session.late.status', mutating: false, interruption: 'report', progress: false }] },
      async call() { alive(); return { kind: 'value', value: 'active' } },
    })
  },
})
