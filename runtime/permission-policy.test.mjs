import assert from 'node:assert/strict'
import test from 'node:test'
import { RunStore as WorkflowStore } from './plugins/workflow/store.ts'

const legacy = { preset: 'full-access', allowedTools: ['read'], disallowedTools: ['exec'], fileAccess: 'read-only', toolApproval: 'ask' }
const policy = { preset: 'full-access', allowedTools: ['read'], disallowedTools: ['exec'] }
const at = '2026-10-22T00:00:00.000Z'
const cases = [
  { Store: WorkflowStore, name: 'workflow', version: 2, field: 'policy_cap', record: {
    run_id: 'legacy', name: 'legacy', source: 'user', params: {}, cwd: '/tmp', policy_cap: legacy,
    definition: { schema: 'dim.workflow/v1', name: 'legacy', description: 'Stored run', steps: [{ stage: { id: 'one', prompt: 'done' } }] },
    started_by: 'management', created_at: at, ended_at: at, state: 'completed', log: [],
    stages: [{ id: 'one', step: 0, state: 'completed', result_text: 'done' }], report: { state: 'none' },
  } },
]

for (const { Store, name, version, field, record } of cases) {
  test(`${name} reads legacy policy fields without rewriting and drops them on the next save`, async () => {
    const path = `${name}/runs.json`
    const original = JSON.stringify({ version, runs: [record] })
    const files = new Map([[path, original]])
    const scope = { async withLock(path, work) {
      return { ok: true, value: await work({
        read: async () => ({ ok: true, value: files.get(path) ?? null }),
        replace: async text => { files.set(path, text); return { ok: true, value: null } },
      }) }
    } }
    const store = new Store(scope)
    const expected = { ...record, [field]: policy }
    assert.deepEqual(await store.read(), [expected])
    assert.equal(files.get(path), original)
    await store.update(() => {})
    assert.deepEqual(JSON.parse(files.get(path)), { version, runs: [expected] })
    for (const document of [
      { version, runs: [record], unknown: true },
      { version, runs: [{ ...record, unknown: true }] },
      { version, runs: [{ ...record, [field]: { ...legacy, preset: 'invalid' } }] },
    ]) {
      const text = JSON.stringify(document)
      files.set(path, text)
      await assert.rejects(new Store(scope).read(), error => error.code === 'invalid_data')
      assert.equal(files.get(path), text)
    }
  })
}
