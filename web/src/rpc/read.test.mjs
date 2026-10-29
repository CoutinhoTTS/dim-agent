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

const { ProtocolError, readBranchCreate, readHistory } = await import("./read.ts")

function page(event) {
  return readHistory({ events: [event], through: event.sequence, nextAfter: event.sequence, done: true })
}

function event(type, name, data, sequence = 1) {
  return { id: `e${sequence}`, sequence, create_at: 1_000 + sequence, type, name, data }
}

test("branch facts are decoded with their line and cut", () => {
  const created = page(event("branch", "create", {
    at: { kind: "sequence", sequence: 3 },
    source: { kind: "host", operation_id: "op-1" },
    branch: "b-7c",
    prompt: "parser race",
  }))
  assert.deepEqual(created.events[0].content, {
    kind: "branch-create",
    branchId: "b-7c",
    at: 3,
    prompt: "parser race",
    source: { kind: "host", operationId: "op-1" },
  })

  const ended = page(event("branch", "ended", {
    branch: "b-7c",
    endEvent: 4,
    source: { kind: "tool", turn_id: "t", request_id: "r", call_id: "c" },
    ancestor: "b-02",
  }))
  assert.deepEqual(ended.events[0].content, {
    kind: "branch-ended",
    branchId: "b-7c",
    source: { kind: "tool", turnId: "t", requestId: "r", callId: "c" },
    ancestor: "b-02",
  })
})

test("raw branch messages and input deletions are decoded", () => {
  const message = page(event("input", "message", {
    blocks: [{ type: "raw", text: "<dim-branch-message from=\"main\">\nhello\n</dim-branch-message>" }],
    mode: "steer",
  }))
  assert.deepEqual(message.events[0].content, {
    kind: "source-input",
    text: "<dim-branch-message from=\"main\">\nhello\n</dim-branch-message>",
    mode: "steer",
  })

  const deleted = page(event("input", "deleted", { input_id: "e9" }, 2))
  assert.deepEqual(deleted.events[0].content, { kind: "input-deleted", inputId: "e9" })
})

test("branch_create replies and malformed branch facts are read strictly", () => {
  assert.deepEqual(readBranchCreate({
    branch: { branchId: "b-7c", parentBranchId: "main", branchPoint: 3, createEvent: 8 },
  }), { branchId: "b-7c", parentBranchId: "main", branchPoint: 3, createEvent: 8 })

  assert.throws(
    () => page(event("branch", "create", {
      at: { kind: "sequence", sequence: 3 },
      source: { kind: "unknown" },
      branch: "b-7c",
      prompt: "parser race",
    })),
    ProtocolError,
  )
})
