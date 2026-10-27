import assert from "node:assert/strict"
import { registerHooks } from "node:module"
import test from "node:test"

registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context)
    } catch (error) {
      if ((specifier.startsWith("./") || specifier.startsWith("../")) && !specifier.endsWith(".ts")) {
        return next(`${specifier}.ts`, context)
      }
      throw error
    }
  },
})

const { groupRows } = await import("./work.ts")

/** A settled thought that was never written (interrupted) must not render a blank work card. */
test("a settled thinking row with no text is omitted", () => {
  const streaming = { kind: "thinking", key: "t2", text: "", streaming: true }
  const answer = { kind: "assistant", key: "a1", text: "Answer", streaming: false }
  const display = groupRows([
    { kind: "thinking", key: "t1", text: "", streaming: false },
    streaming,
    answer,
  ])
  assert.deepEqual(display, [
    { key: "work-t2", kind: "work", items: [{ id: "t2", kind: "thinking", content: "", running: true }] },
    { key: "a1", kind: "row", row: answer },
  ])
})
