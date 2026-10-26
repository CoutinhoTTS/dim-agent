import { definePlugin } from '@arcships/rutis'
import type { PartRegistry } from '../../npm/plugin.js'
import * as cordisBranch from './cordis-branch.ts'

// Registers the unmodified Cordis 4 plugin of `cordis-branch.ts` as a branch
// part: each branch mounts a copy, whose names resolve to that branch.
export default definePlugin({
  name: 'CordisBranchRow',
  inject: ['partRegistry'],
  async apply(ctx) {
    await ctx.use<PartRegistry>('partRegistry').registerDefinition('branch', 'cordis-branch-fixture', cordisBranch)
  },
})
