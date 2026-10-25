import { definePlugin } from '@arcships/rutis'
import provides from './session-part.json' with { type: 'json' }
import type { SessionMethods, ToolRegistry } from '../../npm/plugin.js'

type Call = {
  sessionId: string; branchId: string; turnId: string; callId: string
  arguments: { text: string }; config: { cwd: string }
}
type MethodCall = { callId: string; method: string; params: { sessionId: string } }

export default definePlugin({
  name: 'SessionTypeScriptFixture',
  inject: ['toolRegistry', 'sessionMethods'],
  provides,
  async apply(ctx, config: { badDeclaration?: boolean; badMethods?: boolean | 'unsupported'; processOwner?: boolean; namespace?: string }) {
    const generation = crypto.randomUUID()
    const owner = config.processOwner ? ctx.use<{ status(): { generation: number } }>('exampleProcess') : undefined
    let stopped = false
    let calls = 0
    let fastFinished = false
    let finishFast: (() => void) | undefined
    let toolStarted = false
    let startTool: (() => void) | undefined
    const alive = () => { if (stopped) throw new Error('provider unloaded') }
    const pending = new Set<Promise<void>>()
    ctx.effect(async () => {
      // Cancellation is asynchronous; keep owned state until accepted calls settle.
      await Promise.all(pending)
      stopped = true
    })
    // A declaration without its risk is refused; the row keeps its methods.
    let refused: string | undefined
    await ctx.use<ToolRegistry>('toolRegistry').register(
      {
        name: 'ts_echo', description: 'TypeScript flow fixture', inputSchema: { type: 'object' },
        source: 'extension', ...(config.badDeclaration ? {} : { risk: 'session' }),
      },
      async (call: Call, cancelled: Promise<void>) => {
        alive()
        calls++
        if (call.arguments.text === 'malformed') return { content: { invalid: true } }
        if (call.arguments.text === 'fast') {
          cancelled.then(() => { fastFinished = true; finishFast?.() })
        }
        if (call.arguments.text === 'cancel') {
          toolStarted = true; startTool?.()
          await cancelled
        }
        return { content: JSON.stringify({ ...call, generation, calls, processGeneration: owner?.status().generation }) }
      },
    ).catch((error: Error) => { refused = error.message })
    // Invalid declarations, or a namespace the row may not take, are refused,
    // which fails the row.
    const namespace = config.namespace ?? 'session.typescript'
    const declarations = config.badMethods === 'unsupported'
      ? [{ name: 'session.typescript.bad', mutating: false, interruption: 'report', progress: false, notifications: [] }]
      : config.badMethods
        ? [{ name: 'session.typescript.bad', mutating: false }]
        : ['status', 'wait_fast', 'wait_tool', 'error', 'malformed', 'undefined', 'extra_value', 'extra_error', 'handed_off', 'report', 'drain'].map(name => ({
          name: `${namespace}.${name}`, mutating: false,
          interruption: name === 'drain' ? 'drain' : 'report', progress: true,
        }))
    await ctx.use<SessionMethods>('sessionMethods').register(
      namespace, declarations,
      async (call: MethodCall, cancelled: Promise<void>, progress: (data: unknown) => void) => {
        let done!: () => void
        const settled = new Promise<void>(resolve => { done = resolve })
        pending.add(settled)
        try {
          alive()
          if (call.method.endsWith('.error')) return { code: `owned_${generation}`, message: 'fixture error' }
          if (call.method.endsWith('.undefined')) return { kind: 'value', value: undefined }
          if (call.method.endsWith('.extra_value')) return { kind: 'value', value: null, extra: true }
          if (call.method.endsWith('.extra_error')) return { code: 'fixture', message: 'fixture error', extra: true }
          if (call.method.endsWith('.handed_off')) return { kind: 'handed_off' }
          if (call.method.endsWith('.malformed')) return { invalid: true }
          if (call.method.endsWith('.wait_fast') && !fastFinished) await new Promise<void>(resolve => { finishFast = resolve })
          if (call.method.endsWith('.wait_tool') && !toolStarted) await new Promise<void>(resolve => { startTool = resolve })
          const hold = call.method.endsWith('.report') || call.method.endsWith('.drain')
          progress({ phase: 'started', generation })
          if (hold) await cancelled
          alive()
          progress({ phase: 'settled', generation })
          return { kind: 'value', value: { sessionId: call.params.sessionId, generation, pid: process.pid, calls, cancelled: hold, alive: !stopped, refused } }
        } finally {
          pending.delete(settled)
          done()
        }
      },
    )
  },
})
