import { ProtocolError, readPermissionPolicy } from "./read"
import type { ModelId, PermissionPolicy, PermissionPreset } from "./protocol"

export type Schedule = { every_minutes: number } | { at: string } | { cron: string; tz?: string }
type Origin = "user" | { tool: { session_id: string; branch_id: string } }
export type ScheduledRun = {
  run_id: string; job_id: string; state: string; created_at: string; ended_at?: string
  detail: string; session_id?: string; policy?: PermissionPolicy
}
export type ScheduledJob = {
  id: string; name: string; prompt: string; cwd: string; schedule: Schedule; enabled: boolean
  mode: "create" | "reuse"; model?: ModelId; policy?: PermissionPreset; max_run_minutes: number
  agent_id?: string; session_id?: string; revision: number; created_by: Origin; policy_cap: PermissionPolicy
  next_window?: string; schedule_error?: string; last_run?: ScheduledRun
}
export type JobInput = Pick<ScheduledJob, "name" | "prompt" | "cwd" | "schedule" | "mode" | "max_run_minutes" | "enabled" | "model" | "policy" | "agent_id" | "session_id">
type Authority = { origin: Origin; cap: PermissionPolicy; effective_policy: PermissionPolicy; report_to: { session_id: string; branch_id: string } | null }

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new ProtocolError("Invalid Scheduler reply: expected object")
  return value as Record<string, unknown>
}
function string(value: unknown): string {
  if (typeof value !== "string") throw new ProtocolError("Invalid Scheduler reply: expected string")
  return value
}
function integer(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) throw new ProtocolError("Invalid Scheduler reply: expected nonnegative integer")
  return value
}
function optional<T>(value: unknown, read: (value: unknown) => T): T | undefined { return value === undefined || value === null ? undefined : read(value) }
function list<T>(value: unknown, read: (value: unknown) => T): T[] {
  if (!Array.isArray(value)) throw new ProtocolError("Invalid Scheduler reply: expected array")
  return value.map(read)
}
function preset(value: unknown): PermissionPreset {
  if (value === "read-only" || value === "workspace-write" || value === "full-access") return value
  throw new ProtocolError("Invalid Scheduler permission preset")
}
function policy(value: unknown): PermissionPolicy { return readPermissionPolicy(value, "schedule.policy") }
function target(value: unknown): { session_id: string; branch_id: string } {
  const v = object(value)
  return { session_id: string(v.session_id), branch_id: string(v.branch_id) }
}
function origin(value: unknown): Origin { return value === "user" ? value : { tool: target(object(value).tool) } }
function schedule(value: unknown): Schedule {
  const v = object(value)
  if ("every_minutes" in v) return { every_minutes: integer(v.every_minutes) }
  if ("at" in v) return { at: string(v.at) }
  return { cron: string(v.cron), ...(v.tz == null ? {} : { tz: string(v.tz) }) }
}
export function readScheduledRun(value: unknown): ScheduledRun {
  const v = object(value)
  return { run_id: string(v.run_id), job_id: string(v.job_id), state: string(v.state), created_at: string(v.created_at), ended_at: optional(v.ended_at, string), detail: string(v.detail), session_id: optional(v.session_id, string), policy: optional(v.policy, policy) }
}
function job(value: unknown): ScheduledJob {
  const v = object(value)
  if (typeof v.enabled !== "boolean" || (v.mode !== "create" && v.mode !== "reuse")) throw new ProtocolError("Invalid Scheduler job state")
  return {
    id: string(v.id), name: string(v.name), prompt: string(v.prompt), cwd: string(v.cwd), schedule: schedule(v.schedule), enabled: v.enabled,
    mode: v.mode, model: optional(v.model, (value) => { const m = object(value); return { providerId: string(m.providerId), modelId: string(m.modelId) } }),
    policy: optional(v.policy, preset), max_run_minutes: integer(v.max_run_minutes), agent_id: optional(v.agent_id, string), session_id: optional(v.session_id, string),
    revision: integer(v.revision), created_by: origin(v.created_by), policy_cap: policy(v.policy_cap), next_window: optional(v.next_window, string), schedule_error: optional(v.schedule_error, string), last_run: optional(v.last_run, readScheduledRun),
  }
}
export function readScheduledJobs(value: unknown): ScheduledJob[] { return list(object(value).jobs, job) }
export function readScheduledHistory(value: unknown): ScheduledRun[] { return list(object(value).runs, readScheduledRun) }
export function readScheduledJob(value: unknown): ScheduledJob { return job(object(value).job) }
export function readScheduledSave(value: unknown): { job: ScheduledJob; transition: { from: Authority | null; to: Authority } } {
  const v = object(value), transition = object(v.transition)
  const authority = (value: unknown): Authority => {
    const a = object(value)
    return { origin: origin(a.origin), cap: policy(a.cap), effective_policy: policy(a.effective_policy), report_to: a.report_to === null ? null : target(a.report_to) }
  }
  return { job: job(v.job), transition: { from: transition.from === null ? null : authority(transition.from), to: authority(transition.to) } }
}
export function readScheduledDelete(value: unknown): string { return string(object(value).job_id) }
