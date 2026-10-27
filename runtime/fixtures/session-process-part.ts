import { definePlugin } from '@arcships/rutis'
import { partRow } from './part-row.ts'
import { part } from './session-part.ts'

export default partRow('session', definePlugin({
  ...part,
  name: 'ProcessDependentSessionFixture',
  inject: ['exampleProcess', ...part.inject],
}))
