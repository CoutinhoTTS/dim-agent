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

const { Projection } = await import("./projection.ts")

function event(sequence, content) {
  return { id: `e${sequence}`, sequence, createAt: 1_000 + sequence, content }
}

test("a user message carries the cut a child line opens at", () => {
  const projection = new Projection()
  projection.apply(event(5, { kind: "input", text: "try RwLock\nfor the cache lock", content: [{ type: "text", text: "try RwLock\nfor the cache lock" }], blocks: [] }))
  projection.apply(event(6, {
    kind: "branch-create",
    branchId: "b-7c",
    at: 4,
    prompt: "parser race",
    source: { kind: "host", operationId: "op" },
  }))

  const [user, line] = projection.rows
  assert.equal(user.kind, "user")
  assert.equal(user.sequence, 5)
  assert.equal(user.cut, 4)
  assert.equal(line.kind, "line")
  assert.equal(line.branchId, "b-7c")
  assert.equal(line.purpose, "parser race")
  assert.equal(line.before, "try RwLock")
  assert.equal(line.failed, undefined)

  projection.apply(event(7, {
    kind: "branch-create-failed",
    branchId: "b-7c",
    status: "failed",
    error: "boom",
  }))
  assert.equal(projection.rows[1].failed, true)
})

test("a thought settles with the span of its block, not the time to its first event", () => {
  const projection = new Projection()
  projection.apply(event(1, { kind: "model-start", requestId: "r1" }))
  projection.apply(event(2, { kind: "thinking", requestId: "r1", delta: "weigh" }))
  assert.equal(projection.rows[0].kind, "thinking")
  assert.equal(projection.rows[0].streaming, true)
  assert.equal(projection.rows[0].elapsedMs, undefined)

  projection.apply(event(5, { kind: "text", requestId: "r1", delta: "answer" }))
  const thought = projection.rows[0]
  assert.equal(thought.streaming, false)
  assert.equal(thought.elapsedMs, 3)
})

test("a thought opened only by progress settles from its producer start", () => {
  const projection = new Projection()
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", text: "", textOffset: 0, thinking: "weigh", thinkingOffset: 0, thinkingStartedAt: 1_002 })
  assert.equal(projection.rows[0].kind, "thinking")
  assert.equal(projection.rows[0].text, "weigh")
  assert.equal(projection.rows[0].elapsedMs, undefined)

  projection.apply(event(5, { kind: "model-end", requestId: "r1" }))
  assert.equal(projection.rows[0].streaming, false)
  assert.equal(projection.rows[0].elapsedMs, 3)
})

test("a coalesced clear rebuilds the replaced attempt's thought row", () => {
  const projection = new Projection()
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", thinkingPartId: "first", text: "", textOffset: 0, thinking: "failed attempt", thinkingOffset: 0, thinkingStartedAt: 1_000 })
  const first = projection.rows[0]
  assert.equal(first.kind, "thinking")
  // The clear frame was coalesced away; the next notice is the new attempt's.
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", thinkingPartId: "second", text: "", textOffset: 0, thinking: "a longer successful thought", thinkingOffset: 0, thinkingStartedAt: 1_002 })
  const thoughts = projection.rows.filter((row) => row.kind === "thinking")
  assert.equal(thoughts.length, 1, JSON.stringify(projection.rows))
  assert.notEqual(thoughts[0].key, first.key, "the rebuilt thought row has a new identity")
  assert.equal(thoughts[0].text, "a longer successful thought")

  projection.apply({ id: "second", sequence: 4, createAt: 1_003, content: { kind: "thinking", requestId: "r1", delta: "a longer successful thought" } })
  projection.apply(event(5, { kind: "text", requestId: "r1", delta: "Done." }))
  const thought = projection.rows.find((row) => row.kind === "thinking")
  assert.equal(thought.elapsedMs, 3, "the span starts at the new attempt")
})

test("a coalesced clear before text leaves no empty thought row", () => {
  const projection = new Projection()
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", thinkingPartId: "first", text: "", textOffset: 0, thinking: "failed attempt", thinkingOffset: 0, thinkingStartedAt: 1_000 })
  // The clear frame was coalesced away; the next notice starts the new attempt's text.
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", text: "partial answer", textOffset: 0, thinking: "", thinkingOffset: 0 })
  projection.apply(event(5, { kind: "text", requestId: "r1", delta: "partial answer" }))
  const thoughts = projection.rows.filter((row) => row.kind === "thinking")
  assert.equal(thoughts.length, 0, JSON.stringify(projection.rows))
  assert.equal(projection.rows.find((row) => row.kind === "assistant").text, "partial answer")
})

function failedThought() {
  return { sessionId: "s", branchId: "main", requestId: "r1", thinkingPartId: "first", text: "", textOffset: 0, thinking: "failed attempt", thinkingOffset: 0, thinkingStartedAt: 1_000 }
}

test("a durable thought before the retry notice rebuilds the row", () => {
  const projection = new Projection()
  projection.apply(event(1, { kind: "model-start", requestId: "r1" }))
  projection.progress(failedThought())
  const replaced = projection.rows[0]
  // The progress notice is coalesced; the retry's first committed thought arrives first.
  projection.apply({ id: "second", sequence: 2, createAt: 1_002, content: { kind: "thinking", requestId: "r1", delta: "a longer successful thought" } })
  const thoughts = projection.rows.filter((row) => row.kind === "thinking")
  assert.equal(thoughts.length, 1, JSON.stringify(projection.rows))
  assert.notEqual(thoughts[0].key, replaced.key, "the rebuilt thought row has a new identity")
  assert.equal(thoughts[0].text, "a longer successful thought")
  projection.apply(event(3, { kind: "text", requestId: "r1", delta: "Done." }))
  assert.equal(projection.rows.find((row) => row.kind === "thinking").elapsedMs, 1, "the span starts at the new attempt")
})

test("a durable text before the retry notice leaves no empty thought row", () => {
  const projection = new Projection()
  projection.apply(event(1, { kind: "model-start", requestId: "r1" }))
  projection.progress(failedThought())
  projection.apply(event(2, { kind: "text", requestId: "r1", delta: "answer" }))
  assert.equal(projection.rows.filter((row) => row.kind === "thinking").length, 0, JSON.stringify(projection.rows))
  assert.equal(projection.rows.find((row) => row.kind === "assistant").text, "answer")
})

test("a durable tool event before the retry notice leaves no empty thought row", () => {
  const projection = new Projection()
  projection.apply(event(1, { kind: "model-start", requestId: "r1" }))
  projection.progress(failedThought())
  projection.apply(event(2, { kind: "tool-use-start", requestId: "r1", callId: "call", name: "echo" }))
  assert.equal(projection.rows.filter((row) => row.kind === "thinking").length, 0, JSON.stringify(projection.rows))
  projection.apply(event(3, { kind: "model-end", requestId: "r1", error: "boom", failure: failure() }))
  const row = projection.rows.at(-1)
  assert.equal(row.kind, "error")
  assert.ok(!row.text.startsWith("The response above is incomplete."), row.text)

  const second = new Projection()
  second.apply(event(1, { kind: "model-start", requestId: "r1" }))
  second.progress(failedThought())
  second.apply(event(2, { kind: "tool-use", requestId: "r1", callId: "call", name: "echo", input: {} }))
  assert.equal(second.rows.filter((row) => row.kind === "thinking").length, 0, JSON.stringify(second.rows))
})

test("a stale progress snapshot does not reopen a replaced thought", () => {
  const projection = new Projection()
  projection.apply(event(1, { kind: "model-start", requestId: "r1" }))
  const failed = failedThought()
  projection.progress(failed)
  projection.apply({ id: "second", sequence: 2, createAt: 1_002, content: { kind: "thinking", requestId: "r1", delta: "success" } })
  // The page replays its cached snapshot, still the replaced attempt's.
  projection.progress(failed)
  const thoughts = projection.rows.filter((row) => row.kind === "thinking")
  assert.equal(thoughts.length, 1, JSON.stringify(projection.rows))
  assert.equal(thoughts[0].text, "success", "the stale snapshot leaves the new attempt's thought alone")
})

test("a tool start settles a thought before its arguments arrive", () => {
  const projection = new Projection()
  projection.apply(event(2, { kind: "thinking", requestId: "r1", delta: "weigh" }))
  projection.apply(event(3, { kind: "tool-use-start", requestId: "r1", callId: "call", name: "echo" }))
  assert.equal(projection.rows[0].streaming, false)
  assert.equal(projection.rows[0].elapsedMs, 1)

  projection.apply(event(50, { kind: "tool-use", requestId: "r1", callId: "call", name: "echo", input: {} }))
  assert.equal(projection.rows[0].elapsedMs, 1, "the completed call does not move the settled span")
})

test("an answer that continues after a tool call shows its new segment instead of an empty row", () => {
  const projection = new Projection()
  projection.apply(event(2, { kind: "text", requestId: "r1", delta: "Write the file" }))
  projection.apply(event(3, { kind: "tool-use", requestId: "r1", callId: "call", name: "exec", input: {} }))
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", text: " and run it", textOffset: 14, thinking: "", thinkingOffset: 0 })

  const answers = () => projection.rows.filter((row) => row.kind === "assistant").map((row) => [row.text, row.streaming])
  assert.deepEqual(answers(), [["Write the file", false], [" and run it", true]])

  projection.apply(event(4, { kind: "text", requestId: "r1", delta: " and run it" }))
  assert.deepEqual(answers(), [["Write the file", false], [" and run it", true]], "the committed segment settles its own progress once, without repeating the earlier segment")

  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", text: " now", textOffset: 25, thinking: "", thinkingOffset: 0 })
  assert.deepEqual(answers(), [["Write the file", false], [" and run it now", true]], "progress offsets count from the request's start, across the tool call")

  projection.apply(event(5, { kind: "text", requestId: "r1", delta: " now" }))
  projection.apply(event(6, { kind: "model-end", requestId: "r1" }))
  assert.deepEqual(answers(), [["Write the file", false], [" and run it now", false]])
})

test("a thought that continues after a tool call shows its new segment", () => {
  const projection = new Projection()
  projection.apply(event(2, { kind: "thinking", requestId: "r1", delta: "weigh" }))
  projection.apply(event(3, { kind: "tool-use-start", requestId: "r1", callId: "call", name: "exec" }))
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", text: "", textOffset: 0, thinking: " the options", thinkingOffset: 5, thinkingStartedAt: 1_004 })

  const thoughts = projection.rows.filter((row) => row.kind === "thinking").map((row) => [row.text, row.streaming])
  assert.deepEqual(thoughts, [["weigh", false], [" the options", true]])
})

test("a history read's counts cover the rows of a request that began before its window", () => {
  const projection = new Projection()
  projection.apply(event(2, { kind: "thinking", requestId: "r1", delta: "options" }))
  projection.apply(event(3, { kind: "tool-use-start", requestId: "r1", callId: "call", name: "exec" }))
  projection.apply(event(4, { kind: "text", requestId: "r1", delta: "file" }))
  projection.apply(event(5, { kind: "tool-use", requestId: "r1", callId: "call", name: "exec", input: {} }))
  projection.activeOutput({ requestId: "r1", textCharacters: 14, thinkingCharacters: 12 })
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", text: " and run it", textOffset: 14, thinking: " again", thinkingOffset: 12, thinkingStartedAt: 1_006 })

  const rows = projection.rows.map((row) => [row.kind, row.text, row.streaming])
  assert.deepEqual(rows, [
    ["thinking", "options", false],
    ["assistant", "file", false],
    ["tool", undefined, undefined],
    ["thinking", " again", true],
    ["assistant", " and run it", true],
  ])
})

test("a retry starts a request's committed counts from nothing", () => {
  const projection = new Projection()
  projection.apply(event(2, { kind: "text", requestId: "r1", delta: "Write the file" }))
  projection.apply(event(3, { kind: "tool-use", requestId: "r1", callId: "call", name: "exec", input: {} }))
  // The producer restarts the request: its offsets count from the start, so a retry opens at zero.
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", text: "", textOffset: 0, thinking: "", thinkingOffset: 0 })
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", text: " and run it", textOffset: 0, thinking: "", thinkingOffset: 0 })

  const answers = projection.rows.filter((row) => row.kind === "assistant").map((row) => [row.text, row.streaming])
  assert.deepEqual(answers, [["Write the file", false], [" and run it", true]], "the new attempt's tail is not swallowed by the replaced attempt's count")
})

test("branch messages, ends and the last turn end become rows", () => {
  const projection = new Projection()
  projection.apply(event(1, {
    kind: "source-input",
    text: "<dim-branch-message from=\"b-7c\" to=\"main\">\nStop &amp;&amp; look at parser.rs.\n</dim-branch-message>",
    blocks: [],
    mode: "steer",
  }))
  projection.apply(event(2, {
    kind: "branch-end",
    source: { kind: "host", operationId: "op" },
  }))
  projection.apply(event(3, { kind: "turn-end", outcome: "completed" }))

  const [message, ended] = projection.rows
  assert.deepEqual(message, {
    kind: "source-message",
    key: "notice-1",
    from: "b-7c",
    steer: true,
    final: false,
    body: "Stop && look at parser.rs.",
  })
  assert.equal(ended.kind, "line-ended")
  assert.equal(ended.issuer, "operation")
  assert.equal(projection.lastTurnEnd, 3)
})

test("replayed events do not duplicate rows", () => {
  const projection = new Projection()
  const input = event(1, { kind: "input", text: "hello", content: [{ type: "text", text: "hello" }], blocks: [] })
  assert.equal(projection.apply(input), true)
  assert.equal(projection.apply(input), false)
  assert.equal(projection.rows.length, 1)
})

test("a user row carries the Skills its message loaded", () => {
  const projection = new Projection()
  projection.apply(event(5, { kind: "input", text: "check the build", content: [{ type: "text", text: "check the build" }], skills: ["checkin"], blocks: [] }))
  assert.deepEqual(projection.rows, [{
    kind: "user",
    key: "user-5",
    text: "check the build",
    skills: ["checkin"],
    at: 1_005,
    sequence: 5,
    cut: 4,
    inputId: "e5",
    settled: false,
  }])
})

test("a hook-bearing user row carries the number of hook contexts", () => {
  const projection = new Projection()
  projection.apply(event(5, { kind: "input", text: "check the build", content: [{ type: "text", text: "check the build" }], skills: ["checkin"], hooks: 2, blocks: [] }))
  assert.deepEqual(projection.rows, [{
    kind: "user",
    key: "user-5",
    text: "check the build",
    skills: ["checkin"],
    hooks: 2,
    at: 1_005,
    sequence: 5,
    cut: 4,
    inputId: "e5",
    settled: false,
  }])
})

function failure(fields = {}) {
  return {
    kind: "server_error",
    status: 503,
    providerCode: "overloaded",
    requestId: "req_1",
    reason: "The server is overloaded",
    cause: [],
    attempts: 1,
    waitedMs: 0,
    ...fields,
  }
}

test("a failure row is one line and keeps its details", () => {
  const projection = new Projection()
  projection.apply(event(2, { kind: "model-end", requestId: "r1", error: "boom", failure: failure() }))
  const row = projection.rows.at(-1)
  assert.equal(row.kind, "error")
  assert.equal(row.text, "The provider reported a server error: The server is overloaded. Try again later or choose another model.")
  assert.equal(row.details.kind, "server_error")
})

test("a failure after a text row starts with the incomplete sentence", () => {
  const projection = new Projection()
  projection.apply(event(2, { kind: "text", requestId: "r1", delta: "partial" }))
  projection.apply(event(3, { kind: "model-end", requestId: "r1", error: "boom", failure: failure() }))
  const row = projection.rows.at(-1)
  assert.equal(row.kind, "error")
  assert.ok(row.text.startsWith("The response above is incomplete. The provider reported a server error"), row.text)
})

test("a shown model failure is forgotten at every turn end", () => {
  const projection = new Projection()
  projection.apply(event(2, { kind: "model-end", requestId: "r1", error: "boom", failure: failure() }))
  projection.apply(event(3, { kind: "turn-end", outcome: "completed" }))
  projection.apply(event(4, { kind: "turn-end", outcome: "failed", error: "boom" }))
  assert.equal(projection.rows.filter((row) => row.kind === "error").length, 2)
})

test("a cancelled turn end forgets the model failure too", () => {
  const projection = new Projection()
  projection.apply(event(2, { kind: "model-end", requestId: "r1", error: "boom", failure: failure() }))
  projection.apply(event(3, { kind: "turn-end", outcome: "cancelled" }))
  projection.apply(event(4, { kind: "turn-end", outcome: "failed", error: "boom" }))
  assert.deepEqual(
    projection.rows.filter((row) => row.kind === "error" || row.kind === "stopped").map((row) => row.kind),
    ["error", "stopped", "error"],
  )
})

test("the turn repeat of a model failure is dropped", () => {
  const projection = new Projection()
  projection.apply(event(2, { kind: "model-end", requestId: "r1", error: "boom", failure: failure() }))
  projection.apply(event(3, { kind: "turn-end", outcome: "failed", error: "boom" }))
  assert.equal(projection.rows.filter((row) => row.kind === "error").length, 1)
})

test("a turn failure that is not the model failure still prints", () => {
  const projection = new Projection()
  projection.apply(event(2, { kind: "model-end", requestId: "r1", error: "boom", failure: failure() }))
  projection.apply(event(3, { kind: "turn-end", outcome: "failed", error: "tool crashed" }))
  const errors = projection.rows.filter((row) => row.kind === "error")
  assert.deepEqual(errors.map((row) => row.text), ["The provider reported a server error: The server is overloaded. Try again later or choose another model.", "tool crashed"])
})

test("a Stop hook's follow-up is a notice row, not the operator's words", () => {
  const projection = new Projection()
  projection.apply(event(5, {
    kind: "source-input",
    text: '<dim-hooks kind="follow-up">\nrun the suite\nfix parser::tests\nif a &lt; b &gt; c &amp;&amp; d, keep the diff small\n</dim-hooks>',
    blocks: [],
    mode: "queue",
  }))
  assert.deepEqual(projection.rows, [{
    kind: "hook-follow-up",
    key: "notice-5",
    body: "run the suite\nfix parser::tests\nif a < b > c && d, keep the diff small",
    lines: 3,
  }])

  // The same channel carries SessionStart and PostToolUse context, and a Goal
  // continuation; none of them draws a row.
  for (const text of [
    '<dim-hooks kind="session-start">\nthe project conventions\n</dim-hooks>',
    '<dim-hooks kind="post-tool" call_id="c1" tool="exec">\nthe command failed\n</dim-hooks>',
    '<dim-goal kind="continuation" branch="main">\nkeep going\n</dim-goal>',
    '<dim-hooks>\nthe hook body mentions kind="follow-up"\n</dim-hooks>',
    '<dim-hooks-extra kind="follow-up">\nnot a follow-up\n</dim-hooks-extra>',
  ]) {
    projection.apply(event(projection.cursor + 1, { kind: "source-input", text, blocks: [] }))
    assert.equal(projection.rows.length, 1, text)
  }
})

test("a user row is settled by the turn end after its consumption, across an earlier window", () => {
  const say = (sequence, text) => event(sequence, { kind: "input", text, content: [{ type: "text", text }], blocks: [] })
  const applied = (sequence, input) => event(sequence, { kind: "input-applied", inputId: `e${input}` })
  const consumed = (sequence, input) => event(sequence, { kind: "input-consumed", inputId: `e${input}` })
  const settled = (projection) => Object.fromEntries(projection.rows.filter(row => row.kind === "user").map(row => [row.text, row.settled]))

  const earlier = new Projection()
  for (const item of [say(1, "first"), applied(2, 1), consumed(3, 1), say(4, "queued")]) earlier.apply(item)
  assert.deepEqual(settled(earlier), { first: false, queued: false })

  const latest = new Projection()
  for (const item of [
    event(5, { kind: "turn-end", outcome: "completed" }),
    applied(6, 4),
    consumed(7, 4),
    say(8, "steered"),
    applied(9, 8),
  ]) latest.apply(item)
  latest.prepend(earlier, "before-5")
  assert.deepEqual(settled(latest), { first: true, queued: false, steered: false }, "the queued input was consumed after the turn end that settled the first")

  latest.apply(event(10, { kind: "turn-end", outcome: "cancelled" }))
  assert.deepEqual(settled(latest), { first: true, queued: true, steered: false }, "a stop ends the turn before a request consumes the steer")

  latest.apply(say(11, "next"))
  latest.apply(applied(12, 11))
  latest.apply(consumed(13, 8))
  latest.apply(consumed(14, 11))
  assert.deepEqual(settled(latest), { first: true, queued: true, steered: false, next: false })
  latest.apply(event(15, { kind: "turn-end", outcome: "completed" }))
  assert.deepEqual(settled(latest), { first: true, queued: true, steered: true, next: true })
})

test("a zero-output retry takes the replaced thought on its terminal fact", () => {
  const projection = new Projection()
  projection.progress(failedThought())
  projection.apply(event(2, { kind: "model-end", requestId: "r1", error: "boom", failure: failure({ attempts: 2, retried: true }) }))
  assert.equal(projection.rows.filter((row) => row.kind === "thinking").length, 0, JSON.stringify(projection.rows))
  const row = projection.rows.at(-1)
  assert.equal(row.kind, "error")
  assert.ok(!row.text.startsWith("The response above is incomplete."), row.text)
})

test("a retry that showed its own thought keeps it on the terminal fact", () => {
  const projection = new Projection()
  projection.progress(failedThought())
  projection.progress({ ...failedThought(), thinkingPartId: "second", thinking: "final thought", thinkingStartedAt: 2_000 })
  projection.apply(event(3, { kind: "model-end", requestId: "r1", error: "boom", failure: failure({ attempts: 2, retried: true, thinkingPartId: "second" }) }))
  const thoughts = projection.rows.filter((row) => row.kind === "thinking")
  assert.equal(thoughts.length, 1, JSON.stringify(projection.rows))
  assert.equal(thoughts[0].streaming, false)
  assert.equal(thoughts[0].text, "", "only progress showed it")
  const row = projection.rows.at(-1)
  assert.ok(row.text.startsWith("The response above is incomplete. The provider reported a server error"), row.text)
})

test("a steer cancellation takes the replaced thought on its terminal fact", () => {
  const projection = new Projection()
  projection.progress(failedThought())
  projection.apply(event(2, { kind: "model-end", requestId: "r1", cancelled: "replaced by steer" }))
  assert.equal(projection.rows.filter((row) => row.kind === "thinking").length, 0, JSON.stringify(projection.rows))
})

test("a delayed snapshot cannot reopen a thought after committed text", () => {
  const projection = new Projection()
  projection.apply(event(1, { kind: "text", requestId: "r1", delta: "answer" }))
  projection.progress(failedThought())
  assert.equal(projection.rows.filter((row) => row.kind === "thinking").length, 0, JSON.stringify(projection.rows))
  assert.equal(projection.rows.find((row) => row.kind === "assistant").text, "answer")
})

test("a delayed snapshot cannot extend a shorter committed thought", () => {
  const projection = new Projection()
  projection.apply({ id: "second", sequence: 1, createAt: 1_001, content: { kind: "thinking", requestId: "r1", delta: "new" } })
  projection.progress(failedThought())
  const thoughts = projection.rows.filter((row) => row.kind === "thinking")
  assert.equal(thoughts.length, 1, JSON.stringify(projection.rows))
  assert.equal(thoughts[0].text, "new")
})

test("a new thought block after committed text is kept", () => {
  const projection = new Projection()
  projection.apply(event(1, { kind: "text", requestId: "r1", delta: "answer" }))
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", thinkingPartId: "next", text: "", textOffset: 6, thinking: "next thought", thinkingOffset: 0 })
  const thoughts = projection.rows.filter((row) => row.kind === "thinking")
  assert.equal(thoughts.length, 1, JSON.stringify(projection.rows))
  assert.equal(thoughts[0].text, "next thought")
})

test("committed text keeps the terminal's incomplete claim through a new thought", () => {
  const projection = new Projection()
  projection.apply(event(1, { kind: "text", requestId: "r1", delta: "answer" }))
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", thinkingPartId: "next", text: "", textOffset: 6, thinking: "next thought", thinkingOffset: 0 })
  projection.apply(event(3, { kind: "tool-use-start", requestId: "r1", callId: "call", name: "echo" }))
  projection.apply(event(4, { kind: "model-end", requestId: "r1", error: "boom", failure: failure() }))
  const row = projection.rows.at(-1)
  assert.equal(row.kind, "error")
  assert.ok(row.text.startsWith("The response above is incomplete. The provider reported a server error"), row.text)
})

test("a committed thought does not hold back the same notice's text", () => {
  const projection = new Projection()
  projection.apply(event(1, { kind: "thinking", requestId: "r1", delta: "weigh" }))
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", thinkingPartId: "e1", text: "hello", textOffset: 0, thinking: "weigh", thinkingOffset: 0 })
  const thoughts = projection.rows.filter((row) => row.kind === "thinking")
  assert.equal(thoughts.length, 1, JSON.stringify(projection.rows))
  assert.equal(thoughts[0].text, "weigh", "the committed thought is not shown twice")
  assert.equal(projection.rows.find((row) => row.kind === "assistant")?.text, "hello", "the same notice's text is not held back by the thought")
})

test("a closed thought does not hold back the same notice's text", () => {
  const projection = new Projection()
  projection.apply(event(1, { kind: "thinking", requestId: "r1", delta: "weigh" }))
  projection.apply(event(2, { kind: "tool-use-start", requestId: "r1", callId: "call", name: "read_file" }))
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", thinkingPartId: "e1", text: "hello", textOffset: 0, thinking: "weigh", thinkingOffset: 0 })
  assert.equal(projection.rows.filter((row) => row.kind === "thinking").length, 1, JSON.stringify(projection.rows))
  assert.equal(projection.rows.find((row) => row.kind === "assistant")?.text, "hello", "the tool start does not swallow the text after it")
})

test("committed text does not hold back the same notice's thought", () => {
  const projection = new Projection()
  projection.apply(event(1, { kind: "text", requestId: "r1", delta: "hi" }))
  projection.progress({ sessionId: "s", branchId: "main", requestId: "r1", thinkingPartId: "next", text: "hi", textOffset: 0, thinking: "weigh", thinkingOffset: 0 })
  assert.equal(projection.rows.find((row) => row.kind === "assistant")?.text, "hi")
  const thoughts = projection.rows.filter((row) => row.kind === "thinking")
  assert.equal(thoughts.length, 1, JSON.stringify(projection.rows))
  assert.equal(thoughts[0].text, "weigh", "the same notice's thought is not held back by the text")
})
