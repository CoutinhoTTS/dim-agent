import { definePlugin } from '@arcships/rutis'
import type { PartRegistry } from '../../../npm/plugin.js'
import branch from './branch.ts'
import session from './session.ts'

// Goal (Goal.md §1) is one row: it registers its Session part, which writes
// change inputs by branch id, and its branch part, which folds the branch's
// events and decides continuation. Each Session and branch mounts a copy.
export default definePlugin({
  name: 'Goal',
  inject: ['partRegistry'],
  provides: {},
  async apply(ctx) {
    const parts = ctx.use<PartRegistry>('partRegistry')
    await parts.registerDefinition('session', 'GoalSession', session)
    await parts.registerDefinition('branch', 'GoalBranch', branch)
  },
})
