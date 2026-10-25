import type { Context } from '@deepseek-ai/cordis'

// A plugin for rutis's Cordis 4 as written for Cordis, with no rutis or Dim
// SDK, mounted as a branch row. The names it injects resolve to its own
// branch's copies, so the marker it writes names its own Session and branch.
export const name = 'cordis-branch-fixture'
export const inject = ['dispatcher', 'timelineIdentity', 'treeCommands']

export async function apply(ctx: Context) {
  const { sessionId, branchId } = await ctx.get('timelineIdentity').get()
  let written = false
  await ctx.get('dispatcher').listen('cordis-branch-fixture', ['turn/end'], async () => {
    if (written) return
    written = true
    await ctx.get('treeCommands').input(branchId, {
      blocks: [{ type: 'raw', text: `<cordis-marker session="${sessionId}" branch="${branchId}"/>` }],
      mode: 'queue',
    })
  })
}
