import { z } from 'zod'
import type { JsonObject } from '../../../npm/plugin.js'
import { id, managementJob, patchSchema, timestamp, type Caller } from './data.ts'

export const unloadingBeforeCall = 'tool provider generation is unloading; this call was not made. The tool is offered again once its provider is active; a later request uses the current provider.'
export const unloadingAfterCall = "tool provider generation is unloading; this call may already have taken effect. Check the operation's state before repeating it with the current provider."

export function bounded(text: string, limit: number): string {
  const bytes = Buffer.from(text)
  if (bytes.length <= limit) return text
  let end = limit
  while ((bytes[end] & 0xc0) === 0x80) end--
  return bytes.subarray(0, end).toString('utf8')
}

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
  import_legacy: z.strictObject({ source_id: id, fingerprint: z.string().regex(/^[0-9a-f]{64}$/),
    job: managementJob.omit({ id: true, agent_id: true }).required({ name: true, schedule: true, prompt: true, cwd: true, mode: true, enabled: true }),
    enabled_at: timestamp.optional(),
  }),
  delete: z.strictObject({ job_id: id }),
  set_enabled: z.strictObject({ job_id: id, enabled: z.boolean() }),
  run_now: z.strictObject({ job_id: id }),
  history: z.strictObject(history),
}
export type JobRequest = z.infer<typeof toolRequest> & { caller: Caller }
export type JobReply = { content: string; records: JsonObject } | { code: string; message: string }
export interface ScheduleJobs { call(request: JobRequest): Promise<JobReply> }
