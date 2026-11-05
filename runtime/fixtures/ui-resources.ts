import { definePlugin } from '@arcships/rutis'
import type { SessionMethods, UiResources, ResourceRef } from '../../npm/plugin.js'
import { partRow } from './part-row.ts'

const part = definePlugin({
  name: 'UiResourceFixture',
  inject: ['sessionMethods', 'uiResources'],
  async apply(ctx, config: { path: string }) {
    const resources = ctx.use<UiResources>('uiResources')
    await ctx.use<SessionMethods>('sessionMethods').register('session.preview', [
      { name: 'session.preview.publish', mutating: true, interruption: 'report', progress: false },
      { name: 'session.preview.withdraw', mutating: true, interruption: 'report', progress: false },
    ], async call => {
      if (call.method === 'session.preview.publish') {
        const reference = await resources.publishFile({ path: config.path, mediaType: 'text/plain' })
        return { kind: 'value', value: { sessionId: reference.sessionId, id: reference.id } }
      }
      await resources.withdraw(call.params.reference as ResourceRef)
      return { kind: 'value', value: {} }
    })
  },
})
export default partRow('session', part)
