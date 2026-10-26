import { definePlugin } from '@arcships/rutis'
import { partRow } from './part-row.ts'
import provides from './settings-owner.json' with { type: 'json' }
import type { SessionMethods } from '../../npm/plugin.js'

type Envelope = { ok: boolean; value?: unknown; error?: { code: string; message: string } }
type Settings = { read(request: unknown): Promise<Envelope>; update(request: unknown): Promise<Envelope> }
type MethodCall = { method: string; params: { sessionId: string; [key: string]: unknown } }

// The owner of source `example` (settings-service §11): it reads and writes
// its document through `settings`; each method returns the envelope unchanged.
// It also records the Settings events it listens to on the bus (§4.5).
export const part = definePlugin({
  name: 'SettingsOwnerFixture',
  provides,
  inject: ['settings', 'sessionMethods'],
  async apply(ctx) {
    const settings = ctx.use<Settings>('settings')
    const events: { name: string; payload: unknown }[] = []
    for (const name of ['settings/example', 'settings/projects']) {
      ctx.on(name, (payload: unknown) => { events.push({ name, payload }) })
    }
    await ctx.use<SessionMethods>('sessionMethods').register(
      'session.example',
      ['read', 'update', 'events'].map(name => ({
        name: `session.example.${name}`, mutating: false, interruption: 'report', progress: false,
      })),
      async (call: MethodCall) => {
        const { sessionId: _, ...request } = call.params
        if (call.method.endsWith('.events')) return { kind: 'value', value: events }
        const reply = call.method.endsWith('.read') ? await settings.read(request) : await settings.update(request)
        return { kind: 'value', value: reply }
      },
    )
  },
})

export default partRow('session', part)
