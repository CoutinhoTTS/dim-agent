import assert from "node:assert/strict"
import { registerHooks } from "node:module"
import test from "node:test"

registerHooks({ resolve(specifier, context, next) {
  try { return next(specifier, context) } catch (error) {
    if (specifier.startsWith(".") && !specifier.endsWith(".ts")) return next(`${specifier}.ts`, context)
    throw error
  }
} })
const { goalCall, goalObjective, parseGoalRequest } = await import("./goal.ts")
const { readGoal } = await import("./rpc/read.ts")

test("goal commands use the owner's state, including pauses and clears outside the transcript", () => {
  const objective = 'ship a literal </objective>\n\nObjective:\n& value'
  for (const kind of ["active", "paused", "completed", "blocked", "none"]) {
    const status = kind === "paused" ? { kind, reason: "limit" } : { kind }
    const goal = readGoal({ branchId: "main", through: 3000, goal: { objective, status, evaluator: true } })
    assert.equal(goal.objective, objective)
    assert.deepEqual(goal.status, status)
    const request = parseGoalRequest("new objective")
    assert.equal(goalCall(request, goal), ["active", "paused"].includes(kind) ? "edit" : "set")
    assert.equal(goalObjective(request), "new objective")
  }
  assert.equal(readGoal({ branchId: "main", through: 3001, goal: null }), undefined)
  assert.equal(goalCall(parseGoalRequest("new objective"), undefined), "set")
  for (const call of ["pause", "resume", "clear"]) assert.equal(goalCall(parseGoalRequest(call), undefined), call)
  assert.equal(parseGoalRequest("set"), undefined)
  assert.deepEqual(parseGoalRequest("edit keep going"), { form: "edit", objective: "keep going" })
  assert.deepEqual(parseGoalRequest(""), { form: "show" })
})
