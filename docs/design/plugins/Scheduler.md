# Scheduler

Status: Draft. Overall design: [overview §5.8](../overview.md#58-scheduler). Phase: P2. Language: TypeScript first (overview §11.3).

Scheduler is a process plugin that keeps scheduled jobs and their run history in process storage, and, when a window is due while the daemon is alive, creates or reuses a non-interactive Session and submits the job prompt as a Queue input marked with its own source block. It contributes the process-level `schedule` tool (risk `persistent_configuration`) and the `schedule.*` management methods. It writes no Timeline event: a run's prompt is ordinary input, and its outcome is read from the run Session's fixed prefix.

## 1. User flows

1. The user runs `/scheduled` (alias `/schedule`) and saves a job: cron `0 9 * * 1-5`, prompt, working directory, resource node, model, mode `create`. At 09:00 Scheduler creates a non-interactive Session through Agents and submits the prompt; the history shows `running`, then `completed` with the Session id.
2. The model, in an interactive Session, calls `schedule` with `action: "create"`; Approval asks because the category is `persistent_configuration`; once allowed, the job is saved with the calling Session's cwd and resource node as defaults.
3. A job in mode `reuse` runs in the same Session each time; the user opens that Session to read accumulated results.
4. The laptop sleeps over three windows; on wake they are recorded as `missed` and nothing runs until the next window (feature list: no catch-up).
5. `schedule.run_now` runs a job immediately, subject to the same overlap rule.
6. A job's previous run is still running when its next window comes; the window is recorded `skipped: overlap`.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Job definitions, validation, schedule evaluation, timers | Session creation mechanics and non-interactive configuration (Agents) |
| Run records and history; overlap, busy, offline and missed-window policy | Executing the prompt (the run Session's Loop) |
| The `schedule` tool and `schedule.*` management methods | Approvals inside run Sessions (Approval) |
| Writing the prompt through session-access; reading run outcomes from the prefix | Notifications on run end (Notifications observes Sessions; A16) |
| — | Catch-up of missed windows (never) |

## 3. Scope and dependencies

Single process plugin `SchedulerPlugin`; no Session or branch part (overview §3.2).

**Contracts depended on** (none names Scheduler):

| Contract | Owner | Status | Use |
| --- | --- | --- | --- |
| `ProcessToolSource` | ToolRegistry ([process-tool-contribution](../contracts/process-tool-contribution.md)) | P0 contract | `schedule` tool |
| `SessionAccess` | Agents ([session-access](../contracts/session-access.md)) | P0 contract | Acquire reused Session, write prompt, lookup, read prefix, stop |
| Session launch with creation configuration (resource node, non-interactive, model, cwd), returning a `SessionLease` | Agents | Overview §7.2 Agents supplement; proposed, shape in §6.4 | `create` mode and first run of `reuse` |
| `SessionEvents` | [session-broadcast-subscription](../contracts/session-broadcast-subscription.md) | P0 contract | `turn/end` hints for run completion |
| Risk category `persistent_configuration` | Permission | Overview §7.1 Permission supplement | Admission of `schedule` |
| Management method contribution | Management | Overview §5.16 | `schedule.*` |
| Plugin configuration via typed Settings operations | Settings (#3) | Overview §7.2 | Scheduler settings (A13) |
| Process storage | Scheduler-owned file on the control node via FileStorage (A12) | Proposed | Jobs and runs |

**Contracts defined here**: the job and run data formats (§5), the `schedule` tool (§6.1), the `schedule.*` methods (§6.2), source marker `dim-scheduler` and submission ids `scheduler:<run id>` (session-access conventions).

| Part | Scope | Injects | Provides |
| --- | --- | --- | --- |
| `SchedulerPlugin` | Process (control node) | `SessionAccess`, Agents launch service, `SessionEvents`, `FileStorage`, Settings | `ProcessToolSource` under `scheduler`; management contribution `schedule` |

## 4. Events

None (overview §5.8). The prompt is an `input/message` written through session-access; its consumption (`input/applied`, `input/consumed`), the turn it drives (`turn/start`, `turn/end`) and any interrupt are existing facts on the run Session's branch. The tool call and its result are existing facts on the calling branch. Job and run state is process storage: it does not affect any branch's later behavior.

## 5. State

All in Scheduler's process storage, format version 1 (A12); nothing is rebuilt from Timeline.

| Record | Fields |
| --- | --- |
| Job | `id`, `name`, `schedule` (§6.3), `prompt` (text, ≤ 32 KiB), `cwd`, `resource_node` (`None` = control node), `model` (optional; `None` = Agents default), `agent_id` (optional profile), `mode` (`create` \| `reuse`), `session_id` (bound by `reuse` on first run), `enabled`, `max_run_minutes` (default 60, A7), `revision`, `created_by` (`user` \| `tool:<session_id>`) |
| Run | `run_id`, `job_id`, `window` (scheduled instant or `manual`), `state`, `session_id`, `branch_id` (`main`), `input_event_id`, `submission_id`, timestamps, `outcome` (`completed` \| `failed` \| `cancelled` with reason), `detail` |

Run states: `skipped` (overlap, capacity, disabled, node unavailable), `missed`, `launching` (decided, before the input write), `submitted` (input committed), `running` (input consumed), terminal `completed`/`failed`/`cancelled`/`timed_out`/`lost`. History keeps the last 100 runs per job (A8). Writes use storage-level atomic replacement and `revision` checks so `save` from the tool and from management cannot overwrite each other silently.

## 6. Interfaces

### 6.1 Tool `schedule`

Process-level contribution, key `scheduler`, `requires: { interactive: true, roles: Some({leader}) }` (A9): non-interactive run Sessions and collaboration members never see it, so a run cannot schedule further runs. Risk `persistent_configuration` for all actions (A10).

| Parameter | Type | Rule |
| --- | --- | --- |
| `action` | `list` \| `get` \| `create` \| `update` \| `delete` \| `set_enabled` \| `run_now` \| `history` | Required |
| `job_id` | string | All but `list`, `create` |
| `job` | object: `name`, `schedule`, `prompt`, `cwd?`, `model?`, `mode?`, `max_run_minutes?` | `create`, `update` (partial for update) |
| `enabled` | bool | `set_enabled` |
| `limit` | integer ≤ 50 | `history` |

`cwd` and `resource_node` default to the calling `ProcessToolSession`; the tool cannot set a resource node other than the caller's (A11). Result `content`: a short text summary with `job_id`, next window, or the requested list; `structured_content` holds the records. Validation errors return an error result.

### 6.2 Management methods (contributed to Management)

| Method | Parameters | Result |
| --- | --- | --- |
| `schedule.list` | `{}` | Jobs with `next_window`, last run |
| `schedule.save` | `{job, revision?}` | Saved job; stale `revision` → `schedule_conflict` |
| `schedule.delete` | `{job_id}` | Removes the job and history; an active run is not stopped (A14) |
| `schedule.set_enabled` | `{job_id, enabled}` | Pause/resume |
| `schedule.run_now` | `{job_id}` | `run_id` or the skip reason |
| `schedule.history` | `{job_id, limit?}` | Runs, newest first |

CLI: `/scheduled`, `/schedule` and a `dimcode schedule` subcommand call these methods.

### 6.3 Schedule formats

| Kind | Form | Rule |
| --- | --- | --- |
| `cron` | `{cron: "m h dom mon dow", tz?: IANA}` | Five fields, standard ranges, lists, ranges, steps; `tz` defaults to the control node's time zone; nonexistent local times (DST gap) are skipped, repeated times fire once |
| `every` | `{every_minutes: n}` | `n` ≥ 1; windows at `anchor + k·n`, anchor = time the job was last enabled |
| `at` | `{at: RFC 3339 instant}` | One window; after it fires or is missed, the job is disabled |

Granularity is one minute (A1). A window fires only if the timer observes it within 60 s of its instant (A2); otherwise it is recorded `missed` (one record per job collapsing all windows missed in a stretch) and skipped. The timer is evaluated against wall clock on every tick and on wake, so suspended time is never caught up.

### 6.4 Session launch (depended on; Agents supplement)

```rust
// Proposed by the Agents supplement (overview §7.2); Scheduler depends on this shape.
async fn launch(&self, config: LaunchConfig) -> Result<SessionLease, LaunchError>;
struct LaunchConfig { agent_id: Option<String>, cwd: String, resource_node: Option<String>,
    model: Option<ModelSelection>, interactive: bool /* false */ }
```

Permission policy of the run Session is the Agents default for non-interactive Sessions; Scheduler passes none (A15).

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Trigger | A timer task registered with `ctx.effect` computes the next window per enabled job. At a window: apply overlap, capacity and enablement rules (§8); persist the run as `launching` with a fresh `run_id` before any external effect |
| Session | `create`: `launch(...)` returns a lease. `reuse`: `SessionAccess::source("scheduler").acquire(session_id)` (restores if unloaded); on first run, `launch` and bind `session_id` to the job |
| Write | `lease.input("main", SourceInput { submission_id: "scheduler:<run_id>", marker: { kind: "scheduled-run", attrs: [job, run], body: job name }, blocks: [], user: Some(prompt text), mode: Queue })` → `Session::input` → `TimelineContext::input` → `Timeline::append`. Persist `submitted` with the committed event id |
| Consumption | InputQueue (Dispatcher subscription) appends `input/applied`; Loop appends `input/consumed { input_id }` and drives the turn. Scheduler holds no Dispatcher subscription |
| Completion | Scheduler subscribes to `turn/end` filtered to Sessions of active runs. On a notice, a `Gap`, or a once-per-minute sweep (A3), it reads `lease.prefix("main", at_least)` and decides from consumed facts only: the run is `running` once `input/consumed { input_id = input_event_id }` exists; it ends at the first `turn/end` after that consumption, with that turn's outcome |
| Timeout | When `max_run_minutes` elapses with the run unfinished: if consumed, `lease.stop("main", "scheduled run timed out")` (turn-scoped `stop/interrupt`) and the run ends `timed_out` once the `turn/end` is read; if not consumed, the run ends `timed_out` and the input stays queued (§14) |
| State | Process storage only; no `StateCell`. The Session's InputQueue and Loop state remain theirs |
| Recheck | Before writing, Scheduler re-reads the job and run from storage (job still enabled, revision unchanged, run still `launching`). It never reads the Session's capability state; whether the Session is busy is not consulted (§8) |
| Reload / restart | Timers and the subscription end in the disposer; leases drop. The new generation loads storage and settles: `launching` → acquire the Session if known and `lookup(submission_id)`: `Accepted` → `submitted` (resume tracking; reading is not a side effect), `Unknown` → `lost` ("did not survive recovery"; never resubmitted); `submitted`/`running` → resume tracking from the prefix. Windows that passed meanwhile are `missed` |

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Previous run of the job still active when a window fires | — (process storage) | Window `skipped: overlap`; no input (A4) | Scheduler |
| `run_now` while a run is active | — | Rejected with `skipped: overlap` | Scheduler |
| Reused Session busy (turn running or inputs pending) | prior facts < run input | Input is written anyway as Queue; consumed after the running turn ends; history notes the wait (A5) | InputQueue, Loop |
| Reused Session: user Steer/Queue/Append concurrently | Timeline order | Each applied by mode in sequence order; the run is tracked by its own `input_id` | InputQueue |
| User interrupts the run's turn | consumed < `stop/interrupt` < `turn/end` | Run `cancelled` (from `turn/end` outcome) | Loop; Scheduler records |
| Interrupt before the run input | interrupt < input | Input starts a turn normally | Loop |
| Input applied but not consumed, then interrupt | applied < interrupt | Stays pending, consumed by the next turn; run still `submitted` | Loop |
| Branch close or Session closed/deleted before the write | — | `SessionClosed`/`BranchMissing`; run `failed: session_missing`; `reuse` job keeps the binding (A6) | Scheduler |
| Session closed while the run is active | — | Prefix read returns `BranchMissing`/`SessionClosed`; run `failed` | Scheduler |
| Tool batch in the run turn not settled | — | Nothing observed until `turn/end` | Loop |
| Compaction during the run | — | No effect on tracking: `input/consumed` and `turn/end` stay in the local history read by `prefix` | — |
| Child branch created in the run Session | — | Ignored; runs use branch `main` only | Scheduler |
| Run Session idle-unloaded with the input pending | — | Committed input restored and adopted on load; Scheduler's `acquire` for tracking restores it | AgentIndex |
| `turn/end` notice lost (`Gap`, reload) | — | Minute sweep reads the prefix | Scheduler |
| Scheduler reload or daemon restart mid-run | — | §7 settlement; never resubmitted | Scheduler |
| Daemon down over a window | — | `missed`; no catch-up (feature list, overview §5.8) | Scheduler |
| Resource node offline at trigger, `create` | — | `launch` fails at the startup deadline → `skipped: node_unavailable`; not retried (A6) | Agents, Scheduler |
| Resource node offline, `reuse` | — | Input commits; capabilities Pending; turn starts after recovery; `max_run_minutes` applies | rutis lifecycle, Scheduler |
| Pending approval in a run Session | `approval/request` | Waits for `approval.respond` from any client or until timeout | Approval; Scheduler (timeout) |
| Duplicate/late `schedule.save` | revision | Stale revision rejected | Scheduler |
| Duplicate `run_now` | — | Second call sees the active run: skipped | Scheduler |
| `schedule` tool call interrupted after save | save < interrupt | Saved job stays (persistent configuration is the effect); tool result cancelled | ToolExecutor; Scheduler |
| Client detach | — | No effect | — |

## 9. Context mapping

| Where | What the model sees | Form | Placement and survival |
| --- | --- | --- | --- |
| Calling Session | `schedule` declaration | Tool declaration in adopted configuration (interactive leaders only) | Configuration; survives compaction |
| Calling Session | Result of `schedule` | `ToolResult.content` | Batch result position; summary only after compaction |
| Run Session | Marker `<dim-scheduler kind="scheduled-run" job="..." run="...">job name</dim-scheduler>`, then the prompt as user content | Raw block + user text of one Queue input | At its `input/consumed` position; unkeyed, so only via summary after compaction; child branches inherit it if consumed before the cut |

Run Sessions are non-interactive: no `ask_user`, no `schedule` tool. Prior runs in a `reuse` Session remain history like any input. The model never sees job records, run history, timers, other runs, or diagnostics.

## 10. Errors and recovery

| Case | Result |
| --- | --- |
| Invalid job (schedule, prompt size, unknown node) | `schedule_invalid` (management) or error tool result |
| Launch failure | Run `failed` with reason; window not retried |
| Write `Reloading` | `lookup` then at most one retry within the window's 60 s grace |
| `PublicationUnknown` | Run stays `launching`; settled by `lookup` after recovery |
| Storage unreadable | Scheduler fails to apply (no silent reset) |
| Process exit | Timers stop; no settlement written until the next start (§7) |

Error codes: `schedule_invalid`, `schedule_conflict`, `schedule_not_found`, `service_reloading`.

## 11. Security

`schedule` is admitted by Approval and Permission as `persistent_configuration` (whether FullAccess allows it is overview §13). A run writes input only; its tools are admitted under the run Session's policy, and approvals are not pre-granted. A job created by a tool is marked `created_by` for review. Jobs store no credentials. The run Session uses only the resources of its configured resource node through Session-bound services; Scheduler itself touches none. Job prompts can be read by anyone with Management access.

## 12. Acceptance

Through management methods, the `schedule` tool and events, with a controllable clock:

1. Each schedule kind computes the expected windows, including DST gap/overlap and `at` disabling itself.
2. A due `create` job yields a non-interactive Session whose first input carries the marker and the prompt; the run ends `completed`.
3. Overlap: a window during an active run is `skipped: overlap`.
4. `reuse` with a busy Session: input queued behind the running turn; run starts after it.
5. User interrupt yields `cancelled`; timeout yields `timed_out` with a `stop/interrupt`.
6. Clock jump over windows yields one `missed` record and no runs.
7. Restart with a `launching` run: `Accepted` resumes tracking, `Unknown` becomes `lost`; no duplicate input.
8. `schedule` absent in non-interactive Sessions; asked under the default policy.
9. Node unavailable at trigger: `skipped: node_unavailable`.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Formats `cron`, `every`, `at`; one-minute granularity | Feature list ("recurrence"); judgment | Formats change |
| A2 | 60 s grace for a window to count as on time | Judgment | Tighter or looser missed detection |
| A3 | Once-per-minute prefix sweep for active runs | Subscription is lossy (contract); judgment | Missed completions until next notice |
| A4 | Overlap policy: skip the new window | Judgment ("missed windows skipped") | Queue or parallel runs |
| A5 | Reused busy Session: enqueue rather than skip | Judgment | Busy windows skipped |
| A6 | Failures (session missing, node unavailable) never retry or fall back to `create` | Judgment; overview §3 constraint 4 | Fallback policy |
| A7 | `max_run_minutes` default 60 | Judgment | Long runs unbounded |
| A8 | Keep the last 100 runs per job | Judgment | Retention setting |
| A9 | Tool only for interactive leader Sessions | Overview §5.1 non-interactive rule; judgment | Runs could create jobs |
| A10 | One risk category for all actions, including reads | Overview §5.8 | Split read-only tool |
| A11 | Tool-created jobs use the caller's resource node | Judgment (overview §3.1) | Tool may target other nodes |
| A12 | Scheduler-owned versioned storage file via FileStorage on the control node | Overview §5.8 "process storage"; no generic process store exists | Storage service changes |
| A13 | Settings hold only plugin settings (enablement, max concurrent runs = 4), not jobs | Judgment | Jobs in Settings |
| A14 | Deleting a job does not stop its active run | Judgment | Delete stops the run |
| A15 | Run permission policy is Agents' non-interactive default | Overview §7.2 Agents; judgment | Per-job policy field |
| A16 | Run-end notifications are Notifications' business, from its own subscription | Overview §5.11 | Scheduler would call Notifications |

## 14. Open questions

| Question | Description |
| --- | --- |
| Liveness | Overview §13: feature list says interactive terminal alive; this design requires the daemon alive |
| Unconsumed input on timeout | Withdrawing it needs InputQueue's `input/withdrawn` service exposed through session-access |
| Launch contract | The Agents supplement must define `launch` (non-interactive, resource node) |
| FullAccess | Whether `persistent_configuration` asks under FullAccess (overview §13) |
