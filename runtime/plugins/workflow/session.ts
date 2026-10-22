import { definePlugin } from '@arcships/rutis'
import type { ToolContribution } from '../../../npm/plugin.js'
import { toolRequest, type WorkflowRuns } from './requests.ts'

export default definePlugin({
  name: 'WorkflowSession',
  inject: ['workflowRuns'],
  provides: { 'toolContribution:workflow': { declaration: 'sync', execute: 'async' } },
  apply(ctx) {
    const owner = ctx.use<WorkflowRuns>('workflowRuns')
    let closed = false
    ctx.effect(() => { closed = true })
    const tool: ToolContribution = {
      declaration() {
        return { name: 'workflow', description: 'List workflow definitions, start a run, inspect stored stage results, or stop a run.',
          source: 'extension', risk: 'workflow',
          inputSchema: { type: 'object', required: ['action'], additionalProperties: false, properties: {
            action: { type: 'string', enum: ['list', 'start', 'status', 'stop'] },
            name: { type: 'string' }, source: { type: 'string', enum: ['project', 'user', 'plugin'] },
            params: { type: 'object', additionalProperties: { type: 'string' } }, run_id: { type: 'string' },
          } },
        }
      },
      async execute(call, cancellation) {
        if (closed) return { content: 'workflow: service_reloading', isError: true }
        if (call.config.interactive === false) return { content: 'workflow is available only in interactive Sessions.', isError: true }
        const request = toolRequest.safeParse(call.arguments)
        if (!request.success) return { content: `workflow: invalid_params\n${request.error.message}`, isError: true }
        let cancelled = false
        void cancellation.then(() => { cancelled = true }, () => { cancelled = true })
        await Promise.resolve()
        if (closed || cancelled) return { content: 'workflow: cancelled before handover', isError: true }
        const reply = await owner.call({ ...request.data, caller: {
          sessionId: call.sessionId, branchId: call.branchId, callId: call.callId, cwd: call.config.cwd,
        } })
        if ('code' in reply) return {
          content: reply.code === 'invalid_definition' ? 'workflow: invalid_definition. Use list to inspect the definition issues.' : `workflow: ${reply.code}`,
          isError: true,
        }
        return { content: reply.content, structuredContent: reply.structured }
      },
    }
    ctx.provide('toolContribution:workflow', tool)
  },
})
