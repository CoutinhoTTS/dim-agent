import { definePlugin } from '@arcships/rutis'
import { partRow } from './part-row.ts'
import type { BranchEvent, Services } from '../../npm/plugin.js'

export const part = definePlugin({
  name: 'BranchPartFixture',
  provides: {},
  inject: ['dispatcher', 'timeline', 'timelineIdentity', 'treeCommands'],
  async apply(ctx) {
    const dispatcher = ctx.use<Services['dispatcher']>('dispatcher')
    const timeline = ctx.use<Services['timeline']>('timeline')
    const tree = ctx.use<Services['treeCommands']>('treeCommands')
    const { branchId } = await ctx.use<Services['timelineIdentity']>('timelineIdentity').get()
    const seen: string[] = []
    let written = false
    let reported = false
    let cutoff = 0n
    let replayingMarker = false
    const step = (event: BranchEvent, live: boolean) => {
      const key = `${event.type}/${event.name}`
      seen.push(key)
      if (key === 'input/message' && JSON.stringify(event.data).includes('c5-marker')) written = true
      if (key === 'input/message' && JSON.stringify(event.data).includes('c5-rebuilt')) reported = true
      if (live && key === 'turn/end' && !written) {
        written = true
        void tree.input(branchId, {
          blocks: [{ type: 'raw', text: `<c5-marker branch="${branchId}" seen="${seen.join(',')}"/>` }],
          mode: 'queue',
        })
      } else if (live && key === 'turn/end' && replayingMarker && !reported) {
        reported = true
        void tree.input(branchId, {
          blocks: [{ type: 'raw', text: `<c5-rebuilt from="${cutoff}" folded="${seen.length}"/>` }],
          mode: 'queue',
        })
      }
    }
    await dispatcher.consume(
      'branch-part-fixture',
      ['input/message', 'turn/start', 'turn/end'],
      async () => {
        cutoff = BigInt(await dispatcher.committed())
        if (cutoff > 0n) {
          const page = await timeline.query({
            scope: 'branch',
            include_publishing: true,
            from_sequence: 1n,
            order_by: 'sequence',
            direction: 'ascending',
            limit: cutoff,
            offset: 0n,
          })
          for (const event of page.events) step(event, false)
        }
        replayingMarker = written
        return cutoff
      },
      async event => step(event, true),
    )
  },
})

export default partRow('branch', part)
