import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { registerHooks, stripTypeScriptTypes } from "node:module"
import test from "node:test"

registerHooks({
  resolve(specifier, context, next) {
    try { return next(specifier, context) } catch (error) {
      if (specifier.startsWith(".") && !specifier.endsWith(".ts")) return next(`${specifier}.ts`, context)
      throw error
    }
  },
  load(url, context, next) {
    if (url.endsWith(".ts")) return { format: "module", source: stripTypeScriptTypes(readFileSync(new URL(url), "utf8"), { mode: "transform" }), shortCircuit: true }
    return next(url, context)
  },
})
const { createPanelContext, panelTitle, panelUnavailable, resourceUrl } = await import("../src/features/extensions/context.ts")
const { readExtensions, readNotification } = await import("../src/rpc/read.ts")
const { RpcError } = await import("../src/rpc/client.ts")
const panel = {
  id: "preview", entry: "panel.js", title: { default: "Preview", translations: { zh: "预览" } }, placement: "session-panel",
  requires: ["session.state", "session.history", "session.submit", "session.interrupt", "resources"],
  methods: [{ name: "session.preview.list", scope: "session" }, { name: "session.preview.open", scope: "branch" }],
}
const environment = { sessionId: "session", branchId: "worker", theme: "dark", locale: "en" }

test("a panel observes its bound view and calls only declared targets until disposal", async () => {
  const calls = []
  const event = { id: "event", sequence: 1, create_at: 1, type: "preview", name: "ready", data: { files: [{ id: "file", nested: { width: 24 } }] }, tags: ["preview"] }
  const environmentListeners = new Set()
  const activityListeners = new Set()
  let currentEnvironment = environment
  let currentActivity = { branch: { running: false, stopping: true }, sessionRunning: true }
  const subscribe = (listeners, value) => listener => { listeners.add(listener); listener(value()); return () => listeners.delete(listener) }
  const host = {
    environment,
    observeEnvironment: subscribe(environmentListeners, () => currentEnvironment),
    observeActivity: subscribe(activityListeners, () => currentActivity),
    unavailable() { assert.fail("available provider") },
    client: { async call(method, params, read) {
      calls.push({ method, params })
      return read(method === "session.history" ? { events: [event], through: 1, nextAfter: 1, done: true } : method === "session.submit" ? { event } : { sequence: 1 })
    } },
  }
  const binding = createPanelContext(panel, host)
  const context = binding.context
  const environments = []
  const activities = []
  context.observeEnvironment(value => environments.push(value))
  context.services.session.state.observe(value => activities.push(value))
  assert.deepEqual(environments, [environment])
  assert.deepEqual(activities, [currentActivity])
  currentEnvironment = { ...environment, theme: "light", locale: "zh-CN" }
  for (const listener of environmentListeners) listener(currentEnvironment)
  currentActivity = { branch: { running: false, stopping: false }, sessionRunning: true }
  for (const listener of activityListeners) listener(currentActivity)
  assert.equal(environments.at(-1).locale, "zh-CN")
  assert.equal(context.environment.locale, "en")
  assert.equal(activities.at(-1).sessionRunning, true)
  await context.methods["session.preview.list"]({ page: 1 })
  await context.methods["session.preview.open"]({ file: "image" })
  assert.deepEqual((await context.services.session.history.read({ limit: 32 })).events, [event])
  assert.deepEqual(await context.services.session.submit({ user: { content: [{ type: "text", text: "hi" }] }, mode: "queue", submissionId: "input-1" }), { event })
  assert.deepEqual(await context.services.session.interrupt(), { sequence: 1 })
  assert.equal(calls[0].params.sessionId, "session")
  assert.equal(calls[0].params.branchId, undefined)
  for (const call of calls.slice(1)) assert.equal(call.params.branchId, "worker")
  const count = calls.length
  await assert.rejects(context.methods["session.preview.open"]({ sessionId: "other" }), { code: "invalid_binding" })
  await assert.rejects(context.services.session.history.read({ branchId: "other", limit: 32 }), { code: "invalid_binding" })
  assert.equal(calls.length, count)
  assert.equal(context.call, undefined)
  assert.equal(context.methods["provider.delete"], undefined)
  assert.equal(context.services.resources.url({ sessionId: "session", id: "file-id" }), "/api/ui/sessions/session/resources/file-id")
  assert.throws(() => context.services.resources.url({ sessionId: "other", id: "file-id" }), { code: "invalid_binding" })
  binding.dispose()
  assert.equal(context.signal.aborted, true)
  assert.equal(environmentListeners.size, 0)
  assert.equal(activityListeners.size, 0)
  await assert.rejects(context.methods["session.preview.list"]({}), { code: "binding_closed" })
  assert.throws(() => context.services.resources.url({ sessionId: "session", id: "file-id" }), { code: "binding_closed" })
})

test("late results cannot revive a disposed binding and provider errors retain their identity", async () => {
  let complete
  const errors = []
  const host = { environment, observeEnvironment() {}, observeActivity() {}, unavailable: error => errors.push(error), client: { call: () => new Promise(resolve => { complete = resolve }) } }
  const binding = createPanelContext(panel, host)
  const pending = binding.context.methods["session.preview.list"]({})
  binding.dispose()
  complete({ accepted: true })
  await assert.rejects(pending, { code: "binding_closed" })
  assert.deepEqual(errors, [])
  const rejected = new RpcError(-32000, "unloaded", "service_reloading")
  host.client.call = async () => { throw rejected }
  const replacement = createPanelContext(panel, host)
  await assert.rejects(replacement.context.methods["session.preview.list"]({}), error => error === rejected)
  assert.deepEqual(errors, [rejected])
  replacement.dispose()
})

test("directory parsing, localization and capability selection keep the browser contract finite", () => {
  const directory = { revision: 3, extensions: [{ pluginName: "preview", generation: "snapshot", panels: [panel] }], errors: [] }
  assert.deepEqual(readExtensions(directory), { ...directory, extensions: [{ ...directory.extensions[0], panels: [{ ...panel, icon: undefined }] }] })
  assert.deepEqual(readNotification("ui.extensions.changed", { revision: 4 }), { method: "ui.extensions.changed", revision: 4 })
  assert.throws(() => readExtensions({ ...directory, revision: "3" }))
  assert.equal(panelTitle(panel, "zh-CN"), "预览")
  assert.equal(panelUnavailable(panel, () => true, panel.methods.map(method => method.name)), undefined)
  assert.equal(panelUnavailable({ ...panel, requires: ["future.api"] }, () => true, []), "Unsupported capability: future.api")
  assert.equal(panelUnavailable(panel, () => false, []), "Unavailable capability: session.history")
  assert.equal(panelUnavailable(panel, () => true, []), "Unavailable method: session.preview.list")
  assert.equal(resourceUrl("snapshot", "assets/with spaces.js"), "/api/ui/extensions/snapshot/assets/with%20spaces.js")
  const minimal = createPanelContext({ ...panel, requires: [], methods: [] }, { environment })
  assert.deepEqual(minimal.context.services, {})
  assert.deepEqual(minimal.context.methods, {})
  minimal.dispose()
  assert.throws(() => createPanelContext({ ...panel, requires: ["host.raw"] }, { environment }), { code: "missing_capability" })
})
