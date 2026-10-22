import { z } from 'zod'
import type { JsonObject } from '../../../npm/plugin.js'
import { id, managementJob, patchSchema, type Caller } from './data.ts'

export const unloadingBeforeCall = 'schedule: provider unavailable before handover; no call was made. Use the current provider once it is active.'
export const unloadingAfterCall = 'schedule: provider unavailable after handover; this call may already have taken effect. Check job and run history before repeating it.'

const history = { job_id: id, limit: z.number().int().min(1).max(50).default(20) }
export const toolRequest = z.discriminatedUnion('action', [
  z.strictObject({ action: z.literal('list') }),
  z.strictObject({ action: z.literal('get'), job_id: id }),
  z.strictObject({ action: z.literal('create'), job: patchSchema }),
  z.strictObject({ action: z.literal('update'), job_id: id, job: patchSchema }),
  z.strictObject({ action: z.literal('delete'), job_id: id }),
  z.strictObject({ action: z.literal('set_enabled'), job_id: id, enabled: z.boolean() }),
  z.strictObject({ action: z.literal('run_now'), job_id: id }),
  z.strictObject({ action: z.literal('history'), ...history }),
])
export const management = {
  list: z.strictObject({}),
  save: z.strictObject({ job: managementJob, revision: z.number().int().min(0).optional() }),
  delete: z.strictObject({ job_id: id }),
  set_enabled: z.strictObject({ job_id: id, enabled: z.boolean() }),
  run_now: z.strictObject({ job_id: id }),
  history: z.strictObject(history),
}
export type JobRequest = z.infer<typeof toolRequest> & { caller: Caller }
export type JobReply = { content: string; records: JsonObject } | { code: string; message: string }
export interface ScheduleJobs { call(request: JobRequest): Promise<JobReply> }
