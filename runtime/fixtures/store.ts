import { definePlugin } from '@arcships/rutis'
import type { ManagementMethods, MethodReply, Store } from '../../npm/plugin.js'
import { writeFile, rm } from 'node:fs/promises'
import assert from 'node:assert/strict'

export default definePlugin({
  name: 'StoreFixture',
  provides: {},
  inject: ['managementMethods', 'store'],
  async apply(ctx) {
    const store = ctx.use<Store>('store')
    let release: (() => void) | undefined
    await ctx.use<ManagementMethods>('managementMethods').register(
      'storefixture',
      ['status', 'create', 'cycle', 'remove', 'refuse', 'release'].map(name => ({ name: `storefixture.${name}`, mutating: true, interruption: 'report', progress: false })),
      async (call: { method: string; params: any }): Promise<MethodReply> => {
        const params = call.params
        if (call.method === 'storefixture.status') return { kind: 'value', value: {} }
        if (call.method === 'storefixture.create') {
          if (!params.create) throw new Error('missing creation request')
          const result = await store.createOwnedSession(params.create)
          return { kind: 'value', value: result }
        }
        if (call.method === 'storefixture.release') { release?.(); return { kind: 'value', value: {} } }
        if (call.method === 'storefixture.refuse') {
          try {
            await store.removeWorktree(params.directory, async () => {
              throw Object.assign(new Error('fixture refusal'), { code: 'invalid' })
            })
            throw new Error('removal unexpectedly succeeded')
          } catch (error) {
            if (!(error instanceof Error) || !('code' in error) || typeof error.code !== 'string') throw error
            return { kind: 'value', value: { code: error.code, message: error.message } }
          }
        }
        if (call.method === 'storefixture.remove') {
          const removed = await store.removeWorktree(params.directory, async path => {
            const gate = new Promise<void>(resolve => { release = resolve })
            await writeFile(params.ready, path)
            await gate
            await rm(path, { recursive: true })
          })
          return { kind: 'value', value: removed }
        }
        const { sessionId, forkId, grant } = params
        await store.addGrant({ sessionId, path: grant, kind: 'dir', mode: 'write' })
        await store.addGrant({ sessionId, path: grant, kind: 'dir', mode: 'read' })
        const grants = await store.grants({ sessionId })
        await store.setSessionLanguage({ sessionId, language: 'zh-CN' })
        const config = await store.sessionConfigText({ sessionId })
        if (config.configText === null) throw new Error('missing source')
        await store.fork({ sourceSessionId: sessionId, sessionId: forkId, at: 0, configText: config.configText })
        const fork = await store.hasRecords({ sessionId: forkId })
        const removed = await store.delete({ sessionId: forkId })
        const absent = await store.hasRecords({ sessionId: forkId })
        const origin = await store.creationRecord({ sessionId })
        const owner = await store.owner({ sessionId })
        const query = await store.query({ scope: {
          key: { sessionId, branchId: 'main' }, through: 0,
          segments: [{ branchId: 'main', from: 1, through: 0, storedThrough: 0, tail: [] }],
        }, query: { select: { kind: 'count' } } })
        assert.deepEqual(query, { result: { kind: 'count', count: 0 }, next: null })
        if (owner.agentId === null) throw new Error('missing source owner')
        const catalog = await store.catalog({ request: {}, version: 3, hosts: [owner.agentId], closed: [] })
        assert.deepEqual(catalog.sessions.map(session => session.sessionId), [sessionId])

        await store.removeGrant({ sessionId, path: grant })
        let missing: { code: string; message: string } | null = null
        try {
          await store.setSessionLanguage({ sessionId: 'wire-missing', language: null })
        } catch (error) {
          if (!(error instanceof Error) || !('code' in error) || typeof error.code !== 'string') throw error
          missing = { code: error.code, message: error.message }
        }
        return { kind: 'value', value: { grants, config, fork, removed, absent, origin, owner, missing } }
      },
    )
  },
})
