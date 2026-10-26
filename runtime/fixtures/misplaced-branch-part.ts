import { partRow } from './part-row.ts'
import { part } from './branch-part.ts'

// The branch fixture registered as a Session part: it names branch services,
// so its copy cannot be built in a Session.
export default partRow('session', part)
