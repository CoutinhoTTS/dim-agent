import assert from 'node:assert/strict'
import test from 'node:test'
import { load } from '@arcships/rutis/testing'
import plugin from './index.ts'

const flush = () => new Promise(resolve => setImmediate(resolve))
const pending = () => new Promise(() => {})
const deferred = () => Promise.withResolvers()
const ok = value => ({ ok: true, value })

async function fixture(t, config = {}) {
  const closing = deferred()
  const state = {
    document: { enabled: true, maxLength: 80, model: null },
    metadata: { revision: 0, title: null },
    events: [], requests: [], pages: [], notices: [],
    complete: async () => ({ text: '“A short title!!!”\nIgnore this line', stopReason: 'end_turn' }),
    beforeWrite: undefined,
    ...config,
  }
  const services = {
    settings: { read: async () => state.settingsError
      ? { ok: false, error: { code: 'invalid_document', message: 'private diagnostic' } }
      : ok({ revision: 'r', document: state.document }) },
    sessions: { history: async request => {
      assert.equal(request.branchId, 'main')
      assert.ok(request.atLeast === undefined || request.through === undefined)
      state.pages.push(request)
      const through = request.through ?? state.events.at(-1).sequence
      const events = state.events.filter(e => e.sequence > (request.after ?? 0) && e.sequence <= through).slice(0, 2)
      return ok({ through, events })
    } },
    sessionMetadata: {
      read: async () => { await state.beforeRead?.(); return ok(state.metadata) },
      setTitle: async request => {
        await state.beforeWrite?.(request)
        if (request.expectedRevision !== state.metadata.revision) return { ok: false, error: { code: 'conflict', message: 'Changed' } }
        state.metadata = { revision: state.metadata.revision + 1, title: request.title }
        state.notices.push(state.metadata)
        return ok({ revision: state.metadata.revision })
      },
    },
    auxiliaryModel: { complete: async (request, cancelled) => {
      state.requests.push(request)
      return state.complete(request, cancelled)
    } },
  }
  const loaded = await load(plugin, { services, emits: ['session.event'] })
  t.after(() => loaded.unload())
  loaded.service('sessionEvents:session-title').bind({ sessionId: 'session', config: {
    collaborationRole: 'leader', model: { providerId: 'creation', modelId: 'stale' }, ...config.creation,
  } }, closing.promise)
  const commit = (type, name, data) => {
    const sequence = state.events.length + 1
    const event = { id: `e${sequence}`, sequence, type, name, create_at: 0, data }
    state.events.push(event)
    return event
  }
  const adopted = (providerId = 'provider', modelId = 'adopted') => commit('context', 'configured', {
    timeline: 'main', revision: crypto.randomUUID(), settings: { providerId, modelId },
  })
  const emit = (event, branchId = 'main', sessionId = 'session') => loaded.emit('session.event', { sessionId, branchId, event })
  const input = async (content, branchId = 'main') => {
    const event = commit('input', 'message', content === null ? { blocks: [{ type: 'raw', text: 'private raw' }] } : { user: { content } })
    await emit(event, branchId)
    return event
  }
  const apply = async (event, branchId = 'main', sessionId = 'session') => {
    const applied = commit('input', 'applied', { input_id: event.id })
    await emit(applied, branchId, sessionId)
    return applied
  }
  const submit = async (text, branchId = 'main') => {
    await apply(await input([{ type: 'text', text }], branchId), branchId)
    await flush()
  }
  const rename = (title, cancelled = pending()) => loaded.service('sessionMethods:session.rename').call({
    callId: crypto.randomUUID(), method: 'session.rename', params: { sessionId: 'session', title },
  }, cancelled, () => {})
  return { state, loaded, closing, commit, adopted, emit, input, apply, submit, rename }
}

test('titles the first adopted operator input through fixed pages; rename wins and clear uses current settings', async t => {
  const p = await fixture(t)
  p.adopted()
  const deleted = await p.input([{ type: 'text', text: 'deleted secret' }])
  p.commit('input', 'deleted', { input_id: deleted.id })
  const raw = await p.input(null)
  await p.apply(raw)
  const first = await p.input([{ type: 'text', text: '😀'.repeat(2100) }, { type: 'image', source: { type: 'url', mediaType: 'image/png', url: 'secret-image' } }])
  const started = deferred()
  const finish = deferred()
  p.state.complete = async () => { started.resolve(); return finish.promise }
  await p.apply(first)
  await started.promise
  await p.submit('later secret')
  assert.equal(p.state.requests.length, 1)
  const request = p.state.requests[0]
  assert.deepEqual(request.model, { providerId: 'provider', modelId: 'adopted' })
  assert.equal([...request.request.messages[0].content].length, 2000)
  assert.ok(!JSON.stringify(request).includes('secret'))
  assert.ok(!('maxOutputTokens' in request.request))
  assert.match(request.request.system, /at most 8 words/)
  const renamed = await p.rename('  Manual 😀  ')
  assert.deepEqual(renamed.value, { title: 'Manual 😀', titleSource: 'manual', revision: 1 })
  finish.resolve({ text: 'Automatic loser', stopReason: 'end_turn' })
  await flush()
  assert.deepEqual(p.state.metadata.title, { text: 'Manual 😀', source: 'manual' })
  assert.equal(p.state.notices.length, 1)
  await p.submit('already titled')
  assert.equal(p.state.requests.length, 1)
  await p.rename(null)
  p.state.document = { enabled: true, maxLength: 10, model: { providerId: 'explicit', modelId: 'chosen' } }
  p.state.complete = async () => ({ text: '“😀'.repeat(15) + '!!!”\nprivate line', stopReason: 'end_turn' })
  await p.submit('clear triggers from first')
  assert.deepEqual(p.state.requests.at(-1).model, p.state.document.model)
  assert.equal([...p.state.metadata.title.text].length, 10)
  assert.equal(p.state.metadata.title.source, 'auto')
  assert.equal(p.state.requests.at(-1).request.messages[0].content, request.request.messages[0].content)
  assert.ok(p.state.pages.some(page => page.through !== undefined))
  for (const [generated, expected] of [['"Fix login bug. \nmore"', 'Fix login'], ['Short. \nmore', 'Short']]) {
    await p.rename(null)
    p.state.complete = async () => ({ text: generated, stopReason: 'end_turn' })
    await p.submit('normalization still uses first input')
    assert.equal(p.state.metadata.title.text, expected)
  }
})

test('eligibility, image-only input, raw/child/foreign adoption and disabled settings do not generate', async t => {
  for (const creation of [{ interactive: false }, { collaborationRole: 'member' }]) {
    const p = await fixture(t, { creation })
    p.adopted()
    await p.submit('not eligible')
    assert.equal(p.state.requests.length, 0)
    assert.equal((await p.rename('Manual')).kind, 'value')
  }
  const p = await fixture(t)
  p.adopted()
  await p.apply(await p.input(null))
  await p.submit('child', 'child')
  const foreign = await p.input([{ type: 'text', text: 'other session' }])
  await p.apply(foreign, 'main', 'other')
  await flush()
  assert.equal(p.state.requests.length, 0)
  const images = await fixture(t)
  images.adopted()
  await images.apply(await images.input([{ type: 'image', source: { type: 'url', mediaType: 'image/png', url: 'image' } }]))
  await flush()
  const imagePages = images.state.pages.length
  await images.submit('text after image-only first input')
  assert.equal(images.state.requests.length, 0)
  assert.equal(images.state.pages.length, imagePages)
  const disabled = await fixture(t, { document: { enabled: false, maxLength: 80, model: null } })
  let metadataReads = 0
  disabled.state.beforeRead = () => { metadataReads++ }
  disabled.adopted()
  await disabled.submit('disabled')
  await disabled.submit('still disabled')
  assert.equal(disabled.state.requests.length, 0)
  assert.equal(disabled.state.pages.length, 0)
  assert.equal(metadataReads, 0)
  disabled.state.document.enabled = true
  await disabled.submit('enabled')
  assert.equal(disabled.state.requests[0].request.messages[0].content, 'disabled')
  const external = await fixture(t)
  external.adopted('external-agent:node:agent', 'default')
  await external.submit('private external conversation')
  assert.equal(external.state.requests.length, 0)
  const externalPages = external.state.pages.length
  await external.submit('still using external agent')
  assert.equal(external.state.pages.length, externalPages)
  await external.emit(external.adopted('native-provider', 'native-model'))
  await external.submit('native model after switch')
  assert.deepEqual(external.state.requests[0].model, { providerId: 'native-provider', modelId: 'native-model' })
  assert.equal(external.state.requests[0].request.messages[0].content, 'private external conversation')
  const missing = await fixture(t)
  await missing.submit('no adopted model yet')
  assert.equal(missing.state.requests.length, 0)
  missing.adopted('new-provider', 'new-model')
  await missing.submit('after adoption')
  assert.deepEqual(missing.state.requests[0].model, { providerId: 'new-provider', modelId: 'new-model' })
  assert.equal(missing.state.requests[0].request.messages[0].content, 'no adopted model yet')
})

test('overlapping adoption reads produce one title from the first operator input', async t => {
  const p = await fixture(t)
  p.adopted()
  const release = deferred()
  p.state.beforeRead = () => release.promise
  await p.submit('first operator input')
  await p.submit('second operator input')
  release.resolve()
  await flush()
  assert.equal(p.state.requests.length, 1)
  assert.equal(p.state.requests[0].request.messages[0].content, 'first operator input')
  assert.equal(p.state.metadata.revision, 1)
})

test('three failed attempts stop until reload; Settings failures never use a fallback document', async t => {
  const p = await fixture(t, { settingsError: true })
  p.adopted()
  for (let i = 0; i < 3; i++) await p.submit('first source')
  p.state.settingsError = false
  await p.submit('no fourth attempt')
  assert.equal(p.state.requests.length, 0)
  await p.loaded.unload()
  const restored = await fixture(t, { events: p.state.events, metadata: p.state.metadata })
  await restored.submit('reload permits another attempt')
  assert.equal(restored.state.requests[0].request.messages[0].content, 'first source')
  const broken = await fixture(t, { settingsError: true })
  for (const title of [0, '', 'line\nbreak']) {
    assert.equal((await broken.rename(title)).appError, 'invalid_params')
  }
  const unknown = await broken.loaded.service('sessionMethods:session.rename').call({
    callId: 'unknown-method', method: 'session.unknown', params: { title: 'Valid' },
  }, pending(), () => {})
  assert.equal(unknown.appError, 'invalid_params')
  assert.equal((await broken.rename('Manual')).appError, 'storage_failed')
  assert.equal(broken.state.metadata.revision, 0)
})

test('manual validation, bounded conflict retries and cancellation before store handover', async t => {
  const p = await fixture(t, { document: { enabled: true, maxLength: 10, model: null } })
  for (const title of ['', '   ', 'line\nbreak', '\u0001text', 'x'.repeat(11), 1]) {
    assert.equal((await p.rename(title)).appError, 'invalid_params')
  }
  assert.equal(p.state.metadata.revision, 0)
  let conflicts = 0
  p.state.beforeWrite = () => {
    if (conflicts++ < 3) p.state.metadata = { revision: p.state.metadata.revision + 1, title: { text: 'Other', source: 'manual' } }
  }
  assert.equal((await p.rename('😀'.repeat(10))).value.title, '😀'.repeat(10))
  p.state.beforeWrite = () => { p.state.metadata = { revision: p.state.metadata.revision + 1, title: { text: 'Other', source: 'manual' } } }
  assert.equal((await p.rename('Contended')).appError, 'service_reloading')
  const before = p.state.metadata.revision
  assert.equal((await p.rename('Cancelled', Promise.resolve())).appError, 'service_reloading')
  assert.equal(p.state.metadata.revision, before)
})

test('closing cancels and joins the original auxiliary call; admitted store writes settle once', async t => {
  const p = await fixture(t)
  p.adopted()
  const entered = deferred()
  const cancelled = deferred()
  const settled = deferred()
  p.state.complete = async (_request, cancellation) => {
    entered.resolve()
    await cancellation()
    cancelled.resolve()
    await settled.promise
    throw new Error('cancelled')
  }
  await p.apply(await p.input([{ type: 'text', text: 'first' }]))
  await entered.promise
  p.closing.resolve()
  await cancelled.promise
  let unloaded = false
  const unload = p.loaded.unload().then(() => { unloaded = true })
  await flush()
  assert.equal(unloaded, false)
  settled.resolve()
  await unload
  assert.equal(p.state.metadata.revision, 0)
  const writer = await fixture(t)
  writer.adopted()
  const admitted = deferred()
  const release = deferred()
  writer.state.beforeWrite = async () => { admitted.resolve(); await release.promise }
  await writer.apply(await writer.input([{ type: 'text', text: 'store handover' }]))
  await admitted.promise
  writer.closing.resolve()
  const end = writer.loaded.unload()
  release.resolve()
  await end
  assert.equal(writer.state.metadata.title.source, 'auto')
  assert.equal(writer.state.notices.length, 1)
})

test('the generation budget cancels the original call and consumes a failed attempt', async t => {
  const p = await fixture(t)
  p.adopted()
  t.mock.timers.enable({ apis: ['setTimeout'] })
  let joined = 0
  p.state.complete = async (_request, cancelled) => {
    await cancelled()
    joined++
    throw new Error('cancelled')
  }
  for (let i = 0; i < 3; i++) {
    await p.submit('budget')
    t.mock.timers.tick(30_000)
    await flush()
  }
  assert.equal(joined, 3)
  await p.submit('limit reached')
  assert.equal(p.state.requests.length, 3)
  assert.equal(p.state.metadata.title, null)
})
