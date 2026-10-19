# Recall

Status: Draft. Issue: [#129](https://github.com/arcships/dim/issues/129). Milestone: M2. Language:
Rust. Scope: [ADR 002](../../adr/002-plugin-scope.md) (Session). Depends on
[ContextCompaction](ContextCompaction.md) (F1, implemented) and the `toolContribution` attach rule
(C3, implemented).

Recall is a Session plugin that gives the model one read-only tool, `recall`. It tells the model
the history of its own branch as an account of what happened: what the user said, what the model
said and did, what came back, and what else happened along the way. It reads published Timeline
events through the Branch facade and returns bounded text as a tool result. It writes no event,
holds no state and changes no other module or type.

## 1. Problem and scope

A compaction replaces the earlier conversation with a summary
([ContextCompaction §4.3](ContextCompaction.md#43-events)). The summary keeps a paraphrase; the
user's exact words and examples, and content fetched earlier, are gone from the model's view. The
Timeline still holds every event.

Recall is not a view of "what is behind the cut". It is memory of the whole branch history, the
inherited prefix included, the way a person recalls a day: a compaction is one thing that
happened in it, not a boundary.

**Out of scope:** other branches and Sessions, raw event data, and any change to compaction, the
summary or context assembly.

## 2. How the model recalls

Recall works like a session view in a client, told to the model instead of shown to a user, at
three levels:

1. **Turns.** One line per turn: when, what the user asked, what the model did, how the turn
   ended. Paged from the latest turn back.
2. **A turn.** The turn told in order: one line per thing that happened, each with its time and
   id.
3. **One thing.** Its full text, by id.

A search (`about`, `tool`) returns the matching lines of level 2 from every turn, with the same
ids. `about` matches each thing's full text (a call's whole input and result `content`, a reply's
whole text), not the shortened line, so a word deep inside a fetched page or a sub-agent's report
is found. Every result ends with the arguments of the next step, so the first call needs no prior
knowledge.

```
 recall()                         recall(open="e41c07d2")              recall(open="3f9a2c1b")
 +----------------------------+   +-------------------------------+    +-------------------------+
 | Turn 14:21 e41c07d2        |   | 14:03 9b1e4d70 user: Use the  |    | Rate limits: 600        |
 |   user: "Stop. Don't …"    |-->|   staging bucket, never prod… |--->| requests per minute per |
 | Turn 14:03 c2d8a915        |   | 14:04 3f9a2c1b you called     |    | key; bursts of 50 …     |
 |   user: "Use the staging…" |   |   web_fetch → ok, "Rate lim…" |    | End.                    |
 +----------------------------+   +-------------------------------+    +-------------------------+
```

## 3. The narrative projection

`project_history` cannot tell the story: it builds model requests, so it splits a call into a
`ToolUse` part and a separate `ToolResult` message and keeps nothing else. Approvals, background
execution, sub-agents and calls that never ran are not in it. Recall therefore has its own
projection over events. It selects what counts with the same rules as context assembly (§3.3),
and it tells it in its own shape.

### 3.1 The pinned read

At the start of each call the tool reads `head = branch.sequence()` (the published sequence) and
`events = branch.history(Some(head))`, the branch's inherited prefix followed by its own events.
Everything else is computed from this one fixed list, so a call never mixes two heads. The pattern
is the one `wait` already uses ([wait.rs](../../../crates/dim-agent-capabilities/src/wait.rs)): a
Session tool that resolves the calling branch from `ToolExecutionContext.branch_id` through
`TreeCommands`. Recall does not apply compaction: it walks the whole list.

### 3.2 Things: an origin event and what joins it

The projection folds events into **things**. A thing has one **origin event** and the later
events that join it by an id the events already carry. A thing's line shows the origin's time and
id; its ending is the state left after every joined event is folded in.

| Thing | Origin event | Joined events (join key) |
| --- | --- | --- |
| User input | `input/message` with user content | `input/consumed` (`input_id`) |
| Runtime block | a block of an `input/message` | `input/consumed` (`input_id`); a later block with the same `key` |
| Your reply | `step/model-start` | `step/text` deltas, then optionally `step/model-content` as the final content, then `step/model-output-end` (`request_id`) |
| Tool call | `step/tool-use-start` | `step/tool-use` with the complete arguments, when it arrived (`call_id`); `approval/request` with source `tool` (`call_id`), then `approval/respond` or `approval/cancelled` (`approval_id`); the item of `step/tool-result` (`call_id`); `exec/started` (`call_id`) and `exec/ended` (`task_id`); `collaboration/request` (`call_id`), then `collaboration/message` (`call_id`) and `collaboration/result` (`request_event_id`); `branch/create` with source `tool` (`call_id`) |
| Interrupt | `stop/interrupt` | — |
| Permission change | `permission/changed` | — |
| Compaction | `context/compaction` with outcome `completed` | — |
| Fork | this branch's `branch/start` with a `parent` | — |

The last four never entered a model context, but they are part of what happened: the user's
decisions and the shape of the conversation.

The origins are the first event of each output and each call, not their completion, because an
interruption or failure can end either before it completes. This is the fold `project_history`
already applies ([history.rs](../../../crates/dim-agent-capabilities/src/history.rs)):

- A reply collects its `step/text` deltas; a later `step/model-content` replaces them with the final
  content. `step/model-output-end` ends it as completed, failed or cancelled. Thinking is dropped. A
  request whose folded text is empty (only calls, or only thinking) is no reply line; its calls are
  told on their own.
- A call takes its name from `step/tool-use-start`. Its arguments are those of `step/tool-use` when
  that arrived; otherwise they were never committed and are not shown. A call closed by its
  output's failure gets the closing result that `model_provider` writes with `call_event_id`
  absent, so it ends as `not run` (§3.4).

Every other event is machinery. It is folded as an intermediate state or dropped, and never told
on its own: `model/*`, `context/prepare`, `context/budget`, `context/configured`,
`context/compact`, `context/compaction-start`, `step/thinking` deltas, `step/tool-call`,
`step/tool-start`, `step/tool-completed`, `prompt/configure-*`, `input/applied`, `input/withdraw`,
`input/deleted`. `turn/start` and `turn/end` delimit turns (§3.6).

### 3.3 What counts

The projection counts with the rules context assembly uses, so "what the model went through" has
one definition:

- an input counts once it is consumed; an input that is pending, withdrawn or deleted is no thing;
- a reply counts when its output completed, or as the partial text an interruption or failure
  left;
- a tool result is told as the model received it: its `content`, not `structuredContent`.

A flow test holds the two projections together: every event that `project_history` places in
context is the origin of, or joined to, a thing (§14).

### 3.4 How each thing is told

| Thing | Line |
| --- | --- |
| User input | `user: <text>`, verbatim, images as `[image]`; a `steer` input is `user (while you were working): <text>` |
| Runtime block | by its tag and `source` attribute, not its wrapper: `rule <source> set|replaced|revoked`, `state <source> updated|cleared`, otherwise `<tag>: <first line>` |
| Your reply | `you: <text>`, thinking left out; an output that did not complete is `your reply was cut off (<reason>): <partial text>`, with the committed text and the outcome of its `step/model-output-end`; an output the head ends inside is `you (still writing): <text so far>` |
| Tool call | `you called <tool> <input brief> → <ending>`; a call whose arguments never arrived is `you started calling <tool> (arguments not received) → <ending>` |
| Interrupt | `the user interrupted the turn` |
| Permission change | `permission mode changed to <mode>` |
| Compaction | `the conversation up to here was summarized` |
| Fork | `this branch was forked from <parent branch>`; its first input, told next, is its task |

Recall reads blocks generically, by tag, `source` and `key`; it knows no producer, so a new kind
of block needs no change to it.

**A tool call's ending** is the folded state, the first that applies:

| Joined events | Ending |
| --- | --- |
| `approval/respond` deny | `denied by the user` |
| `approval/cancelled` | `approval cancelled: <reason>` |
| a result that closed a call never dispatched (`call_event_id` absent) | `not run: <result text>` |
| `collaboration/result` | `sub-agent <agent> answered: <text>` (or its failure) |
| `exec/ended` | `ran in background task <task>; finished at HH:MM: ok|error|cancelled: <text>` |
| `exec/started` without `exec/ended` | `running in background task <task>` |
| a result with `is_error` | `error: <text>` |
| a result | `ok: <text>` |
| none (the head ends inside the call) | `no result yet` |

**The input brief** is generic, for every tool alike: the scalar fields of `input` as
`key=value`, strings cut at about 60 characters, nested values as `{…}`. Opening the call shows the
full input.

**The result** is the `content` text; blobs and images are `[file]` and `[image]`; context messages
the tool added are `with <n> context messages`. Opening the call shows all of it. When the result
arrives in a later minute than the call, the ending carries its time.

### 3.5 Not telling twice

Some of what happened is still in front of the model. Recall points to it instead of repeating
it:

- a block whose `key` is still live (the current rule or state) is told as
  `… (current version in your context)`; opening it shows the version delivered then;
- the latest summary is in context; its compaction line says so and does not repeat it.

A block delivered again under the same key with the same text is told once.

### 3.6 Turns

A turn is the span of one `turn_id` from `turn/start` to `turn/end`; the inputs it consumed belong
to it. Things outside any turn (a compaction between turns, the fork) are lines of their own
between turns. A turn's line in the index gives its first user text, the tools it called with
counts, and its outcome from `turn/end` (`completed`, `failed`, `cancelled`). A turn without
`turn/end` in the pinned history is `in progress` when it is the calling turn, and `continued on
the parent branch` when it is the parent's turn that a child's inherited prefix cuts into.

## 4. Handles: time and event id

Every line carries the time of its event (`create_at`) and its event id. The id is how the model
opens it.

- Event ids are UUIDs. Recall shows the first 8 hex characters and accepts any prefix that is
  unique in the pinned history, as git does with commit hashes; an ambiguous prefix is an error
  that lists the matching longer prefixes.
- An id stays valid forever on this branch and on its child branches, which share the prefix.
  Timeline sequences cannot serve: each segment of a forked history numbers from 1.
- A turn's id is the id of its `turn/start` event.

Times are shown in UTC: a date line `YYYY-MM-DD (UTC)` starts each page and each change of day,
and lines carry `HH:MM`.

## 5. Tool contract

### 5.1 Declaration

`toolContribution`, Session scope, key `recall`
([session-boundary](../contracts/session-boundary.md)):

| Field | Value |
| --- | --- |
| `name` | `recall` |
| `risk` | `session` |
| `source` | `extension` |
| `description` | "Recall what happened earlier in this conversation: what the user said, what you did and what came back. With no arguments it lists past turns; open a turn or anything in it by its id, or search with about." |

The declaration is the only prompt. The summary, the replacement and the system prompt do not
mention the tool (§7).

### 5.2 Input

```json
{
  "type": "object",
  "properties": {
    "about":  {"type": "string", "minLength": 1, "maxLength": 200,
               "description": "Words to look for; all must appear, in any order, ignoring case."},
    "tool":   {"type": "string", "minLength": 1,
               "description": "Only calls to this tool."},
    "before": {"type": "string", "description": "An id; show what came before it."},
    "open":   {"type": "string", "description": "An id; show that turn, or that thing in full."},
    "offset": {"type": "integer", "minimum": 0,
               "description": "With open: continue from here, a line of a turn or a character of one thing."}
  },
  "additionalProperties": false
}
```

The arguments route the call:

| Arguments | Result |
| --- | --- |
| none, or `before` | the turn index, latest turns first page |
| `about` and/or `tool`, optionally `before` | the matching lines from every turn |
| `open` = a turn's id, optionally `offset` | that turn told line by line, from line `offset` (0 first) |
| `open` = a line's id, optionally `offset` | that thing in full |

`open` with `about`, `tool` or `before`, and `offset` without `open`, are argument errors.

### 5.3 Limits (provisional)

| Limit | Value |
| --- | --- |
| Whole `content` | at most 10,000 characters |
| Turn index, search | at most 20 lines; each at most 200 characters |
| One thing in full | at most 8,000 characters from `offset` |

Characters are Unicode scalar values, so CJK text is never cut inside a character. A result never
exceeds the limit: a page stops before the line that would exceed it, and a turn longer than one
page continues with `open=<turn> offset=<next line>`. A turn's lines only grow at its end, so
an offset returned once keeps pointing at the same line. The values are tuned with #128.

## 6. What the model sees

The text is plain lines; it carries no wrapper tag. `structuredContent` is for UI and persistence
only: `{level: "turns" | "turn" | "search" | "full", ids: [...], next?: {...}}`.

**Turn index:**

```
2026-10-17 (UTC)
14:21 e41c07d2 turn: user "Stop. Don't upload anything yet, only write the files locally." — you called shell ×2 — completed
14:03 c2d8a915 turn: user "Use the staging bucket, never prod. Name files YYYY-MM-DD-report.csv…" — you called web_fetch, shell ×4, edit ×2 — cancelled
13:40 a7730e5f the conversation up to here was summarized (that summary is in your context)
...
Earlier: before=a7730e5f. Open one: open=<id>.
```

**A turn (`open=c2d8a915`):**

```
2026-10-17 (UTC)
14:03 9b1e4d70 user: Use the staging bucket, never prod. Name files YYYY-MM-DD-report.csv, e.g. 2026-10-01-report.csv.
14:03 51c0aa3e you: I'll check the API limits first.
14:04 3f9a2c1b you called web_fetch url=https://example.com/api/limits → ok: "Rate limits: 600 requests per minute per key…"
14:10 8d2f6b04 rule AGENTS.md replaced (current version in your context)
14:12 c0e95a71 you called shell command="aws s3 cp report.csv s3://prod-reports/…" → denied by the user
14:20 77ab3e19 the user interrupted the turn
End of turn. Open one: open=<id>.
```

**One thing (`open=3f9a2c1b`):** a heading line `14:04 3f9a2c1b you called web_fetch`, the full
input, then the full result; then `Continue: open=3f9a2c1b offset=<next>.` or `End.`

| Result | Last line |
| --- | --- |
| Turn index | `Earlier: before=<id>. Open one: open=<id>.` or `Nothing earlier.` |
| Search | `More: before=<id>.` or `No more.`; nothing at all: `Nothing matches <the filters, restated>.` |
| A turn | `End of turn. Open one: open=<id>.` or `More of this turn: open=<id> offset=<next>.` |
| One thing | `Continue: open=<id> offset=<next>.` or `End.` |

Errors (`ToolResult::error`) say how to fix the call: `recall: open shows one turn or thing; drop
about, tool and before.`, `recall: offset needs open.`, `No turn or line has id <prefix>.`,
`<prefix> matches several: <longer prefixes>.`, `<id> has <len> characters.`, `Turn <id> has <n> lines.`; an unreadable
branch returns the facade's error.

Old content is shown as text the model reads, not as instructions to it: lines are labelled by who
said or did what, and the model treats them as it treats any tool result.

## 7. Discovery

The model learns about the tool from its declaration, in every request where the tool is
configured. Rejected:

- **A line in the summary prompt or `summary_message` naming the tool.** The replacement is stored
  in the durable `context/compaction` record, so the tool name would be frozen into history, and
  the summary strategy, a core part, would depend on an optional tool (principles §3, ADR 002 D3).
- **Declaring the tool only after a compaction.** Tool tables are Session-scoped and compaction is
  per branch; it would need a new ToolRegistry mechanism (principles §4 rule 8). Recall is also
  useful before any compaction, to find an exact earlier message in a long context.

## 8. Interactions

| Situation | Behaviour |
| --- | --- |
| Later compactions | Nothing moves: recall covers the whole history and ids never change. Each compaction is one more line |
| Compaction during the call | Cannot happen on the calling branch: compaction runs in a request's preparation or as a Loop step, never while a tool batch runs ([ContextCompaction §5.4](ContextCompaction.md#54-manual-compaction)). The pinned head keeps the result consistent regardless |
| Branch fork inside a Session | The child recalls the inherited prefix with the same ids, then its fork line and its own events |
| Session fork, deletion | A forked Session has its own history. Closing a Session cancels calls in flight through the tool's cancellation token |
| Interrupt | `stop/interrupt` cancels the call's token. The tool checks it after the read and before rendering and returns `HostError::Cancelled` |
| Reload | The tool holds no state; a call in flight ends and ToolExecutor writes its terminal result |
| Recall results | A recall is itself a tool call, so later recalls tell it like any other |
| Context growth | Each call is bounded (§5.3); threshold compaction bounds the rest |

## 9. What it does not do

- **No writes and no type changes.** No event, `StateCell`, snapshot or broadcast. The narrative
  projection is Recall's own; `project_history` and its types are unchanged.
- **No new context-input path.** The result reaches the model only as a tool result (AGENTS rule
  10, principles §4 rule 6).
- **No raw events and no machinery.** Only what §3 tells.
- **No other branch or Session.** The branch comes from `ToolExecutionContext.branch_id`.
- **No cache.** Each call reads the pinned history (§13).

## 10. Security and permission

- **Risk `session`.** Allowed under the read-only and workspace-write presets without asking
  ([permission.rs](../../../crates/dim-agent-capabilities/src/permission.rs)), as for `wait`. It
  returns what already entered this branch's model context and the few experience events of §3.2;
  it touches no file, process or network.
- **Trust boundary: the model configured on the branch now.** A result goes to the provider and
  model configured at the time of the call, which need not be the one that first received the
  content; a branch can adopt another model (`session.agent.update`;
  [configuration.rs](../../../crates/dim-app-server/src/agents/configuration.rs)). This is the
  boundary the branch already has: after a switch, the next request sends the new provider the
  whole projected context. Recall extends it to events no longer in context, one bounded call at a
  time. A user who must keep earlier content from another provider starts a new Session for it.
- **Untrusted content.** Old events can contain fetched pages or injected text. They come back as
  labelled lines inside a tool result, the same standing they had when first read.
- **Tracing.** Nothing beyond the tool call and result facts.

## 11. Rule check

| Rule | Answer |
| --- | --- |
| Principles §4 rule 1 (commands for others' state) | Recall changes no other module's state. Its result is written by ToolExecutor. |
| Principles §4 rule 2 (handover) | Not applicable: it takes over no object. |
| Principles §4 rule 3 (before callbacks) | None. Argument checks are inside the tool handler. |
| Principles §4 rule 4 (no business meaning in core) | No core change. It reads runtime blocks generically by tag and key, not by producer. |
| Principles §4 rule 5 (atomic batch) | Not applicable: it writes nothing; its read is pinned to one published head. |
| Principles §4 rule 6 (context from handed-over events) | It selects with the context-assembly rules and adds only public experience events; its result reaches the model only as a tool result. |
| Principles §4 rule 7 (public events and services) | It uses `TreeCommands`, the Branch facade and public events and their join ids. It reads no plugin's internal state. |
| Principles §4 rule 8 (infrastructure unchanged) | Timeline, Dispatcher, the envelope, assembly, the facades and the projection types are unchanged. |
| Principles §4 rule 9 (only facts later decisions use) | No new event. |
| Principles §1 (Capability and App Server boundary) | A Session Capability in `dim-agent-capabilities`, installed by the App Server in the product profile. |
| ADR 002 D1 (scope follows state) | Session: it offers a tool and holds no state; the data belongs to the branch and is read per call. |
| ADR 002 D2 (no wiring modules) | One `toolContribution`. |
| ADR 002 D3 (dependencies) | Core only. If it is absent, only the tool is absent (C3 omission rule). |
| ADR 002 D4 (configuration) | No business configuration; limits are provisional constants (§5.3, #128). |
| ADR 002 D5 (language-neutral contracts) | The contract (§5, §6) is a neutral declaration and text; the Rust form is Appendix A. |

**Impact (public content):** one tool name, `recall`. No event, method, wire field, service or
type change.

## 12. Requirements raised to mechanism owners

None. Considered and not needed: a paged history read on the facade (the selection needs the whole
prefix anyway) and a per-branch tool declaration (§7).

## 13. Decisions

| Item | Decision |
| --- | --- |
| What it is | The branch's history told to the model, like a session view in a client |
| Range | The whole branch history, inherited prefix included; compaction is an event, not a boundary |
| Projection | Its own: things folded from an origin event and the events joined to it by existing ids (§3.2); counted with the context-assembly rules, plus interrupts, permission changes, compactions and the fork |
| Tool calls | One line per call with a generic input brief and the folded ending: approval, not run, background task, sub-agent, error or ok (§3.4) |
| Levels | Turn index → a turn → one thing in full; search returns lines of level 2 |
| Handles | Time and event id on every line; ids shown as unique prefixes |
| Not repeated | Live rules and state, the latest summary, streaming deltas, duplicate deliveries |
| Prompt | The declaration only (§5.1) |
| Risk | `session` |
| Assembly | Product profile only; not in `--models` or mock profiles |
| State, writes, type changes | None |
| Cost | No cache: each call reads and walks the pinned history. A cache keyed by branch and head inside the tool is added only if a measurement shows the need; the facade does not change |
| Delivery | Milestone M2 (#129); implementation follows the M2 plan |

## 14. Acceptance (flow tests)

Flow tests in `crates/dim-agent-capabilities/tests` with a test strategy, plus one product-profile
test in `crates/dim-app-server/tests`:

1. **Turn index.** A call with no arguments lists turns latest first with time, id, first user
   text, tools with counts and outcome; `before=` pages back to `Nothing earlier.`
2. **A turn.** Opening a turn tells its user text verbatim, its replies, each call with its result
   on one line, and an interrupt, in order, each with time and id.
3. **One thing.** Opening a call's id shows its full input and result, paged with `offset` to
   `End.`; CJK text is never split.
4. **Selection.** Withdrawn and deleted inputs, thinking and machinery events never appear; a
   runtime block appears by its tag and source, and a live keyed block says its current version is
   in context. Every event `project_history` places in context belongs to a thing.
5. **Compaction.** After two compactions, recall still reaches the first turn; each compaction is
   one line; ids returned before a compaction still open the same things after it.
6. **Search.** `about` with words in another order and case finds a user message from before the
   first compaction, and a word deep inside a long tool result finds that call; `tool` keeps only
   that tool's calls.
7. **Tool call endings.** One call each that is denied, never dispatched, moved to a background
   task that later ends, answered by a sub-agent, failed and succeeded is told on one line with the
   ending of §3.4; a steer input is told as the user speaking while the model worked.
8. **Ids and forks.** A short unique prefix opens; an ambiguous prefix lists longer ones. A child
   created by the `branch` tool opens inherited ids, tells its fork line from its own
   `branch/start`, and tells the parent turn its prefix cuts into as `continued on the parent
   branch`; the calling turn is `in progress`.
9. **Bounds and no side effects.** No result exceeds 10,000 characters; the Timeline gains only
   `step/tool-call` and `step/tool-result`.
10. **Permission.** Under the read-only preset the tool runs without an approval request.
11. **A long turn.** A turn longer than one page ends with `More of this turn: open=<turn>
    offset=<n>.`; following each returned continuation reaches the end of the turn, every line
    exactly once, and no page exceeds the content limit.
12. **Interrupted output.** On the existing cancellation and failure shapes (as in
    `context_flow/checkpoint_projection.rs`): partial `step/text` ended by a failed or cancelled
    `step/model-output-end` with no `step/model-content` is told as a cut-off reply with exactly
    the committed text; a call with `step/tool-use-start` and no `step/tool-use` is told as
    started, without arguments, ending `not run`; thinking never appears.

## Appendix A. Rust binding

```rust
// dim-agent-capabilities/src/recall.rs (new)
pub const RECALL_TOOL: &str = "recall";
pub struct RecallPlugin { scope: SessionScope }
//   Deps = (InSession<TreeCommands>,)
//   apply: ctx.provide_as(scope.named_key::<ToolContribution>(RECALL_TOOL),
//                         ToolContribution { definition, handler: RecallTool(tree) })
struct RecallTool(Arc<TreeCommands>);
impl ToolHandler for RecallTool { /* §3–§6 */ }

// dim-app-server/src/main.rs, product profile (beside ProductSummaryStrategyPlugin)
builder.session_plugin(RecallPlugin::new)
       .session_tool(RECALL_TOOL);
```
