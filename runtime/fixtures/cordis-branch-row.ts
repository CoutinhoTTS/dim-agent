import { definePlugin } from '@arcships/rutis'
import type { PartRegistry } from '../../npm/plugin.js'
import * as cordisBranch from './cordis-branch.ts'

export default definePlugin({
  name: 'CordisBranchRow',
  inject: ['partRegistry'],
  async apply(ctx) {
    await ctx.use<PartRegistry>('partRegistry').registerDefinition('branch', 'cordis-branch-fixture', cordisBranch)
  },
})
