import { definePlugin } from '@arcships/rutis'
import { z } from 'zod'
import type { JsonObject, ManagementMethodsContribution, PluginFiles, SessionPermissions, Sessions, Settings } from '../../../npm/plugin.js'
import { ScheduleError } from './data.ts'
import { Scheduler } from './engine.ts'
import { Jobs } from './jobs.ts'
import { management, type ScheduleJobs } from './requests.ts'
import { RunStore } from './store.ts'
import { windowAfter } from './windows.ts'

export default definePlugin({
  name: 'Scheduler', config: z.toJSONSchema(z.strictObject({})),
  inject: ['sessions', 'sessionPermissions', 'settings', 'pluginFiles'],
  provides: { scheduleJobs: { call: 'async' }, 'managementMethods:schedule': { describe: 'async', call: 'async' } },
  async apply(ctx) {
    const jobs = new Jobs(ctx.use<Settings>('settings'), ctx.use<SessionPermissions>('sessionPermissions'))
    const scope = await ctx.use<PluginFiles>('pluginFiles').open()
    const scheduler = new Scheduler(jobs, ctx.use<Sessions>('sessions'), new RunStore(scope))
    ctx.effect(() => scheduler.close())
    await scheduler.restore()

    async function list(): Promise<JsonObject> {
      const { document } = await jobs.read()
      const runs = await scheduler.store.read()
      return { jobs: document.jobs.map(job => {
        const ownRuns = runs.filter(run => run.job_id === job.id).sort((a, b) => b.created_at.localeCompare(a.created_at) || b.run_id.localeCompare(a.run_id))
        const windows = ownRuns.filter(run => run.window !== 'manual').map(run => run.window).sort()
        const after = windows.length ? Date.parse(windows.at(-1)!) : Date.parse(job.enabled_at!) - 1
        const next = job.enabled ? windowAfter(job, after) : undefined
        return { ...job, next_window: next === undefined ? null : new Date(next).toISOString(), last_run: ownRuns[0] ?? null }
      }) }
    }
    const history = async (jobId: string, limit: number) => ({ runs: (await scheduler.store.read()).filter(run => run.job_id === jobId)
      .sort((a, b) => b.created_at.localeCompare(a.created_at) || b.run_id.localeCompare(a.run_id)).slice(0, limit) })
    async function remove(jobId: string) {
      const result = await jobs.delete(jobId)
      await scheduler.prune()
      return result
    }

    const owner: ScheduleJobs = {
      async call(request) {
        try {
          return await scheduler.track(async () => {
            let records: JsonObject
            if (request.action === 'list') records = await list()
            else if (request.action === 'get') records = { job: await jobs.get(request.job_id) }
            else if (request.action === 'create' || request.action === 'update') {
              records = await jobs.save({ ...request.job, ...(request.action === 'update' ? { id: request.job_id } : {}) }, request.caller)
            } else if (request.action === 'set_enabled') records = { job: await jobs.setEnabled(request.job_id, request.enabled) }
            else if (request.action === 'delete') records = await remove(request.job_id)
            else if (request.action === 'run_now') records = await scheduler.accept(request.job_id)
            else records = await history(request.job_id, request.limit)
            return { content: JSON.stringify(records), records }
          })
        } catch (error) {
          if (error instanceof ScheduleError) return { code: error.code, message: error.message }
          throw error
        }
      },
    }
    ctx.provide('scheduleJobs', owner)

    const methods: ManagementMethodsContribution = {
      async describe() {
        return ['list', 'save', 'delete', 'set_enabled', 'run_now', 'history'].map(name => ({
          name: `schedule.${name}`, mutating: name !== 'list' && name !== 'history',
          interruption: name === 'list' || name === 'history' ? 'report' : 'drain', progress: false,
        }))
      },
      async call(call, cancellation) {
        let cancelled = false
        void cancellation.then(() => { cancelled = true }, () => { cancelled = true })
        await Promise.resolve()
        if (cancelled) return { code: -32000, appError: 'service_reloading', message: 'Scheduler call cancelled before admission' }
        try {
          return await scheduler.track(async () => {
            let value: JsonObject
            switch (call.method) {
              case 'schedule.list': management.list.parse(call.params); value = await list(); break
              case 'schedule.save': {
                const input = management.save.parse(call.params)
                value = await jobs.save(input.job, undefined, input.revision)
                break
              }
              case 'schedule.delete': value = await remove(management.delete.parse(call.params).job_id); break
              case 'schedule.set_enabled': {
                const input = management.set_enabled.parse(call.params)
                value = { job: await jobs.setEnabled(input.job_id, input.enabled) }
                break
              }
              case 'schedule.run_now': value = await scheduler.accept(management.run_now.parse(call.params).job_id); break
              case 'schedule.history': {
                const input = management.history.parse(call.params)
                value = await history(input.job_id, input.limit)
                break
              }
              default: return { code: -32601, appError: 'unknown_method', message: 'Unknown Scheduler method' }
            }
            return { kind: 'value' as const, value }
          })
        } catch (error) {
          if (error instanceof z.ZodError) return { code: -32602, appError: 'schedule_invalid', message: error.message }
          if (error instanceof ScheduleError) return { code: -32000, appError: error.code, message: error.message }
          throw error
        }
      },
    }
    ctx.provide('managementMethods:schedule', methods)
    ctx.on('settings.changed', (event: { source: string }) => { if (event.source === 'schedule') scheduler.changed() })
    scheduler.startTimer()
  },
})
