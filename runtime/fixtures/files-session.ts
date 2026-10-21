import { definePlugin } from '@arcships/rutis'
import type { PluginFiles, SessionMethodsContribution } from '../../npm/plugin.js'

export default definePlugin({
  name: 'PublicFilesSessionFixture',
  inject: ['pluginFiles'],
  provides: { 'sessionMethods:session.files': { describe: 'async', call: 'async' } },
  async apply(ctx) {
    const scope = await ctx.use<PluginFiles>('pluginFiles').open()
    ctx.effect(() => scope.close())
    const methods: SessionMethodsContribution = {
      async describe() {
        return ['read', 'write'].map(name => ({
          name: `session.files.${name}`, mutating: name === 'write',
          interruption: 'drain', progress: false,
        }))
      },
      async call(call) {
        const result = await scope.withLock('public-files/state.json', async file =>
          call.method === 'session.files.write'
            ? await file.replace('public Session text')
            : await file.read())
        return { kind: 'value', value: result }
      },
    }
    ctx.provide('sessionMethods:session.files', methods)
  },
})
