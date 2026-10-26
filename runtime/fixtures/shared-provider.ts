import { definePlugin } from '@arcships/rutis'
import { partRow } from './part-row.ts'

// Provides a Session name a row lists in `instanceNames`: each Session has
// its own copy, and one provider per name per instance (rutis usage §3.1).
export const part = definePlugin({
  name: 'SharedProviderFixture',
  provides: { exampleShared: { ping: 'sync' } },
  apply(ctx) {
    ctx.provide('exampleShared', { ping: () => 'pong' })
  },
})

export default partRow('session', part)
