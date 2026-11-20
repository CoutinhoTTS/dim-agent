import type { ToolCallRenderer, TimelineToolInlineParameter, TimelineToolDetailSection } from './types'
import type { ChatUiToolRenderBlock } from './types'
import { classifyTimelineToolCategory, getTimelineToolLabelKey } from '../tool-summary'
import {
  readArray,
  readStructuredRecord,
  readParamsRecord,
  readablePairLines,
  listSection,
  textParam,
  DETAIL_LINE_MAX,
  INPUT_PREVIEW_LINE_MAX,
} from './utils'

const isAgentTool = (toolName: string): boolean => {
  return classifyTimelineToolCategory(toolName) === 'subagent'
}

interface AgentParams {
  action?: string
  prompt?: string
  taskTitle?: string
  agentName?: string
  providerId?: string
  modelId?: string
  taskId?: string
  reason?: string
}

const readAgentParams = (block: ChatUiToolRenderBlock): AgentParams => {
  const params = readParamsRecord(block) ?? {}
  return {
    action: typeof params.action === 'string' ? params.action.trim().toLowerCase() : '',
    prompt: typeof params.prompt === 'string' ? params.prompt : undefined,
    taskTitle: typeof params.taskTitle === 'string' ? params.taskTitle : undefined,
    agentName: typeof params.agentName === 'string' ? params.agentName : undefined,
    providerId: typeof params.providerId === 'string' ? params.providerId : undefined,
    modelId: typeof params.modelId === 'string' ? params.modelId : undefined,
    taskId: typeof params.taskId === 'string' ? params.taskId : undefined,
    reason: typeof params.reason === 'string' ? params.reason : undefined,
  }
}

export const agentRenderer: ToolCallRenderer = {
  id: 'agent',

  match(toolName, _params) {
    return isAgentTool(toolName)
  },

  getLabelKey(toolName, params) {
    return getTimelineToolLabelKey(toolName, classifyTimelineToolCategory(toolName), params ?? undefined)
  },

  getPrimaryParameter(block): TimelineToolInlineParameter | null {
    const agentParams = readAgentParams(block)

    if (agentParams.action === 'create') {
      // chip shows agentName; primary parameter is only the task title
      return textParam(agentParams.taskTitle)
    }
    if (agentParams.action === 'cancel') {
      return textParam(agentParams.agentName)
    }
    // 'list' and 'query_provider_model' have no parameter
    return null
  },

  getDetailSections(block): TimelineToolDetailSection[] {
    const agentParams = readAgentParams(block)
    const result = readStructuredRecord(block.toolResult)
    const sections: TimelineToolDetailSection[] = []

    if (agentParams.action === 'create') {
      // Show prompt text
      const prompt = agentParams.prompt
      if (prompt) {
        sections.push({
          id: 'agent-prompt',
          kind: 'preview',
          lines: [prompt],
          truncated: false,
        })
      }
    } else if (agentParams.action === 'list') {
      // Show agent list from result
      const agents = readArray(result, ['agents'])
      if (agents) {
        const list = listSection('agents', agents, DETAIL_LINE_MAX)
        if (list) sections.push(list)
      }
    } else if (agentParams.action === 'query_provider_model') {
      // Show model list from result
      const text = typeof result?.text === 'string' ? result.text : null
      if (text) {
        sections.push({
          id: 'models',
          kind: 'summary',
          lines: text.split('\n').filter(Boolean),
          truncated: false,
        })
      }
    }
    // 'cancel' has no detail

    return sections
  },

  hasDetails(block): boolean {
    return this.getDetailSections(block).length > 0
  },

  getInputPreview(params): string[] {
    return readablePairLines(
      params,
      ['agentName', 'taskTitle', 'modelId', 'prompt', 'goal', 'description', 'task', 'reason'],
      INPUT_PREVIEW_LINE_MAX,
    )
  },
}
