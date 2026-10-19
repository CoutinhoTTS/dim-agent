import { definePlugin } from '@arcships/rutis'
import type { AuxiliaryModel } from '../../npm/plugin.d.ts'
import provides from './auxiliary-model.json' with { type: 'json' }

export default definePlugin({
  name: 'AuxiliaryModelFixture',
  inject: ['auxiliaryModel'],
  provides,
  apply(ctx) {
    const auxiliary = ctx.use<AuxiliaryModel>('auxiliaryModel')
    ctx.provide('sessionMethods:session.auxiliary', {
      async describe() {
        return [{ name: 'session.auxiliary.complete', mutating: false, interruption: 'report', progress: false }]
      },
      async call(call: { params: { request: Parameters<AuxiliaryModel['complete']>[0] } }) {
        try {
          return { kind: 'value', value: await auxiliary.complete(call.params.request) }
        } catch (error) {
          if (!(error instanceof Error) || !('code' in error)) throw error
          return { kind: 'value', value: { error: { code: error.code, message: error.message } } }
        }
      },
    })
  },
})
