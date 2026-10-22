# Workflows in the CLI task view

Status: **Proposal; not Ready or implemented**. This is the Workflow slice of
[#264](https://github.com/arcships/dim/issues/264). It extends the accepted
[command/member task list](background-task-list.md), implemented separately in
[#324](https://github.com/arcships/dim/pull/324). The optional
[Workflow package](../../runtime/plugins/workflow/README.md) is delivered by
[#306](https://github.com/arcships/dim/pull/306). Approve this view contract before
implementation; integrate on the command/member implementation when available.
This does not complete #263's other transcript/model-policy work.

## 1. Scope and existing interfaces

The same `/tasks` (`/background`) panel lists associated Workflow runs, opens
stored stage results/logs and stops individual runs or a finite captured batch.
The existing status row includes their observed running/stopping count. This
view neither edits definitions nor starts runs, installs packages or changes
Session configuration.

Use the package's delivered [management contract](plugins/Workflow.md#a3-management-methods):

| Existing interface | Use |
| --- | --- |
| `hello.features` | Discover the actual `workflow.status` and `workflow.stop` methods |
| `workflow.status {}` | Read the stored run array, including ownership, state, stages and logs |
| `workflow.stop {run_id}` | Ask Workflow to settle its run; the reply contains the resulting stored run |

The implementation is in the package's [management contribution](../../runtime/plugins/workflow/index.ts)
and [run owner](../../runtime/plugins/workflow/engine.ts). `status` without an ID
returns the retained run array; it is not a definition list, paged history or
server task registry. `workflow.list` lists definitions and is not used here.

Management is local and process-global. Select runs whose tool
`started_by: {kind:"tool", session_id, ...}` or `report_to.session_id` identifies
the displayed Session.
A management-started run with no matching report recipient is outside this view.
Match Session identity, not cwd, workflow name or branch; the same rule includes
runs from another branch of that Session. This filter defines presentation,
not a new authorization boundary or a claim that management methods are scoped.

Workflow stores its own process records. Its tool results and final report
inputs do not form a stream of every run/stage transition. Existing Session
subscriptions still serve commands and members; they cannot supply Workflow's
live run inventory. Read Workflow's existing management state rather than
adding an event, subscription, Timeline write or host interface.

## 2. Read lifetime and projection

When `workflow.status` is advertised, read once after the current Session is
attached, then one second after each successful reply while that attachment
remains current. This includes when the panel is closed, so the status-row count
can discover new management-started runs addressed to this Session. The count is
an observation and may lag by a read interval; there is no cross-owner snapshot
or real-time guarantee.

Keep at most one Workflow status read outstanding in the existing Session
worker, including across attachment replacements. An old read retains its slot
until reply or connection teardown; a replacement's initial read waits for that
slot. A slow reply does not start another read. Panel detail uses that same
projection and refresh; it does not create another timer, request or attachment.
The command/member history reader and ordinary leader controls remain independent.
Use the worker's existing response loop and private attachment identity.

Serialize Workflow refresh and its controls: an admitted read settles before a
queued Workflow stop is sent, and queued stops take precedence over the next
scheduled read. Resume refresh after those controls settle. This gives the view
an ordered observation without a new revision field, epoch or result fence;
leader input and other owners' notices continue through the response loop.

Decode the external reply once at the RPC boundary into the fields needed by
the view. Do not duplicate Workflow's persisted-file validator, definition
validation, authority checks or recovery logic. Reuse the existing timestamp
conversion for the owner's UTC timestamps. Adopt a valid complete reply in one
projection update; a malformed/error reply adopts no partial rows.

The current Session's existing projection owns the derived rows and stage/log
details. Use kind plus `run_id` as the Workflow row identity; do not use name,
stage Session or tool-call identity. Merge associated runs into the same task
collection, preserving command/member keys. Retain every observed unfinished
item and the latest 50 actual terminal outcomes across kinds, ordered by the
owner's end timestamp with the existing stable identity tie-break. Workflow's
own 100-run persistence retention is unchanged.

| Owner data | View meaning |
| --- | --- |
| `name`, `source`, `run_id` | Label, source and exact control/detail identity |
| `created_at`, `ended_at` | Elapsed time and actual terminal ordering; not RPC receipt time |
| `state: running / stopping` | Unfinished work; keep both in the background count |
| `state: completed / failed / stopped / interrupted` | Show the actual terminal state; no invented exit code |
| `stages[]` | Ordered stage IDs, step indexes, states, optional Session IDs and stored results/reasons |
| `log[]`, `report.state` | Stored diagnostics and actual report settlement, separate from run outcome |

Definition snapshots, parameter values and permission caps are not displayed or
copied into a second client store. A pending report does not reopen a terminal
run. No event is written merely because a run was read or displayed.

A complete owner snapshot determines the currently retained Workflow inventory.
A record no longer returned must not be relabeled completed. If selected detail
is no longer available, show that explicitly and return to the current list;
do not synthesize a terminal result or recover it from prose in report inputs.

## 3. Loading, errors and attachment changes

If `workflow.status` is absent, the panel continues its existing command/member
behavior without fake Workflow rows. A read-only integration may show runs when
status exists but stop does not; the action is unavailable in that case.

When status is advertised but still loading or failed, include Workflow in the
panel's existing incomplete-inventory guard for Ctrl+X. Known single-item stops
remain available. An absent optional method does not block command/member
stop-all. The next connection reevaluates its advertised methods; there is no
new hot-registration contract.

Before the first reply, show Workflow loading separately from branch-history
loading. A successful empty associated set is empty, not an error. On an RPC,
service-reloading or decode failure, retain the last validated rows as **last
observed** and show the error; do not present their count as a fresh total or
clear them to zero. Stop automatic refresh after failure. Reopening `/tasks`
retries once, then resumes the interval after success. If no snapshot ever
succeeded, show unavailable rather than an empty Workflow inventory. Existing
transport response limits still apply; do not add a summary endpoint or file
fallback to conceal a rejected read.

Session replacement, disconnect or worker shutdown invalidates this reader with
the existing attachment identity and cancels its next scheduled read. An admitted
old read settles through the existing response loop and cannot update the new
projection, even after returning to the same Session ID. A new attachment starts
with a new read. Closing only the panel does not stop observation or a run.

Errors, leader approvals/questions and other overlays use the existing modal
priority. Leader notifications, draft, selected model, permission display and
conversation position continue to belong to the leader. A view neither submits
input nor applies permission/model configuration to any stage Session.

## 4. Layout and interaction

```text
+-- Background tasks ---------------------------------------------+
| > nightly-check       workflow   Running      02:13              |
|   cargo build         command    Running      00:18              |
|   inspect imports     agent      Completed    00:42              |
| Workflow: last observed; refresh failed                          |
| Enter details  x stop selected  Ctrl+X stop all  Esc close        |
+-----------------------------------------------------------------+

+-- nightly-check - workflow -------------------------------------+
| Run: <run_id>              Source: user             Running      |
| Step  Stage        State         Session                         |
| 1     analyze      completed     <session_id>                    |
| 2     tests        running       <session_id>                    |
| 2     lint         waiting       -                               |
| analyze result: <stored result text>                             |
| Log: <stored log entries>                                        |
| Report: none                                                    |
| x stop workflow  Up/Down/PgUp/PgDn scroll  Esc back               |
+-----------------------------------------------------------------+
```

Enter on a Workflow opens its read-only stage/result/log detail, including ended
runs. Stage Session IDs remain labels, not editable Session navigation. The
existing scrolling, wrapping, terminal sanitization, theme and localization
apply. Follow new snapshots without resetting the selected run or scroll on
unrelated updates. Do not auto-scroll someone reading older content.

Use the existing panel width and minimum-screen behavior. At 48 columns, put
kind/state and identity on wrapped lines rather than losing the stop target;
body content scrolls above persistent key hints. Loading, empty and error text
must remain legible. Externally supplied names, results and logs cannot inject
terminal control sequences.

| Key | List | Workflow detail |
| --- | --- | --- |
| Enter | Open selected detail | No submission or stage navigation |
| x | Stop selected unfinished run when the list filter is empty | Stop this unfinished run |
| Ctrl+X | Existing finite stop-all action | Ignored, as in existing detail views; Esc returns to list |
| Esc | Close panel, preserving leader draft | Return to list; no stop or leader interrupt |
| Navigation keys | Existing list selection/filter behavior | Scroll stored stages/results/logs |

## 5. Stops and settlement

Single-run stop uses its exact `run_id` with `workflow.stop`. Workflow owns stage
withdrawal, interruption, joining and final state. Never replace that call with
stops of the listed stage Sessions: those would bypass the run owner. A terminal
row has no stop action. An unavailable method or reply is an explicit error,
not permission to fall back to another control.

Extend the existing finite stop-all batch with the known unfinished Workflow
run IDs when it is admitted. Keep commands addressed by branch/task and members
by delivered Session, with existing deduplication. Each Workflow run is included
once and sent serially with the other captured targets. New runs/stages are not
added to an admitted batch. Starting stages do not require a Session identity
because the run owner is already addressable.

A Session switch before admission refuses stale controls. After admission, the
batch keeps its original targets; it never retargets a run to the replacement
Session. Panel closure does not cancel accepted mutations. Track reply/error
settlement through the existing worker so a pending Workflow stop does not block
leader input, notices or independent reads. Do not retry an uncertain mutation
on disconnect; the next valid status read resolves what the owner stored.

Only the owner's returned/polled state determines the row outcome. A stop racing
completion may return a completed stage within a stopped run. A failed or missing
reply does not fabricate stopped. Preserve the package's accepted later-turn
stop and retained-input limits, and the task panel's existing statement that a
finite batch is not a fence against later reports, input or new work.

## 6. Implementation acceptance

Reuse the existing CLI PTY flows and the real optional-package App Server flow;
no new test harness, backend hook or mock call-order suite is needed.

- With the actual package installed, tool-started and management-started runs
  exercise both association fields. Include an unrelated run with the same cwd,
  a matching non-main branch and an unassociated management run. Restore the
  client and obtain inventory from status without depending on old tool results.
- Show sequential/parallel stage state and actual terminal results/logs. Verify
  elapsed timestamps, combined terminal retention/order, empty/loading/error,
  unavailable methods, refresh failure/reopen, sanitization and 48-column layout.
- Hold one status reply while the leader accepts input and notifications, then
  switch away and back to the same Session. The old reply cannot populate or
  retarget the new view; there is no second pending read from detail or a timer.
- Single stop and a mixed finite stop-all use the real run ID and existing other
  targets. Include completion during stop, a failed mutation, a run appearing
  after capture and a Session switch before/after admission. Confirm actual
  Workflow settlement; no direct stage stop is used by the CLI. A held refresh
  settles before its queued stop; refresh cannot overwrite the stop result with
  a preceding snapshot. Loading/error blocks all-task capture without disabling
  known single targets.
- Preserve the leader draft/model/permission and approval priority. Exit/reload
  releases only owned client resources; it neither cancels unrelated runs nor
  changes optional package installation, persisted format or native ownership.

These are future implementation gates. This documentation provides no passing
Workflow UI/runtime evidence and does not close #264.

## 7. Impact, rule check and ablation

Implementation is confined to the CLI's existing Session worker, projection/view
model, task panel, localization, related README and public flow tests. The
Workflow package, App Server, Rpc, core, rutis, storage formats and assembly gain
no interface or behavior. No new Capability or wiring plugin is needed.

### Requirements raised to mechanism owners

None. Existing process management reads and the run owner's stop operation are
sufficient. Session notices cannot stand in for process state; repeat the existing
read instead of adding a notification mechanism.

| Rule | Application |
| --- | --- |
| §4.1 commands/results | Existing typed owner method performs the stop and supplies its result; the client writes no result fact |
| §4.2 handover | Accepted run/input work is not revoked by closing the view or changing attachment |
| §4.3 before | No callback or Timeline write |
| §4.4 core neutrality | No Workflow case in Rpc, Transport, Timeline or Dispatcher |
| §4.5 batches | No cross-owner atomic snapshot or Timeline batch is claimed |
| §4.6 context | The UI never injects read data into model context |
| §4.7 public access | Only advertised management methods and returned owner records |
| §4.8 infrastructure | No mechanism requirement; existing client I/O and attachment identity suffice |
| §4.9 necessary facts | Derived display only; no event, index, persistence or subscription |
| §1 / ADR 002 D1 | Existing CLI owns the view; Workflow retains process-run state and work |
| ADR 002 D2 | No management adapter, registry, second connection or polling framework |
| ADR 002 D3 | Optional method discovery; absence affects only Workflow display/control |
| ADR 002 D4 | No setting or loader row; installation stays explicit |
| ADR 002 D5 | Existing named methods/JSON/errors/cancellation; no Rust-only public contract |

Ablation: one existing task collection and one worker-owned read cover both list
and detail. The read interval is needed to observe process-run transitions and
new associated runs without a notice stream; it adds no backend scheduler. A
single existing stop-batch queue owns finite controls. No run registry, detail
attachment, second persisted store, public event, lock or extra server method
serves this view.
