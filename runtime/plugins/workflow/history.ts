import type { Sessions } from '../../../npm/plugin.js'
import { bounded, type Stage } from './store.ts'

/** One stage's published prefix, keyed by the receipt of its own input. */
export class StageHistory {
  readonly sessions: Sessions
  readonly sessionId: string
  readonly inputId: string
  after = 0
  through: number | undefined
  applied = false
  consumed = false
  deleted = false
  interrupted = false
  turn: string | undefined
  text = ''
  outcome: Pick<Stage, 'state' | 'reason' | 'result_text'> | undefined

  constructor(sessions: Sessions, sessionId: string, inputId: string) {
    this.sessions = sessions
    this.sessionId = sessionId
    this.inputId = inputId
  }

  async read(): Promise<boolean> {
    for (;;) {
      const page = await this.sessions.history({ sessionId: this.sessionId, branchId: 'main', after: this.after,
        ...(this.through === undefined ? {} : { through: this.through }), limit: 256,
      })
      if (!page.ok) {
        if (page.error.code === 'reloading') return false
        if (page.error.code === 'session_closed' || page.error.code === 'branch_missing') {
          if (!this.outcome) this.outcome = { state: 'failed', reason: 'session_closed' }
          return true
        }
        if (!this.outcome) this.outcome = { state: 'failed', reason: page.error.code }
        return true
      }
      this.through = page.value.through
      for (const event of page.value.events) {
        this.after = event.sequence
        const { type, name, data } = event
        if (type === 'input' && data.input_id === this.inputId) {
          if (name === 'applied') this.applied = true
          if (name === 'deleted') this.deleted = true
          if (name === 'consumed') this.consumed = true
        }
        if (this.applied && !this.consumed && type === 'stop' && name === 'interrupt') this.interrupted = true
        if (!this.consumed || this.outcome) continue
        if (type === 'step' && name === 'model-start') {
          this.turn = data.turn_id as string
          this.text = ''
        }
        if (type === 'step' && name === 'text' && data.turn_id === this.turn) {
          this.text = bounded(this.text + (data.delta as string), 64 * 1024)
        }
        if (type === 'turn' && name === 'end') {
          if (data.status === 'completed') this.outcome = { state: 'completed', result_text: this.text }
          else if (data.status === 'failed') this.outcome = { state: 'failed', reason: data.error as string }
          else this.outcome = { state: 'stopped', reason: data.reason as string }
        }
      }
      if (!page.value.events.length || this.after >= this.through) {
        this.after = this.through
        this.through = undefined
        if (!this.consumed && (this.deleted || (this.applied && this.interrupted))) this.outcome = { state: 'stopped' }
        return true
      }
    }
  }
}
