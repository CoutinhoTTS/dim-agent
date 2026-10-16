# Pursuit

Status: Deferred. Scope: branch. Not part of the [overall design](../overview.md) plugin catalog.

> **Deferred ([#44](https://github.com/arcships/dim/issues/44)).** Goal is implemented first. Judging completion with evaluator branches needs a larger design than this document, which also depends on dynamic events (#2) for its own `pursuit/*` events and on Rpc reaching it by type. It is kept for reference and is not scheduled.

Pursuit is an experimental alternative to [Goal](Goal.md) for the same requirement ([feature list](../../feature-list.md), Agent behavior: keep working toward an explicit objective and record its completion or blockage). It differs in who decides completion: Goal continues until the working model reports completion through `goal_update`; Pursuit asks an **evaluator**, a separate branch of the same Session, after every turn. Both continue the same branch automatically, so a Session enables at most one of them. Pursuit uses its own names (`pursuit/*` events, `session.pursuit.*` methods, `/pursue`) and shares no contract with Goal.

Pursuit keeps a standing objective on a branch. While the pursuit is open, every `turn/end` on that branch starts an evaluator: a child branch cut at that `turn/end`, which checks the facts (reads files, runs checks) and reports whether the objective is done. Not done: Pursuit queues a continuation carrying the evaluator's feedback. Done or blocked: Pursuit closes the pursuit. The model working on the pursuit does not judge its own completion.

## 1. User flow

```
 user: /pursue "make the CLI tests pass"
   └─ session.pursuit.set ─▶ pursuit/open ─▶ Pursuit queues the objective ─▶ turn runs
 turn/end N ─▶ pursuit open ─▶ evaluator branch cut at N
                              checks the facts, ends with a verdict
            ◀─ pursuit/verdict{after N, open G}
               ├─ continue → Queue input: objective + feedback ─▶ next turn
               ├─ done     → pursuit/close{completed}   (user sees the summary)
               └─ blocked  → pursuit/close{blocked}     (user sees what is needed)
 user: Esc / stop ─▶ stop/interrupt ─▶ pursuit/close{interrupted}
 user: /pursue resume ─▶ pursuit/open ─▶ continues with the same objective
```

## 2. Responsibility

| Pursuit does | Pursuit does not |
| --- | --- |
| Hold the objective and whether it is open, per branch | Change InputQueue, Loop, Permission or branch creation |
| Start one evaluator per `turn/end` while open, and settle it | Give the evaluator any authority beyond the branch's own |
| Queue continuations and close the pursuit on verdicts, interrupts and the limit | Provide a model tool; completion is the evaluator's verdict |

## 3. Scope and dependencies

Pursuit is one branch capability, `PursuitPlugin`. There is no Session or process part: Rpc reaches it per call through `Session::service::<Pursuit>(branch)`, as it reaches [Approval](../../architecture/rust-agent/plugins/Approval.md).

| Injects | Purpose |
| --- | --- |
| `B:Timeline` | Write `pursuit/*` events and Pursuit's inputs |
| `B:Dispatcher` | Consume `pursuit/*`, `turn/start`, `turn/end` and `stop/interrupt` in sequence order |
| `S:TreeCommands` | Create, await, read and close evaluator branches |
| Settings business source | `pursuit.maxContinuations`, read at each verdict |

| Provides | Purpose |
| --- | --- |
| `B:Pursuit` | `write(change)` and `get()` for the Session methods (§6) |

## 4. Events

Pursuit declares three events on the branch Timeline. They are not model context, so a write that changes nothing (for example a second `pause`) is harmless and needs no check by the writer.

| Event | Data | Written by | Consumed by |
| --- | --- | --- | --- |
| `pursuit/open` | `{objective?: string}`; present: set or edit, absent: resume | `B:Pursuit.write` | Pursuit: opens the pursuit and queues the objective |
| `pursuit/close` | `{reason: "paused" \| "cleared" \| "interrupted" \| "completed" \| "blocked" \| "limit", summary?: string}` | `B:Pursuit.write` (paused, cleared); Pursuit itself (the rest) | Pursuit: closes the pursuit |
| `pursuit/verdict` | `{after: N, open: G, result: "done" \| "continue" \| "blocked", feedback: string}`; `G` is the sequence of the `pursuit/open` in effect when the evaluator started | Pursuit, when its evaluator for `turn/end` N ends, or at load for an evaluator that did not survive recovery (§8) | Pursuit: closes or continues (§5.2) |

Why they cannot be derived: whether a pursuit is open and what the evaluator decided are not facts of any existing event. `pursuit/verdict` is also the terminal fact of the evaluator, which Pursuit started on this branch ([time bus §2](../../architecture/rust-agent/time-bus.md)); the evaluator's start is the existing `branch/create` on this branch.

## 5. State and decisions

### 5.1 State

Pursuit reads only its branch's **local** events; `pursuit/*` events in the ancestor prefix are not folded, so a child branch, an evaluator included, has no pursuit of its own.

| Value | Derivation |
| --- | --- |
| `objective` | The latest `objective` in a `pursuit/open`; none after `pursuit/close{cleared}` |
| `open` | The latest of `pursuit/open`, `pursuit/close` and `stop/interrupt {scope: turn}` is `pursuit/open`, and `objective` exists. An interrupt closes the pursuit at its own sequence, so the cancelled `turn/end` that follows it starts no evaluator even before Pursuit's `pursuit/close{interrupted}` commits |
| `count` | Pursuit's continuation inputs (`kind="continuation"`) after the latest `pursuit/open`, so only continuations actually written use the budget |

All of it is rebuilt by folding the local events at load. In memory only: the evaluator task in flight.

### 5.2 Decisions

All decisions are taken after the whole batch of the consumed event.

| Consumed event | Action |
| --- | --- |
| `pursuit/open` that leaves the pursuit open | Write the objective input (Queue) |
| `turn/end` N, pursuit open | Start the evaluator for N (§5.3) |
| `stop/interrupt {scope: turn}` that closed an open pursuit | Write `pursuit/close{interrupted}`, the record and the closing note for it |
| `pursuit/close` | Write the closing note (Append); close the evaluator in flight, if any |
| `pursuit/verdict` for N | Applies only if the pursuit is open, `G` is still the latest `pursuit/open` and there is no `turn/start` after N; otherwise ignored. Then: `done` → `pursuit/close{completed}`; `blocked` → `pursuit/close{blocked}`; `continue` → `pursuit/close{limit}` if `count >= maxContinuations`, else the continuation input (Queue) |

A verdict therefore applies only to the pursuit and the turn it evaluated. A `pursuit/open` after `G` (set, edit or resume, including after a pause or clear) makes it stale even before any new turn starts; a `turn/start` after N means user input arrived during evaluation, and that turn's own `turn/end` is evaluated instead. Reaching the limit is treated like a blocked pursuit: the pursuit closes, and `resume` reopens it with the count restarted.

### 5.3 Evaluator

1. Record `G`, the latest `pursuit/open` consumed through N. `S:TreeCommands.branch(parent = this branch, at = N, {source: host, operation_id: "pursuit:N", prompt})`. The child inherits the branch history up to N, and its permission policy is the branch's own at N (Permission replays the ancestor prefix). The prompt is the evaluator input (§7).
2. Await the child's idle, read its last assistant message, and parse the verdict at this boundary: a last line `DONE: <summary>`, `CONTINUE: <feedback>` or `BLOCKED: <reason>`. Anything else is `continue` with the message as feedback.
3. Write `pursuit/verdict{after: N, open: G, ...}`, then close the child. A failed child (creation or turn failure) yields `blocked` with feedback "evaluation failed".

At most one evaluator runs per branch: a new `turn/end` only happens after a new turn, which already makes the running evaluator's verdict stale, so Pursuit closes it and starts the new one.

## 6. Interfaces

Session methods in Rpc; each resolves `branchId` (default `main`) and calls `B:Pursuit`.

| Method | Params | Writes |
| --- | --- | --- |
| `session.pursuit.set` | `{sessionId, branchId?, objective}` | `pursuit/open{objective}` |
| `session.pursuit.edit` | `{sessionId, branchId?, objective}` | `pursuit/open{objective}` (also reopens a closed pursuit) |
| `session.pursuit.pause` | `{sessionId, branchId?}` | `pursuit/close{paused}` |
| `session.pursuit.resume` | `{sessionId, branchId?}` | `pursuit/open{}` |
| `session.pursuit.clear` | `{sessionId, branchId?}` | `pursuit/close{cleared}` |
| `session.pursuit.get` | `{sessionId, branchId?}` | — ; returns `{objective, open, reason?, summary?, count, limit, through}` |

Writing methods return `{sequence}` and never read the state; `through` in `get` is the last sequence Pursuit has consumed, so a client sees its write applied once `through >= sequence`. CLI: `/pursue <objective>`, `/pursue edit|pause|resume|clear`, and `/pursue` alone shows `get`.

## 7. Context mapping

Everything the model sees is an `input/message` written by Pursuit, whose first raw block is `<dim-pursuit kind="...">`, escaped body, `</dim-pursuit>`. There are no keyed blocks: every objective and continuation input restates the objective, so nothing depends on compaction replaying an older block.

| Input | Mode | Text |
| --- | --- | --- |
| objective | Queue | `kind="objective"`: `Objective:` / `<objective>` / blank / `Work toward this objective. After each turn an evaluator checks the result and tells you what is still missing.` |
| continuation | Queue | `kind="continuation" count="k" limit="L"`: `Objective:` / `<objective>` / blank / `The evaluator found it is not done yet:` / `<feedback>` |
| closing note | Append | `kind="closed" reason="..."`: `The pursuit is closed (<reason>). Do not continue working toward it on your own.` plus `<summary>` when present |
| evaluator prompt (child) | first input | `Objective:` / `<objective>` / blank / `Check whether the work so far achieves this objective. Verify facts by reading files and running checks; do not change anything. End with one line: DONE: <summary>, CONTINUE: <what is missing>, or BLOCKED: <what the user must provide>.` |

## 8. Errors and recovery

| Case | Settlement |
| --- | --- |
| Interrupt during a turn or during evaluation | `pursuit/close{interrupted}`; the evaluator is closed; its verdict, if written, is ignored |
| Pause or clear during evaluation | Same as above, with its own reason |
| Pursuit reload or process restart with an evaluator in flight | The task ends with the generation. Started work is not re-executed ([overview constraint 4](../overview.md#62-constraints)): at load, for an evaluator `branch/create` (`operation_id = "pursuit:N"`) without a later `pursuit/verdict` for N, Pursuit closes the leftover child and writes `pursuit/verdict{after: N, open: G, blocked, "evaluation did not survive recovery"}`. If it still applies, the pursuit closes as blocked; the user resumes it explicitly |
| Evaluator branch fails to create or its turn fails | `pursuit/verdict{blocked, "evaluation failed"}` |
| Branch close | Pursuit unloads; the evaluator is closed with the parent branch |
| Unknown branch, closed Session | `branch_missing`, `session_closed` |

## 9. Security

The evaluator has exactly the branch's permission policy at N; its tool calls pass Approval like any branch's, so it gains nothing the main branch does not have. Its prompt asks it not to change anything; that is guidance, not isolation. Users cannot forge Pursuit inputs: `session.submit` accepts no raw blocks, and objective, feedback and summary are escaped. Diagnostics record Session, branch, decision and reason, never the objective text.

## 10. Acceptance

Flow tests through the Session methods and Timeline events, with a fake model for both the branch and the evaluator:

1. Set on an idle branch: the objective input starts a turn; at its end an evaluator branch is created at that `turn/end`; `CONTINUE` queues a continuation with the feedback; `DONE` writes `pursuit/close{completed}` and no further turn.
2. `BLOCKED` closes with the reason; an unparsable verdict counts as `continue`.
3. Limit: with `maxContinuations = 2`, two continuations are written and the third applicable `continue` writes `pursuit/close{limit}`; `resume` reopens with the count restarted.
4. Interrupt during a turn and during evaluation: `pursuit/close{interrupted}`, evaluator closed, a late verdict ignored.
5. User input during evaluation: the verdict is ignored (a `turn/start` follows N); the user turn's end is evaluated.
6. Reload and restart with an evaluator in flight: the evaluator is not rerun; one `blocked` verdict settles it and the pursuit closes; `resume` continues.
7. A child branch of a branch with an open pursuit has no pursuit; the evaluator branch does not start evaluators.
8. The evaluator's policy equals the branch's policy at N.
9. Pause twice, resume without an objective, clear then resume: no state change and no error.
10. Reopen before the next turn: while evaluating N under `G`, `clear` then `set` (and separately `pause` then `resume`) before any `turn/start`; the old evaluator's `DONE` or `BLOCKED` is ignored and the new pursuit stays open.
11. A stale `continue` (user input started a turn during evaluation) writes no continuation and does not use the budget.

## 11. Open questions

| Question | Note |
| --- | --- |
| Read-only evaluator | Narrowing the evaluator below the branch's policy needs the policy at creation; `branch/create` carries none today, so a change after creation can lose to the first request |
| Cost | One evaluator run per turn; evaluating only every k turns or only when the model ends without tool calls are possible refinements |
| Evaluator branches in listings | They are closed after their verdict; whether clients hide them is a client decision |
