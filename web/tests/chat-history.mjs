import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { registerHooks, stripTypeScriptTypes } from "node:module"
import test from "node:test"
import { createPinia, setActivePinia } from "pinia"

registerHooks({
  resolve(specifier, context, next) {
    if (context.parentURL?.endsWith("/stores/chat.ts") && ["./connection", "./sessions", "./ui"].includes(specifier)) {
      const store = specifier.slice(2)
      return { url: `data:text/javascript,export const use${store[0].toUpperCase()}${store.slice(1)}Store=()=>globalThis.historyStores.${store}`, shortCircuit: true }
    }
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
const { useChatStore } = await import("../src/stores/chat.ts")
const { RpcError, RpcClient, ConnectionLost } = await import("../src/rpc/client.ts")
const event = (sequence, content) => ({ id: `e${sequence}`, sequence, createAt: sequence, content })
const input = (sequence, text) => event(sequence, { kind: "input", text, blocks: [] })
const page = (events, through, nextAfter = through, done = true) => ({ events, through, nextAfter, done })
const flush = () => new Promise(resolve => setTimeout(resolve, 25))
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no }); return { promise, resolve, reject } }

function session(t, history) {
  setActivePinia(createPinia())
  globalThis.historyStores = {
    connection: { supports: () => false, client: { async call(method, params) {
      if (method === "session.history") return history(params)
      return {}
    } } },
    sessions: { byId: new Map(), setRunning() {}, refresh: async () => {}, forget() {} },
    ui: { resetSessionTabs() {} },
  }
  const chat = useChatStore()
  t.after(() => chat.$dispose())
  chat.open("session")
  return chat
}
const receive = (chat, branchId, event) => chat.receive({ method: "session.event", sessionId: "session", branchId, event })

test("a failed branch page cannot let live events skip the missing prefix on retry", async t => {
  let calls = 0
  const second = deferred()
  const chat = session(t, async ({ branchId, after }) => {
    if (branchId === "main") return page([event(1, { kind: "branch-create", branchId: "worker", at: 0, prompt: "work", source: { kind: "host", operationId: "op" } })], 1)
    calls++
    if (calls === 1) return page([input(1, "first")], 4, 2, false)
    if (calls === 2) { assert.equal(after, 2); return second.promise }
    assert.equal(after, 2, "resume from the raw page cursor, not the visible event or newer live output")
    return page([input(3, "missing")], 4)
  })
  await flush()
  receive(chat, "worker", input(4, "buffered"))
  second.reject(new RpcError(-32000, "read failed", "storage_failed"))
  await flush()
  assert.equal(chat.views.session.branches.worker.status, "failed")
  receive(chat, "worker", input(5, "after failure"))
  await flush()
  assert.deepEqual(chat.views.session.branches.worker.rows.map(row => row.text), ["first"])
  chat.retryLine("session", "worker")
  await flush()
  assert.deepEqual(chat.views.session.branches.worker.rows.map(row => row.text), ["first", "missing", "buffered", "after failure"])
  assert.equal(chat.views.session.branches.worker.status, "ready")
})

test("reconnect keeps the newest unpersisted branch snapshot until history has loaded", async t => {
  let workerPage = page([], 0)
  const chat = session(t, ({ branchId }) => branchId === "main" ? page([], 0) : workerPage)
  await flush()
  chat.receive({ method: "session.state", sessionId: "session", branchId: "worker", running: true, stopping: false })
  await flush()
  const held = deferred()
  workerPage = held.promise
  chat.resync()
  for (const text of ["old snapshot", "latest snapshot"]) chat.receive({ method: "session.model.progress", progress: { sessionId: "session", branchId: "worker", requestId: "r1", text, textOffset: 0, thinking: "", thinkingOffset: 0 } })
  await flush()
  held.resolve(page([], 0))
  await flush()
  assert.equal(chat.views.session.branches.worker.rows.find(row => row.kind === "assistant").text, "latest snapshot")

  let sequence = 0
  for (const [kind, terminalSource] of ["model-end", "tool-use", "turn-end"].flatMap(kind => ["notification", "history"].map(source => [kind, source]))) {
    const ending = deferred()
    workerPage = ending.promise
    chat.resync()
    chat.receive({ method: "session.model.progress", progress: { sessionId: "session", branchId: "worker", requestId: "r1", text: "unwritten tail", textOffset: 0, thinking: "", thinkingOffset: 0 } })
    await flush()
    const terminal = event(++sequence, kind === "turn-end" ? { kind, outcome: "completed" } : kind === "tool-use" ? { kind, requestId: "r1", callId: `call-${sequence}`, name: "read_file", input: {} } : { kind, requestId: "r1" })
    if (terminalSource === "notification") receive(chat, "worker", terminal)
    ending.resolve(page(terminalSource === "history" ? [terminal] : [], terminal.sequence))
    await flush()
    assert.equal(chat.views.session.branches.worker.rows.filter(row => row.kind === "assistant").length, 1)
    assert.equal(chat.views.session.branches.worker.rows.find(row => row.kind === "assistant").streaming, false, "a terminal fact must not resurrect a buffered snapshot")
  }
})

test("main-history failure buffers live output until an explicit retry completes history", async t => {
  let fail = true
  const chat = session(t, () => {
    if (fail) throw new Error("session.history timed out")
    return page([input(1, "stored")], 1)
  })
  await flush()
  assert.equal(chat.views.session.status, "failed")
  receive(chat, "main", input(2, "new live event"))
  await flush()
  assert.deepEqual(chat.views.session.rows, [])
  fail = false
  chat.resync()
  await flush()
  assert.equal(chat.views.session.status, "ready")
  assert.equal(chat.views.session.rows[0].text, "stored")
})

test("a request timeout leaves the socket usable and reports a failure rather than a disconnect", async t => {
  const NativeSocket = globalThis.WebSocket
  const nativeLocation = globalThis.location
  let socket
  globalThis.location = { protocol: "http:", host: "fixture" }
  globalThis.WebSocket = class {
    static OPEN = 1
    readyState = 1
    constructor() { socket = this }
    send() {}
    close() {}
  }
  t.after(() => { globalThis.WebSocket = NativeSocket; globalThis.location = nativeLocation })
  const client = new RpcClient({ phase() {}, ready() {}, notification() {}, invalid() {} })
  client.start()
  await assert.rejects(client.call("session.history", {}, x => x, 1), error => !(error instanceof ConnectionLost) && /timed out/.test(error.message))
  const next = client.call("daemon.ping", {}, x => x, 1000)
  socket.onmessage({ data: JSON.stringify({ jsonrpc: "2.0", id: 2, result: { ok: true } }) })
  assert.deepEqual(await next, { ok: true })
  client.stop()
})
