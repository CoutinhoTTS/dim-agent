import { definePlugin } from '@arcships/rutis'
import type { PartRegistry, Settings } from '../../../npm/plugin.d.ts'
import manifest from './dim-plugin.json' with { type: 'json' }
import part from './part.ts'
import { settings as declaration } from './settings.ts'

export default definePlugin({
  name: 'SessionTitle', inject: manifest.code.inject,
  async apply(ctx) {
    await ctx.use<Settings>('settings').register(declaration)
    await ctx.use<PartRegistry>('partRegistry').registerDefinition('session', 'SessionTitle', part)
  },
})
