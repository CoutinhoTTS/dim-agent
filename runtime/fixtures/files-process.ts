import { createConnection } from 'node:net'
import { once } from 'node:events'
import { definePlugin } from '@arcships/rutis'
import type { FsHost, ManagementMethods, PluginFiles } from '../../npm/plugin.js'

export default definePlugin({
  name: 'PublicFilesProcessFixture',
  inject: ['pluginFiles', 'fsHost', 'managementMethods'],
  provides: {},
  async apply(ctx, config: { gate?: { host: string; port: number } }) {
    const pluginFiles = ctx.use<PluginFiles>('pluginFiles')
    const scope = await pluginFiles.open()
    const fs = ctx.use<FsHost>('fsHost')
    ctx.effect(() => scope.close())
    await ctx.use<ManagementMethods>('managementMethods').register(
      'files',
      ['read', 'write', 'hold', 'close', 'missing', 'invalid', 'bytes', 'envelope', 'list', 'remove', 'invalid_names', 'directory', 'pinned', 'absent', 'stream', 'failed_stream', 'cancel_stream'].map(name => ({
        name: `files.${name}`,
        mutating: ['write', 'hold', 'close', 'bytes', 'remove', 'directory', 'stream', 'failed_stream', 'cancel_stream'].includes(name),
        interruption: 'drain', progress: name === 'hold',
      })),
      async (call, _cancelled, progress) => {
        if (call.method === 'files.cancel_stream') {
          const upload = await pluginFiles.open()
          const entered = Promise.withResolvers<void>()
          const source = Promise.withResolvers<null>()
          const written = upload.writeStream('media/work/input.bin', () => {
            entered.resolve()
            return source.promise
          })
          try {
            await entered.promise
            await upload.close()
            return { kind: 'value', value: await written }
          } finally { source.resolve(null) }
        }
        if (call.method === 'files.stream' || call.method === 'files.failed_stream') {
          let chunk = 0
          const result = await scope.writeStream('media/work/input.bin', async () => {
            if (call.method === 'files.failed_stream' && chunk === 1) throw new Error('source unavailable')
            if (chunk === 17) return null
            return new Uint8Array(1024 * 1024).fill(++chunk)
          })
          return { kind: 'value', value: result }
        }
        if (call.method === 'files.directory') return { kind: 'value', value: await scope.directory('media/work') }
        if (call.method === 'files.absent') return { kind: 'value', value: await scope.openRead('public-files/missing.bin') }
        if (call.method === 'files.pinned') {
          const opened = await scope.openRead('public-files/blob.bin')
          if (!opened.ok || !opened.value) throw new Error('expected a pinned file')
          const { readId, size } = opened.value
          try {
            const { bytes } = await fs.readAt({ readId, offset: 1, limit: 3 })
            return { kind: 'value', value: { bytes, size } }
          } finally { await fs.closeRead({ readId }) }
        }
        if (call.method === 'files.close') {
          await scope.close()
          return { kind: 'value', value: null }
        }
        if (call.method === 'files.bytes') {
          const result = await scope.withLock('public-files/blob.bin', async file => {
            const replaced = await file.replaceBytes(new Uint8Array([0, 1, 2, 250, 255, 128]))
            if (!replaced.ok) return replaced
            const read = await file.readBytes()
            if (!read.ok) return read
            return { ok: true, value: read.value === null ? null : Array.from(read.value) }
          })
          return { kind: 'value', value: result }
        }
        if (call.method === 'files.envelope') {
          const result = await scope.withLock('public-files/state.json', async file => {
            const read = await file.readBytes()
            if (!read.ok) return read
            return { ok: true, value: read.value === null ? null : new TextDecoder().decode(read.value) }
          })
          return { kind: 'value', value: result }
        }
        if (call.method === 'files.list') {
          return { kind: 'value', value: await scope.list('public-files') }
        }
        if (call.method === 'files.remove') {
          return { kind: 'value', value: await scope.remove('public-files/blob.bin') }
        }
        if (call.method === 'files.invalid_names') {
          return {
            kind: 'value',
            value: [
              await scope.list('../escape'),
              await scope.remove('/absolute'),
              await scope.list(''),
            ],
          }
        }
        const result = call.method === 'files.missing'
          ? await scope.withLock('unprovisioned/runs.json', () => { throw new Error('missing parent entered callback') })
          : await scope.withLock(call.method === 'files.invalid' ? 'public-files/invalid.json' : 'public-files/state.json', async file => {
            if (call.method === 'files.hold') {
              if (!config.gate) throw new Error('fixture requires a gate')
              const socket = createConnection(config.gate)
              ctx.effect(() => { socket.destroy() })
              try {
                await once(socket, 'connect')
                progress({ phase: 'started' })
                await once(socket, 'data')
                return await file.replace('settled public process text')
              } finally { socket.destroy() }
            }
            return call.method === 'files.write'
              ? await file.replace('public process text')
              : await file.read()
          })
        return { kind: 'value', value: result }
      },
    )
  },
})
