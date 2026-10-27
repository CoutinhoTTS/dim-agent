import { definePlugin } from '@arcships/rutis'
import { partRow } from './part-row.ts'
import provides from './tree-create.json' with { type: 'json' }
import type { SessionMethods } from '../../npm/plugin.js'

type TreeCommands = {
  create(parent: string, at: number | bigint, options: unknown, cancelled: () => Promise<void>): Promise<string>
}
type MethodCall = { method: string; params: { sessionId: string; at: number; cancel: boolean } }

export const part = definePlugin({
  name: 'TreeCreateFixture',
  provides,
  inject: ['treeCommands', 'sessionMethods'],
  async apply(ctx) {
    const tree = ctx.use<TreeCommands>('treeCommands')
    await ctx.use<SessionMethods>('sessionMethods').register(
      'session.treeprobe',
      [{ name: 'session.treeprobe.create', mutating: true, interruption: 'report', progress: false }],
      async (call: MethodCall) => {
        const { at, cancel } = call.params
        const options = { prompt: 'probe child', source: { kind: 'host', operation_id: crypto.randomUUID() } }
        const cancelled = () => (cancel ? Promise.resolve() : new Promise<void>(() => {}))
        try {
          return { kind: 'value', value: { id: await tree.create('main', at, options, cancelled) } }
        } catch (error) {
          return { kind: 'value', value: { error: (error as { code?: string }).code ?? String(error) } }
        }
      },
    )
  },
})

export default partRow('session', part)
