import { definePlugin } from '@arcships/rutis'
import part from './session-part.ts'

export default definePlugin({
  name: 'ProcessDependentSessionFixture',
  inject: ['exampleProcess'],
  provides: part.provides,
  apply: part.apply,
})
