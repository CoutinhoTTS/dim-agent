# Goal

Status: Draft. Overall design: [overview §5.2](../overview.md#52-goal). Phase: P2. Language: TypeScript first ([overview §11.3](../overview.md#113-assignment)).

Goal keeps a standing objective on a branch and continues the branch automatically until the model reports the objective completed or blocked, the user pauses or interrupts, or a continuation limit is reached. The goal and its status exist only as a keyed raw input block written through [session-access](../contracts/session-access.md); continuation is decided from a fixed Timeline prefix after a `turn/end` notice from the [broadcast subscription](../contracts/session-broadcast-subscription.md). Goal is a process plugin and writes no events of its own.

## 1. User flows

1. **Set.** The user runs `/goal <objective>`. The CLI calls `session.goal.set`; Goal writes an `active` goal block in Queue mode (A1). An idle branch starts a turn; the model works toward the objective.
2. **Continue.** The turn ends normally. Goal receives the `turn/end` notice, reads the prefix through the published head, finds the goal active with nothing else pending, and writes a continuation input in Queue mode. The next turn starts.
3. **Complete or block.** The model calls `goal_update {status: "completed" | "blocked", summary}`. Before returning, the handler writes a `completed` or `blocked` block under the same key. The turn ends; Goal sees a non-active goal and does not continue.
4. **Interrupt.** The user presses stop. The turn-scoped `stop/interrupt` notice makes Goal write a `paused` block (reason `interrupted`). No continuation follows.
5. **Pause, resume, edit, clear.** `session.goal.pause` writes `paused` (reason `user`); `resume` writes `active` again in Queue mode, which restarts work; `edit` writes a block with the new objective and the current status; `clear` retires the key.
6. **Limit.** After `maxContinuations` consecutive continuations without operator input (A4, A5), Goal writes `paused` (reason `limit`) instead of continuing.
7. **Inspect.** `session.goal.get` returns the current goal view of a branch (A8).

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Goal block content, status transitions and their write timing | Turn scheduling, input adoption and consumption (InputQueue, Loop) |
| Continuation decision and the continuation input | Stopping a running turn (pause never interrupts, A7) |
| `goal_update` tool and `session.goal.*` Session methods | Rendering in the CLI; `/goal` command parsing |
| Pausing on turn-scoped interrupt, failure and limit | Recovering missed `turn/end` notices across restart (not replayed, overview §5.2) |
| Its source name `goal`, key `goal:current`, configuration key `goal.maxContinuations` | Model context assembly and compaction (history projection) |

## 3. Scope and dependencies

**Contracts depended on:** [`SessionEvents`](../contracts/session-broadcast-subscription.md) (live trigger only), [`SessionAccess`](../contracts/session-access.md) (fixed-prefix reads and input writes under source `goal`), [`ProcessToolSource`](../contracts/process-tool-contribution.md) (the `goal_update` tool), the public data contracts of `input/message`, `input/consumed`, `turn/start`, `turn/end` and `stop/interrupt` (`crates/dim-agent-types/src/events.rs`), and the plugin configuration of the loader editable layer (overview §1, #3). Goal never reads InputQueue or Loop state and never names another plugin's types; it reads only facts in the prefix.

**Contracts defined here:** the `GoalControl` service (consumed by Rpc for `session.goal.*`), the `goal_update` tool, the goal block and continuation formats under source `goal` (§9), and configuration key `goal.maxContinuations`.

| Part | Scope | Injects | Provides | Status |
| --- | --- | --- | --- | --- |
| `Goal` | Process (App Server) | `Arc<SessionEvents>`, `Arc<SessionAccess>` | `ProcessToolSource` named `goal`; `GoalControl` | Proposed |
| Rpc | Process | `When<Arc<GoalControl>>` | `session.goal.set/edit/pause/resume/clear/get` | Supplement (see §14) |
| ToolRegistry | Session | `When<ProcessContributions<ProcessToolSource>>` | adopts `goal_update` | Supplement (contract) |

When Goal is not configured, `session.goal.*` return `method_not_found` and no tool is declared (overview §3 constraint 10, `When`).

## 4. Events

None. Every goal state is an `input/message` fact written through the Session input port (overview §6 "`goal/*`"):

| Fact Goal writes | Form | Why no new event |
| --- | --- | --- |
| Goal state (`active`, `paused`, `completed`, `blocked`) | `input/message` whose first raw block is the goal marker with key `goal:current` | It is model-visible content that must stay in effect and survive compaction; a keyed block already provides both |
| Clear | Same marker with `retire` | `retire` already ends a key |
| Continuation | `input/message`, Queue, marker kind `continuation`, no key | Its count is derivable by scanning markers |
| Pause on interrupt | `paused` block | Derivable from `stop/interrupt`, but written so the model sees it and compaction keeps it |

Events Goal consumes (read in the prefix, never subscribed on a Dispatcher):

| Event | Use |
| --- | --- |
| `turn/end` (notice and prefix) | Trigger; `TurnOutcome` decides continue / pause on failure |
| `stop/interrupt` (notice and prefix) | `scope: turn` after the latest goal block pauses the goal |
| `input/message` | Goal blocks, continuations, operator input (count reset), pending driving input |
| `input/consumed` | Whether a driving input is still pending |
| `turn/start` | Whether a newer turn already started |

## 5. State

Goal owns no persistent state. The goal of branch `B` is a pure function of a prefix of `B` (ancestor prefix plus local events), recomputed on every decision:

| Field | Derivation over the prefix |
| --- | --- |
| `G` | Latest `input/message` whose first raw block has key `goal:current`. None, or `retire`, means no goal |
| `status`, `owner`, `reason`, `objective` | Attributes and body of `G`'s marker (Goal parses only its own format) |
| `interrupted` | A `stop/interrupt {scope: turn}` in `B`'s local events with sequence greater than `G` |
| `last_operator` | Latest `input/message` with `user` content (any source or the user) |
| `count` | Goal continuation inputs with sequence greater than `max(G, last_operator)` (A5) |
| `pending_driving` | Local `input/message` with effective mode Queue or Steer whose event id has no `input/consumed` in the prefix |
| `turn_open` | A `turn/start` after the latest `turn/end` |

The prefix is the event history, not the model context, so compaction does not change any field. In-memory runtime state only: one serial lane per `(session, branch)` that orders Goal's own read-decide-write sequences, and the subscription. Neither is rebuilt; a new generation starts empty.

## 6. Interfaces

### Tool `goal_update`

| Item | Value |
| --- | --- |
| Parameters | `{status: "completed" \| "blocked", summary: string}`; `summary` 1..2000 characters (A10) |
| Result | `ToolResult.content`: `Goal marked completed.` or `Goal marked blocked.`; errors: `No active goal on this branch.`, `The goal belongs to branch <id>.` (A3) |
| Risk category | `session` (existing; writes only Session input) (A9) |
| Requirements | `interactive: false`, `roles: None` (offered to every Session) (A9) |
| Set key / version | `goal`; version changes only when Goal's generation changes |

### Session methods (Rpc)

| Method | Params | Result | Write |
| --- | --- | --- | --- |
| `session.goal.set` | `{sessionId, branchId?, objective}` | `{goal, sequence}` | `active` block, Queue |
| `session.goal.edit` | `{sessionId, branchId?, objective}` | `{goal, sequence}` | Block with new objective and current status; Queue if `active`, Append if `paused`/`blocked` (A1) |
| `session.goal.pause` | `{sessionId, branchId?}` | `{goal, sequence}` | `paused` (reason `user`), Append |
| `session.goal.resume` | `{sessionId, branchId?}` | `{goal, sequence?}` | `active`, Queue; on an already active goal only when `!turn_open && !pending_driving`, else no write (A13) |
| `session.goal.clear` | `{sessionId, branchId?}` | `{sequence}` | Retire block, Append |
| `session.goal.get` | `{sessionId, branchId?}` | `{goal: GoalView \| null, through}` | None (A8) |

`branchId` defaults to `main` (A15). `GoalView = {status, objective, reason?, summary?, owner, count, limit, sequence}`. Allowed transitions: `set` from any state; `edit` from `active`, `paused`, `blocked`; `pause` from `active`; `resume` from `paused`, `blocked`, `active`; `clear` from any live goal.

### Service

```rust
/// Process service consumed by Rpc; key TypeKey::of::<GoalControl>().
impl GoalControl {
    async fn set(&self, s: &str, b: &str, objective: String) -> Result<GoalWrite, GoalError>;
    async fn edit(&self, s: &str, b: &str, objective: String) -> Result<GoalWrite, GoalError>;
    async fn pause(&self, s: &str, b: &str) -> Result<GoalWrite, GoalError>;
    async fn resume(&self, s: &str, b: &str) -> Result<GoalWrite, GoalError>;
    async fn clear(&self, s: &str, b: &str) -> Result<GoalWrite, GoalError>;
    async fn get(&self, s: &str, b: &str) -> Result<(Option<GoalView>, u64), GoalError>;
}
pub struct GoalWrite { pub goal: Option<GoalView>, pub sequence: Option<u64> }
```

Configuration (loader editable layer): `goal.maxContinuations: u32`, default 20, range 1..=1000 (A4). Read at each decision; a change applies to the next decision. No management methods, no CLI subcommand beyond `/goal`.

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Subscribe | In `apply`: `SessionEvents::subscribe(ctx, {types: {"turn/end", "stop/interrupt"}, sessions: None, branches: None}, 256)`; the stream task is registered with `ctx.effect` |
| Trigger | Per notice, the task enqueues the work on the `(session, branch)` lane and returns to `next()` |
| Read | `access.source("goal").get(session)` (loaded only; Goal never restores a Session for a notice, A14), then `lease.prefix(branch, at_least = event.sequence)` |
| Write | `lease.input(branch, SourceInput { submission_id: "goal:<uuid>", marker, blocks: [], user: None, mode })` → `Session::input` → `Branch::input` → `TimelineContext::input` → `Timeline::append` (no registration callback). Goal waits for the returned committed event and then `lease.published` before releasing the lane, so its next read sees its own write |
| Consumption | InputQueue's Dispatcher subscription applies by mode (`input/applied`); Loop consumes (`input/consumed`); WaitBranch records Steer arrivals (Goal never writes Steer). Handlers register work and return; Goal registers nothing on any Dispatcher |
| StateCell | None owned. Input state stays in InputQueue's `StateCell<InputSnapshot>` and Loop's `LoopSnapshot`, which Goal never reads |
| Recheck | Every write is preceded, inside the lane, by a prefix read; the decision uses only facts in that prefix (time bus §4). Facts committed after `through` are ordered before the write and resolved by InputQueue and Loop |
| Terminal facts | Goal's work has no in-flight terminal fact: a decision either commits one input or none. The goal's own terminal state (`completed`/`blocked`) is written by the `goal_update` handler before it returns, so it precedes the batch's `step/tool-result` |
| Reload | Goal reload ends the stream and lanes; the new generation subscribes in `apply` and takes over from the next notice it receives; it never retries an earlier decision. An uncertain write (`Reloading`, `PublicationUnknown`) of the old generation either committed (then it is a fact, deduplicated by `pending_driving`) or did not (no continuation until the next trigger). In-flight `goal_update` calls are settled by ToolExecutor as reload (process-tool contract) |
| Restart | Same as reload. Missed `turn/end` notices are not replayed; an active goal waits for the next `turn/end` or `resume`. Committed, unconsumed continuations are restored with the Session and adopted (existing restore rule) |

**Continuation decision** on `turn/end` at sequence `N` on branch `B`, all conditions over the prefix:

| # | Condition | If false |
| --- | --- | --- |
| C1 | `G` exists, not retired, `status = active`, `owner = B` | Stop |
| C2 | No turn-scoped interrupt on `B` after `G` | Write `paused` (`interrupted`) and stop |
| C3 | The `turn/end` at `N` is `completed` | `failed`: write `paused` (`turn_failed`) (A6); `cancelled`: handled by C2 |
| C4 | `N` is the latest `turn/end` and `!turn_open` | Stop (a newer turn's end will trigger) |
| C5 | `!pending_driving` (includes Goal's own unconsumed continuation) | Stop |
| C6 | `count < maxContinuations` | Write `paused` (`limit`) and stop |
| — | All hold | Write continuation (Queue) with `count + 1` |

**Interrupt notice** at `M` (`scope: turn`) on `B`: if C1 holds and `G < M`, write `paused` (`interrupted`). `scope: branch` is ignored (the branch is closing; writes would return `BranchMissing`).

**Gap notice**: for each affected loaded Session, Goal runs the continuation decision for every branch with `N` = the latest `turn/end` in its prefix (A11).

**`goal_update` handler**: takes the lane of `(session_id, ctx.branch_id)`, reads `prefix(branch, head)`, requires C1 (error result otherwise), writes the terminal block in Append mode, awaits commit, returns. On cancellation before the write it writes nothing and returns `Tool cancelled.`; once `input` was called the write is not withdrawn.

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| User Queue input while a goal turn runs | input < `turn/end` N, unconsumed | C5 fails: no continuation; the user's turn runs; its `turn/end` continues with count reset | Goal (C5), InputQueue |
| User input between Goal's read and write | read < input < continuation | Both Queue; applied in order: user turn, then continuation turn. Count reset by the user input; the model sees both | InputQueue, Loop |
| User input after the continuation | continuation < input | Continuation turn first; Queue input next turn; Steer joins the continuation turn | InputQueue, Loop |
| Steer during a goal turn | steer consumed in turn | Same turn; resets count if it carries `user` content | Loop; Goal (A5) |
| Steer after `turn/end`, before decision | `turn/end` < steer | Steer starts a turn; C4 or C5 fails; that turn's end decides | Goal |
| Append input while idle | — | Not driving; continuation proceeds; Append consumed with its first request | Loop |
| Interrupt before Goal's read | interrupt < read | C2: `paused` written, no continuation | Goal |
| Interrupt between read and write | read < interrupt < continuation | Continuation starts one turn; the lane then handles the interrupt notice and writes `paused` (Append) after it; the model sees `paused` at its next request and stops; that turn's end fails C1. At most one extra turn (overview §5.2) | Loop; Goal |
| Interrupt after the continuation started its turn | consumed < interrupt | Turn cancelled; `paused` written | Loop; Goal |
| Interrupt after the continuation was applied, before consumption | applied < interrupt | Continuation stays pending (no turn); `paused` written; the next driving input consumes both in order; the model sees `paused` last | Loop; Goal |
| Interrupt during `goal_update` after its write | block < interrupt | Goal is `completed`/`blocked`; no pause (C1 false) | Goal |
| Interrupt during `goal_update` before its write | interrupt < handler write | Handler cancelled, no write; goal paused by the interrupt | ToolExecutor; Goal |
| Resume vs a pending interrupt notice | interrupt < resume block | Interrupt notice handled later finds `G > M`: no pause | Goal (sequence) |
| Branch-scoped interrupt or branch close | close < write | Write returns `BranchMissing`/`SessionClosed`; nothing written; lanes dropped | Runtime facade |
| Tool batch not settled | terminal block < `step/tool-result` | Block consumed after the batch result in the same turn; `turn/end` decision sees non-active goal | ToolExecutor, Loop |
| Two `goal_update` calls in one batch | lane order | Second finds no active goal: error result | Goal |
| `goal_update` vs `session.goal.edit`/`clear` | lane order | The later reads the earlier's block: edit of `completed` fails `goal_not_editable`; tool after clear fails | Goal |
| Compaction between a goal write and its consumption | write < compaction < consumed | Block delivered at its consumed position after the summary; later baselines replay it | History projection |
| Compaction after several goal blocks | — | Only the latest `goal:current` block is replayed; continuations survive only in the summary; Goal's count reads facts, unaffected | History projection; Goal |
| Child cut between a goal write and its consumption | write < cut < consumed | Child does not adopt it; child's goal view has `owner ≠ child`: no continuation | InputQueue (child); Goal |
| Child cut after the goal block was consumed | consumed < cut | Child model sees the parent goal (owner attribute names the parent); no continuation on the child; `goal_update` on the child is rejected (A3) | Goal |
| Goal set on a child branch | — | Independent key on the child's Timeline; parent unaffected | History projection |
| Session fork (SessionStore supplement) | facts copied | Fork's `main` owns the copied goal; continues after its own `turn/end` (A12) | Goal |
| Goal reload during a decision | — | Write committed or not; new generation does not retry; C5 deduplicates later | Goal |
| Goal reload during `goal_update` | — | Batch settled as reload; a committed terminal block stands | ToolExecutor |
| Process restart | `turn/end` missed | Not replayed; goal stays `active` until the next `turn/end` or `resume` | Goal |
| Session idle unload | `Ended { Unloaded }` | Lanes dropped; notices stop; RPC methods `acquire` (restore) before writing | AgentIndex; Goal |
| Session close | `Ended { Closed }` | RPC returns `session_closed` | Goal |
| Duplicate or reordered notices, Gap | — | Per-branch notices are ordered; duplicates fail C4/C5; Gap triggers re-evaluation (A11) | Goal |
| Duplicate `resume` | — | Second sees `turn_open` or `pending_driving`: no write | Goal (A13) |
| Duplicate `set` | — | Each writes a block; count resets | Goal (A13) |
| Turn failed | `turn/end {failed}` | `paused` (`turn_failed`) | Goal (A6) |
| Limit reached | count = limit | `paused` (`limit`) | Goal |
| Resource node offline | Session capabilities Pending | Inputs commit; turns start when capabilities recover | rutis lifecycle |
| Client detach | — | No effect; continuation proceeds without a client | — |

## 9. Context mapping

Every Goal input is one `input/message` with no `user` content whose only raw block is the source marker built by session-access as `<dim-goal kind="..." ...>` + newline + escaped body + newline + `</dim-goal>`. The model sees each as one user-role message, verbatim (session-access §9). Text below is the model-visible contract (A2); `<objective>` and `<summary>` are escaped by `context_block`.

| Write | Key | Mode | Exact text |
| --- | --- | --- | --- |
| `active` | `goal:current` | Queue (Append on edit when not active) | `<dim-goal kind="goal" status="active" branch="B">` / `Objective:` / `<objective>` / blank / `This is the user's standing goal for branch B. Keep working toward it across turns. When it is fully achieved, call goal_update with status "completed". If you cannot make progress without the user, call goal_update with status "blocked" and explain why.` / `</dim-goal>` |
| `paused` | `goal:current` | Append | Same open tag with `status="paused" reason="interrupted\|user\|limit\|turn_failed"`; body `Objective:` / `<objective>` / blank / `The goal is paused (<reason phrase>). Do not continue working toward it on your own; respond only to the user's latest input.` |
| `completed` / `blocked` | `goal:current` | Append | `status="completed"` or `"blocked"`; body `Objective:` / `<objective>` / blank / `Result:` or `Blocker:` / `<summary>` / blank / `The goal is completed.` or `The goal is blocked.` + ` Do not continue working toward it.` |
| clear | `goal:current`, `retire` | Append | `<dim-goal kind="goal" status="cleared" branch="B">` / `The previous goal was cleared by the user. Disregard it.` |
| continuation | none | Queue | `<dim-goal kind="continuation" count="k" limit="L">` / `Continue working toward the active goal. If it is completed or blocked, call goal_update.` |

Reason phrases: `interrupted` → "the user interrupted the turn", `user` → "paused by the user", `limit` → "the automatic continuation limit was reached", `turn_failed` → "the last turn failed".

| Aspect | Rule |
| --- | --- |
| Placement | At the `input/consumed` position: Queue blocks at the start of the turn they drive; Append blocks with the next model request of a running turn, or with the first request of the next turn when idle |
| In effect | The latest `goal:current` block is the goal in effect; earlier blocks stay at their positions in uncompacted history |
| Compaction | The baseline replays the latest live `goal:current` block verbatim (so a completed goal never replays as `active`); retired goals are not replayed; continuations and earlier goal blocks survive only through the summary |
| Child branches | Inherit blocks consumed before the cut, including a live `goal:current` naming the parent as `branch` |
| Tool | `goal_update` declaration (name, description "Report that the current goal is completed or blocked.", schema) from the adopted configuration; its result at the tool-result position |
| Non-interactive Sessions | Same mapping; continuations drive turns without a user present |
| Never visible | `submission_id`, mode, count of the user's earlier inputs, Goal's lanes, notices, diagnostics, other plugins' events |

## 10. Errors and recovery

| Error code | Cause |
| --- | --- |
| `invalid_params` | Empty or oversize objective (A10), unknown branch id format |
| `no_goal` | `pause`, `resume`, `edit`, `clear` without a live goal |
| `goal_not_active` | `pause` on a non-active goal |
| `goal_not_editable` | `edit` or `resume` on `completed` (use `set`) |
| `session_closed`, `session_unavailable`, `branch_missing` | Mapped from `AccessError` |
| `service_reloading` | `AccessError::Reloading` or Goal reloading; the caller may retry, the write did not happen or is found by `get` |
| `write_uncertain` | `PublicationUnknown`; `get` after recovery shows whether the block is in effect |

Goal never retries a write on its own except after `lookup(submission_id)` returns `Unknown` within the same generation. Process exit leaves only committed facts.

## 11. Security

Goal writes only `input/message` through session-access; resulting turns run under the Session's Permission and Approval policy, and a goal grants no authority. Users cannot forge goal blocks: `session.submit` accepts no raw blocks and `context_block` escapes the objective, so an objective containing `</dim-goal>` cannot end the element. `goal_update` is admitted as category `session`. Objectives and summaries are Timeline content like user input; diagnostics record only Session id, branch id, decision and reason, never objective text.

## 12. Acceptance

Flow tests through Session methods, the `goal_update` tool and Timeline events (fake model):

1. Set on an idle branch starts a turn; normal end writes a continuation; `goal_update completed` ends continuation; the completed block precedes the batch's `step/tool-result`.
2. Limit: with `maxContinuations = 2`, the third decision writes `paused (limit)`; operator input resets the count.
3. Every row of §8 with an interrupt: before read, between read and write (injected delay; at most one extra turn), after start, after apply; interrupt during `goal_update` before and after its write.
4. Pending user Queue/Steer input suppresses continuation; Append does not.
5. Compaction after `completed`: the next request's baseline contains the `completed` block, not `active`.
6. Child branch inherits the block, gets no continuation, and `goal_update` there returns the owner error; a goal set on the child is independent.
7. Duplicate `turn/end` handling (resubscribe) writes at most one continuation; Gap re-evaluation continues a stalled goal.
8. Restart with an active idle goal: no continuation until `resume`; a committed unconsumed continuation is adopted after restore.
9. Unloaded Session: `session.goal.pause` restores it and writes; notices never restore.
10. `edit`, `clear` (retired key not replayed), forged `</dim-goal>` in an objective is escaped.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | `set`, `resume` and `edit` of an active goal use Queue; all other writes use Append | Judgment (overview §5.2 fixes Queue only for continuation) | Set would not start work on an idle branch, or pause would start a turn |
| A2 | Exact block formats and model-visible texts in §9 | Judgment; marker format from session-access A2 | Text and parsing change together; stored history keeps old text |
| A3 | A goal belongs to the branch named in its block; other branches never continue or update it | Judgment from child-branch inheritance | Child branches would continue or complete the parent's goal |
| A4 | `goal.maxContinuations` default 20, range 1..=1000 | Judgment (overview §5.2 states a limit, not a value) | Different default |
| A5 | Count resets on a new goal block and on any input carrying `user` content | Judgment | Automatic notices (e.g. Steer blocks) would reset or not reset the count differently |
| A6 | A failed turn pauses the goal (`turn_failed`) | Judgment (avoid retry loops on provider errors) | Failed turns would continue and consume the limit |
| A7 | Pause does not interrupt the running turn | Judgment; overview §5.2 only writes a block | Pause would also call `stop` |
| A8 | `session.goal.get` exists | Judgment (CLI must show status without parsing Goal's blocks) | CLI derives status itself, coupling to Goal's format |
| A9 | `goal_update` risk `session`, offered to all Sessions and roles, parameters as in §6 | Judgment | Tool hidden in some Sessions or admitted under another category |
| A10 | Objective ≤ 4000 characters, summary ≤ 2000 | Judgment | Other limits |
| A11 | A Gap notice triggers re-evaluation of affected Sessions | Judgment from the subscription contract | Goals may stall after a dropped `turn/end` |
| A12 | A forked Session's `main` continues a copied goal | Judgment | Fork would need to retire the copied key |
| A13 | `resume` on an active idle goal restarts work; duplicate `set` is not deduplicated | Judgment (overview §5.2 "continues when the user resumes") | Restarted goals need `set` again; or `set` needs a request id |
| A14 | Notices never restore unloaded Sessions; RPC methods do | Judgment | Unloaded Sessions would be restored by background continuation |
| A15 | `branchId` defaults to `main`; any branch may hold a goal | Judgment | Goal limited to `main` |
| A16 | Rpc reaches Goal through optional `GoalControl` | Judgment from overview §9 "Session methods go into Rpc" | A generic Session-method contribution point replaces it (§14) |

## 14. Open questions

| Question | Description |
| --- | --- |
| Session-method contribution | Overview §9 puts Session methods in Rpc but defines no contribution point for process plugins; A16 makes Rpc inject `When<GoalControl>`, which names a consumer-specific service. A generic contribution (like Management's) would remove that coupling |
| "Goal mode" | The feature list says "`/goal`: switch Goal mode"; this design has no mode beyond the presence of a goal |
| Overview §13 "How events enter context" | Goal designs for adopted input only |
| Extra turn | Overview §5.2 accepts at most one extra turn on an interrupt race; an InputQueue withdrawal of Goal's unconsumed continuation (overview §7.1 `input/withdrawn`) could remove it, at the cost of depending on that service |
