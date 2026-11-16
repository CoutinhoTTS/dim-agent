import { definePlugin } from '@arcships/rutis'
import type { ManagementMethods, HooksService } from '../../npm/plugin.js'

export default definePlugin({
  name: 'HooksServiceFixture',
  provides: {},
  inject: ['managementMethods', 'hooksService'],
  async apply(ctx) {
    const hooks = ctx.use<HooksService>('hooksService')
    await ctx.use<ManagementMethods>('managementMethods').register(
      'hooksprobe',
      ['register', 'read', 'withdraw'].map(action => ({
        name: `hooksprobe.${action}`, mutating: action !== 'read', interruption: 'report', progress: false,
      })),
      async call => {
        if (call.method === 'hooksprobe.register') {
          const params = call.params
          if (params === null || typeof params !== 'object' || Array.isArray(params) || typeof params.dir !== 'string' || params.document === undefined) throw new Error('expected dir and document')
          await hooks.register({ plugin: 'foreign-kit', dir: params.dir, document: params.document })
        } else if (call.method === 'hooksprobe.withdraw') {
          await hooks.withdraw({ plugin: 'foreign-kit' })
        }
        return { kind: 'value', value: { plugins: await hooks.plugins({}), refusal: hooks.refusal({ plugin: 'foreign-kit' }) } }
      },
    )
  },
})
