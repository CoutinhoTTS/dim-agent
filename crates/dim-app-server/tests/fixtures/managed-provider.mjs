import { definePlugin } from '__RUTIS_SDK__'
import { appendFile, readFile } from 'node:fs/promises'
import { join } from 'node:path'
import { randomUUID } from 'node:crypto'

const methods = ['providers', 'status', 'attemptStatus', 'catalog', 'credential', 'recoverRejected', 'finishRejected', 'release', 'quiesce', 'usage']
export default definePlugin({
  name: 'ManagedFixture',
  inject: ['pluginFiles'],
  provides: { managedProvider: Object.fromEntries(methods.map(name => [name, 'async'])) },
  config: { type: 'object', required: ['home'], properties: { home: { type: 'string' } } },
  async apply(ctx, { home }) {
    const files = await ctx.use('pluginFiles').open()
    ctx.effect(() => files.close())
    const stored = await files.withLock('fixture-state', async file => {
      const written = await file.replace('mounted')
      return written.ok ? file.read() : written
    })
    if (!stored.ok || !stored.value.ok || stored.value.value !== 'mounted') throw Error('fixture storage failed')
    const { baseUrl, entry } = JSON.parse(await readFile(join(home, 'fixture.json'), 'utf8'))
    let account = { credentialEpoch: randomUUID(), generation: 1, account: { issuer: 'https://accounts.example', subject: 'fixture', email: 'fixture@example.com', name: 'Fixture', grantedScopes: ['model'] } }
    const currentAccount = async () => {
      const latest = JSON.parse(await readFile(join(home, 'fixture.json'), 'utf8'))
      if (latest.account) account = latest.account
      return account
    }
    const usage = { plan: { name: 'Fixture plan', status: 'active', expiresAt: '2026-10-28T00:00:00Z' }, credits: { usedUnits: 25000, totalUnits: 300000, resetsAt: '2026-10-28T00:00:00Z' }, windows: [{ durationHours: 5, tokenUsed: 1200, tokenCap: 10000 }], features: [{ key: 'web_search', used: 211, allowance: 1500, unit: 'call' }] }
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
        if (providerId === 'crash-runtime') process.exit(77)
        if (providerId === 'terminate-runtime') return files.withLock('fixture-state', async file => {
          const saved = await file.replace('before-crash')
          if (!saved.ok) throw Error('fixture storage failed')
          return new Promise(() => {})
        })
        return { ok: true, value: await currentAccount() }
      },
      async attemptStatus() { return { ok: true, value: { status: 'failed', error: 'private-error-secret' } } },
      async usage({ providerId }) { return providerId === 'managed-fixture' ? { ok: true, value: usage } : { ok: false, error: { code: 'token_missing', message: 'signed out' } } },
      async catalog() { return { ok: true, value: { account: await currentAccount(), models: [{ entry, protocol: 'openai-compatible', unavailableReason: null, zdr: false }] } } },
      async credential() { await record('credential'); return { ok: true, value: lease() } },
      async recoverRejected({ leaseId }) {
        if (!leases.has(leaseId)) return { ok: false, error: { code: 'lease_not_found', message: 'lease expired' } }
        recovered = true
        await record('recoverRejected')
        return { ok: true, value: lease() }
      },
      async finishRejected() { await record('finishRejected'); return { ok: true, value: {} } },
      async release({ leaseId }) { leases.delete(leaseId); await record('release'); return { ok: true, value: {} } },
      async quiesce() { await files.quiesce(); return { ok: true, value: {} } },
    })
    ctx.effect(async () => { leases.clear(); await record('close') })
  },
})
