import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { registerHooks, stripTypeScriptTypes } from "node:module"
import test from "node:test"
import { compileScript, parse } from "vue/compiler-sfc"
import { createRenderer, nextTick, reactive } from "vue"

globalThis.Document = class {}
globalThis.ShadowRoot = class {}
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "../../stores/connection") return { url: "data:text/javascript,export const useConnectionStore=()=>globalThis.defaultsConnection", shortCircuit: true }
    if (specifier === "../../stores/sessions") return { url: "data:text/javascript,export const useSessionsStore=()=>({workspaces:[{cwd:'/project',name:'Project'}]})", shortCircuit: true }
    if (specifier === "../../stores/models") return { url: "data:text/javascript,export const useModelsStore=()=>({providers:[{providerId:'fixture',name:'Fixture',models:[{id:'a',name:'A'},{id:'b',name:'B'}]}],load:async()=>{},error:''})", shortCircuit: true }
    if (specifier === "vue-i18n") return { url: "data:text/javascript,export const useI18n=()=>({locale:{value:'en'}})", shortCircuit: true }
    try { return next(specifier, context) } catch (error) {
      if (specifier.startsWith(".") && !/\.(ts|vue)$/.test(specifier)) return next(`${specifier}.ts`, context)
      throw error
    }
  },
  load(url, context, next) {
    if (url.endsWith(".ts")) return { format: "module", source: stripTypeScriptTypes(readFileSync(new URL(url), "utf8"), { mode: "transform" }), shortCircuit: true }
    if (!url.endsWith(".vue")) return next(url, context)
    const { descriptor } = parse(readFileSync(new URL(url), "utf8"))
    return { format: "module", source: stripTypeScriptTypes(compileScript(descriptor, { id: "settings-defaults", inlineTemplate: true }).content, { mode: "transform" }), shortCircuit: true }
  },
})
const { default: PermissionsSettings } = await import("../src/features/settings/PermissionsSettings.vue")
const { default: AgentSettings } = await import("../src/features/settings/AgentSettings.vue")
const node = (type, text = "") => ({ type, text, children: [], props: {}, parent: undefined, options: [], selectedIndex: -1, addEventListener() {}, getRootNode() { return {} } })
const renderer = createRenderer({
  createElement: type => node(type), createText: text => node("text", text), createComment: text => node("comment", text),
  patchProp(element, key, _, value) { element.props[key] = value },
  setText(element, text) { element.text = text }, setElementText(element, text) { element.text = text; element.children = [] },
  parentNode: element => element.parent, nextSibling: element => element.parent?.children[element.parent.children.indexOf(element) + 1],
  insert(element, parent, anchor) { if (element.parent) element.parent.children.splice(element.parent.children.indexOf(element), 1); parent.children.splice(anchor ? parent.children.indexOf(anchor) : parent.children.length, 0, element); element.parent = parent },
  remove(element) { if (element.parent) element.parent.children.splice(element.parent.children.indexOf(element), 1); element.parent = undefined },
})
const all = element => [element, ...element.children.flatMap(all)]
const text = element => (element.type === "comment" ? "" : element.text) + element.children.map(text).join("")
const flush = async () => { for (let i = 0; i < 12; i++) { await Promise.resolve(); await nextTick() } }
const button = (root, label) => all(root).find(el => el.type === "button" && text(el) === label)
const field = (root, label) => all(root).find(el => el.type === "label" && text(el).startsWith(label)).children.find(el => el.type === "select")
const submit = async (root, label) => { await all(root).find(el => el.type === "form" && text(el).includes(label)).props.onSubmit({ preventDefault() {} }); await flush() }
function mount(t, component, call, supports = () => true) {
  globalThis.window = { confirm: () => true }
  globalThis.defaultsConnection = reactive({ phase: "connected", supports, client: { call: async (method, params, read) => read(await call(method, params)) } })
  const root = node("root"), app = renderer.createApp(component)
  app.mount(root)
  t.after(() => { app.unmount(); delete globalThis.window; delete globalThis.defaultsConnection })
  return root
}

test("isolation modes preserve inactive network, reset both flags and retain conflicting drafts without retry", async t => {
  let document = { sandbox: true, network: true }, revision = "exec:1", conflict = false
  const writes = []
  const root = mount(t, PermissionsSettings, async (method, params) => {
    if (method === "permission.rules") return { revision: "rules:1", commandRules: [] }
    if (method === "host.execution.backend") return { available: false, reason: "fixture backend unavailable" }
    if (method === "settings.update") {
      writes.push(params)
      if (conflict) throw new Error("settings_conflict: reload before saving")
      assert.equal(params.expectedRevision, revision)
      document = params.document
      revision += "+"
    } else assert.equal(method, "settings.read")
    return { revision, document }
  })
  await flush()
  assert.match(text(root), /fixture backend unavailable/)
  const mode = field(root, "Mode")
  mode.props["onUpdate:modelValue"]("off"); await flush()
  await submit(root, "Save isolation settings")
  assert.deepEqual(writes[0].document, { sandbox: false, network: true })
  globalThis.window.confirm = () => false
  await button(root, "Reset to default").props.onClick(); await flush()
  assert.equal(writes.length, 1)
  globalThis.window.confirm = () => true
  await button(root, "Reset to default").props.onClick(); await flush()
  assert.deepEqual(writes[1].document, { sandbox: false, network: false })
  mode.props["onUpdate:modelValue"]("isolated"); await flush()
  await submit(root, "Save isolation settings")
  assert.deepEqual(writes[2].document, { sandbox: true, network: false })
  mode.props["onUpdate:modelValue"]("network"); await flush()
  conflict = true
  await submit(root, "Save isolation settings")
  assert.deepEqual(writes[3].document, { sandbox: true, network: true })
  assert.equal(writes.length, 4)
  assert.match(text(root), /settings_conflict/)
  assert.equal(button(root, "Save isolation settings").props.disabled, false)
  assert.deepEqual(writes.map(write => [write.source, write.scope]), Array(4).fill(["execution", "user"]))
})

test("unsupported isolation does not display an invented default", async t => {
  const root = mount(t, PermissionsSettings, async () => ({ revision: "rules:1", commandRules: [] }), method => method !== "settings.read")
  await flush()
  assert.match(text(root), /Process isolation settings are not available/)
  assert.equal(button(root, "Save isolation settings"), undefined)
})

test("default-model scope changes preserve cancelled or failed drafts and never scope user-wide controls", async t => {
  const writes = []
  let fail = false
  const root = mount(t, AgentSettings, async (method, params) => {
    if (method === "provider.selection") {
      if (fail) throw new Error("project unavailable")
      return { selection: { provider_id: "fixture", model_id: "a", version: 7 } }
    }
    if (method === "collaboration.model_policy") return { revision: "policy:1", policy: { mode: "agent" } }
    if (method === "host.exec_shell.get") return { revision: "shell:1", shells: [] }
    writes.push([method, params])
    if (method === "provider.select") return { provider_id: params.providerId, model_id: params.modelId, version: 8 }
    if (method === "collaboration.set_model_policy") return { revision: "policy:2" }
    throw new Error(`unexpected ${method}`)
  })
  await flush()
  const scope = field(root, "Default model scope"), model = field(root, "Model"), policy = field(root, "Model policy")
  model.props["onUpdate:modelValue"](JSON.stringify(["fixture", "b"])); await flush()
  globalThis.window.confirm = () => false
  const cancelled = { value: "/project" }
  await scope.props.onChange({ target: cancelled }); await flush()
  assert.equal(cancelled.value, "")
  globalThis.window.confirm = () => true
  fail = true
  const failed = { value: "/project" }
  await scope.props.onChange({ target: failed }); await flush()
  assert.equal(failed.value, "")
  assert.match(text(root), /project unavailable/)
  await submit(root, "Save defaults")
  assert.equal(writes[0][1].cwd, undefined)
  fail = false
  await scope.props.onChange({ target: { value: "/project" } }); await flush()
  model.props["onUpdate:modelValue"](JSON.stringify(["fixture", "b"]))
  policy.props["onUpdate:modelValue"]("inherit"); await flush()
  await submit(root, "Save defaults")
  assert.deepEqual(writes[1], ["provider.select", { cwd: "/project", providerId: "fixture", modelId: "b", expectedVersion: 7 }])
  assert.deepEqual(writes[2], ["collaboration.set_model_policy", { expectedRevision: "policy:1", policy: { mode: "inherit" } }])
})
