import { definePlugin } from '@arcships/rutis'
import { type InputMessage, changes } from './goal.ts'
import type { SessionMethods, ToolRegistry } from '../../../npm/plugin.js'

type TreeCommands = { input(id: string, input: InputMessage): Promise<{ sequence: number | bigint }> }
type Call = { branchId: string; arguments: { status?: unknown; summary?: unknown } }
type MethodCall = { method: string; params: { sessionId: string; branchId?: unknown; objective?: unknown; judge?: unknown } }
type Reply = { kind: 'value'; value: unknown } | { code: string; message: string }

const OBJECTIVE_LIMIT = 4000
const SUMMARY_LIMIT = 2000
const invalid = (message: string): Reply => ({ code: 'invalid_params', message })

const text = (value: unknown, limit: number) =>
  typeof value === 'string' && value.trim() !== '' && [...value].length <= limit ? value : undefined

// GoalPlugin (Goal.md §1, A.1, A.2): the tool and the methods validate only
// their own parameters and write one change input to the branch by id. They
// never read Goal's state; the branch part decides what a change does.
export default definePlugin({
  name: 'GoalSession',
  provides: {},
  inject: ['treeCommands', 'toolRegistry', 'sessionMethods'],
  async apply(ctx) {
    const tree = ctx.use<TreeCommands>('treeCommands')
    await ctx.use<ToolRegistry>('toolRegistry').register(
      {
        name: 'goal_update',
        description: 'Report that the current goal is completed or blocked.',
        inputSchema: {
          type: 'object',
          properties: {
            status: { type: 'string', enum: ['completed', 'blocked'] },
            summary: { type: 'string', minLength: 1, maxLength: SUMMARY_LIMIT },
          },
          required: ['status', 'summary'],
          additionalProperties: false,
        },
        source: 'extension',
        risk: 'session',
      },
      async (call: Call) => {
        const { status } = call.arguments
        const summary = text(call.arguments.summary, SUMMARY_LIMIT)
        if ((status !== 'completed' && status !== 'blocked') || !summary) {
          return { content: 'goal_update needs status "completed" or "blocked" and a summary of 1 to 2000 characters.', isError: true }
        }
        await tree.input(call.branchId, changes.report(call.branchId, status, summary))
        return { content: `goal_update recorded: ${status}.` }
      },
    )
    await ctx.use<SessionMethods>('sessionMethods').register(
      'session.goal',
      ['set', 'edit', 'pause', 'resume', 'clear'].map(name => ({
        name: `session.goal.${name}`, mutating: true, interruption: 'report', progress: false,
      })),
      async (call: MethodCall): Promise<Reply> => {
        const { branchId = 'main' } = call.params
        if (typeof branchId !== 'string' || branchId === '') return invalid('branchId must be a non-empty string')
        const op = call.method.slice('session.goal.'.length)
        let input: InputMessage
        if (op === 'set' || op === 'edit') {
          const objective = text(call.params.objective, OBJECTIVE_LIMIT)
          if (!objective) return invalid(`objective must be 1 to ${OBJECTIVE_LIMIT} characters`)
          if (op === 'edit') {
            input = changes.edit(branchId, objective)
          } else {
            const judge = call.params.judge ?? 'model'
            if (judge !== 'model' && judge !== 'evaluator') return invalid('judge must be "model" or "evaluator"')
            input = changes.set(branchId, objective, judge)
          }
        } else if (op === 'pause' || op === 'resume' || op === 'clear') {
          input = changes[op](branchId)
        } else {
          return invalid(`unknown method ${call.method}`)
        }
        const event = await tree.input(branchId, input)
        return { kind: 'value', value: { sequence: event.sequence } }
      },
    )
  },
})
