import { definePlugin } from '@arcships/rutis'
import { partRow } from './part-row.ts'
import provides from './session-late.json' with { type: 'json' }
import type { SessionMethods, ToolRegistry } from '../../npm/plugin.js'

export const part = definePlugin({
  name: 'LateSessionTypeScriptFixture',
  inject: ['lateGate', 'toolRegistry', 'sessionMethods'],
  provides,
  async apply(ctx) {
    let stopped = false
    ctx.effect(() => { stopped = true })
    const alive = () => { if (stopped) throw new Error('provider unloaded') }
    await ctx.use<ToolRegistry>('toolRegistry').register(
      { name: 'late', description: 'Late tool', inputSchema: { type: 'object' }, source: 'extension', risk: 'session' },
      async () => { alive(); return { content: 'late result' } },
    )
    await ctx.use<SessionMethods>('sessionMethods').register(
      'session.late',
      [{ name: 'session.late.status', mutating: false, interruption: 'report', progress: false }],
      async () => { alive(); return { kind: 'value', value: 'active' } },
    )
  },
})

export default partRow('session', part)
