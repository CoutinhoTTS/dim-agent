import { definePlugin } from '@arcships/rutis'
import type { BranchLifecycle } from '../../npm/plugin.js'
import { partRow } from './part-row.ts'

const opening = definePlugin({
  name: 'OpeningReceiptFixture',
  inject: ['branchLifecycle'],
  async apply(ctx) {
    await ctx.use<BranchLifecycle>('branchLifecycle').registerOpening(async () => {
      throw new Error('opening completion refused')
    })
  },
})
export default partRow('branch', opening)
