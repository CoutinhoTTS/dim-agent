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
    if (specifier === "../../stores/connection") return { url: "data:text/javascript,export const useConnectionStore=()=>globalThis.skillsConnection", shortCircuit: true }
    if (specifier === "../../stores/sessions") return { url: "data:text/javascript,export const useSessionsStore=()=>({workspaces:[{cwd:'/project-a',name:'Project A'},{cwd:'/project-b',name:'Project B'}]})", shortCircuit: true }
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
    return { format: "module", source: stripTypeScriptTypes(compileScript(descriptor, { id: "skills-flow", inlineTemplate: true }).content, { mode: "transform" }), shortCircuit: true }
  },
})
const { default: SkillSettings } = await import("../src/features/settings/SkillSettings.vue")
const node = (type, text = "") => ({ type, text, children: [], props: {}, parent: undefined, addEventListener() {}, getRootNode() { return {} } })
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
const button = (root, label) => all(root).find(el => el.type === "button" && (text(el) === label || el.props["aria-label"] === label))
const skill = (id, enabled = true, shadowedBy) => ({ id, name: id, description: `Use ${id}`, filePath: `/skills/${id}/SKILL.md`, allowedTools: [], enabled, shadowedBy })
function mount(t, call, supports = () => true) {
  globalThis.skillsConnection = reactive({ phase: "connected", supports, client: { call: async (method, params, read) => read(await call(method, params)) } })
  const root = node("root"), app = renderer.createApp(SkillSettings)
  app.mount(root)
  t.after(() => app.unmount())
  return root
}

test("installed skills retain search and scope across reloads and authoritative toggles", async t => {
  let entries = [skill("authoring"), skill("design", false), skill("shadowed", true, "authoring")]
  const writes = []
  const root = mount(t, async (method, params) => {
    if (method === "skills.catalog") return { version: 1, entries, issues: [] }
    writes.push([method, params])
    if (method === "skills.set_enabled") entries = entries.map(entry => entry.id === params.id ? { ...entry, enabled: params.enabled } : entry)
    else assert.equal(method, "skills.refresh")
    return {}
  })
  await flush()
  assert.equal(button(root, "Disable shadowed").props.disabled, true)
  assert.equal(all(root).filter(el => el.type === "details").length, 3)
  assert.equal(all(root).filter(el => el.type === "input").length, 1, "search is the only text input")
  await all(root).find(el => el.type === "select").props.onChange({ target: { value: "/project-b" } }); await flush()
  const search = all(root).find(el => el.type === "input")
  search.props["onUpdate:modelValue"]("AUTHOR")
  await flush()
  assert.equal(all(root).filter(el => el.type === "li").length, 1)
  await button(root, "Disable authoring").props.onClick(); await flush()
  assert.ok(button(root, "Enable authoring"))
  await button(root, "Reload skills").props.onClick(); await flush()
  assert.deepEqual(writes, [["skills.set_enabled", { cwd: "/project-b", id: "authoring", enabled: false }], ["skills.refresh", { cwd: "/project-b" }]])
  assert.equal(all(root).find(el => el.type === "select").props.value, "/project-b")
  assert.equal(search.props["onUpdate:modelValue"] !== undefined, true)
  assert.equal(all(root).filter(el => el.type === "li").length, 1)
  assert.match(text(root), /Skills reloaded/)
  search.props["onUpdate:modelValue"]("no-match"); await flush()
  assert.match(text(root), /No matching skills/)
})

test("failed reloads preserve the list, expose a retry and never report success", async t => {
  let fail = false
  const root = mount(t, async method => {
    if (fail) throw new Error("Discovery failed")
    assert.equal(method, "skills.catalog")
    return { version: 1, entries: [skill("authoring")], issues: [{ path: "/bad/SKILL.md", message: "Invalid metadata" }] }
  })
  await flush()
  assert.match(text(root), /Invalid metadata/)
  fail = true
  await button(root, "Reload skills").props.onClick(); await flush()
  assert.match(text(root), /Discovery failed/)
  assert.doesNotMatch(text(root), /Skills reloaded/)
  assert.ok(button(root, "Disable authoring"))
  fail = false
  await button(root, "Reload").props.onClick(); await flush()
  assert.doesNotMatch(text(root), /Discovery failed/)
})
