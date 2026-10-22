# Pursuit

Status: Draft, Ready candidate ([#146](https://github.com/arcships/dim/issues/146), milestone M2; no product decision remains, [§12](#12-what-remains-for-ready)). Scope: an amendment of [Goal](Goal.md)'s Session and branch parts; no new Capability ([ADR 002](../../adr/002-plugin-scope.md)). Language: TypeScript, in Goal's package `runtime/plugins/goal` ([Goal §10](Goal.md#10-implementation)). Depends on: Goal (P5, implemented by [#199](https://github.com/arcships/dim/pull/199)), C5 ([branch TypeScript binding](../branch-typescript.md)); both delivered. Milestone M2, not part of the current plan ([roadmap](../roadmap.md) D3).

Requirement ([feature list](../../feature-list.md), Agent behavior): keep working toward an explicit objective and record its completion or blockage, **with completion judged by an evaluator instead of the model doing the work**.

**Pursuit is a judge mode of Goal.** A goal set with `judge: "evaluator"` runs exactly as any Goal goal, with Goal's continuations, budget, interrupt, pause and limit. One thing changes: when a turn of the working branch ends and Goal does not continue because the worker reported the goal completed, Goal creates an **evaluator branch**. The evaluator checks the work in one turn and ends it with a conclusion: `DONE`, `CONTINUE` with what is at least still missing and the evidence, or `BLOCKED` with what only the user can provide. When the evaluator's turn ends, Goal on the evaluator writes the conclusion back to the working branch as an input, and Goal on the working branch decides: `DONE` completes the goal (only the evaluator can), `CONTINUE` becomes one continuation, `BLOCKED` blocks it. This replaces the experimental design (#17).

## 1. Decisions

| # | Decision | Why |
| --- | --- | --- |
| P1 | A judge mode of Goal, not a separate plugin | One owner of continuation; a separate plugin would race Goal on the same `turn/end` or read Goal's state (§4 rule 7) |
| P2 | The judge is chosen per goal by `set`, recorded on the `set` block | It is the user's intent for that goal, a Timeline fact |
| P3 | Evaluate when the worker's completion report stops Goal | That is the only point where the worker decides the goal is done. A worker's `blocked` stands (it needs the user); a limit, or the user's own interrupt or pause, is not a completion and starts nothing |
| P4 | Under the evaluator judge the worker's `completed` is a request; the status stays `active` | Only the evaluator completes the goal |
| P5 | The evaluator is a child branch cut at that `turn/end` | It sees what was done and why, runs under the same policy, and is an ordinary branch created with `treeCommands.create` |
| P6 | The evaluator writes its conclusion to the working branch as input; Goal there decides | A branch reads only its own events; cross-branch communication is input by id ([branch TypeScript binding](../branch-typescript.md#subscription-rebuild-and-recovery), item 6). Callers write, owners decide |

## 2. User flow

```
 user: session.goal.set {objective: "make the CLI tests pass", judge: "evaluator"}
   └─ goal active ─▶ turn ─▶ continuation ─▶ turn ...             (Goal, unchanged)
 worker: goal_update completed "all tests pass"                  (request; still active)
 turn/end N ─▶ Goal(B) does not continue ─▶ create evaluator E at N (operation goal:evaluate:N)
 E: checks, replies "... \n CONTINUE:\n - 2 tests still fail: <output>"
 E turn/end ─▶ Goal(E) writes the conclusion to B
 Goal(B):
   ├─ DONE     → completed (by evaluator)
   ├─ CONTINUE → continuation: "list may be incomplete; assess, continue, report again"
   ├─ BLOCKED  → blocked (by evaluator)
   └─ failed   → paused (evaluation_failed)
```

Clients read it all from the working branch's events ([Goal A.3](Goal.md#a3-reading-the-goal)): the judge on the `set` block, the evaluation as a `branch/create` with operation `goal:evaluate:<N>`, the conclusion input and Goal's outcome.

## 3. Changes to Goal

| Part | Change |
| --- | --- |
| `session.ts` | `session.goal.set` accepts `judge?: "model" \| "evaluator"` (default `model`; other values `invalid_params`) and marks the `set` objective block `judge="evaluator"` |
| `goal.ts` | The fold reads `judge` (from the `set`, kept by `edit`); under the evaluator judge a worker `completed` block leaves the status `active` and marks a request; the decisions and texts below |
| `branch.ts` | Adds the Dispatcher keys `branch/create` and `branch/create-failed`; creates the evaluator; on an evaluator branch, takes the evaluator role (§4.2) |

No new service, injection, Settings field or event type. The evaluator branch is an ordinary host-sourced child; its first input is the prompt ([TimelineTree `first_input`](../../../crates/dim-agent-capabilities/src/timeline_tree.rs)). Goal there has no goal of its own and writes the start `clear` as on any child ([Goal §3.4](Goal.md#34-a-new-branch-clears-the-inherited-goal)), so it never continues.

## 4. Decisions in Goal

Every decision is taken as Goal takes its own: by the Dispatcher handler from consumed state, through an owned task that rechecks before writing ([Goal A.4](Goal.md#a4-continuation-decision)).

### 4.1 Working branch

| Consumed event | Condition | Action |
| --- | --- | --- |
| `turn/end` N | Goal's C1–C4 hold (active goal, not interrupted, turn `completed`, latest and undecided), the judge is `evaluator`, and the turn contains a worker `completed` block | Instead of Goal's continuation decision: `treeCommands.create(B, N, {prompt, source: {host: {operation_id: "goal:evaluate:N"}}}, cancelled)`. The `branch/create` marks `N` decided, so a reload does not create again |
| `branch/create-failed` for `goal:evaluate:N`, or `create` throws | — | Pause (`evaluation_failed`) |
| Conclusion input for evaluation `N` | The first one for `N`, from the branch that `N`'s `branch/create` names | `done`: `completed` by the evaluator. `blocked`: `blocked` by the evaluator. `continue`: Goal's continuation decision C1 and C6 (active, under `maxContinuations`), then a continuation with the Pursuit text, or pause (`limit`). `failed`: pause (`evaluation_failed`) |

Later conclusions for the same `N`, or conclusions from another branch, decide nothing. Everything else is Goal's: while the evaluator works, the working branch behaves as any active goal (user input, interrupts, pauses and continuations follow Goal's rules), and the conclusion is applied when it arrives, in sequence order with whatever happened meanwhile.

### 4.2 Evaluator branch

Goal recognizes the role from the branch's first local input: user text whose first line is `<dim-goal kind="evaluate" branch="B" evaluation="N">`, where `B` is the `parent` of its `branch/start`.

| Consumed event | Condition | Action |
| --- | --- | --- |
| `turn/end` M, status `completed` | The reply has a conclusion | Write it to `B` with `treeCommands.input` (Append), marked `evaluation="N" evaluator="E"` |
| `turn/end` M, status `completed` | No conclusion | Write a follow-up (Queue, `after="M"`) on the evaluator branch asking it to end with one of the three conclusions |
| `turn/end` M, status `failed` or `cancelled` | — | Write `failed` to `B` |

The reply is the final model request of the turn: the `delta` of every `step/text` with that `turn_id` and the `request_id` of the turn's last `step/model-start`, concatenated in sequence order (a line can span records; earlier requests and thinking are excluded). The conclusion starts at the last line beginning `DONE:`, `CONTINUE:` or `BLOCKED:` and runs to the end, so a `CONTINUE` list can span lines; it is truncated to 4000 characters.

A reload takes the decision for the latest `turn/end` again, as Goal does: a restart that cut the evaluator's turn ends it `cancelled` (a terminal fact), so `failed` is written; a conclusion already written may be written again and is ignored by the working branch, which takes only the first.

## 5. Context mapping

Markers, escaping and keys are Goal's ([A.5](Goal.md#a5-model-visible-texts)). `N` is the request's `turn/end`, `B` the working branch, `E` the evaluator.

| Write | Where, key | Mode | Text |
| --- | --- | --- | --- |
| evaluator prompt | E, first input | Queue | `<dim-goal kind="evaluate" branch="B" evaluation="N">` / `You are an evaluator. Branch B reported that its goal is completed. Only you decide whether it is. Do not continue the work, do not change files or settings, and do not call goal_update.` / blank / `Objective:` / `<objective>` / blank / `Reported result:` / `<summary>` / blank / `Verify the facts yourself by reading files and running checks; do not rely on the report or on earlier messages. End your reply with exactly one of:` / `DONE: <the verified result>` / `CONTINUE: followed by the items that are at least still missing, one per line, each with its evidence` / `BLOCKED: <what only the user can provide>` |
| follow-up | E, no key | Queue | `<dim-goal kind="evaluate-again" after="M" branch="E">` / `Your reply did not end with a conclusion. End your reply with exactly one of DONE:, CONTINUE: or BLOCKED: as described above.` |
| conclusion | B, no key | Append | `<dim-goal kind="report" result="done\|continue\|blocked\|failed" evaluation="N" evaluator="E" branch="B">` / `Evaluator report:` / `<conclusion, or "the evaluator's turn did not complete">` |
| `done` | B, `goal:status` | Append | `<dim-goal kind="goal" op="completed" by="evaluator" evaluation="N" branch="B">` / `Result:` / `<result>` / blank / `An evaluator verified that the goal for branch B is completed. Do not continue working toward it on your own.` |
| `blocked` | B, `goal:status` | Append | `op="blocked" by="evaluator"`; `Blocker:` / `<what is needed>` / blank / `An evaluator found that the goal for branch B cannot proceed without the user. Do not continue working toward it on your own.` |
| `continue` | B, Goal continuation | Queue | `<dim-goal kind="continuation" count="k" limit="L" evaluation="N" branch="B">` / `An evaluator checked your report that the goal for branch B is completed and found it is not achieved yet. Its report above lists items that are at least still missing; the list may be incomplete. Assess yourself how far the goal still is, then continue working toward it. When you think it is achieved, call goal_update with status "completed" again.` |
| `failed` | B, `goal:status` | Append | Goal's pause with reason phrase "the evaluator could not check the result" |

Under the evaluator judge a worker `completed` block means `active` in the fold; it stays the latest status block until an outcome or a change replaces it, so the replay still matches the fold (Goal A16).

## 6. Rule check

Rules: [principles §1 and §4](../../architecture/rust-agent/principles.md), [ADR 002](../../adr/002-plugin-scope.md) D1–D5.

| Rule | How this document complies |
| --- | --- |
| §4.1 Command events for others' state | Creating the evaluator is a command to TimelineTree; the conclusion is an input to the working branch, where Goal, the owner, decides |
| §4.2 No recall after handoff | Nothing is withdrawn; a duplicate conclusion simply decides nothing |
| §4.3 Only before callbacks, never block | Not applicable: no before callback |
| §4.4 No business meaning in core | No change to Timeline, Dispatcher, TimelineTree, InputQueue or Loop |
| §4.5 Batch | Decisions follow consumed events in sequence order; each outcome is one input |
| §4.6 Context only from handed-off events | The worker sees the conclusion only through adopted inputs; the evaluator's reply stays on its branch |
| §4.7 Public events and services only | Goal reads only its own branch's public events and calls exported services; no branch reads another |
| §4.8 Requirements to mechanism owners | None |
| §4.9 Only facts later decisions use enter the Timeline | No new event type; `branch/create` records the evaluation, the conclusion input is what the owner decides on, the follow-up is what makes the evaluator conclude |
| §1 Capability and App Server boundary | No new Capability; nothing in App Server |
| ADR 002 D1 Scope | The evaluation and its outcome are working-branch state of Goal's branch part there; the evaluator role is Goal's branch part on the evaluator |
| ADR 002 D2 No wiring modules | Nothing names Goal |
| ADR 002 D3 Scope dependencies | Unchanged from Goal |
| ADR 002 D4 Configuration | The judge is a per-goal Timeline fact; no new configuration |
| ADR 002 D5 Language-neutral contracts | The `judge` parameter, markers and operation id are data |

## 7. Requirements raised to mechanism owners

None.

## 8. Security

The evaluator is an ordinary child branch with the working branch's permission policy and Approval ([ADR 003](../../adr/003-approval-scope.md)); "do not change anything" is guidance, not isolation. A conclusion counts only when it comes from the branch the working branch's own `branch/create` names, so a host branch created by an operator with a forged prompt decides nothing. Diagnostics record sequences and the result kind, never objective or report text.

## 9. Acceptance

Flows with a fake model for worker and evaluator, in `goal.test.mjs` and `transport_flow/goal.rs`:

1. Evaluator judge: continuations are Goal's. The worker reports `completed`; the status stays `active`; at that turn's end one evaluator is created at `N` with `goal:evaluate:N` and no continuation is written. `DONE` gives `completed` by the evaluator.
2. `CONTINUE` with a two-line list gives the conclusion and one continuation (`count` + 1); `BLOCKED` gives `blocked`; at `maxContinuations`, `CONTINUE` gives pause (`limit`).
3. A worker `blocked`, a limit pause, a user interrupt and a user pause create no evaluator.
4. An evaluator reply without a conclusion gets one follow-up on the evaluator branch, and its next conclusion is applied.
5. A failed or cancelled evaluator turn, and a failed creation, give pause (`evaluation_failed`).
6. Restart during the evaluation: no second evaluator; a cut evaluator turn gives `failed`; a conclusion written twice is applied once.
7. A conclusion from a branch that is not the recorded evaluator decides nothing.
8. The streamed conclusion is reassembled across `step/text` records and ignores an earlier request's `DONE:`.
9. Model judge: Goal's acceptance unchanged.

## 10. Later options

Not needed for this design; possible additions if users ask:

| Option | Note |
| --- | --- |
| Read-only or cheaper evaluator | Needs a branch configured before its first input and a per-branch Permission narrowing; neither exists |
| CLI `/pursue` | The CLI has no `/goal` commands yet; `/pursue` is `/goal` with `judge: "evaluator"` |

## 11. Ablation review

What is left, and why each piece is needed:

| Piece | Why |
| --- | --- |
| `judge` on `set` | Which goals are judged by an evaluator |
| Worker `completed` as a request under the evaluator judge | Only the evaluator completes the goal |
| Create at the stopping `turn/end` | The trigger; `branch/create` is also what keeps a reload from creating twice |
| Evaluator role in Goal's branch part | The evaluator branch must report its conclusion, and only a part on that branch can read its reply |
| Conclusion input to the working branch | The only cross-branch path; Goal there decides |
| First conclusion from the recorded evaluator only | Reloads may write it twice, and an operator could create a lookalike branch |
| Follow-up when the reply has no conclusion | Otherwise the evaluation would end without a result |
| `failed` on a failed or cancelled evaluator turn or creation | These are terminal facts; without it the goal would stay active with no evaluation running |

Removed from the earlier draft: claim and reopen states, deadline and `evaluationTimeoutSeconds`, the `goal/evaluation-reported` record, special rules for user input during an evaluation, and the requirements to stop and retire branches.

## 12. What remains for Ready

No product decision remains; this design waits for maintainer confirmation.
