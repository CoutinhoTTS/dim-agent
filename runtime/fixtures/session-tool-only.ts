import { definePlugin } from '@arcships/rutis'
import provides from './session-tool-only.json' with { type: 'json' }

export default definePlugin({
  name: 'SessionToolOnlyFixture',
  provides,
  apply(ctx) {
    ctx.provide('toolContribution:ts_echo', {
      declaration() {
        return {
          name: 'ts_echo', description: 'Tool-only flow fixture',
          inputSchema: { type: 'object' }, source: 'extension', risk: 'session',
        }
      },
      async execute(call: unknown) {
        return { content: JSON.stringify(call) }
      },
    })
  },
})
