import { definePlugin } from '@arcships/rutis'
import provides from './session-part.json' with { type: 'json' }

type Call = {
  sessionId: string; branchId: string; turnId: string; callId: string
  arguments: { text: string }; config: { cwd: string }
}
type MethodCall = { callId: string; method: string; params: { sessionId: string } }

export default definePlugin({
  name: 'SessionTypeScriptFixture',
  provides,
  apply(ctx, config: { badDeclaration?: boolean; badMethods?: boolean | 'unsupported' }) {
    const generation = crypto.randomUUID()
    let stopped = false
    let calls = 0
    let fastFinished = false
    let finishFast: (() => void) | undefined
    let toolStarted = false
    let startTool: (() => void) | undefined
    const alive = () => { if (stopped) throw new Error('provider unloaded') }
    ctx.effect(() => { stopped = true })
    ctx.provide('toolContribution:ts_echo', {
      declaration() {
        alive()
        return {
          name: config.badDeclaration ? 'wrong_name' : 'ts_echo',
          description: 'TypeScript flow fixture', inputSchema: { type: 'object' },
          source: 'extension', risk: 'session',
        }
      },
      async execute(call: Call, cancelled: Promise<void>) {
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
        alive()
        return { content: JSON.stringify({ ...call, generation, calls }) }
      },
    })
    ctx.provide('sessionMethods:session.typescript', {
      async describe() {
        alive()
        if (config.badMethods === 'unsupported') return [{ name: 'session.typescript.bad', mutating: false, interruption: 'report', progress: false, notifications: [] }]
        if (config.badMethods) return [{ name: 'session.typescript.bad', mutating: false }]
        return ['status', 'wait_fast', 'wait_tool', 'error', 'malformed', 'report', 'drain'].map(name => ({
          name: `session.typescript.${name}`, mutating: false,
          interruption: name === 'drain' ? 'drain' : 'report', progress: true,
        }))
      },
      async call(call: MethodCall, cancelled: Promise<void>, progress: (data: unknown) => void) {
        alive()
        if (call.method.endsWith('.error')) return { code: -32011, appError: `owned_${generation}`, message: 'fixture error' }
        if (call.method.endsWith('.malformed')) return { invalid: true }
        if (call.method.endsWith('.wait_fast') && !fastFinished) await new Promise<void>(resolve => { finishFast = resolve })
        if (call.method.endsWith('.wait_tool') && !toolStarted) await new Promise<void>(resolve => { startTool = resolve })
        const hold = call.method.endsWith('.report') || call.method.endsWith('.drain')
        progress({ phase: 'started', generation })
        if (hold) await cancelled
        alive()
        progress({ phase: 'settled', generation })
        return { kind: 'value', value: { sessionId: call.params.sessionId, generation, pid: process.pid, calls, cancelled: hold, alive: !stopped } }
      },
    })
  },
})
