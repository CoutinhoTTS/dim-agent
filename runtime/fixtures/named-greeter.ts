import { definePlugin } from '@arcships/rutis'
import { partRow } from './part-row.ts'
import provides from './named-greeter.json' with { type: 'json' }
import type { SessionMethods } from '../../npm/plugin.js'

type Greeter = { hello(): Promise<string> }

// Reads `testGreeter`, which a Rust part provides by name in each Session,
// and answers `session.greeter.ask` with what that Session's copy says. It
// provides `namedGreeter` in each Session for a Rust part to read.
export const part = definePlugin({
  name: 'NamedGreeterFixture',
  provides,
  inject: ['testGreeter', 'sessionMethods'],
  async apply(ctx) {
    const greeter = ctx.use<Greeter>('testGreeter')
    const ask = async () => greeter.hello()
    ctx.provide('namedGreeter', { ask })
    await ctx.use<SessionMethods>('sessionMethods').register(
      'session.greeter',
      [{ name: 'session.greeter.ask', mutating: false, interruption: 'report', progress: false }],
      async () => ({ kind: 'value', value: await ask() }),
    )
  },
})

export default partRow('session', part)
