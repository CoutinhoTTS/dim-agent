import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { registerHooks, stripTypeScriptTypes } from "node:module"
import test from "node:test"
import { compileScript, parse } from "vue/compiler-sfc"
import { createRenderer, nextTick, reactive, ref } from "vue"

globalThis.Document = class {}
globalThis.ShadowRoot = class {}
globalThis.window = { innerWidth: 1280, innerHeight: 900, addEventListener() {}, removeEventListener() {} }
const components = /\/(ScheduledSettings|SchedulePlan|SelectField|ConfirmDialog)\.vue$/
registerHooks({
  resolve(specifier, context, next) {
    if (specifier.startsWith("../../stores/")) {
      const store = specifier.split("/").at(-1)
      return { url: `data:text/javascript,export const use${store[0].toUpperCase()}${store.slice(1)}Store=()=>globalThis.scheduleStores.${store}`, shortCircuit: true }
    }
    if (specifier === "vue-i18n") return { url: "data:text/javascript,export const useI18n=()=>({t:key=>key,locale:{value:'en'}})", shortCircuit: true }
    if (specifier === "vue-router") return { url: "data:text/javascript,export const useRoute=()=>globalThis.scheduleRoute;export const useRouter=()=>({replace:async to=>globalThis.scheduleNavigations.push(to)});export const RouterLink={inheritAttrs:false,props:['to'],setup:(props,{slots})=>()=>slots.default?.()}", shortCircuit: true }
    try { return next(specifier, context) } catch (error) {
      if (specifier.startsWith(".") && !/\.(ts|vue)$/.test(specifier)) return next(`${specifier}.ts`, context)
      throw error
    }
  },
  load(url, context, next) {
    if (url.endsWith(".ts")) return { format: "module", source: stripTypeScriptTypes(readFileSync(new URL(url), "utf8"), { mode: "transform" }), shortCircuit: true }
    if (!components.test(url)) return next(url, context)
    const { descriptor } = parse(readFileSync(new URL(url), "utf8"))
    return { format: "module", source: stripTypeScriptTypes(compileScript(descriptor, { id: "schedule-flow", inlineTemplate: true }).content, { mode: "transform" }), shortCircuit: true }
  },
})
const { default: ScheduledSettings } = await import("../src/features/settings/ScheduledSettings.vue")
const { RpcError } = await import("../src/rpc/client.ts")
const { settingsDirtyKey } = await import("../src/features/settings/context.ts")
const node = (type, text = "") => ({ type, text, children: [], props: {}, parent: undefined, value: "", style: {}, addEventListener() {}, focus() {}, matches() { return false }, getBoundingClientRect() { return { left: 20, top: 20, bottom: 60, width: 300, height: 120 } }, querySelector() {}, showPopover() {}, hidePopover() {}, getRootNode() { return {} }, showModal() { this.props.open = true }, close() { this.props.open = false } })
const renderer = createRenderer({
  createElement: type => node(type), createText: text => node("text", text), createComment: text => node("comment", text),
  patchProp(element, key, _, value) { element.props[key] = value; if (key === "value") { element.value = value; element._value = value } },
  setText(element, text) { element.text = text }, setElementText(element, text) { element.text = text; element.children = [] },
  parentNode: element => element.parent, nextSibling: element => element.parent?.children[element.parent.children.indexOf(element) + 1],
  insert(element, parent, anchor) { if (element.parent) element.parent.children.splice(element.parent.children.indexOf(element), 1); parent.children.splice(anchor ? parent.children.indexOf(anchor) : parent.children.length, 0, element); element.parent = parent },
  remove(element) { if (element.parent) element.parent.children.splice(element.parent.children.indexOf(element), 1); element.parent = undefined },
})
const all = element => [element, ...element.children.flatMap(all)]
const text = element => (element.type === "comment" ? "" : element.text) + element.children.map(text).join("")
const flush = async () => { for (let i = 0; i < 12; i++) { await Promise.resolve(); await nextTick() } }
const button = (root, label) => all(root).find(el => el.type === "button" && text(el) === label)
const field = (root, label) => all(root).find(el => el.type === "label" && text(el).startsWith(label))?.children.find(el => ["input", "textarea", "select"].includes(el.type))
const dropdown = (root, label) => all(root).find(el => el.props.role === "combobox" && el.props["aria-label"] === label)
async function select(root, label, option) {
  const trigger = dropdown(root, label)
  await trigger.props.onClick()
  await flush()
  const item = all(trigger.parent).find(el => el.props.role === "option" && text(el) === option)
  assert.ok(item, `Expected ${option} in ${label}`)
  await item.props.onClick()
  await flush()
}
async function change(element, value, event = "onChange") {
  element.props["onUpdate:modelValue"](value)
  if (element.props[event]) await element.props[event]({ target: { value } })
  await flush()
}
const submit = root => all(root).find(el => el.type === "form").props.onSubmit({ preventDefault() {} })
const policy = { preset: "read-only", allowedTools: [], disallowedTools: [] }
const job = (extra = {}) => ({ id: "job-1", name: "Audit", prompt: "Review changes", cwd: "/old-project", schedule: { cron: "7 */3 * * 1-5" }, mode: "reuse", session_id: "bound", policy_cap: policy, enabled: true, max_run_minutes: 23, revision: 9, created_by: "user", next_window: null, last_run: null, ...extra })
const saved = job => ({ job, transition: { from: null, to: { origin: "user", cap: policy, effective_policy: policy, report_to: null } } })
function mount(t, route, call, supports = () => true) {
  globalThis.scheduleRoute = { name: "settings-scheduled-edit", params: { jobId: "job-1" }, query: {}, ...route }
  globalThis.scheduleNavigations = []
  globalThis.scheduleStores = {
    connection: reactive({ phase: "connected", supports, client: { call: async (method, params, read) => read(await call(method, params)) } }),
    models: { available: false, providers: [] }, ui: { lastSessionId: "current" },
    sessions: { byId: new Map([["current", { cwd: "/current-project" }]]), workspaces: [{ cwd: "/current-project" }, { cwd: "/other-project" }] },
    toast: { push() {} },
  }
  const root = node("root"), app = renderer.createApp(ScheduledSettings)
  app.provide(settingsDirtyKey, ref(false))
  app.mount(root)
  t.after(() => app.unmount())
  return root
}

test("editing changes only the requested fields and preserves a complex plan and session binding", async t => {
  const writes = []
  const root = mount(t, {}, async (method, params) => {
    if (method === "schedule.list") return { jobs: [job({ model: { providerId: "missing", modelId: "saved" } })] }
    writes.push(params)
    return saved(job({ ...params.job, revision: 10 }))
  })
  await flush()
  assert.equal(button(root, "Save changes").props.disabled, true)
  assert.match(text(root), /Custom Cron/)
  await select(root, "Model", "missing / saved (unavailable)")
  await change(field(root, "Name"), "Renamed", "onInput")
  await submit(root)
  assert.deepEqual(writes, [{ job: { id: "job-1", name: "Renamed" }, revision: 9 }])
  assert.equal(globalThis.scheduleNavigations[0].name, "settings-scheduled-detail")
})

test("new tasks use the current workspace and compile selected weekdays and minute time without owner fields", async t => {
  const writes = []
  const root = mount(t, { name: "settings-scheduled-new", params: {} }, async (method, params) => {
    if (method === "schedule.list") return { jobs: [job()] }
    writes.push(params)
    return saved(job({ ...params.job, mode: "create", session_id: undefined }))
  })
  await flush()
  assert.equal(text(dropdown(root, "CLI host workspace")), "/current-project")
  assert.deepEqual(all(dropdown(root, "CLI host workspace").parent).filter(el => el.props.role === "option").map(text), ["/current-project", "/other-project", "/old-project"])
  await change(field(root, "Name"), "Weekly", "onInput")
  await select(root, "CLI host workspace", "/other-project")
  await select(root, "Frequency", "Every week")
  const checkbox = all(root).find(el => el.type === "input" && el.props.type === "checkbox" && el.props.value === 2)
  await change(checkbox, [2, 6])
  await change(field(root, "Run time"), "17:43", "onInput")
  await submit(root)
  assert.equal(writes.length, 1)
  assert.deepEqual(writes[0], { job: { name: "Weekly", prompt: "", cwd: "/other-project", schedule: { cron: "43 17 * * 2,6", tz: Intl.DateTimeFormat().resolvedOptions().timeZone }, mode: "create", max_run_minutes: 60, enabled: true } })
})

test("one-off and interval editors submit only the selected plan with minute precision", async t => {
  for (const once of [true, false]) {
    let written
    const root = mount(t, { name: "settings-scheduled-new", params: {} }, async (method, params) => {
      if (method === "schedule.list") return { jobs: [] }
      written = params.job
      return saved(job(params.job))
    })
    await flush()
    await change(field(root, "Name"), "Plan", "onInput")
    if (once) {
      await button(root, "Once").props.onClick()
      await flush()
      await change(field(root, "Date and time"), "2026-11-05T13:27", "onInput")
    } else {
      await select(root, "Plan", "Every N minutes")
      await change(field(root, "Interval in minutes"), 73, "onInput")
    }
    await submit(root)
    assert.deepEqual(written.schedule, once ? { at: new Date(2026, 10, 5, 13, 27).toISOString() } : { every_minutes: 73 })
  }
})

test("run, pause and delete use their own management actions without authorizing saves", async t => {
  const calls = []
  let current = job()
  const root = mount(t, { name: "settings-scheduled-detail" }, async (method, params) => {
    if (method === "schedule.list") return { jobs: [current] }
    calls.push([method, params])
    if (method === "schedule.run_now") return { run_id: "accepted", job_id: current.id, state: "launching", created_at: "2026-11-05T05:27:00Z", detail: "" }
    if (method === "schedule.set_enabled") { current = { ...current, enabled: params.enabled }; return { job: current } }
    if (method === "schedule.delete") return { job_id: current.id }
    assert.fail(`Unexpected management action ${method}`)
  })
  await flush()
  const running = button(root, "Run now").props.onClick()
  await flush()
  button(root, "confirm").props.onClick()
  await running
  await flush()
  assert.match(text(root), /Launching/)
  await button(root, "Pause").props.onClick()
  await flush()
  const deleting = button(root, "Delete task").props.onClick()
  await flush()
  button(root, "confirm").props.onClick()
  await deleting
  assert.deepEqual(calls, [["schedule.run_now", { job_id: "job-1" }], ["schedule.set_enabled", { job_id: "job-1", enabled: false }], ["schedule.delete", { job_id: "job-1" }]])
  assert.equal(globalThis.scheduleNavigations[0].name, "settings-scheduled")
})

test("conflicts preserve the draft and a timeout cannot repeat a create", async t => {
  for (const [failure, route] of [[new RpcError(-32000, "changed", "schedule_conflict"), {}], [new Error("timed out; outcome unknown"), { name: "settings-scheduled-new", params: {} }]]) {
    let writes = 0
    const root = mount(t, route, async method => {
      if (method === "schedule.list") return { jobs: [job()] }
      writes++
      throw failure
    })
    await flush()
    await change(field(root, "Name"), "Keep this draft", "onInput")
    await submit(root)
    await submit(root)
    assert.equal(writes, 1)
    assert.equal(field(root, "Name").value, "Keep this draft")
    assert.match(text(root), failure instanceof RpcError ? /draft is preserved/ : /outcome is unknown/)
  }
})

test("missing Web admission never falls back to Settings or file reads", async t => {
  const root = mount(t, { name: "settings-scheduled", params: {} }, () => assert.fail("No method may bypass missing admission"), () => false)
  await flush()
  assert.match(text(root), /not admitted for Web access/)
  assert.equal(button(root, "New task"), undefined)
})
