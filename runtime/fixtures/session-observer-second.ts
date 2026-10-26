import provides from './session-observer-second.json' with { type: 'json' }
import { partRow } from './part-row.ts'
import { observer } from './session-observer.ts'

export default partRow('session', observer('session.second', provides))
