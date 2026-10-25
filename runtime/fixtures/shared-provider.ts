import { definePlugin } from '@arcships/rutis'

// Provides a name its row declares: each Session has its own copy, and
// one provider per name per instance (rutis usage §3.1).
export default definePlugin({
  name: 'SharedProviderFixture',
  provides: { exampleShared: { ping: 'sync' } },
  apply(ctx) {
    ctx.provide('exampleShared', { ping: () => 'pong' })
  },
})
