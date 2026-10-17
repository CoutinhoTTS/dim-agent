import { definePlugin } from '@arcships/rutis'
import provides from './settings-owner.json' with { type: 'json' }

type Envelope = { ok: boolean; value?: unknown; error?: { code: string; message: string } }
type Settings = { read(request: unknown): Promise<Envelope>; update(request: unknown): Promise<Envelope> }
type MethodCall = { method: string; params: { sessionId: string; [key: string]: unknown } }

// The owner of source `example` (settings-service §11): it reads and writes
// its document through `settings`; each method returns the envelope unchanged.
// It also records the Settings events the host delivers (§4.5).
export default definePlugin({
  name: 'SettingsOwnerFixture',
  provides,
  inject: ['settings'],
  apply(ctx) {
    const settings = ctx.use<Settings>('settings')
    const events: { name: string; payload: unknown }[] = []
    for (const name of ['settings.changed', 'settings.project_authorized']) {
      ctx.on(name, (payload: unknown) => { events.push({ name, payload }) })
    }
    ctx.provide('sessionMethods:session.example', {
      async describe() {
        return ['read', 'update', 'events'].map(name => ({
          name: `session.example.${name}`, mutating: false, interruption: 'report', progress: false,
        }))
      },
      async call(call: MethodCall) {
        const { sessionId: _, ...request } = call.params
        if (call.method.endsWith('.events')) return { kind: 'value', value: events }
        const reply = call.method.endsWith('.read') ? await settings.read(request) : await settings.update(request)
        return { kind: 'value', value: reply }
      },
    })
  },
})
