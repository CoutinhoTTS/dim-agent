import type { Sessions } from '../../../npm/plugin.js'
import type { Run } from './store.ts'

/** The published prefix following this run's own input; no idle wait or cross-Session subscription. */
export class RunHistory {
  readonly sessions: Sessions
  readonly sessionId: string
  readonly inputId: string
  after: number
  through: number | undefined
  applied = false
  consumed = false
  deleted = false
  interrupted = false
  outcome: Pick<Run, 'state' | 'detail'> | undefined

  constructor(sessions: Sessions, sessionId: string, inputId: string, after = 0) {
    this.sessions = sessions; this.sessionId = sessionId; this.inputId = inputId; this.after = after
  }

  async read(): Promise<boolean> {
    for (;;) {
      let page
      try {
        page = await this.sessions.history({ sessionId: this.sessionId, branchId: 'main', after: this.after,
          ...(this.through === undefined ? {} : { through: this.through }), limit: 256,
        })
      } catch (error) {
        console.error('Scheduler history', this.sessionId, error instanceof Error ? error.message : String(error))
        return false
      }
      if (!page.ok) {
        if (page.error.code === 'reloading') return false
        this.outcome ??= { state: 'failed', detail: page.error.code }
        return true
      }
      this.through = page.value.through
      for (const event of page.value.events) {
        this.after = event.sequence
        if (event.type === 'input' && event.data.input_id === this.inputId) {
          if (event.name === 'applied') this.applied = true
          if (event.name === 'consumed') this.consumed = true
          if (event.name === 'deleted') this.deleted = true
        }
        if (this.applied && !this.consumed && event.type === 'stop' && event.name === 'interrupt') this.interrupted = true
        if (this.consumed && !this.outcome && event.type === 'turn' && event.name === 'end') {
          this.outcome = event.data.status === 'completed' ? { state: 'completed', detail: 'completed' }
            : event.data.status === 'failed' ? { state: 'failed', detail: 'turn_failed' }
            : { state: 'cancelled', detail: 'interrupted' }
        }
      }
      if (!page.value.events.length || this.after >= this.through) {
        this.after = this.through
        this.through = undefined
        if (this.deleted && !this.consumed) this.outcome = { state: 'cancelled', detail: 'withdrawn' }
        return true
      }
    }
  }
}
