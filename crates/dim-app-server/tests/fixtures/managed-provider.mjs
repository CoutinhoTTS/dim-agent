import { definePlugin } from '__RUTIS_SDK__'
import { appendFile, readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { randomUUID } from 'node:crypto'

const methods = ['providers', 'status', 'attemptStatus', 'catalog', 'credential', 'recoverRejected', 'finishRejected', 'release', 'quiesce']
export default definePlugin({
  name: 'ManagedFixture',
  provides: { managedProvider: Object.fromEntries(methods.map(name => [name, 'async'])) },
  config: { type: 'object', required: ['home'], properties: { home: { type: 'string' } } },
  async apply(ctx, { home }) {
    const { baseUrl, entry } = JSON.parse(await readFile(join(home, 'fixture.json'), 'utf8'))
    const account = { credentialEpoch: randomUUID(), generation: 1, account: { issuer: 'https://accounts.example', subject: 'fixture', grantedScopes: ['model'] } }
    const leases = new Set()
    let recovered = false
    const record = method => appendFile(join(home, 'calls.jsonl'), JSON.stringify({ method }) + '\n')
    const lease = () => {
      const leaseId = randomUUID()
      leases.add(leaseId)
      return { leaseId, account, protocol: 'openai-compatible', baseUrl, headers: {}, bearer: recovered ? 'recovered-fixture-secret' : 'initial-fixture-secret' }
    }
    ctx.provide('managedProvider', {
      async providers() { return { ok: true, value: [{ providerId: 'managed-fixture', displayName: 'Managed fixture', loginMethods: ['browser'] }] } },
      async status({ providerId }) {
        if (providerId === 'fail') return { ok: false, error: { code: 'invalid_store', message: 'private-error-secret' } }
        if (providerId === 'terminate-runtime') { setTimeout(() => process.exit(77), 20); return new Promise(() => {}) }
        return { ok: true, value: account }
      },
      async attemptStatus() { return { ok: true, value: { status: 'failed', error: 'private-error-secret' } } },
      async catalog() { return { ok: true, value: { account, models: [{ entry, protocol: 'openai-compatible', unavailableReason: null, zdr: false }] } } },
      async credential() { await record('credential'); return { ok: true, value: lease() } },
      async recoverRejected({ leaseId }) {
        if (!leases.has(leaseId)) return { ok: false, error: { code: 'lease_not_found', message: 'lease expired' } }
        recovered = true
        await record('recoverRejected')
        return { ok: true, value: lease() }
      },
      async finishRejected() { await record('finishRejected'); return { ok: true, value: {} } },
      async release({ leaseId }) { leases.delete(leaseId); await record('release'); return { ok: true, value: {} } },
      async quiesce() { return { ok: true, value: {} } },
    })
    ctx.effect(async () => { leases.clear(); await record('close') })
  },
})
