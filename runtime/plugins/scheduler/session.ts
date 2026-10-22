import { definePlugin } from '@arcships/rutis'
import { z } from 'zod'
import type { JsonSchema, ToolContribution } from '../../../npm/plugin.js'
import { patchSchema } from './data.ts'
import { bounded, toolRequest, unloadingBeforeCall, unloadingAfterCall, type JobReply, type ScheduleJobs } from './requests.ts'

const jobSchema = z.toJSONSchema(patchSchema) as JsonSchema
// Native tool schemas require a root type even when an object union uses anyOf.
jobSchema.properties!.schedule.type = 'object'

export default definePlugin({
  name: 'SchedulerSession', inject: ['scheduleJobs'],
  provides: { 'toolContribution:schedule': { declaration: 'sync', execute: 'async' } },
  apply(ctx) {
    const owner = ctx.use<ScheduleJobs>('scheduleJobs')
    let closing = false
    ctx.effect(() => { closing = true })
    const tool: ToolContribution = {
      declaration() {
        return { name: 'schedule', source: 'extension', risk: 'persistent_configuration',
          description: 'Manage scheduled jobs across this host, including other workspaces. Every create/update, even a rename, replaces the saving branch and authority cap; future reports go to this branch. An omitted policy keeps its stored value, whose default can change with the saving origin. Inspect the old-to-new authority transition in the result.',
          inputSchema: { type: 'object', required: ['action'], additionalProperties: false, properties: {
            action: { type: 'string', enum: ['list', 'get', 'create', 'update', 'delete', 'set_enabled', 'run_now', 'history'] },
            job_id: { type: 'string' }, enabled: { type: 'boolean' }, limit: { type: 'integer', minimum: 1, maximum: 50 },
            job: jobSchema,
          } },
        }
      },
      async execute(call, cancellation) {
        if (closing) return { content: unloadingBeforeCall, isError: true }
        if (call.config.interactive === false || call.config.collaborationRole === 'member') {
          return { content: 'schedule is available only in interactive leader Sessions.', isError: true }
        }
        const request = toolRequest.safeParse(call.arguments)
        if (!request.success) return { content: `schedule: schedule_invalid\n${bounded(request.error.message, 4 * 1024)}`, isError: true }
        let cancelled = false
        void cancellation.then(() => { cancelled = true }, () => { cancelled = true })
        await Promise.resolve()
        if (closing) return { content: unloadingBeforeCall, isError: true }
        if (cancelled) return { content: 'schedule: cancelled before handover', isError: true }
        let reply: JobReply
        try {
          reply = await owner.call({ ...request.data, caller: { sessionId: call.sessionId, branchId: call.branchId, cwd: call.config.cwd } })
        } catch (error) {
          if (closing) return { content: unloadingAfterCall, isError: true }
          throw error
        }
        if (closing) return { content: unloadingAfterCall, isError: true }
        return 'code' in reply ? { content: `schedule: ${reply.code}\n${reply.message}`, isError: true }
          : { content: reply.content, structuredContent: reply.records }
      },
    }
    ctx.provide('toolContribution:schedule', tool)
  },
})
