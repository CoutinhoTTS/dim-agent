import assert from "node:assert/strict"
import test from "node:test"
import { copyText } from "./src/clipboard.ts"

/** The insecure-context fallback must keep the textarea at 16px, off-screen, so iOS Safari does not zoom and nothing flashes. */
test("clipboard fallback uses an off-screen 16px textarea", (t) => {
  const calls = []
  const field = {
    value: "",
    style: { cssText: "" },
    attributes: new Map(),
    setAttribute(name, value) { this.attributes.set(name, value) },
    select() { calls.push("select") },
    setSelectionRange(start, end) { calls.push(["range", start, end]) },
    remove() { calls.push("remove") },
  }
  const previous = {
    window: globalThis.window,
    document: globalThis.document,
    HTMLElement: globalThis.HTMLElement,
  }
  t.after(() => {
    globalThis.window = previous.window
    globalThis.document = previous.document
    globalThis.HTMLElement = previous.HTMLElement
  })
  globalThis.HTMLElement = class HTMLElement {}
  globalThis.window = { isSecureContext: false }
  globalThis.document = {
    activeElement: null,
    createElement(tag) { assert.equal(tag, "textarea"); return field },
    body: { append(node) { assert.equal(node, field) } },
    execCommand(command) { calls.push(command); return true },
  }

  return copyText("hello").then(() => {
    assert.match(field.style.cssText, /font-size:16px/)
    assert.match(field.style.cssText, /left:-9999px/)
    assert.equal(field.value, "hello")
    assert.equal(field.attributes.get("readonly"), "")
    assert.deepEqual(calls, ["select", ["range", 0, 5], "copy", "remove"])
  })
})
