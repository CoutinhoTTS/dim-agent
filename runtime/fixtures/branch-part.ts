import { definePlugin } from '@arcships/rutis'
import { partRow } from './part-row.ts'
import type { BranchEvent, Services } from '../../npm/plugin.js'

export const part = definePlugin({
  name: 'BranchPartFixture',
  provides: {},
  inject: ['dispatcher', 'timelineIdentity', 'treeCommands'],
  async apply(ctx) {
    const dispatcher = ctx.use<Services['dispatcher']>('dispatcher')
    const tree = ctx.use<Services['treeCommands']>('treeCommands')
    const { branchId } = await ctx.use<Services['timelineIdentity']>('timelineIdentity').get()
    const seen: string[] = []
    let written = false
    let reported = false
    let replayingMarker = false
    let cutoff = 0n
    let rebuilt!: () => void
    const ready = new Promise<void>(resolve => { rebuilt = resolve })
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
    await dispatcher.listen('branch-part-fixture', ['input/message', 'turn/start', 'turn/end'], async event => {
      await ready
      if (BigInt(event.sequence) > cutoff) step(event, true)
    })
    cutoff = BigInt(await dispatcher.committed())
    if (cutoff > 0n) for (const event of await dispatcher.local(0, cutoff)) step(event, false)
    replayingMarker = written
    rebuilt()
  },
})

export default partRow('branch', part)
