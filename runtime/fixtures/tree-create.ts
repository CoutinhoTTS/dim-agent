import { definePlugin } from '@arcships/rutis'
import provides from './tree-create.json' with { type: 'json' }

type TreeCommands = {
  create(parent: string, at: number | bigint, options: unknown, cancelled: () => Promise<void>): Promise<string>
}
type MethodCall = { method: string; params: { sessionId: string; at: number; cancel: boolean } }

// Creates a child of `main` through `treeCommands.create`, passing the
// caller's cancellation explicitly: one that never completes, or one that
// has already completed.
export default definePlugin({
  name: 'TreeCreateFixture',
  provides,
  inject: ['treeCommands'],
  apply(ctx) {
    const tree = ctx.use<TreeCommands>('treeCommands')
    ctx.provide('sessionMethods:session.treeprobe', {
      async describe() {
        return [{ name: 'session.treeprobe.create', mutating: true, interruption: 'report', progress: false }]
      },
      async call(call: MethodCall) {
        const { at, cancel } = call.params
        const options = { prompt: 'probe child', source: { kind: 'host', operation_id: crypto.randomUUID() } }
        const cancelled = () => (cancel ? Promise.resolve() : new Promise<void>(() => {}))
        try {
          return { kind: 'value', value: { id: await tree.create('main', at, options, cancelled) } }
        } catch (error) {
          return { kind: 'value', value: { error: (error as { code?: string }).code ?? String(error) } }
        }
      },
    })
  },
})
