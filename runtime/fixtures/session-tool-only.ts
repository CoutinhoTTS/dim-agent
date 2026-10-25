import { definePlugin } from '@arcships/rutis'
import provides from './session-tool-only.json' with { type: 'json' }
import type { ToolRegistry } from '../../npm/plugin.js'

export default definePlugin({
  name: 'SessionToolOnlyFixture',
  inject: ['toolRegistry'],
  provides,
  async apply(ctx) {
    await ctx.use<ToolRegistry>('toolRegistry').register(
      {
        name: 'ts_echo', description: 'Tool-only flow fixture',
        inputSchema: { type: 'object' }, source: 'extension', risk: 'session',
      },
      async call => ({ content: JSON.stringify(call) }),
    )
  },
})
