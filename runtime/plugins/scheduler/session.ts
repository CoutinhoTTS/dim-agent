import { definePlugin } from '@arcships/rutis'
import { z } from 'zod'
import type { GuideEntries, JsonSchema, ToolRegistry } from '../../../npm/plugin.js'
import { patchSchema } from './data.ts'
import { bounded, toolRequest, unloadingBeforeCall, unloadingAfterCall, type JobReply, type ScheduleJobs } from './requests.ts'

const jobSchema = z.toJSONSchema(patchSchema) as JsonSchema
jobSchema.properties!.schedule.type = 'object'

// The dim-scheduler entry (Scheduler §5.2): both kinds it writes, verbatim.
const ENTRY = '<dim-scheduler kind="scheduled-run" job="nightly-build" run="run-7f2">: a scheduled job\'s saved prompt, delivered when the job runs, and <dim-scheduler kind="report" job="nightly-build" run="run-7f2">: how that run ended. job names the job, run names the run, in both. In the scheduled-run tag the body is the job\'s name, and the message after the tag is the prompt saved for the job: the request this run carries out, in the operator\'s channel. In the report tag the body states the run\'s outcome and, when known, the session it ran in: the scheduler\'s account of how the run ended, information, not a request.'

/// The `schedule` declaration's description in force when the factory
/// configuration sets none.
const DESCRIPTION = 'Manage scheduled jobs across this host, including other workspaces. Every create/update, even a rename, replaces the saving branch and authority cap; future reports go to this branch. An omitted policy keeps its stored value, whose default can change with the saving origin. Inspect the old-to-new authority transition in the result.'

export default (description: string = DESCRIPTION) => definePlugin({
  name: 'SchedulerSession', inject: ['scheduleJobs', 'toolRegistry', 'guideEntries'],
  provides: {},
  async apply(ctx) {
    const owner = ctx.use<ScheduleJobs>('scheduleJobs')
    let closing = false
    ctx.effect(() => { closing = true })
    await ctx.use<GuideEntries>('guideEntries').register('session', { markers: ['dim-scheduler'], order: 110, text: ENTRY })
    await ctx.use<ToolRegistry>('toolRegistry').register(
      { name: 'schedule', source: 'extension', risk: 'persistent_configuration',
        description,
        inputSchema: { type: 'object', required: ['action'], additionalProperties: false, properties: {
          action: { type: 'string', enum: ['list', 'get', 'create', 'update', 'delete', 'set_enabled', 'run_now', 'history'] },
          job_id: { type: 'string' }, enabled: { type: 'boolean' }, limit: { type: 'integer', minimum: 1, maximum: 50 },
          job: jobSchema,
        } },
      },
      async (call, cancellation) => {
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
    )
  },
})
