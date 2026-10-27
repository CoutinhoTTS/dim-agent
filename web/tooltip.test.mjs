import assert from "node:assert/strict"
import test from "node:test"
import { vTooltip } from "./src/components/tooltip.ts"

class FakeElement {
  constructor(tag) {
    this.tag = tag
    this.attributes = new Map()
    this.listeners = new Map()
    this.style = {}
    this.open = false
    this.focusVisible = false
    this.isConnected = true
  }
  setAttribute(name, value) { this.attributes.set(name, String(value)) }
  getAttribute(name) { return this.attributes.get(name) ?? null }
  removeAttribute(name) { this.attributes.delete(name) }
  addEventListener(type, listener) {
    const listeners = this.listeners.get(type) ?? []
    listeners.push(listener)
    this.listeners.set(type, listeners)
  }
  removeEventListener(type, listener) {
    this.listeners.set(type, (this.listeners.get(type) ?? []).filter((item) => item !== listener))
  }
  dispatch(type, event = {}) {
    for (const listener of this.listeners.get(type) ?? []) listener({ currentTarget: this, ...event })
  }
  matches(selector) {
    if (selector === ":popover-open") return this.open
    if (selector === ":focus-visible") return this.focusVisible
    return false
  }
  showPopover() { this.open = true }
  hidePopover() { this.open = false }
  append() {}
  getBoundingClientRect() { return { top: 100, left: 100, right: 120, bottom: 120, width: 20, height: 20 } }
}

const windowListeners = new Map()
const documentListeners = new Map()
globalThis.window = {
  innerWidth: 1024,
  addEventListener(type, listener) {
    const listeners = windowListeners.get(type) ?? []
    listeners.push(listener)
    windowListeners.set(type, listeners)
  },
  removeEventListener() {},
}
globalThis.document = {
  activeElement: null,
  body: new FakeElement("body"),
  createElement(tag) { return new FakeElement(tag) },
  addEventListener(type, listener) {
    const listeners = documentListeners.get(type) ?? []
    listeners.push(listener)
    documentListeners.set(type, listeners)
  },
  removeEventListener() {},
}

function dispatch(map, type, event = {}) {
  for (const listener of map.get(type) ?? []) listener(event)
}

/** aria-label already names the control; describe it only when the tooltip adds text. */
test("aria-describedby is set only when the tooltip text differs from aria-label", (t) => {
  const target = new FakeElement("button")
  target.focusVisible = true
  target.setAttribute("aria-label", "Copy path")
  vTooltip.mounted(target, { value: "Copy path" })
  t.after(() => vTooltip.beforeUnmount(target))

  target.dispatch("focus")
  assert.equal(target.getAttribute("aria-describedby"), null)

  target.dispatch("keydown", { key: "Escape" })
  target.setAttribute("aria-label", "Copy")
  target.dispatch("focus")
  assert.equal(target.getAttribute("aria-describedby"), "ui-tooltip")
})

/** Returning to the window refocuses the last element; that focus is not the user's and must not show a tooltip. */
test("refocusing after a window blur does not show the last element's tooltip", (t) => {
  const target = new FakeElement("button")
  target.focusVisible = true
  target.setAttribute("aria-label", "Copy")
  vTooltip.mounted(target, { value: "Copy path" })
  t.after(() => vTooltip.beforeUnmount(target))

  document.activeElement = target
  dispatch(windowListeners, "blur")
  target.dispatch("focus")
  assert.equal(target.getAttribute("aria-describedby"), null)

  target.dispatch("focus")
  assert.equal(target.getAttribute("aria-describedby"), "ui-tooltip")
})
