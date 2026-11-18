# CLI current activity

Design for [#1659](https://github.com/arcships/dim/issues/1659). Implementation
requires a subsequent decision. [Timeline UI](timeline-ui.md) owns the display
layout, and [CLI commands](cli-commands.md#command-menu) owns the command menu.

## Display

Use the existing composer status and task strip in scrollback and fullscreen.
The selected branch determines the foreground status. The task strip and
`/tasks` retain their existing scope, labels, controls and terminal fade.
Permission and question interactions take precedence, retry retains its
countdown, and a disconnected or synchronizing view says “Status pending sync”
before making any claim about current execution.

| Confirmed activity | Composer text |
| --- | --- |
| Current model request before visible output | Waiting for model |
| Current request with text or thinking progress | Generating or Thinking, following the existing streaming distinction |
| Started foreground tool | Running tool, with its existing display label when available |
| Multiple active calls | Running N tools |
| Single active wait with a known scoped target | Waiting for task: CI checks |
| Active wait without a target | Waiting for a background task or new input |
| Active wait with a specified target whose details are unavailable | Waiting for task: details unavailable |
| Background tasks alone | Background running |
| Restored task inventory with an open turn and unresolved activity | Background running · wait relation unknown, when a task is visible |

A wait concurrent with other active calls belongs in the aggregate. The aggregate
does not claim the entire branch is blocked by one call. When no current phase is
confirmed and an open turn is known, use a neutral “Working” status. An idle line
keeps the existing idle treatment when its branch has no confirmed background work.
`LineState::Waiting` continues to mean an open line with no running turn.

Proposed scrollback with the default tool detail setting:

```text
> Fix the issue and verify
  [committed tool history]

  o CI checks                       2m13s
+-----------------------------------------+
| >                                       |
+-- Waiting for task: CI checks · 1m40s ---+
```

Proposed fullscreen, with the outer lines denoting the viewport:

```text
+---------------------------------------------+
| > Fix the issue and verify                  |
|   [tool history at current detail setting]  |
|                                             |
|   o CI checks                         2m13s |
| +-----------------------------------------+ |
| | >                                       | |
| +-- Waiting for task: CI checks · 1m40s ---+ |
+---------------------------------------------+
```

Here the task row's clock is task runtime and the composer clock is wait time.
At normal widths label them “Running”, “Waiting” or “This turn” in context.
At narrow widths retain the phase and object first, clip by terminal cell width,
then drop elapsed text if necessary. Reuse the existing border priorities and
keep the input area and history scrolling available. The markers and bracketed
history above are diagram placeholders.

```text
  o Dev server                       8m02s
+-----------------------------------------+
| >                                       |
+-- Generating · this turn 24s ------------+
```

When a wait settles, clear its label immediately and show the next confirmed
phase. Task success means the command reported success. Timeout settles the
wait while the task may keep running. Failure and cancellation follow the
task's own terminal fact. Show wait elapsed only with a known clock origin for
the observed tool invocation, and turn elapsed only with the authoritative
turn start. Omit an unavailable duration.

## Evidence and recovery

Baseline source review at `077892025020a2875a559b432dfa3256878422ff`:

- [Public client facts](clients.md) feed the current worker and projection.
  Use current-turn and request identity with ModelProvider progress. The output
  identity and counts in [session.turn.get](input-state-read.md#turn-identity-read)
  describe the latest open model output at the fixed cutoff.
- The [public event types](../../crates/dim-agent-types/src/v1.rs) expose
  `step/tool-call`, `step/tool-start`, `step/tool-completed` and
  `step/tool-result`. Join the
  dispatched tool name and arguments to its start by Session, branch, turn,
  request, dispatch event ID and call ID. A proposed call or arguments still
  streaming do not show execution. A tool start confirms execution began and
  supplies an observed invocation clock, with no guarantee of the exact
  WaitBranch admission time. Use per-call completion to retire a call even
  while its batch is still open. Batch settlement, turn end and attachment
  changes retire the corresponding claims as well. For an overlapping or
  incomplete trace, show only the activity the evidence supports.
- The [WorkspaceTools owner](../architecture/rust-agent/plugins/WorkspaceTools.md)
  publishes background handoff through `exec/started`, terminal state through
  `exec/ended`, and pending tasks through `session.exec.pending`. Join
  `wait(taskId)` only to an exact task identity in the same Session and branch.
  Follow the existing task projection's title/description fallback. The tools
  may be running before a background handoff exists. Exec handoff and tool-call
  completion can leave the task running independently. For other notice
  producers, use their public scoped task identity if it is available, otherwise
  use the unavailable-details text. A label, an exec origin call ID, or an open
  task row alone conveys no wait relationship.
- [Existing inventory reads](../../crates/dim-cli/src/session_host/inventory.rs)
  and the bounded transcript restore use owner cutoffs and later live facts.
  Reuse that handoff. Key any foreground claim to the selected attachment and
  branch. Reconcile in Timeline order, deduplicate by branch and sequence within
  the attachment, and ensure a terminal fact wins over a stale inventory or
  replayed start. Invalidate current claims during disconnect or branch change.
  On restore, display a named wait only when the bounded evidence and handoff
  establish the matching open invocation and target. Missing starts, missing
  settlement coverage, failed reads or a window gap leave the relationship
  unknown. A pending inventory establishes tasks, and the turn read establishes
  turn/output identity. Neither enumerates active waits.
- [Wait](../architecture/rust-agent/plugins/Wait.md) owns wait admission and
  settlement through the branch-local `wait` face. ToolExecution owns the tool
  lifecycle facts. A started tool can still settle immediately (for example,
  admission refusal or an already available notice). Treat its terminal fact
  as authoritative. Exact restored wait identity and elapsed time across clients
  would require those owners to agree on a public read contract covering call
  identity, target, admitted start, settlement, fixed cutoff, live handoff and
  cancellation/reload. A client reconnect and a wait generation reload have
  different semantics. The present WaitBranch rebuild restores open-turn status,
  and reload settles accepted calls. This display proposal consumes existing
  client data and introduces no service face, Timeline event or stored format.

Keep the [terminal signaling contract](plugins/Tui.md#terminal-channels) for
OSC 7501, OSC 9;4, title, notifications and cleanup. Permission and question
remain the user-blocked states. The composer strings do not select OSC states.
Update from existing notifications and reuse the local repaint cadence, with
elapsed text changing at most once per second. Model input, request counts,
tool calls and execution stay independent of rendering. UI teardown, a slow
read or a failed frame must leave tool timeout, cancellation and execution with
their owners. There is no history scan, timer RPC, remote CI poll or model
progress request added for the display.

## Verification plan for implementation

- Reuse the existing scripted tool and wait flows in
  [local_execution.rs](../../tests/integration/tests/local_execution.rs).
  Follow request → thinking/text → exec → handoff → wait → result → next
  request. Exercise success, failure, cancellation, wait timeout, steer and
  stop. Assert the wait clears on its own settlement and task state follows
  its owner. Compare model input, requests, tool calls and task outcomes
  before and after. Count RPCs during a quiet timer interval and delay an
  owner reply while checking that input and tool settlement still proceed.
- Use projection and frame coverage for foreground execution alongside a
  running dev server, parallel calls with one early completion, unscoped
  wait, unknown target, absent title or clock origin, approval, question and
  retry. Exercise colliding task IDs across Sessions/branches, cut and live
  handoff, duplicated and delayed facts, terminal before stale pending reply,
  and a wait start outside the window. Check OSC bytes through the existing
  terminal status checks independently of the composer text.
- With a disposable `DIMCODE_HOME` and deterministic provider, capture both
  terminal modes at 100, 60 and 32 columns using a synthetic command titled
  “CI checks”. Check default visibility without opening `/tasks`, history
  scroll and detail toggles, line changes, disconnect/reconnect, stop and
  resume. Keep this scenario eligible for the local tmux suite when
  [#1803](https://github.com/arcships/dim/issues/1803) lands. Its migration,
  CI selection and suite infrastructure belong to that issue. Before that
  suite is available, use the existing isolated local terminal harness.

The existing composer, live projection and task strip cover the proposed
display. Keeping separate call and task identities is necessary because their
lifetimes diverge at handoff. A second store, panel, periodic reader or activity
service adds no evidence for the conservative recovery case. The remaining
decision is whether a later delivery needs exact restored waits. The
conservative display can be reviewed and implemented independently of that
owner contract.
