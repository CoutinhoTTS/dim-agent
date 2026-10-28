import { z } from 'zod'
import { ScheduleError, type Schedule } from './data.ts'

export const recurrence = z.strictObject({ rrule: z.string().min(1), created_at: z.iso.datetime({ offset: true }), next_run_at: z.iso.datetime({ offset: true }).nullish() })
export type LegacyRecurrence = z.infer<typeof recurrence>

export function legacySchedule(input: LegacyRecurrence): { schedule: Schedule; anchor: string } {
  const created = new Date(input.created_at)
  created.setSeconds(0, 0)
  const anchor = created.toISOString()
  const fields = new Map<string, string>()
  for (const segment of input.rrule.trim().toUpperCase().replace(/^RRULE:/, '').split(';')) {
    const pair = segment.split('=')
    if (pair.length !== 2) throw new ScheduleError('schedule_invalid', 'Invalid legacy recurrence')
    const [key, value] = pair.map(value => value.trim())
    if (!['FREQ', 'INTERVAL', 'BYHOUR', 'BYMINUTE', 'BYDAY', 'BYMONTH', 'BYMONTHDAY', 'COUNT'].includes(key)
      || !value || fields.has(key)) throw new ScheduleError('schedule_invalid', 'Invalid legacy recurrence properties')
    fields.set(key, value)
  }
  const integer = (key: string, min: number, max: number, fallback?: number) => {
    const raw = fields.get(key)
    const value = raw === undefined ? fallback : /^\d+$/.test(raw) ? Number(raw) : undefined
    if (value === undefined || !Number.isInteger(value) || value < min || value > max) throw new ScheduleError('schedule_invalid', 'Invalid legacy recurrence integer')
    return value
  }
  const frequency = fields.get('FREQ')
  const kinds: Record<string, { required: string[]; rejected: string[] }> = {
    MINUTELY: { required: [], rejected: ['BYHOUR', 'BYMINUTE', 'BYDAY', 'BYMONTH', 'BYMONTHDAY'] },
    HOURLY: { required: [], rejected: ['BYHOUR', 'BYDAY', 'BYMONTH', 'BYMONTHDAY'] },
    DAILY: { required: ['BYHOUR', 'BYMINUTE'], rejected: ['BYDAY', 'BYMONTH', 'BYMONTHDAY'] },
    WEEKLY: { required: ['BYDAY', 'BYHOUR', 'BYMINUTE'], rejected: ['BYMONTH', 'BYMONTHDAY'] },
    MONTHLY: { required: ['BYMONTHDAY', 'BYHOUR', 'BYMINUTE'], rejected: ['BYDAY', 'BYMONTH'] },
    YEARLY: { required: ['BYMONTH', 'BYMONTHDAY', 'BYHOUR', 'BYMINUTE'], rejected: ['BYDAY'] },
  }
  const kind = frequency === undefined ? undefined : kinds[frequency]
  if (!kind || kind.required.some(key => !fields.has(key)) || kind.rejected.some(key => fields.has(key))) {
    throw new ScheduleError('schedule_invalid', 'Legacy recurrence properties do not match its frequency')
  }
  const interval = integer('INTERVAL', 1, 999, 1)
  const hour = integer('BYHOUR', 0, 23, created.getHours())
  const minute = integer('BYMINUTE', 0, 59, frequency === 'HOURLY' ? 0 : created.getMinutes())
  const month = integer('BYMONTH', 1, 12, created.getMonth() + 1)
  const day = integer('BYMONTHDAY', 1, 31, created.getDate())
  const weekdayNames = ['SU', 'MO', 'TU', 'WE', 'TH', 'FR', 'SA']
  const days = fields.get('BYDAY')?.split(',').map(day => weekdayNames.indexOf(day))
  if (days && (days.some(day => day < 0) || new Set(days).size !== days.length)) throw new ScheduleError('schedule_invalid', 'Invalid legacy weekdays')
  if (fields.has('COUNT')) {
    integer('COUNT', 1, 1)
    if (!input.next_run_at) throw new ScheduleError('schedule_import_unsupported', 'One-shot job has no persisted occurrence')
    return { schedule: { at: new Date(input.next_run_at).toISOString() }, anchor }
  }
  const slots = (first: number, end: number, step: number) => {
    const result: number[] = []
    for (let value = first; value < end; value += step) result.push(value)
    return result.join(',')
  }
  let cron: string
  if (frequency === 'MINUTELY' && 60 % interval === 0) cron = `${slots(created.getMinutes() % interval, 60, interval)} * * * *`
  else if (frequency === 'HOURLY' && 24 % interval === 0) cron = `${minute} ${slots(created.getHours() % interval, 24, interval)} * * *`
  else if (frequency === 'DAILY' && interval === 1) cron = `${minute} ${hour} * * *`
  else if (frequency === 'WEEKLY' && interval === 1) cron = `${minute} ${hour} * * ${days!.sort().join(',')}`
  else if (frequency === 'MONTHLY' && 12 % interval === 0) cron = `${minute} ${hour} ${day} ${slots(created.getMonth() % interval + 1, 13, interval)} *`
  else if (frequency === 'YEARLY' && interval === 1) cron = `${minute} ${hour} ${day} ${month} *`
  else throw new ScheduleError('schedule_import_unsupported', 'Recurrence interval has no equivalent native calendar schedule')
  return { schedule: { cron }, anchor }
}
