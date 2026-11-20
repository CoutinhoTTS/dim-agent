import { describe, expect, it } from 'vitest'
import {
  agentRenderer,
  askUserRenderer,
  browserRenderer,
  buildTimelineToolTrace,
  commandRenderer,
  cuaRenderer,
  fileRenderer,
  formatAskUserAnswer,
  genericRenderer,
  mcpRenderer,
  readAskUserAnswer,
  readAskUserAnswers,
  readAskUserOptionLabels,
  readEditReplacements,
  readTodoSnapshotItems,
  readTodoSnapshotValue,
  searchRenderer,
  todosRenderer,
  waitRenderer,
  webRenderer,
  type ChatUiToolRenderBlock,
} from '../src'

const block = (overrides: Partial<ChatUiToolRenderBlock> = {}): ChatUiToolRenderBlock => ({
  id: 'block-1',
  kind: 'tool',
  status: 'complete',
  content: '',
  ...overrides,
})

describe('file renderer', () => {
  it('shows a read line range from the offset and limit', () => {
    expect(fileRenderer.getPrimaryParameter(block({
      toolName: 'read',
      toolParams: { path: '/repo/src/a.ts', offset: 10, limit: 5 },
    }))).toEqual({
      kind: 'i18n',
      key: 'toolDisplay.readLineRange',
      params: { file: 'a.ts', start: '10', end: '14' },
    })
  })

  it('labels an offloaded session blob as cached content instead of a blob filename', () => {
    expect(fileRenderer.getPrimaryParameter(block({
      toolName: 'read',
      toolParams: { path: 'sessions/session-1/blobs/files/blob_abc' },
    }))).toEqual({ kind: 'i18n', key: 'toolDisplay.readCachedContent' })
  })

  it('shows the base name for an edit target', () => {
    expect(fileRenderer.getPrimaryParameter(block({
      toolName: 'edit',
      toolParams: { path: '/repo/src/a.ts' },
    }))).toEqual({ kind: 'text', text: 'a.ts' })
  })
})

describe('command renderer', () => {
  it('uses the first command token as the primary parameter', () => {
    expect(commandRenderer.getPrimaryParameter(block({
      toolName: 'exec',
      toolParams: { command: 'pnpm build --filter app' },
    }))).toEqual({ kind: 'text', text: 'pnpm' })
  })

  it('labels a background command and keeps the command + output sections', () => {
    expect(commandRenderer.getLabelKey('exec', { runInBackground: true }))
      .toBe('toolDisplay.runCommandBackground')

    const sections = commandRenderer.getDetailSections(block({
      toolName: 'exec',
      toolParams: { command: 'pnpm build' },
      toolResult: { output: 'building\nbuilt' },
    }))
    expect(sections.map(section => [section.id, section.lines])).toEqual([
      ['command', ['pnpm build']],
      ['preview', ['building', 'built']],
    ])
  })
})

describe('search renderer', () => {
  it('treats a glob without a pattern as a file listing', () => {
    expect(searchRenderer.getLabelKey('glob', {})).toBe('toolDisplay.listFiles')
    expect(searchRenderer.getPrimaryParameter(block({
      toolName: 'glob',
      toolParams: { path: 'src' },
    }))).toEqual({ kind: 'text', text: 'src' })
  })

  it('lists result files in the detail section', () => {
    const sections = searchRenderer.getDetailSections(block({
      toolName: 'grep',
      toolParams: { pattern: 'needle' },
      toolResult: { structuredContent: { totalMatches: 2, matches: ['a.ts', 'b.ts'] } },
    }))
    expect(sections.map(section => section.id)).toEqual(['search-summary', 'matches'])
    expect(sections[1]!.lines).toEqual(['a.ts', 'b.ts'])
  })
})

describe('web renderer', () => {
  it('shows the host for a fetched url and the query for a search', () => {
    expect(webRenderer.getPrimaryParameter(block({
      toolName: 'web_fetch',
      toolParams: { url: 'https://example.com/a/b?x=1' },
    }))).toEqual({ kind: 'text', text: 'example.com' })

    expect(webRenderer.getPrimaryParameter(block({
      toolName: 'web_search',
      toolParams: { query: 'dim agent' },
    }))).toEqual({ kind: 'text', text: 'dim agent' })
  })
})

describe('todos renderer', () => {
  const items = [
    { id: '1', content: 'Ship the extractor', status: 'completed', priority: 'high' },
    { id: '2', content: 'Update the docs', status: 'pending', priority: 'low' },
  ]

  it('summarizes the completed/pending counts', () => {
    expect(todosRenderer.getPrimaryParameter(block({
      toolName: 'todowrite',
      toolParams: { todos: items },
    }))).toEqual({
      kind: 'i18n',
      key: 'toolDisplay.todosPending',
      params: { pending: 1, total: 2 },
    })
  })

  it('renders validated todo rows and keeps the parsed items on the section', () => {
    const [section] = todosRenderer.getDetailSections(block({
      toolName: 'todowrite',
      toolParams: { todos: items },
    }))

    expect(section).toMatchObject({ id: 'todos', kind: 'summary', truncated: false })
    expect(section!.lines).toEqual([
      '1. [x] Ship the extractor - completed - high',
      '2. [ ] Update the docs - pending - low',
    ])
    expect(section!.todos).toHaveLength(2)
  })

  it('falls back to a raw list when the snapshot is invalid', () => {
    const [section] = todosRenderer.getDetailSections(block({
      toolName: 'todowrite',
      toolParams: { todos: [{ id: '1', content: 'Missing fields' }] },
    }))

    expect(section!.id).toBe('todos')
    expect(section!.todos).toBeUndefined()
    expect(section!.lines.length).toBeGreaterThan(0)
  })
})

describe('todo snapshot reader', () => {
  it('reads a nested snapshot and closes duplicate/invalid input', () => {
    expect(readTodoSnapshotValue({ content: JSON.stringify({ todos: [
      { id: '1', content: 'A', status: 'pending', priority: 'high' },
    ] }) })).toMatchObject({ kind: 'snapshot' })

    expect(readTodoSnapshotItems([
      { id: '1', content: 'A', status: 'pending', priority: 'high' },
      { id: '1', content: 'B', status: 'pending', priority: 'high' },
    ])).toEqual({ kind: 'invalid', message: 'todo id "1" must be unique' })

    expect(readTodoSnapshotItems([{ id: '1', content: 'A', status: 'done', priority: 'high' }]))
      .toEqual({ kind: 'invalid', message: 'todos[0] has an invalid contract' })

    expect(readTodoSnapshotValue({})).toEqual({ kind: 'missing' })
  })
})

describe('ask-user renderer', () => {
  const questions = {
    questions: [{
      id: 'choice',
      header: 'Choice',
      question: 'Choose one',
      options: [{ label: 'A' }, { label: 'B' }],
      multiSelect: true,
      allowCustom: true,
    }],
  }

  it('summarizes array-shaped answers with the custom-answer marker', () => {
    expect(askUserRenderer.getPrimaryParameter(block({
      toolName: 'ask_user',
      toolParams: questions,
      toolResult: { structuredContent: {
        answers: [{ questionId: 'choice', selected: ['A'], custom: 'typed answer' }],
      } },
    }))).toEqual({ kind: 'text', text: 'A, ✎ typed answer' })
  })

  it('keeps the question count while waiting and the declined marker otherwise', () => {
    expect(askUserRenderer.getPrimaryParameter(block({
      toolName: 'ask_user',
      toolParams: questions,
    }))).toEqual({ kind: 'i18n', key: 'toolQuantity.questions', params: { count: 1 } })

    expect(askUserRenderer.getPrimaryParameter(block({
      toolName: 'ask_user',
      toolParams: questions,
      toolResult: { structuredContent: { declined: true } },
    }))).toEqual({ kind: 'i18n', key: 'toolAskUser.declined' })
  })

  it('reads answers by id or header and marks non-option answers', () => {
    const answers = readAskUserAnswers({
      answers: [{ questionId: 'choice', selected: ['B'] }],
    })
    expect(readAskUserAnswer(answers, { id: 'choice', header: 'Choice' }, 0, 'Choice')).toEqual(['B'])
    expect(readAskUserAnswer({ Choice: 'A' }, { question: 'No id' }, 0, 'Choice')).toBe('A')
    const labels = readAskUserOptionLabels(questions.questions[0]!)
    expect(formatAskUserAnswer('custom', labels)).toBe('✎ custom')
    expect(labels).toEqual(new Set(['A', 'B']))
  })

  it('renders the timed-out banner text', () => {
    const trace = buildTimelineToolTrace(block({
      toolName: 'ask_user',
      toolParams: questions,
      toolResult: { structuredContent: { code: 'ask_user_timed_out', waitedMs: 30_000 } },
    }))
    expect(trace.primaryParameter).toEqual({
      kind: 'i18n',
      key: 'toolAskUser.timedOut',
      params: { duration: '30.0s' },
    })
  })
})

describe('agent / wait renderers', () => {
  it('shows the task title for create and the prompt as a preview section', () => {
    const create = block({
      toolName: 'agent',
      toolParams: { action: 'create', taskTitle: 'Review parser', prompt: 'Review the parser' },
    })
    expect(agentRenderer.getPrimaryParameter(create)).toEqual({ kind: 'text', text: 'Review parser' })
    expect(agentRenderer.getDetailSections(create)).toEqual([{
      id: 'agent-prompt',
      kind: 'preview',
      lines: ['Review the parser'],
      truncated: false,
    }])

    expect(agentRenderer.getPrimaryParameter(block({
      toolName: 'agent',
      toolParams: { action: 'cancel', agentName: 'Planner' },
    }))).toEqual({ kind: 'text', text: 'Planner' })
  })

  it('renders wait outcomes from the result and the joined task name while running', () => {
    expect(waitRenderer.getPrimaryParameter(block({
      toolName: 'wait',
      toolParams: { taskId: 'task-1' },
      toolResult: { structuredContent: { reason: 'timeout', waitedMs: 65_000 } },
    }))).toEqual({
      kind: 'i18n',
      key: 'toolWait.timedOut',
      params: { duration: '1m 5s' },
    })

    expect(waitRenderer.getPrimaryParameter(block({
      toolName: 'wait',
      toolParams: { taskId: 'task-1' },
      backgroundTaskNames: { 'task-1': 'Renderer build' },
    }))).toEqual({
      kind: 'i18n',
      key: 'toolWait.waitingFor',
      params: { name: 'Renderer build' },
    })
    expect(waitRenderer.hasDetails(block())).toBe(false)
  })
})

describe('browser / cua renderers', () => {
  it('renders browser action parameters and the cdp detail section', () => {
    expect(browserRenderer.getPrimaryParameter(block({
      toolName: 'browser',
      toolParams: { action: 'scroll', direction: 'down' },
    }))).toEqual({ kind: 'i18n', key: 'toolBrowser.scroll', params: { direction: 'down' } })

    const [section] = browserRenderer.getDetailSections(block({
      toolName: 'browser',
      toolParams: { action: 'cdp', method: 'Runtime.evaluate', params: { a: 1 } },
      toolResult: { output: 'ok' },
    }))
    expect(section).toMatchObject({ id: 'browser-cdp', kind: 'summary' })
    expect(section!.lines).toEqual(['method: Runtime.evaluate', 'params: {"a":1}', 'ok'])
  })

  it('builds the computer-use action label and target', () => {
    expect(cuaRenderer.getPrimaryParameter(block({
      toolName: 'cua-driver__get_window_state',
      toolParams: { window_id: 'w1' },
      toolResult: { structuredContent: { title: 'Safari', elements: [1, 2] } },
    }))).toEqual({
      kind: 'i18n',
      key: 'toolCua.readWindow',
      params: { target: ' · Safari · 2' },
    })
  })
})

describe('mcp / generic renderers', () => {
  const mcpBlock = block({
    toolName: 'read',
    toolParams: { query: 'needle' },
    toolSource: { kind: 'mcp', mcpServerId: 'github', mcpToolName: 'search' },
  })

  it('shows the MCP server identity and its summary/param sections', () => {
    expect(mcpRenderer.getLabelText!(mcpBlock)).toBe('MCP · github/search')
    expect(mcpRenderer.getPrimaryParameter(mcpBlock)).toEqual({ kind: 'text', text: 'needle' })

    expect(mcpRenderer.getDetailSections(mcpBlock).map(section => section.id))
      .toEqual(['mcp-summary', 'mcp-params'])
  })

  it('keeps a generic tool diagnosable through params and result summaries', () => {
    const generic = block({
      toolName: 'third_party_tool',
      toolParams: { path: '/tmp/x' },
      toolResult: { structuredContent: { status: 'ok' } },
    })
    expect(genericRenderer.getPrimaryParameter(generic)).toEqual({ kind: 'text', text: 'third_party_tool' })
    expect(genericRenderer.getDetailSections(generic).map(section => section.id))
      .toEqual(['params', 'generic-summary'])
  })
})

describe('readEditReplacements', () => {
  it('reads the replacements of an edits call in order', () => {
    const result = readEditReplacements({
      edits: [
        { oldText: 'one', newText: 'first' },
        { oldText: 'two', newText: 'second', replaceAll: true },
      ],
    })

    expect(result.legacy).toBe(false)
    expect(result.malformed).toBe(false)
    expect(result.replacements).toEqual([
      { index: 0, oldText: 'one', newText: 'first', replaceAll: false },
      { index: 1, oldText: 'two', newText: 'second', replaceAll: true },
    ])
  })

  it('reads a recorded pre-#1482 call as one replacement', () => {
    const result = readEditReplacements({ oldText: 'before', newText: 'after', replaceAll: true })

    expect(result.legacy).toBe(true)
    expect(result.replacements).toEqual([
      { index: 0, oldText: 'before', newText: 'after', replaceAll: true },
    ])
  })

  it('never mixes an edits call with the top-level pair', () => {
    const result = readEditReplacements({
      oldText: 'legacy-before',
      newText: 'legacy-after',
      edits: [{ oldText: 'current-before', newText: 'current-after' }],
    })

    expect(result.conflict).toBe(true)
    expect(result.replacements).toEqual([
      { index: 0, oldText: 'current-before', newText: 'current-after', replaceAll: false },
    ])
  })

  it('keeps valid entries, reports a malformed list and closes index gaps', () => {
    const result = readEditReplacements({
      edits: [
        { oldText: 'first-before', newText: 'first-after' },
        { oldText: 'missing-new-text' },
        { oldText: 'third-before', newText: 'third-after' },
      ],
    })

    expect(result.malformed).toBe(true)
    expect(result.replacements).toEqual([
      { index: 0, oldText: 'first-before', newText: 'first-after', replaceAll: false },
      { index: 1, oldText: 'third-before', newText: 'third-after', replaceAll: false },
    ])
  })

  it('does not fall back to the top-level pair when an empty edits array is present', () => {
    const result = readEditReplacements({ edits: [], oldText: 'before', newText: 'after' })

    expect(result.malformed).toBe(true)
    expect(result.replacements).toEqual([])
  })

  it('shows each replacement in the file input preview', () => {
    const lines = fileRenderer.getInputPreview!({
      path: '/repo/src/a.ts',
      edits: [
        { oldText: 'old-one', newText: 'new-one' },
        { oldText: 'old-two', newText: 'new-two', replaceAll: true },
      ],
    })

    expect(lines).toContain('/repo/src/a.ts')
    expect(lines).toContain('- old-one')
    expect(lines).toContain('+ new-one')
    expect(lines).toContain('- old-two')
    expect(lines).toContain('+ new-two')
    expect(lines).toContain('  replaceAll')
  })
})
