import assert from 'node:assert/strict'
import { randomUUID } from 'node:crypto'
import { setTimeout as delay } from 'node:timers/promises'
import test from 'node:test'
import { load } from '@arcships/rutis/testing'
import plugin from './index.ts'
import sessionPlugin from './session.ts'

const ok = value => ({ ok: true, value })
const error = code => ({ ok: false, error: { code, message: code, retryable: code === 'reloading' } })
const pending = () => new Promise(() => {})
const definition = (name, steps, parameters = []) => ({ schema: 'dim.workflow/v1', name, description: name, parameters, steps })

async function until(work) {
  const through = Date.now() + 8000
  for (;;) {
    const value = await work()
    if (value) return value
    assert.ok(Date.now() < through, 'Workflow public operation did not settle')
    await delay(5)
  }
}

async function fixture(t, options = {}) {
  const state = {
    document: { definitions: {}, projectEnabled: {}, max_concurrent_runs: 4, max_concurrent_stages: 4 },
    revision: '0', stored: null, launches: [], inputs: [], histories: [], stops: [], deletes: [], files: new Map(),
    sessions: new Map(), receipts: new Map(), behavior: () => 'complete', response: prompt => prompt,
    ...options,
  }
  const event = (sessionId, type, name, data) => {
    const events = state.sessions.get(sessionId)
    const value = { id: randomUUID(), sequence: events.length + 1, create_at: Date.now(), type, name, data }
    events.push(value)
    return value
  }
  let fileTail = Promise.resolve()
  let activeScopes = 0
  const services = {
    settings: {
      read: async () => ok({ revision: state.revision, document: structuredClone(state.document) }),
      update: async request => {
        if (request.expectedRevision !== state.revision) return error('conflict')
        assert.deepEqual(request.document.projectEnabled, state.document.projectEnabled, 'owner cannot enable project content')
        state.document = structuredClone(request.document)
        state.revision = String(Number(state.revision) + 1)
        return ok({ revision: state.revision, document: structuredClone(state.document) })
      },
      projects: () => ok({ projects: [{ root: 7, cwd: '/workspace' }] }),
    },
    pluginFiles: { open: async () => {
      activeScopes++
      let closed = false
      return {
        async withLock(path, work) {
          assert.equal(path, 'workflow/runs.json')
          const before = fileTail
          const done = Promise.withResolvers()
          fileTail = done.promise
          await before
          try {
            assert.equal(closed, false, 'run operation outlived its file scope')
            return ok(await work({
              read: async () => ok(state.stored),
              replace: async text => {
                await state.beforeReplace?.(text)
                state.stored = text
                return ok(null)
              },
            }))
          } finally { done.resolve() }
        },
        quiesce() { return null },
        async close() { await fileTail; closed = true; activeScopes--; return null },
      }
    } },
    workflowFiles: {
      async list({ root }) {
        const prefix = 'project' in root ? `project:${root.project}/` : `package:${root.package}/`
        const names = [...state.files.keys()].filter(name => name.startsWith(prefix)).map(name => name.slice(prefix.length))
        if (!names.length && 'package' in root) throw { code: 'not_found', message: 'missing package workflows' }
        return names.map(name => ({ name, kind: 'file' }))
      },
      async read({ root, path }) {
        const key = 'project' in root ? `project:${root.project}/${path}` : `package:${root.package}/${path}`
        assert.ok(state.files.has(key))
        return new TextEncoder().encode(state.files.get(key))
      },
      async createDirectory({ root }) { assert.deepEqual(root, { project: '/workspace' }) },
      async replace({ root, path, bytes }) {
        assert.deepEqual(root, { project: '/workspace' })
        state.files.set(`project:${root.project}/${path}`, new TextDecoder().decode(bytes))
      },
    },
    sessions: {
      async launch(request) {
        const sessionId = randomUUID()
        state.launches.push(structuredClone(request))
        state.sessions.set(sessionId, [])
        await state.onLaunch?.(request, sessionId)
        return ok({ sessionId, policy: request.starter
          ? { preset: 'workspace-write', allowedTools: [], disallowedTools: [] }
          : request.policy ?? { preset: 'read-only', allowedTools: [], disallowedTools: [] } })
      },
      async input(request) {
        state.inputs.push(structuredClone(request))
        if (request.mode === 'steer') await state.reportInput?.(request)
        const inputId = randomUUID()
        state.receipts.set(request.submissionId, { inputId, sequence: 1 })
        if (request.mode === 'steer') return ok({ inputId, sequence: 1 })
        const message = event(request.sessionId, 'input', 'message', { content: request.content })
        message.id = inputId
        const behavior = state.behavior(request.content[0].text)
        if (behavior === 'held') return ok({ inputId, sequence: message.sequence })
        event(request.sessionId, 'input', 'applied', { input_id: inputId })
        if (behavior === 'applied') return ok({ inputId, sequence: message.sequence })
        const turn = randomUUID()
        event(request.sessionId, 'turn', 'start', { turn_id: turn })
        event(request.sessionId, 'input', 'consumed', { input_id: inputId })
        event(request.sessionId, 'step', 'model-start', { turn_id: turn, request_id: randomUUID() })
        event(request.sessionId, 'step', 'text', { turn_id: turn, delta: state.response(request.content[0].text) })
        if (behavior === 'complete') event(request.sessionId, 'turn', 'end', { turn_id: turn, status: 'completed', reason: 'end' })
        return ok({ inputId, sequence: message.sequence })
      },
      async lookup(request) { return state.receipts.has(request.submissionId)
        ? ok({ status: 'accepted', ...state.receipts.get(request.submissionId) }) : ok({ status: 'unknown' }) },
      async history(request) {
        state.histories.push(structuredClone(request))
        const all = state.sessions.get(request.sessionId)
        if (!all) return error('session_closed')
        const through = request.through ?? all.length
        const fault = await state.historyError?.(request)
        if (fault) return error(fault)
        return ok({ through, events: all.filter(event => event.sequence > (request.after ?? 0) && event.sequence <= through).slice(0, request.limit) })
      },
      async withdraw(request) {
        const all = state.sessions.get(request.sessionId)
        if (!all) return error('session_closed')
        if (all.some(event => event.name === 'applied' && event.data.input_id === request.inputId)) return ok({ status: 'notHeld' })
        return ok({ status: 'deleted', sequence: event(request.sessionId, 'input', 'deleted', { input_id: request.inputId }).sequence })
      },
      async stop(request) {
        state.stops.push(structuredClone(request))
        const all = state.sessions.get(request.sessionId)
        if (!all) return error('session_closed')
        event(request.sessionId, 'stop', 'interrupt', { reason: 'workflow stopping' })
        const turn = all.findLast(event => event.type === 'turn' && event.name === 'start')
        if (turn) event(request.sessionId, 'turn', 'end', { turn_id: turn.data.turn_id, status: 'cancelled', reason: 'stopped' })
        return ok({ sequence: all.length })
      },
      async delete(request) {
        state.deletes.push(request.sessionId)
        if (state.deleteError) return error(state.deleteError)
        state.sessions.delete(request.sessionId)
        return ok({})
      },
    },
  }
  let loaded
  let tool
  const open = async () => {
    loaded = await load(plugin, { config: { packages: options.packages ?? [] }, services })
    tool = await load(sessionPlugin, { services: { workflowRuns: loaded.service('workflowRuns') } })
  }
  await open()
  const close = async () => { await tool.unload(); await loaded.unload() }
  t.after(async () => { await close(); assert.equal(activeScopes, 0) })
  const method = (name, params = {}) => loaded.service('managementMethods:workflow').call({
    callId: randomUUID(), method: `workflow.${name}`, params,
  }, pending(), () => {})
  const execute = (arguments_, config = {}) => tool.service('toolContribution:workflow').execute({
    sessionId: 'starter', branchId: 'branch', turnId: 'turn', callId: randomUUID(), arguments: arguments_,
    config: { cwd: '/workspace', collaborationRole: 'leader',
      permissionPolicy: { preset: 'workspace-write', allowedTools: [], disallowedTools: [] },
      model: { providerId: 'test', modelId: 'test' }, systemPrompt: '', tools: ['workflow'], ...config },
  }, pending())
  const settled = id => until(async () => {
    const reply = await method('status', { run_id: id })
    if (reply.kind !== 'value') throw new Error(JSON.stringify(reply))
    return ['completed', 'failed', 'stopped', 'interrupted'].includes(reply.value.state) ? reply.value : undefined
  })
  return { state, services, method, execute, settled, event, reload: async () => { await close(); await open() } }
}

test('tool and management run sequential/parallel steps with a frozen definition and first-launch policy', async t => {
  const f = await fixture(t)
  const saved = await f.method('save', { source: 'user', document: definition('review', [
    { stage: { id: 'analyze', prompt: '{{params.target}}' } },
    { parallel: [{ id: 'a', prompt: 'a={{stages.analyze.result}}' }, { id: 'b', prompt: 'b={{stages.analyze.result}}' }] },
    { stage: { id: 'summary', prompt: '{{stages.a.result}} / {{stages.b.result}}' } },
  ], [{ name: 'target', required: true }]) })
  assert.deepEqual(saved, { kind: 'value', value: { name: 'review' } })
  for (const params of [{}, { target: 'file.ts', extra: 'unknown' }]) {
    const rejected = await f.execute({ action: 'start', name: 'review', params })
    assert.equal(rejected.isError, true)
    assert.match(rejected.content, /invalid_parameters/)
  }
  assert.equal(f.state.launches.length, 0)
  assert.equal(JSON.parse(f.state.stored).runs.length, 0)
  const started = await f.execute({ action: 'start', name: 'review', params: { target: 'file.ts' } })
  assert.equal(started.isError, undefined)
  const id = started.structuredContent.run_id
  assert.ok(started.content.includes(id))
  f.state.document.definitions.review.steps[0].stage.prompt = 'changed source'
  const result = await f.settled(id)
  assert.equal(result.state, 'completed')
  assert.equal(result.stages[3].result_text, 'a=file.ts / b=file.ts')
  assert.equal(new Set(result.stages.map(stage => stage.session_id)).size, 4)
  assert.ok(f.state.launches.every(request => request.agent === undefined && request.model === undefined), 'omitted selections use Agents defaults')
  assert.deepEqual(f.state.launches[0].starter, { sessionId: 'starter', branchId: 'branch' })
  for (const request of f.state.launches.slice(1)) {
    assert.equal(request.starter, undefined)
    assert.deepEqual(request.policy, result.policy_cap)
  }
  await until(() => f.state.inputs.some(input => input.mode === 'steer'))
  const report = f.state.inputs.find(input => input.mode === 'steer')
  assert.ok(report.rawBlocks[0].text.includes(id))
  assert.deepEqual(report.content, [], 'the report is one raw context block without a duplicate user message')
  assert.deepEqual(JSON.parse(f.state.stored).version, 2)
})

test('source validation, project enablement and diagnostics run through management and the tool', async t => {
  const f = await fixture(t)
  const invalid = definition('invalid', [{ parallel: [
    { id: 'one', prompt: '{{stages.two.result}}' }, { id: 'two', prompt: 'x' },
  ] }])
  const rejected = await f.method('save', { source: 'user', document: invalid })
  assert.equal(rejected.appError, 'invalid_definition')
  assert.equal(f.state.revision, '0')
  f.state.document.definitions.invalid = invalid
  const list = await f.execute({ action: 'list' })
  assert.match(list.content, /invalid reference/)
  assert.equal((await f.execute({ action: 'start', name: 'invalid', params: {} })).isError, true)
  assert.equal(f.state.launches.length, 0)
  const project = definition('local', [{ stage: { id: 'one', prompt: '{"a":{"b":1}} }} {{params.value}}' } }], [{ name: 'value', default: '{{literal' }])
  await f.method('save', { source: 'project', cwd: '/workspace', document: project })
  assert.equal((await f.execute({ action: 'start', name: 'local' })).isError, true)
  f.state.document.projectEnabled['/workspace'] = { local: true }
  const started = await f.execute({ action: 'start', name: 'local' })
  const result = await f.settled(started.structuredContent.run_id)
  assert.equal(result.stages[0].result_text, '{"a":{"b":1}} }} {{literal')
  const noninteractive = await f.execute({ action: 'list' }, { interactive: false })
  assert.equal(noninteractive.isError, true)
})

test('whole-step prompt overflow admits no valid sibling and leaves first policy absent', async t => {
  const f = await fixture(t)
  await f.method('save', { source: 'user', document: definition('large', [{ parallel: [
    { id: 'short', prompt: 'short' }, { id: 'large', prompt: '{{params.text}}' },
  ] }], [{ name: 'text', required: true }]) })
  const started = await f.execute({ action: 'start', name: 'large', params: { text: 'x'.repeat(32769) } })
  const run = await f.settled(started.structuredContent.run_id)
  assert.equal(run.state, 'failed')
  assert.deepEqual(run.stages.map(stage => stage.state), ['skipped', 'failed'])
  assert.equal(run.stages[1].reason, 'prompt_too_large')
  assert.equal(run.policy_cap, undefined)
  assert.equal(f.state.launches.length, 0)
})

test('stop withdraws a held own input, settles consumed work, and never reports explicit stops', async t => {
  const f = await fixture(t)
  f.state.behavior = prompt => prompt
  await f.method('save', { source: 'user', document: definition('stop', [{ parallel: [
    { id: 'held', prompt: 'held' }, { id: 'running', prompt: 'running' },
  ] }]) })
  const started = await f.execute({ action: 'start', name: 'stop' })
  await until(() => f.state.inputs.filter(input => input.mode === 'queue').length === 2)
  const stopped = await f.method('stop', { run_id: started.structuredContent.run_id })
  assert.equal(stopped.value.state, 'stopped')
  assert.deepEqual(stopped.value.stages.map(stage => stage.state), ['stopped', 'stopped'])
  assert.equal(f.state.stops.length, 1)
  assert.equal(f.state.inputs.filter(input => input.mode === 'steer').length, 0)
})

test('source limits bound active runs and stage admission, and later starts read edited limits', async t => {
  const f = await fixture(t)
  f.state.document.max_concurrent_runs = 1
  f.state.document.max_concurrent_stages = 1
  f.state.behavior = () => 'running'
  await f.method('save', { source: 'user', document: definition('limits', [{ parallel: [
    { id: 'a', prompt: 'a' }, { id: 'b', prompt: 'b' }, { id: 'c', prompt: 'c' },
  ] }]) })
  assert.equal(f.state.document.max_concurrent_runs, 1, 'saving a definition preserves source limits')
  assert.equal(f.state.document.max_concurrent_stages, 1)
  const first = await f.method('start', { name: 'limits', cwd: '/workspace' })
  await until(() => f.state.inputs.length === 1)
  assert.equal((await f.method('start', { name: 'limits', cwd: '/workspace' })).appError, 'limit_exceeded')
  assert.equal(f.state.launches.length, 1)
  const session = f.state.inputs[0].sessionId
  const turn = f.state.sessions.get(session).find(event => event.type === 'turn' && event.name === 'start').data.turn_id
  f.event(session, 'turn', 'end', { turn_id: turn, status: 'completed' })
  await until(() => f.state.inputs.length === 2)
  assert.equal(f.state.launches.length, 2)
  const stopped = await f.method('stop', { run_id: first.value.run_id })
  assert.equal(stopped.value.state, 'stopped')
  assert.equal(stopped.value.stages[2].state, 'skipped')
  f.state.document.max_concurrent_stages = 3
  const second = await f.method('start', { name: 'limits', cwd: '/workspace' })
  await until(() => f.state.inputs.length === 5)
  assert.equal((await f.method('stop', { run_id: second.value.run_id })).value.state, 'stopped')
})

test('an applied input interrupted before consumption settles without another stop or input', async t => {
  const f = await fixture(t, { behavior: () => 'applied' })
  await f.method('save', { source: 'user', document: definition('applied', [{ stage: { id: 'one', prompt: 'held' } }]) })
  const started = await f.method('start', { name: 'applied', cwd: '/workspace' })
  await until(() => f.state.inputs.length === 1)
  f.event(f.state.inputs[0].sessionId, 'stop', 'interrupt', { reason: 'operator' })
  const stopped = await f.method('stop', { run_id: started.value.run_id })
  assert.equal(stopped.value.state, 'stopped')
  assert.equal(stopped.value.stages[0].state, 'stopped')
  assert.equal(f.state.stops.length, 0)
  assert.equal(f.state.inputs.length, 1)
})

test('a notHeld reply after another withdrawal settles from the next published deletion', async t => {
  const f = await fixture(t, { behavior: () => 'held' })
  await f.method('save', { source: 'user', document: definition('withdrawn', [{ stage: { id: 'one', prompt: 'held' } }]) })
  const started = await f.method('start', { name: 'withdrawn', cwd: '/workspace' })
  await until(() => f.state.inputs.length === 1)
  f.services.sessions.withdraw = async request => {
    f.event(request.sessionId, 'input', 'deleted', { input_id: request.inputId })
    return ok({ status: 'notHeld' })
  }
  const stopped = await f.method('stop', { run_id: started.value.run_id })
  assert.equal(stopped.value.state, 'stopped')
  assert.equal(stopped.value.stages[0].state, 'stopped')
  assert.equal(f.state.stops.length, 0)
  assert.equal(f.state.inputs.length, 1)
})

test('a failed parallel stage stops its consumed sibling and skips later steps', async t => {
  const f = await fixture(t, { behavior: () => 'running' })
  let missing
  f.state.onLaunch = (_request, id) => { if (f.state.launches.length === 2) missing = id }
  f.state.historyError = request => request.sessionId === missing ? 'session_closed' : undefined
  await f.method('save', { source: 'user', document: definition('failfast', [
    { parallel: [{ id: 'running', prompt: 'running' }, { id: 'missing', prompt: 'missing' }] },
    { stage: { id: 'later', prompt: 'must not launch' } },
  ]) })
  const started = await f.execute({ action: 'start', name: 'failfast' })
  const run = await f.settled(started.structuredContent.run_id)
  assert.equal(run.state, 'failed')
  assert.deepEqual(run.stages.map(stage => stage.state), ['stopped', 'failed', 'skipped'])
  assert.equal(f.state.launches.length, 2)
  assert.equal(f.state.stops.length, 1)
  await until(() => f.state.inputs.some(input => input.mode === 'steer'))
  assert.ok(f.state.inputs.find(input => input.mode === 'steer').rawBlocks[0].text.includes('status="failed"'))
})

test('cleanup joins an admitted sibling launch after another stage read fails', async t => {
  const f = await fixture(t)
  const entered = Promise.withResolvers()
  const release = Promise.withResolvers()
  f.state.onLaunch = async () => {
    if (f.state.launches.length === 2) { entered.resolve(); await release.promise }
  }
  f.state.historyError = async () => { await entered.promise; throw new Error('stage bridge failed') }
  await f.method('save', { source: 'user', document: definition('join', [{ parallel: [
    { id: 'a', prompt: 'a' }, { id: 'b', prompt: 'b' },
  ] }]) })
  await f.method('start', { name: 'join', cwd: '/workspace' })
  await entered.promise
  await until(async () => {
    try { await f.method('status'); return false }
    catch (error) { return error.message === 'stage bridge failed' }
  })
  let reloaded = false
  const reload = f.reload().then(() => { reloaded = true })
  try {
    await delay(50)
    assert.equal(reloaded, false, 'the original file scope must outlive every admitted sibling operation')
  } finally { release.resolve(); await reload }
})

test('report settlement joins cleanup and recovery never repeats an accepted report', async t => {
  const f = await fixture(t)
  await f.method('save', { source: 'user', document: definition('report', [{ stage: { id: 'one', prompt: 'done' } }]) })
  const entered = Promise.withResolvers()
  const release = Promise.withResolvers()
  f.state.reportInput = async () => { entered.resolve(); await release.promise }
  const started = await f.execute({ action: 'start', name: 'report' })
  await entered.promise
  const stored = JSON.parse(f.state.stored).runs[0]
  assert.equal(stored.run_id, started.structuredContent.run_id)
  assert.deepEqual(stored.report.state, 'pending')
  assert.equal(stored.report.attempt_open, true)
  assert.equal((await f.method('stop', { run_id: stored.run_id })).value.state, 'completed', 'terminal stop does not join or erase a pending report')
  let reloaded = false
  const reload = f.reload().then(() => { reloaded = true })
  await delay(20)
  assert.equal(reloaded, false, 'cleanup must join the background report before closing storage')
  release.resolve()
  await reload
  await until(async () => (await f.method('status', { run_id: stored.run_id })).value.report.state === 'delivered')
  assert.equal(f.state.inputs.filter(input => input.mode === 'steer').length, 1)
  await f.reload()
  assert.equal(f.state.inputs.filter(input => input.mode === 'steer').length, 1)
})

test('terminal and uncertain report replies follow the persisted recovery contract', async t => {
  for (const code of ['rejected', 'failed', 'ambiguous', 'publication_unknown', 'reloading']) await t.test(code, async t => {
    const f = await fixture(t)
    await f.method('save', { source: 'user', document: definition('reply', [{ stage: { id: 'one', prompt: 'done' } }]) })
    const input = f.services.sessions.input
    const lookup = f.services.sessions.lookup
    const calls = []
    let fail = true
    f.services.sessions.input = async request => {
      if (request.mode !== 'steer') return input(request)
      calls.push('input')
      if (!fail) return input(request)
      if (code === 'publication_unknown' || code === 'reloading') await input(request)
      return error(code === 'ambiguous' ? 'failed' : code)
    }
    f.services.sessions.lookup = async request => { calls.push('lookup'); return lookup(request) }
    const started = await f.execute({ action: 'start', name: 'reply' })
    const id = started.structuredContent.run_id
    const status = () => f.method('status', { run_id: id }).then(reply => reply.value)
    if (code === 'rejected') {
      const run = await until(async () => { const run = await status(); return run.report.state === 'undeliverable' && run })
      assert.equal(run.report.reason, 'rejected')
      assert.ok(run.log.some(entry => entry.message.includes('Invalid report rejected')))
      await f.reload()
      assert.deepEqual(calls, ['input'])
    } else if (code === 'publication_unknown' || code === 'reloading') {
      await until(async () => (await status()).report.state === 'delivered')
      assert.deepEqual(calls, ['input', 'lookup'], 'the actual settled reply is reconciled without repeating its accepted input')
      assert.equal(f.state.inputs.filter(input => input.mode === 'steer').length, 1)
    } else {
      await until(async () => {
        const run = await status()
        return run.report.state === 'pending' && !run.report.attempt_open && run.log.some(entry => entry.message.includes('Report input failed: failed'))
      })
      assert.deepEqual(calls, ['input'])
      fail = false
      if (code === 'ambiguous') f.services.sessions.lookup = async () => { calls.push('lookup'); return error('ambiguous') }
      await f.reload()
      const run = await until(async () => { const run = await status(); return run.report.state === 'delivered' && run })
      if (code === 'ambiguous') {
        assert.deepEqual(calls, ['input', 'lookup'])
        assert.ok(run.log.some(entry => entry.message.includes('Duplicate report violation')))
      } else assert.deepEqual(calls, ['input', 'lookup', 'input'], 'a later load starts with lookup before its single retry')
    }
  })
})

test('invalid inner versions and missing pending-attempt markers preserve the original bytes', async t => {
  const source = await fixture(t)
  await source.method('save', { source: 'user', document: definition('seed', [{ stage: { id: 'one', prompt: 'seed' } }]) })
  const accepted = await source.execute({ action: 'start', name: 'seed' })
  await source.settled(accepted.structuredContent.run_id)
  await until(() => JSON.parse(source.state.stored).runs[0].report.state === 'delivered')
  const seeded = JSON.parse(source.state.stored)
  const missing = structuredClone(seeded)
  missing.runs[0].report = { state: 'pending', submission_id: 'workflow:missing' }
  const timestamp = structuredClone(seeded)
  timestamp.runs[0].created_at = '2026-10-22T00:00:00Z'
  const duplicate = structuredClone(seeded)
  duplicate.runs.push(structuredClone(duplicate.runs[0]))
  for (const [name, stored] of [
    ['v1', '{"version":1,"runs":[]}'], ['unknown version', '{"version":999,"runs":[]}'], ['malformed', '{broken'],
    ['missing attempt marker', JSON.stringify(missing)], ['timestamp precision', JSON.stringify(timestamp)], ['duplicate run', JSON.stringify(duplicate)],
  ]) {
    await t.test(name, async t => {
      const f = await fixture(t, { stored })
      const reply = await f.method('status')
      assert.equal(reply.appError, 'invalid_data')
      assert.equal(f.state.stored, stored)
      assert.equal(f.state.launches.length, 0)
    })
  }
})

test('semantic errors, file identities and package collisions remain visible without fallthrough', async t => {
  const f = await fixture(t, { packages: [{ id: 'a', root: '/a' }, { id: 'b', root: '/b' }], files: new Map([
    ['package:a/shared.yaml', JSON.stringify(definition('shared', [{ stage: { id: 'one', prompt: 'a' } }]))],
    ['package:b/shared.yaml', JSON.stringify(definition('shared', [{ stage: { id: 'one', prompt: 'b' } }]))],
  ]) })
  const listed = await f.execute({ action: 'list' })
  assert.match(listed.content, /multiple installed packages/)
  assert.deepEqual(listed.structuredContent.definitions.map(entry => entry.package_id), ['a', 'b'])
  assert.equal((await f.execute({ action: 'start', name: 'shared' })).isError, true)
  const invalid = [
    ...['{{stage.one.result}}', '{{ params.x }}', '{{params.x', 'x'.repeat(32769)]
      .map(prompt => definition('bad', [{ stage: { id: 'one', prompt } }])),
    definition('bad', [{ stage: { id: 'one', prompt: 'first' } }, { stage: { id: 'one', prompt: 'duplicate' } }]),
    definition('bad', [{ stage: { id: 'one', prompt: '{{stages.later.result}}' } }, { stage: { id: 'later', prompt: 'later' } }]),
    definition('bad', [{ stage: { id: 'one', prompt: '{{params.x}}' } }], [{ name: 'x' }, { name: 'x' }]),
  ]
  for (const document of invalid) {
    const revision = f.state.revision
    const reply = await f.method('save', { source: 'user', document })
    assert.equal(reply.appError, 'invalid_definition')
    assert.equal(f.state.revision, revision)
    f.state.document.definitions.bad = document
    const listed = await f.execute({ action: 'list' })
    assert.equal(listed.structuredContent.definitions.find(entry => entry.name === 'bad').valid, false)
    assert.match((await f.execute({ action: 'start', name: 'bad' })).content, /invalid_definition.*Use list/s)
  }
  assert.equal(f.state.launches.length, 0)
  const invalidParameter = definition('bad', [{ stage: { id: 'one', prompt: 'prompt' } }], [{ name: 'invalid name' }])
  assert.equal((await f.method('save', { source: 'user', document: invalidParameter })).appError, 'invalid_definition')
  const prototype = definition('constructor', [{ stage: { id: 'one', prompt: '<{{params.constructor}}>' } }], [{ name: 'constructor' }])
  assert.equal((await f.method('save', { source: 'user', document: prototype })).kind, 'value')
  const run = await f.execute({ action: 'start', name: 'constructor' })
  assert.equal((await f.settled(run.structuredContent.run_id)).stages[0].result_text, '<>')
  f.state.files.set('project:/workspace/constructor.yaml', JSON.stringify(definition('different', [{ stage: { id: 'one', prompt: 'invalid identity' } }])))
  f.state.document.projectEnabled['/workspace'] = { constructor: true }
  assert.equal((await f.execute({ action: 'start', name: 'constructor' })).isError, true)
  assert.equal((await f.execute({ action: 'start', source: 'user', name: 'constructor' })).isError, undefined)
})

test('later substitution overflow preserves the prior result and launches none of the next parallel step', async t => {
  const f = await fixture(t)
  f.state.response = () => 'x'.repeat(32769)
  await f.method('save', { source: 'user', document: definition('later', [
    { stage: { id: 'first', prompt: 'large result' } },
    { parallel: [{ id: 'valid', prompt: 'short' }, { id: 'oversized', prompt: '{{stages.first.result}}' }] },
  ]) })
  const started = await f.execute({ action: 'start', name: 'later' })
  const run = await f.settled(started.structuredContent.run_id)
  assert.equal(run.state, 'failed')
  assert.equal(run.stages[0].result_text.length, 32769)
  assert.deepEqual(run.stages.map(stage => stage.state), ['completed', 'skipped', 'failed'])
  assert.equal(f.state.launches.length, 1)
})

test('a fixed published prefix survives a reloading page without consuming an unrelated turn end', async t => {
  let f
  let failed = false
  f = await fixture(t)
  f.state.onLaunch = (_request, id) => {
    for (let i = 0; i < 600; i++) f.event(id, 'fixture', 'padding', { ordinal: i })
    f.event(id, 'turn', 'end', { turn_id: 'unrelated', status: 'failed', error: 'not this input' })
  }
  f.state.historyError = request => {
    if (request.after > 0 && !failed) { failed = true; return 'reloading' }
  }
  await f.method('save', { source: 'user', document: definition('pages', [{ stage: { id: 'one', prompt: 'own result' } }]) })
  const started = await f.execute({ action: 'start', name: 'pages' })
  const run = await f.settled(started.structuredContent.run_id)
  assert.equal(run.state, 'completed')
  assert.equal(run.stages[0].result_text, 'own result')
  assert.equal(failed, true)
})

test('retention preserves active, pending and referenced ownership and retries failed deletion', async t => {
  t.mock.timers.enable({ apis: ['setInterval'] })
  const seed = await fixture(t)
  await seed.method('save', { source: 'user', document: definition('history', [{ stage: { id: 'one', prompt: 'done' } }]) })
  const started = await seed.method('start', { name: 'history', cwd: '/workspace' })
  await seed.settled(started.value.run_id)
  const first = JSON.parse(seed.state.stored).runs[0]
  const runs = Array.from({ length: 102 }, (_, index) => ({ ...structuredClone(first), run_id: randomUUID(),
    created_at: new Date(Date.UTC(2026, 0, 1) + index - 1).toISOString(),
    ended_at: new Date(Date.UTC(2026, 0, 1) + index).toISOString(),
    stages: [{ ...first.stages[0], session_id: index === 101 ? 'owned-0' : `owned-${index}` }],
  }))
  const f = await fixture(t, { stored: JSON.stringify({ version: 2, runs }), deleteError: 'failed' })
  await until(async () => (await f.method('status')).value.some(run => run.log.some(entry => entry.message === 'Retention delete failed: failed')))
  await until(() => JSON.parse(f.state.stored).runs.length === 101)
  assert.deepEqual(f.state.deletes, ['owned-1'], 'the retained reference protects owned-0')
  f.state.behavior = prompt => prompt === 'held' ? 'held' : 'complete'
  const entered = Promise.withResolvers()
  const release = Promise.withResolvers()
  f.state.reportInput = async () => { entered.resolve(); await release.promise }
  await f.method('save', { source: 'user', document: definition('active', [{ stage: { id: 'one', prompt: 'held' } }]) })
  await f.method('save', { source: 'user', document: definition('pending', [{ stage: { id: 'one', prompt: 'done' } }]) })
  const active = await f.method('start', { name: 'active', cwd: '/workspace' })
  const pending = await f.execute({ action: 'start', name: 'pending' })
  try {
    await entered.promise
    f.state.deleteError = undefined
    t.mock.timers.tick(60_000)
    await until(() => JSON.parse(f.state.stored).runs.length === 102)
    const retained = (await f.method('status')).value
    assert.equal(retained.find(run => run.run_id === active.value.run_id).state, 'running')
    assert.equal(retained.find(run => run.run_id === pending.structuredContent.run_id).report.state, 'pending')
    assert.deepEqual(f.state.deletes, ['owned-1', 'owned-1'])
  } finally {
    release.resolve()
    await f.method('stop', { run_id: active.value.run_id })
  }
})

test('reload retries deletion after it succeeded but removing the ownership record failed', async t => {
  const seed = await fixture(t)
  await seed.method('save', { source: 'user', document: definition('history', [{ stage: { id: 'one', prompt: 'done' } }]) })
  const started = await seed.method('start', { name: 'history', cwd: '/workspace' })
  await seed.settled(started.value.run_id)
  const first = JSON.parse(seed.state.stored).runs[0]
  const runs = Array.from({ length: 101 }, (_, index) => ({ ...structuredClone(first), run_id: randomUUID(),
    created_at: new Date(Date.UTC(2026, 0, 1) + index - 1).toISOString(),
    ended_at: new Date(Date.UTC(2026, 0, 1) + index).toISOString(),
    stages: [{ ...first.stages[0], session_id: `owned-${index}` }],
  }))
  const failed = Promise.withResolvers()
  let fail = true
  const f = await fixture(t, { stored: JSON.stringify({ version: 2, runs }),
    beforeReplace: text => {
      if (fail && JSON.parse(text).runs.length === 100) {
        fail = false
        failed.resolve()
        throw new Error('record replacement failed')
      }
    },
  })
  await failed.promise
  assert.equal((await f.method('status')).value.length, 101)
  assert.deepEqual(f.state.deletes, ['owned-0'])
  await f.reload()
  await until(() => JSON.parse(f.state.stored).runs.length === 100)
  assert.deepEqual(f.state.deletes, ['owned-0', 'owned-0'])
})
