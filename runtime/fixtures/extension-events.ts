import { definePlugin } from '@arcships/rutis'
import type { BranchEvent, BranchServices } from '../../npm/plugin.js'

// A branch part that writes events of its own domain (ADR 001, issue #2).
// After the first live turn end it appends `fixture/turn-seen` through
// `extensionEvents`, recording what the port refused: re-appending the core
// `turn/end` it received, and closing the Timeline. It subscribes to its own
// domain like any other key, and on its event writes a marked input with what
// it received. Rebuilding from history folds its own events, so a restored
// generation does not append again.
export default definePlugin({
  name: 'ExtensionEventsFixture',
  provides: {},
  inject: ['dispatcher', 'extensionEvents', 'timelineIdentity', 'treeCommands'],
  async apply(ctx) {
    const dispatcher = ctx.use<BranchServices['dispatcher']>('dispatcher')
    const events = ctx.use<BranchServices['extensionEvents']>('extensionEvents')
    const tree = ctx.use<BranchServices['treeCommands']>('treeCommands')
    const { branchId } = await ctx.use<BranchServices['timelineIdentity']>('timelineIdentity').get()
    let appended = false
    // Set when the rebuild folded this part's own event: a restored generation.
    let restored = false
    let reported = false
    let cutoff = 0n
    let rebuilt!: () => void
    const ready = new Promise<void>(resolve => { rebuilt = resolve })
    const attempt = async (call: () => Promise<unknown>) => {
      try {
        await call()
        return 'accepted'
      } catch (error) {
        return `rejected: ${(error as Error).message}`
      }
    }
    const step = async (event: BranchEvent, live: boolean) => {
      const key = `${event.type}/${event.name}`
      if (key === 'fixture/turn-seen') {
        appended = true
        if (!live) restored = true
        if (live) {
          await tree.input(branchId, {
            blocks: [{ type: 'raw', text: `<ext-seen key="${key}" data=${JSON.stringify(JSON.stringify(event.data))}/>` }],
            mode: 'queue',
          })
        }
      } else if (live && key === 'turn/end' && !appended) {
        appended = true
        // A core event that matches its contract is still not this part's to write.
        const reserved = await attempt(() => events.append({ type: event.type, name: event.name, data: event.data }))
        const close = await attempt(() => (events as unknown as { close(): Promise<void> }).close())
        await events.append({
          type: 'fixture',
          name: 'turn-seen',
          data: { turn: event.data, reserved, close, methods: Object.keys(events) },
        })
      } else if (live && key === 'turn/end' && restored && !reported) {
        reported = true
        await tree.input(branchId, {
          blocks: [{ type: 'raw', text: `<ext-rebuilt appended="${appended}"/>` }],
          mode: 'append',
        })
      }
    }
    await dispatcher.listen('extension-events-fixture', ['turn/end', 'fixture/*'], async event => {
      await ready
      if (BigInt(event.sequence) > cutoff) await step(event, true)
    })
    cutoff = BigInt(await dispatcher.committed())
    if (cutoff > 0n) for (const event of await dispatcher.local(0, cutoff)) await step(event, false)
    rebuilt()
  },
})
