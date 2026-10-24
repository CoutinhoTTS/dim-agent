import { definePlugin } from '@arcships/rutis'
import provides from './named-greeter.json' with { type: 'json' }

type Greeter = { hello(): Promise<string> }

// Reads `testGreeter`, which a Rust part provides by name in each Session,
// and answers `session.greeter.ask` with what that Session's copy says.
export default definePlugin({
  name: 'NamedGreeterFixture',
  provides,
  inject: ['testGreeter'],
  apply(ctx) {
    const greeter = ctx.use<Greeter>('testGreeter')
    ctx.provide('sessionMethods:session.greeter', {
      async describe() {
        return [{ name: 'session.greeter.ask', mutating: false, interruption: 'report', progress: false }]
      },
      async call() {
        return { kind: 'value', value: await greeter.hello() }
      },
    })
  },
})
