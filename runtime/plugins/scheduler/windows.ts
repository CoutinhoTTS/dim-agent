import { CronExpressionParser } from 'cron-parser'
import type { Job } from './data.ts'

/** Derive windows from the enabled anchor and durable run history, never a timer counter. */
export function windowAfter(job: Job, after: number): number | undefined {
  const schedule = job.schedule
  if ('at' in schedule) {
    const instant = Date.parse(schedule.at)
    return instant > after ? instant : undefined
  }
  const anchor = Date.parse(job.enabled_at!)
  if ('every_minutes' in schedule) {
    const period = schedule.every_minutes * 60_000
    return anchor + Math.max(0, Math.floor((after - anchor) / period) + 1) * period
  }
  const iterator = CronExpressionParser.parse(schedule.cron, { currentDate: Math.max(after, anchor - 1), tz: schedule.tz })
  for (;;) {
    const next = iterator.next()
    // cron-parser advances nonexistent wall times; Scheduler skips those windows.
    if (iterator.includesDate(next)) return next.getTime()
  }
}

export function latestWindow(job: Job, now: number): number {
  const schedule = job.schedule
  if ('at' in schedule) return Date.parse(schedule.at)
  if ('every_minutes' in schedule) {
    const anchor = Date.parse(job.enabled_at!)
    const period = schedule.every_minutes * 60_000
    return anchor + Math.floor((now - anchor) / period) * period
  }
  const iterator = CronExpressionParser.parse(schedule.cron, { currentDate: now + 1, tz: schedule.tz })
  for (;;) {
    const previous = iterator.prev()
    if (iterator.includesDate(previous)) return previous.getTime()
  }
}
