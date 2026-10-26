import { z } from 'zod'
import type { FileScope } from '../../../npm/plugin.js'
import { address, id, policy, ScheduleError, timestamp } from './data.ts'

const runSchema = z.strictObject({
  run_id: id, job_id: id, window: z.union([z.literal('manual'), timestamp]),
  state: z.enum(['skipped', 'missed', 'launching', 'submitted', 'running', 'completed', 'failed', 'cancelled', 'timed_out', 'lost']),
  deadline_at: timestamp.optional(), session_id: id.optional(), session_origin: z.enum(['launch', 'reuse']).optional(),
  policy: policy.optional(), input_event_id: id.optional(), submission_id: id, report_to: address.optional(),
  created_at: timestamp, ended_at: timestamp.optional(), detail: z.string(),
}).superRefine((run, context) => {
  const sessionFields = [run.session_id, run.session_origin, run.policy].filter(value => value !== undefined).length
  const accepted = run.state !== 'skipped' && run.state !== 'missed'
  if (terminal(run) !== (run.ended_at !== undefined)
    || (accepted && !run.deadline_at)
    || (run.state === 'missed' && run.deadline_at !== undefined)
    || (sessionFields !== 0 && sessionFields !== 3)
    || (['submitted', 'running', 'completed', 'cancelled'].includes(run.state) && (!run.input_event_id || sessionFields !== 3))
    || (run.state === 'launching' && run.input_event_id !== undefined)
    || (run.state === 'missed' && (sessionFields !== 0 || run.input_event_id !== undefined || run.report_to !== undefined))
    || (run.state === 'skipped' && (run.input_event_id !== undefined || (!run.deadline_at && (sessionFields !== 0 || run.report_to !== undefined))))
    || (run.input_event_id !== undefined && sessionFields !== 3)
    || run.submission_id !== `scheduler:${run.run_id}`) {
    context.addIssue({ code: 'custom', message: 'Inconsistent Scheduler run record' })
  }
})
const storedSchema = z.strictObject({ version: z.literal(1), runs: z.array(runSchema) }).superRefine((stored, context) => {
  if (new Set(stored.runs.map(run => run.run_id)).size !== stored.runs.length) {
    context.addIssue({ code: 'custom', message: 'Duplicate Scheduler run identity' })
  }
})
export type Run = z.infer<typeof runSchema>
export function terminal(run: { state: string }): boolean { return !['launching', 'submitted', 'running'].includes(run.state) }

/** One decoder for the committed inner-v1 file; native transactions own serialization. */
export class RunStore {
  readonly scope: FileScope
  constructor(scope: FileScope) { this.scope = scope }
  async read(): Promise<Run[]> { return this.access(runs => runs, false) }
  async update<T>(change: (runs: Run[]) => T): Promise<T> { return this.access(change, true) }

  private async access<T>(change: (runs: Run[]) => T, write: boolean): Promise<T> {
    let value!: T
    let problem: unknown
    const outcome = await this.scope.withLock('scheduler/runs.json', async file => {
      try {
        const text = await file.read()
        if (!text.ok) throw new ScheduleError(text.error)
        let runs: Run[]
        if (text.value === null) runs = []
        else {
          try { runs = storedSchema.parse(JSON.parse(text.value)).runs }
          catch { throw new ScheduleError('invalid_data') }
        }
        value = change(runs)
        if (write) {
          const replaced = await file.replace(JSON.stringify({ version: 1, runs }))
          if (!replaced.ok) throw new ScheduleError(replaced.error)
        }
      } catch (error) { problem = error }
      return null
    })
    if (!outcome.ok) throw new ScheduleError(outcome.error)
    if (problem !== undefined) throw problem
    return value
  }
}
