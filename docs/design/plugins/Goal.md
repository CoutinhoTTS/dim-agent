# Goal

Status: Draft. Overall design: [overview §5.2](../overview.md#52-goal). Scope: branch.

Requirement ([feature list](../../feature-list.md), Agent behavior): `/goal` sets, edits, pauses, resumes and clears a goal, and goal continuation keeps working toward an explicit goal and records its completion or blockage.

Goal keeps a standing objective on a branch. While the goal is open, every `turn/end` on that branch starts an **evaluator**: a child branch cut at that `turn/end`, which checks the facts (reads files, runs checks) and reports whether the objective is done. Not done: Goal queues a continuation carrying the evaluator's feedback. Done or blocked: Goal closes the goal. The model working on the goal does not judge its own completion.

## 1. User flow

```
 user: /goal "make the CLI tests pass"
   └─ session.goal.set ─▶ goal/open ─▶ Goal queues the objective ─▶ turn runs
 turn/end N ─▶ goal open ─▶ evaluator branch cut at N
                              checks the facts, ends with a verdict
            ◀─ goal/verdict{after N, open G}
               ├─ continue → Queue input: objective + feedback ─▶ next turn
               ├─ done     → goal/close{completed}   (user sees the summary)
               └─ blocked  → goal/close{blocked}     (user sees what is needed)
 user: Esc / stop ─▶ stop/interrupt ─▶ goal/close{interrupted}
 user: /goal resume ─▶ goal/open ─▶ continues with the same objective
```

## 2. Responsibility

| Goal does | Goal does not |
| --- | --- |
| Hold the objective and whether it is open, per branch | Change InputQueue, Loop, Permission or branch creation |
| Start one evaluator per `turn/end` while open, and settle it | Give the evaluator any authority beyond the branch's own |
| Queue continuations and close the goal on verdicts, interrupts and the limit | Provide a model tool; completion is the evaluator's verdict |

## 3. Scope and dependencies

Goal is one branch capability, `GoalPlugin`. There is no Session or process part: Rpc reaches it per call through `Session::service::<Goal>(branch)`, as it reaches [Approval](../../architecture/rust-agent/plugins/Approval.md).

| Injects | Purpose |
| --- | --- |
| `B:Timeline` | Write `goal/*` events and Goal's inputs |
| `B:Dispatcher` | Consume `goal/*`, `turn/start`, `turn/end` and `stop/interrupt` in sequence order |
| `S:TreeCommands` | Create, await, read and close evaluator branches |
| Settings business source | `goal.maxContinuations`, read at each verdict |

| Provides | Purpose |
| --- | --- |
| `B:Goal` | `write(change)` and `get()` for the Session methods (§6) |

## 4. Events

Goal declares three events on the branch Timeline. They are not model context, so a write that changes nothing (for example a second `pause`) is harmless and needs no check by the writer.

| Event | Data | Written by | Consumed by |
| --- | --- | --- | --- |
| `goal/open` | `{objective?: string}`; present: set or edit, absent: resume | `B:Goal.write` | Goal: opens the goal and queues the objective |
| `goal/close` | `{reason: "paused" \| "cleared" \| "interrupted" \| "completed" \| "blocked" \| "limit", summary?: string}` | `B:Goal.write` (paused, cleared); Goal itself (the rest) | Goal: closes the goal |
| `goal/verdict` | `{after: N, open: G, result: "done" \| "continue" \| "blocked", feedback: string}`; `G` is the sequence of the `goal/open` in effect when the evaluator started | Goal, when its evaluator for `turn/end` N ends, or at load for an evaluator that did not survive recovery (§8) | Goal: closes or continues (§5.2) |

Why they cannot be derived: whether a goal is open and what the evaluator decided are not facts of any existing event. `goal/verdict` is also the terminal fact of the evaluator, which Goal started on this branch ([time bus §2](../../architecture/rust-agent/time-bus.md)); the evaluator's start is the existing `branch/create` on this branch.

## 5. State and decisions

### 5.1 State

Goal reads only its branch's **local** events; `goal/*` events in the ancestor prefix are not folded, so a child branch, an evaluator included, has no goal of its own.

| Value | Derivation |
| --- | --- |
| `objective` | The latest `objective` in a `goal/open`; none after `goal/close{cleared}` |
| `open` | The latest of `goal/open`, `goal/close` and `stop/interrupt {scope: turn}` is `goal/open`, and `objective` exists. An interrupt closes the goal at its own sequence, so the cancelled `turn/end` that follows it starts no evaluator even before Goal's `goal/close{interrupted}` commits |
| `count` | Goal's continuation inputs (`kind="continuation"`) after the latest `goal/open`, so only continuations actually written use the budget |

All of it is rebuilt by folding the local events at load. In memory only: the evaluator task in flight.

### 5.2 Decisions

All decisions are taken after the whole batch of the consumed event.

| Consumed event | Action |
| --- | --- |
| `goal/open` that leaves the goal open | Write the objective input (Queue) |
| `turn/end` N, goal open | Start the evaluator for N (§5.3) |
| `stop/interrupt {scope: turn}` that closed an open goal | Write `goal/close{interrupted}`, the record and the closing note for it |
| `goal/close` | Write the closing note (Append); close the evaluator in flight, if any |
| `goal/verdict` for N | Applies only if the goal is open, `G` is still the latest `goal/open` and there is no `turn/start` after N; otherwise ignored. Then: `done` → `goal/close{completed}`; `blocked` → `goal/close{blocked}`; `continue` → `goal/close{limit}` if `count >= maxContinuations`, else the continuation input (Queue) |

A verdict therefore applies only to the goal and the turn it evaluated. A `goal/open` after `G` (set, edit or resume, including after a pause or clear) makes it stale even before any new turn starts; a `turn/start` after N means user input arrived during evaluation, and that turn's own `turn/end` is evaluated instead. Reaching the limit is treated like a blocked goal: the goal closes, and `resume` reopens it with the count restarted.

### 5.3 Evaluator

1. Record `G`, the latest `goal/open` consumed through N. `S:TreeCommands.branch(parent = this branch, at = N, {source: host, operation_id: "goal:N", prompt})`. The child inherits the branch history up to N, and its permission policy is the branch's own at N (Permission replays the ancestor prefix). The prompt is the evaluator input (§7).
2. Await the child's idle, read its last assistant message, and parse the verdict at this boundary: a last line `DONE: <summary>`, `CONTINUE: <feedback>` or `BLOCKED: <reason>`. Anything else is `continue` with the message as feedback.
3. Write `goal/verdict{after: N, open: G, ...}`, then close the child. A failed child (creation or turn failure) yields `blocked` with feedback "evaluation failed".

At most one evaluator runs per branch: a new `turn/end` only happens after a new turn, which already makes the running evaluator's verdict stale, so Goal closes it and starts the new one.

## 6. Interfaces

Session methods in Rpc; each resolves `branchId` (default `main`) and calls `B:Goal`.

| Method | Params | Writes |
| --- | --- | --- |
| `session.goal.set` | `{sessionId, branchId?, objective}` | `goal/open{objective}` |
| `session.goal.edit` | `{sessionId, branchId?, objective}` | `goal/open{objective}` (also reopens a closed goal) |
| `session.goal.pause` | `{sessionId, branchId?}` | `goal/close{paused}` |
| `session.goal.resume` | `{sessionId, branchId?}` | `goal/open{}` |
| `session.goal.clear` | `{sessionId, branchId?}` | `goal/close{cleared}` |
| `session.goal.get` | `{sessionId, branchId?}` | — ; returns `{objective, open, reason?, summary?, count, limit, through}` |

Writing methods return `{sequence}` and never read the state; `through` in `get` is the last sequence Goal has consumed, so a client sees its write applied once `through >= sequence`. CLI: `/goal <objective>`, `/goal edit|pause|resume|clear`, and `/goal` alone shows `get`.

## 7. Context mapping

Everything the model sees is an `input/message` written by Goal, whose first raw block is `<dim-goal kind="...">`, escaped body, `</dim-goal>`. There are no keyed blocks: every objective and continuation input restates the objective, so nothing depends on compaction replaying an older block.

| Input | Mode | Text |
| --- | --- | --- |
| objective | Queue | `kind="objective"`: `Objective:` / `<objective>` / blank / `Work toward this objective. After each turn an evaluator checks the result and tells you what is still missing.` |
| continuation | Queue | `kind="continuation" count="k" limit="L"`: `Objective:` / `<objective>` / blank / `The evaluator found it is not done yet:` / `<feedback>` |
| closing note | Append | `kind="closed" reason="..."`: `The goal is closed (<reason>). Do not continue working toward it on your own.` plus `<summary>` when present |
| evaluator prompt (child) | first input | `Objective:` / `<objective>` / blank / `Check whether the work so far achieves this objective. Verify facts by reading files and running checks; do not change anything. End with one line: DONE: <summary>, CONTINUE: <what is missing>, or BLOCKED: <what the user must provide>.` |

## 8. Errors and recovery

| Case | Settlement |
| --- | --- |
| Interrupt during a turn or during evaluation | `goal/close{interrupted}`; the evaluator is closed; its verdict, if written, is ignored |
| Pause or clear during evaluation | Same as above, with its own reason |
| Goal reload or process restart with an evaluator in flight | The task ends with the generation. Started work is not re-executed ([overview constraint 4](../overview.md#3-integration-methods)): at load, for an evaluator `branch/create` (`operation_id = "goal:N"`) without a later `goal/verdict` for N, Goal closes the leftover child and writes `goal/verdict{after: N, open: G, blocked, "evaluation did not survive recovery"}`. If it still applies, the goal closes as blocked; the user resumes it explicitly |
| Evaluator branch fails to create or its turn fails | `goal/verdict{blocked, "evaluation failed"}` |
| Branch close | Goal unloads; the evaluator is closed with the parent branch |
| Unknown branch, closed Session | `branch_missing`, `session_closed` |

## 9. Security

The evaluator has exactly the branch's permission policy at N; its tool calls pass Approval like any branch's, so it gains nothing the main branch does not have. Its prompt asks it not to change anything; that is guidance, not isolation. Users cannot forge Goal inputs: `session.submit` accepts no raw blocks, and objective, feedback and summary are escaped. Diagnostics record Session, branch, decision and reason, never the objective text.

## 10. Acceptance

Flow tests through the Session methods and Timeline events, with a fake model for both the branch and the evaluator:

1. Set on an idle branch: the objective input starts a turn; at its end an evaluator branch is created at that `turn/end`; `CONTINUE` queues a continuation with the feedback; `DONE` writes `goal/close{completed}` and no further turn.
2. `BLOCKED` closes with the reason; an unparsable verdict counts as `continue`.
3. Limit: with `maxContinuations = 2`, two continuations are written and the third applicable `continue` writes `goal/close{limit}`; `resume` reopens with the count restarted.
4. Interrupt during a turn and during evaluation: `goal/close{interrupted}`, evaluator closed, a late verdict ignored.
5. User input during evaluation: the verdict is ignored (a `turn/start` follows N); the user turn's end is evaluated.
6. Reload and restart with an evaluator in flight: the evaluator is not rerun; one `blocked` verdict settles it and the goal closes; `resume` continues.
7. A child branch of a branch with an open goal has no goal; the evaluator branch does not start evaluators.
8. The evaluator's policy equals the branch's policy at N.
9. Pause twice, resume without an objective, clear then resume: no state change and no error.
10. Reopen before the next turn: while evaluating N under `G`, `clear` then `set` (and separately `pause` then `resume`) before any `turn/start`; the old evaluator's `DONE` or `BLOCKED` is ignored and the new goal stays open.
11. A stale `continue` (user input started a turn during evaluation) writes no continuation and does not use the budget.

## 11. Open questions

| Question | Note |
| --- | --- |
| Read-only evaluator | Narrowing the evaluator below the branch's policy needs the policy at creation; `branch/create` carries none today, so a change after creation can lose to the first request |
| Cost | One evaluator run per turn; evaluating only every k turns or only when the model ends without tool calls are possible refinements |
| Evaluator branches in listings | They are closed after their verdict; whether clients hide them is a client decision |
