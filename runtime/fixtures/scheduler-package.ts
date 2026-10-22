import { createConnection } from 'node:net'
import { once } from 'node:events'
import { mock } from 'node:test'
import { definePlugin, type Context } from '@arcships/rutis'
import { z } from 'zod'
import scheduler from '@dimcode/scheduler'
import type { FileError, FileLease, ManagementMethodsContribution, PluginFiles, SessionPermissions, Sessions, SessionsError } from '../../npm/plugin.js'

const gate = z.strictObject({
  phase: z.enum(['launch_reply', 'input_reply', 'report_input', 'report_reply', 'reuse_reply', 'policy_reply']),
  host: z.string(), port: z.number().int(),
})
const controls = z.strictObject({ now: z.iso.datetime().optional(), gate: gate.optional() })

// Exercise the real package through its public ports, keeping every native call on its original row.
export default definePlugin({
  name: 'SchedulerPackageProbe',
  inject: scheduler.inject, config: scheduler.config,
  provides: { ...scheduler.provides, 'managementMethods:schedulerprobe': { describe: 'async', call: 'async' } },
  async apply(ctx, config) {
    let selected: z.infer<typeof gate> | undefined
    let clock = false
    let replacementError: FileError | null = null
    let lookupError: SessionsError | null = null
    ctx.effect(() => { if (clock) mock.timers.reset() })
    const pause = async (point: z.infer<typeof gate>['phase']) => {
      const next = selected
      if (next?.phase !== point) return
      selected = undefined
      const socket = createConnection({ host: next.host, port: next.port })
      ctx.effect(() => { socket.destroy() })
      try {
        await once(socket, 'connect')
        for await (const _ of socket) break
      } finally { socket.destroy() }
    }
    const native = ctx.use<Sessions>('sessions')
    const observed: Sessions = {
      async launch(request) {
        const reply = await native.launch(request)
        await pause('launch_reply')
        return reply
      },
      async input(request) {
        if (request.mode === 'steer') await pause('report_input')
        const reply = await native.input(request)
        await pause(request.mode === 'steer' ? 'report_reply' : 'input_reply')
        return reply
      },
      async lookup(request) {
        const reply = await native.lookup(request)
        if (!reply.ok) lookupError = reply.error
        return reply
      },
      history: request => native.history(request),
      withdraw: request => native.withdraw(request), stop: request => native.stop(request),
      idle: request => native.idle(request), delete: request => native.delete(request),
    }
    const permissions = ctx.use<SessionPermissions>('sessionPermissions')
    const observedPermissions: SessionPermissions = {
      async permission(request) { const reply = await permissions.permission(request); await pause('reuse_reply'); return reply },
      async permissionDefaults(request) { const reply = await permissions.permissionDefaults(request); await pause('policy_reply'); return reply },
    }
    const files = ctx.use<PluginFiles>('pluginFiles')
    const observedFiles: PluginFiles = {
      async open() {
        const scope = await files.open()
        return {
          withLock(path, work, cancelled) {
            const execute = (file: FileLease) => work({
              read: () => file.read(), backup: suffix => file.backup(suffix),
              async replace(text) {
                const reply = await file.replace(text)
                if (!reply.ok) replacementError = reply.error
                return reply
              },
            })
            return cancelled === undefined ? scope.withLock(path, execute) : scope.withLock(path, execute, cancelled)
          },
          quiesce: () => scope.quiesce(),
          close: () => scope.close(),
        }
      },
    }
    const methods: ManagementMethodsContribution = {
      async describe() { return [
        { name: 'schedulerprobe.status', mutating: false, interruption: 'report', progress: false },
        { name: 'schedulerprobe.configure', mutating: true, interruption: 'drain', progress: false },
      ] },
      async call(call) {
        if (call.method === 'schedulerprobe.configure') {
          const input = controls.parse(call.params)
          if (input.now !== undefined) {
            const now = Date.parse(input.now)
            if (clock) mock.timers.setTime(now)
            else { mock.timers.enable({ apis: ['Date'], now }); clock = true }
          }
          if (input.gate) selected = input.gate
        }
        return { kind: 'value', value: { pid: process.pid, replacementError, lookupError: lookupError && { ...lookupError } } }
      },
    }
    ctx.provide('managementMethods:schedulerprobe', methods)
    const context: Context = {
      use<T>(name: string): T {
        if (name === 'sessions') return observed as T
        if (name === 'pluginFiles') return observedFiles as T
        if (name === 'sessionPermissions') return observedPermissions as T
        return ctx.use<T>(name)
      },
      provide: (name, value) => ctx.provide(name, value), effect: cleanup => ctx.effect(cleanup),
      on: (name, listener) => ctx.on(name, listener), emit: (name, ...args) => ctx.emit(name, ...args),
    }
    await scheduler.apply(context, config)
  },
})
