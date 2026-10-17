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
ids. Every result ends with the arguments of the next step, so the first call needs no prior
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

## 3. What it tells

### 3.1 The pinned read

At the start of each call the tool reads `head = branch.sequence()` (the published sequence) and
`events = branch.history(Some(head))`, the branch's inherited prefix followed by its own events.
Everything else is computed from this one fixed list, so a call never mixes two heads. The pattern
is the one `wait` already uses ([wait.rs](../../../crates/dim-agent-capabilities/src/wait.rs)): a
Session tool that resolves the calling branch from `ToolExecutionContext.branch_id` through
`TreeCommands`.

### 3.2 The same selection as context assembly

Model context is assembled from events by `project_history`
([history.rs](../../../crates/dim-agent-capabilities/src/history.rs)). Recall selects with the same
rules, so "what the model went through" has one definition:

- an input counts once it is consumed; pending, withdrawn and deleted inputs never appear;
- a model output counts when it completed, or as the partial message an interruption left;
- a tool result counts with its context messages.

Recall uses `project_history` itself without changing it: each entry it yields names the event
that placed it, and that event's kind says what the entry is. Recall does not apply compaction:
it walks the whole list.

Besides what entered context, a few public events are part of what happened although no model ever
read them:

| Event | Told as |
| --- | --- |
| `approval/respond` | the user allowed or denied a call |
| `stop/interrupt` | the user interrupted the turn |
| `permission/changed` | the permission mode changed |
| `context/compaction` (completed) | the conversation up to here was summarized |
| `branch/create` of this branch | this branch was forked from another, with its task |

Everything else is machinery and is never told: `model/*`, `context/prepare`, `context/budget`,
`context/configured`, streaming `step/*` deltas, tool start and completion, prompt configuration
controls, `exec/*` (the tool's own call and result tell it). `turn/start` and `turn/end` only
delimit turns.

### 3.3 How each thing is told

| From | Told as |
| --- | --- |
| The user's content in an input | `user: <text>`, verbatim; images as `[image]` |
| A runtime block in an input | by its tag and source, not its wrapper: `rule AGENTS.md replaced`, `state <source> updated`, `<tag>: <first line>` |
| A model output | `you: <text>`; thinking left out |
| A tool call and its result | one line: `you called <tool> <key input> → ok|error, <length> chars: <text>` |
| An experience event (§3.2) | its sentence from the table above |

A call and its result are one thing, told once under the call's id; opening it shows the full
input and the full result. Recall reads blocks generically, by their tag, `source` attribute and
`key`; it knows no producer, so a new kind of block needs no change to it.

### 3.4 Not telling twice

Some of what happened is still in front of the model. Recall points to it instead of repeating
it:

- a block whose `key` is still live (the current rule or state) is told as
  `… (current version in your context)`; opening it shows the version that was delivered then;
- the latest summary is in context; the compaction line names it and does not repeat it.

Streaming deltas are never shown beside the final content, and a block delivered again under the
same key with the same text is told once.

### 3.5 Turns

A turn is the span from `turn/start` to `turn/end` of one `turn_id`; the inputs it consumed belong
to it. Experience events outside any turn (a compaction run between turns, the fork) are lines of
their own between turns. A turn's line in the index gives its first user text, the tools it
called with counts, and its outcome from `turn/end` (`completed`, `failed`, `cancelled`).

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
               "description": "With open: continue from this character."}
  },
  "additionalProperties": false
}
```

The arguments route the call:

| Arguments | Result |
| --- | --- |
| none, or `before` | the turn index, latest turns first page |
| `about` and/or `tool`, optionally `before` | the matching lines from every turn |
| `open` = a turn's id | that turn told line by line |
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
page continues with `before`. The values are tuned with #128.

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
14:04 3f9a2c1b you called web_fetch https://example.com/api/limits → ok, 12,400 chars: "Rate limits: 600 requests per minute per key…"
14:10 8d2f6b04 rule AGENTS.md replaced (current version in your context)
14:12 c0e95a71 the user denied: shell aws s3 cp report.csv s3://prod-…
14:20 77ab3e19 the user interrupted the turn
End of turn. Open one: open=<id>.
```

**One thing (`open=3f9a2c1b`):** a heading line `14:04 3f9a2c1b you called web_fetch`, the full
input, then the full result; then `Continue: open=3f9a2c1b offset=<next>.` or `End.`

| Result | Last line |
| --- | --- |
| Turn index | `Earlier: before=<id>. Open one: open=<id>.` or `Nothing earlier.` |
| Search | `More: before=<id>.` or `No more.`; nothing at all: `Nothing matches <the filters, restated>.` |
| A turn | `End of turn. Open one: open=<id>.` or `More of this turn: open=<id> before=…` |
| One thing | `Continue: open=<id> offset=<next>.` or `End.` |

Errors (`ToolResult::error`) say how to fix the call: `recall: open shows one turn or thing; drop
about, tool and before.`, `recall: offset needs open.`, `No turn or line has id <prefix>.`,
`<prefix> matches several: <longer prefixes>.`, `<id> has <len> characters.`; an unreadable
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

- **No writes and no type changes.** No event, `StateCell`, snapshot or broadcast; `HistoryEntry`
  and `project_history` are used as they are.
- **No new context-input path.** The result reaches the model only as a tool result (AGENTS rule
  10, principles §4 rule 6).
- **No raw events and no machinery.** Only what §3 tells.
- **No other branch or Session.** The branch comes from `ToolExecutionContext.branch_id`.
- **No cache.** Each call reads the pinned history (§13 Q2).

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
| Principles §4 rule 7 (public events and services) | It uses `TreeCommands`, the Branch facade, `project_history` and public events. It reads no plugin's internal state. |
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

## 13. Decisions and open questions

| Item | Decision |
| --- | --- |
| What it is | The branch's history told to the model, like a session view in a client |
| Range | The whole branch history, inherited prefix included; compaction is an event, not a boundary |
| Selection | Same rules as context assembly (`project_history`), plus approvals, interrupts, permission changes, compactions and the fork |
| Levels | Turn index → a turn → one thing in full; search returns lines of level 2 |
| Handles | Time and event id on every line; ids shown as unique prefixes |
| Not repeated | Live rules and state, the latest summary, streaming deltas, duplicate deliveries |
| Prompt | The declaration only (§5.1) |
| Risk | `session` |
| Assembly | Product profile only; not in `--models` or mock profiles |
| State, writes, type changes | None |

Open questions for the maintainer:

- **Q1. Need.** Implement now and measure use from `step/tool-call` facts named `recall`, with
  post-compaction "exact wording" cases added to #170's evaluation set; or wait for #170.
- **Q2. Cost.** Each call reads and walks the whole pinned history. If that proves slow on long
  histories, the first step is a cache keyed by branch and head inside the tool.

## 14. Acceptance (flow tests)

Flow tests in `crates/dim-agent-capabilities/tests` with a test strategy, plus one product-profile
test in `crates/dim-app-server/tests`:

1. **Turn index.** A call with no arguments lists turns latest first with time, id, first user
   text, tools with counts and outcome; `before=` pages back to `Nothing earlier.`
2. **A turn.** Opening a turn tells its user text verbatim, its replies, each call with its result
   on one line, a denied approval and an interrupt, in order, each with time and id.
3. **One thing.** Opening a call's id shows its full input and result, paged with `offset` to
   `End.`; CJK text is never split.
4. **Selection.** Withdrawn and deleted inputs, thinking and machinery events never appear; a
   runtime block appears by its tag and source, and a live keyed block says its current version is
   in context.
5. **Compaction.** After two compactions, recall still reaches the first turn; each compaction is
   one line; ids returned before a compaction still open the same things after it.
6. **Search.** `about` with words in another order and case finds a user message from before the
   first compaction; `tool` keeps only that tool's calls.
7. **Ids.** A short unique prefix opens; an ambiguous prefix lists longer ones; a child branch
   opens inherited ids.
8. **Bounds and no side effects.** No result exceeds 10,000 characters; the Timeline gains only
   `step/tool-call` and `step/tool-result`.
9. **Permission.** Under the read-only preset the tool runs without an approval request.

## Appendix A. Rust binding

```rust
// dim-agent-capabilities/src/recall.rs (new)
pub const RECALL_TOOL: &str = "recall";
pub struct RecallPlugin { scope: SessionScope }
//   Deps = (InSession<TreeCommands>,)
//   apply: ctx.provide_as(scope.named_key::<ToolContribution>(RECALL_TOOL),
//                         ToolContribution { definition, handler: RecallTool(tree) })
struct RecallTool(Arc<TreeCommands>);
impl ToolHandler for RecallTool { /* §3–§6, using history::project_history */ }

// dim-app-server/src/main.rs, product profile (beside ProductSummaryStrategyPlugin)
builder.session_plugin(RecallPlugin::new)
       .session_tool(RECALL_TOOL);
```
