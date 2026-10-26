import { definePlugin } from '@arcships/rutis'
import { partRow } from './part-row.ts'
import { part } from './session-part.ts'

// The Session part of `session-part.ts`, gated on the process owner's
// `exampleProcess`.
export default partRow('session', definePlugin({
  ...part,
  name: 'ProcessDependentSessionFixture',
  inject: ['exampleProcess', ...part.inject],
}))
