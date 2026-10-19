import { definePlugin } from '@arcships/rutis'
import provides from './session-metadata.json' with { type: 'json' }

type Envelope = { ok: boolean; value?: unknown; error?: { code: string; message: string } }
type SessionMetadata = { read(request: unknown): Promise<Envelope>; setTitle(request: unknown): Promise<Envelope> }
type MethodCall = { method: string; params: { sessionId: string; [key: string]: unknown } }

// Uses `sessionMetadata` as SessionTitle does (session-title-metadata §12):
// each method passes its Session's request through and returns the envelope
// unchanged.
export default definePlugin({
  name: 'SessionMetadataFixture',
  provides,
  inject: ['sessionMetadata'],
  apply(ctx) {
    const metadata = ctx.use<SessionMetadata>('sessionMetadata')
    ctx.provide('sessionMethods:session.titles', {
      async describe() {
        return ['read', 'set'].map(name => ({
          name: `session.titles.${name}`, mutating: false, interruption: 'report', progress: false,
        }))
      },
      async call(call: MethodCall) {
        const reply = call.method.endsWith('.read')
          ? await metadata.read({ sessionId: call.params.sessionId })
          : await metadata.setTitle(call.params)
        return { kind: 'value', value: reply }
      },
    })
  },
})
