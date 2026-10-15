# Scheduler

Status: Draft. Overall design: [overview §4](../overview.md#4-plugin-catalog); scope: [ADR 002](../../adr/002-plugin-scope.md) (process + Session). Phase: P2. Language: TypeScript first ([overview Appendix C](../overview.md#appendix-c-implementation-language)).

> **To refine in design review.** Notifying a run's outcome is Scheduler's own feature. The run-outcome event notice for Notifications and the cross-language named process events it requires (§9.1) are withdrawn with Notifications.

Scheduler keeps scheduled jobs, which are user-written configuration, in its Settings business source `schedule`, and their run history in its process storage, [`pluginFiles` (extended)](../../../crates/dim-app-server/README.md#trusted-plugin-storage). While the daemon is alive and a window is due, the process part creates a non-interactive Session through `sessions.launch` (or reuses one) and writes the job prompt as a Queue input with its own source marker through `sessions.input` ([session-access](../contracts/session-access.md)), as Collaboration reaches its members. The Session part offers the `schedule` tool (risk `persistent_configuration`) to interactive leader Sessions as a `toolContribution`. Operators use the `schedule.*` management methods, a `managementMethods` entry. All contracts are language-neutral named services ([ADR 002 D5](../../adr/002-plugin-scope.md#d5-contracts-are-language-neutral)); the Rust forms are in [Appendix G](#appendix-g-rust-binding). Scheduler writes no Timeline event of its own: a run's outcome is read from the run Session's history and published as a run-outcome event notice.

## 1. Structure

```mermaid
flowchart LR
  subgraph PROC["Process (control node)"]
    Sched["Scheduler: timer, run tracker, job store"]
    MM["managementMethods key schedule"]
    Out["Run-outcome event scheduled_run_outcomes"]
    Agents["Agents: sessions (launch, input, history, idle, stop, withdraw)"]
    Rpc["Rpc method table"]
    Store["Process storage: pluginFiles (extended), run history"]
    Set["Settings: business source schedule (jobs)"]
    Listen["Listeners: Notifications instance of the run Session (no dependency)"]
  end
  subgraph SESS["Interactive leader Session"]
    Tool["SchedulerSession: toolContribution key schedule"]
  end
  subgraph RUN["Run Session (non-interactive)"]
    TL["main Timeline: input/message, input/consumed, turn/end"]
  end
  Tool -->|"enclosing scope"| Sched
  Sched --> Agents
  Sched --> Store
  Sched --> Set
  Sched --> MM
  Rpc -->|"collects managementMethods by name"| MM
  Agents -->|"sessions calls"| TL
  Sched --> Out
  Listen -.-> Out
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
  class Sched,MM,Out,Tool new
```

Jobs and windows span Sessions, so they belong to the process ([ADR 002](../../adr/002-plugin-scope.md) D1); offering a tool to a Session is the Session part's job (D2). The Session part depends on the process part (enclosing scope). The process part depends on Agents (service `sessions`), Settings (business source `schedule`) and `pluginFiles` (extended); it reaches run Sessions only through `sessions`. Dependencies follow scope (D3): an Agents reload reloads Scheduler, and a Scheduler reload reloads its Session parts, failing their in-flight turns; both are rare structural events and an accepted, documented cost. Job edits are business configuration (D4): written through Settings' typed write operation, read at use time on change notification, never a reload.

| Part | Scope | Depends on | Provides |
| --- | --- | --- | --- |
| `Scheduler` | Process (control node) | `sessions` (Agents; `launch` with `policy`), Settings business source `schedule`, `pluginFiles` (extended) | Job access, timer, run tracker; `scheduleJobs` (the tool's access to the job store, A.2); `managementMethods` key `schedule`; process event `scheduled_run_outcomes` |
| `SchedulerSession` | Session, installed when Scheduler is enabled | `scheduleJobs` (`Scheduler`), Session configuration | `toolContribution` key `schedule`, declared only when `interactive` and not a Collaboration member |

**Contracts depended on**: `sessions` ([session-access](../contracts/session-access.md): `launch` with a starter-capped `policy`, Session configuration fields `interactive`/`resourceNode`/`createdBy`, `input` with source raw blocks, fixed-prefix `history`, `idle`, `stop`, `withdraw`; `agent` optional), `toolContribution` (ToolRegistry accepts named tool services, a supplement; [tool contributions](../../architecture/rust-agent/tool-contributions.md)), `managementMethods` collected by Rpc ([session-boundary](../contracts/session-boundary.md)), Permission category `persistent_configuration` (FullAccess allows it, E1), [`pluginFiles` (extended)](../../../crates/dim-app-server/README.md#trusted-plugin-storage) (run history; binary read and write and a format version are added to the existing trusted plugin storage; a TypeScript part uses it natively, a Rust part through the same named service), the Settings business source `schedule` with typed writes under the schema Scheduler declares (Settings supplement, [overview Appendix B](../overview.md#appendix-b-supplements-to-existing-plugins)), the [single-tree event contract](../../architecture/rust-agent/single-tree.md). **Contracts defined here**: job and run formats, the `schedule` tool, the `scheduleJobs` service, the `schedule.*` methods, the run-outcome event (A.6), source name `scheduler` (marker `dim-scheduler`, submission ids `scheduler:<run_id>`).

## 2. Data and state

```mermaid
classDiagram
  class Job {
    id, name, schedule, prompt
    cwd, resource_node, model, agent_id
    mode create or reuse, session_id
    policy, policy_cap
    enabled, max_run_minutes, revision, created_by
  }
  class Run {
    run_id, job_id, window, state
    session_id, input_event_id, submission_id
    outcome, detail, timestamps
  }
  class SettingsSource {
    business source schedule
    typed writes, revision
  }
  class ProcessStorage {
    scheduler root, format version 1
    atomic replace, lock
  }
  class RunSessionTimeline {
    input/message with dim-scheduler marker
    input/applied, input/consumed, input/deleted
    turn/start, turn/end, stop/interrupt
  }
  Job "1" --> "0..100" Run
  SettingsSource *-- Job
  ProcessStorage *-- Run
  Run ..> RunSessionTimeline : decided from history
```

Jobs live in the Settings business source and runs only in process storage; nothing is rebuilt from Timeline and Scheduler owns no `StateCell`. A run's progress is derived from existing facts on the run Session's `main` branch. The Session part holds no state.

## 3. Flows

### 3.1 A due window

```mermaid
sequenceDiagram
  participant T as Scheduler timer
  participant ST as Process storage
  participant AG as Agents
  participant S as Run Session (main)
  participant IQ as InputQueue and Loop
  T->>ST: recheck job (Settings), persist run launching
  alt mode create, or first reuse run
    T->>AG: sessions.launch(agent?, cwd, resourceNode, model?, policy)
    AG-->>T: sessionId
  end
  T->>S: sessions.input(sessionId, main, queue, marker scheduled-run, prompt)
  T->>ST: persist submitted with input_event_id
  S->>IQ: Dispatcher: input/applied, input/consumed, turn
```

The run is persisted `launching` before any external effect, so a restart settles it by lookup instead of resubmitting.

### 3.2 Completion and timeout

```mermaid
sequenceDiagram
  participant R as Run tracker
  participant S as Run Session (main)
  participant ST as Process storage
  R->>S: sessions.idle(main) returns cutoff C
  R->>S: sessions.history up to C
  R->>R: consumed? first turn/end after consumption?
  R->>ST: running or terminal outcome
  R-->>R: terminal: emit RunOutcome notice
  Note over R: max_run_minutes elapsed
  alt input not consumed
    R->>S: sessions.withdraw own input
    R->>ST: deleted gives timed_out, notHeld waits for input/consumed then stops
  else consumed
    R->>S: sessions.stop(main, scheduled run timed out)
    R->>ST: timed_out once turn/end is read
  end
```

Each active run has a task that waits on `sessions.idle` and reads history up to the returned cutoff, as Collaboration and Workflow do; Scheduler does not observe Session broadcasts. If the withdrawal replies `notHeld` (InputQueue already applied the input, so it belongs to Loop), the tracker waits for `input/consumed` and then uses `stop`. A notice is emitted once, after the terminal record is persisted.

### 3.3 Permission of a run

```mermaid
flowchart LR
  Created{"created_by"}
  Tool["tool: cap = policy of the creating Session at save"]
  User["management: cap = host configuration maximum"]
  Default1["default = cap"]
  Default2["default = read-only"]
  Declared["job policy (optional, narrowing)"]
  Eff["effective = min(declared or default, cap)"]
  Created --> Tool --> Default1 --> Eff
  Created --> User --> Default2 --> Eff
  Declared --> Eff
```

Automation never widens authority (E2): the effective policy goes into `sessions.launch`'s `policy`, which Agents caps by the starter (the job's `policy_cap`). A reused Session keeps its own policy; if it exceeds the job's effective policy the run is `skipped: policy`.

## 4. Timing and conflicts

```mermaid
sequenceDiagram
  participant S as Scheduler
  participant U as User
  participant IQ as InputQueue
  S->>IQ: run input (Queue) at seq n
  U->>IQ: session.input.withdraw writes input/withdraw at seq n+1
  IQ->>IQ: input still held: input/deleted
  S->>S: next history read: deleted, never applied
  S->>S: run cancelled (withdrawn)
```

Every pending-input decision excludes deleted inputs, so a deleted run input never leaves the run `submitted` forever.

| Scenario | Sequence order | Outcome | Owner |
| --- | --- | --- | --- |
| Previous run still active when a window fires | — (storage) | `skipped: overlap` (A4) | Scheduler |
| User interrupts the run's turn | consumed < `stop/interrupt` < `turn/end` | Run `cancelled` | Loop; Scheduler records |
| Timeout withdraw after apply | `input/applied` < `input/withdraw` | `notHeld`, nothing written; `stop` after `input/consumed` | InputQueue, Scheduler |
| Agents or Scheduler reload, restart | — | `launching` → lookup: accepted → `submitted`, unknown → `lost`; never resubmitted; tracking resumes | Scheduler |
| Daemon down over a window | — | `missed`; no catch-up (E5) | Scheduler |

Full table: Appendix B.

## 5. Context mapping

| Where | What the model sees | Form | Placement and survival |
| --- | --- | --- | --- |
| Calling Session | `schedule` declaration | Tool declaration in adopted configuration (interactive leaders only) | Configuration; survives compaction |
| Calling Session | `schedule` result | `ToolResult.content` | Batch result position; summary only after compaction |
| Run Session | `<dim-scheduler kind="scheduled-run" job="..." run="...">job name</dim-scheduler>`, then the prompt as user content | Raw marker block + user text of one Queue input | At its `input/consumed` position; unkeyed, so only via summary after compaction; child branches inherit it if consumed before the cut |

Run Sessions are non-interactive: no `ask_user`, no `schedule` tool. Prior runs in a `reuse` Session remain history like any input. The model never sees job records, run history, timers or diagnostics.

## 6. Interfaces (summary)

| Interface | Kind | Purpose |
| --- | --- | --- |
| `schedule` | `toolContribution` (Session, key `schedule`), risk `persistent_configuration` | Model manages jobs |
| `schedule.list/save/delete/set_enabled/run_now/history` | `managementMethods` (process, key `schedule`) | Operators manage jobs |
| `scheduled_run_outcomes` | Process event defined here (A.6) | Live notice per ended run (Notifications listens) |
| `/scheduled`, `/schedule`, `dimcode schedule` | CLI | Call the management methods |
| `sessions` | Depended on (process) | Non-interactive run Session with node, model and starter-capped `policy`; input, history, idle, stop, withdraw |

## 7. Security

`schedule` is admitted by Approval and Permission as `persistent_configuration`; FullAccess allows it (E1), other presets ask or deny. A run only writes input; its tools are admitted under the run Session's policy, capped as in 3.3, and approvals are not pre-granted. Tool-created jobs record `created_by` and are visible for review. Jobs store no credentials. The run Session uses only its own resource node's services; Scheduler touches none. Anyone with management access can read job prompts.

## 8. Rule check

Rules: [principles §1 and §4](../../architecture/rust-agent/principles.md), [ADR 002](../../adr/002-plugin-scope.md) D1–D5.

| Rule | How this document complies |
| --- | --- |
| §4.1 Command events for others' state | Scheduler changes run Sessions only through `sessions` (input, `withdraw` writing `input/withdraw`, `stop` writing `stop/interrupt`); InputQueue and Loop decide and write the results. |
| §4.2 No take-back after hand-off | Before `input/applied` the run input is withdrawn; after it (`notHeld`) Scheduler only uses `stop` once `input/consumed` is read (§3.2). |
| §4.3 Intervene only through before callbacks | Not applicable: Scheduler adds no entry check and no before callback; job validation is the owner's own boundary validation of its methods and tool. |
| §4.4 Core mechanisms carry no business meaning | Scheduler needs no runtime change carrying its meaning; the run-outcome event is its own process event. |
| §4.5 Read the whole batch before deciding | Run decisions read history through the cutoff `C` that `sessions.idle` returns and use only events up to `C`; batches commit whole, so no batch is split at `C` (A.5). |
| §4.6 Model context holds only handed-over events | The run prompt reaches the model only as a consumed Queue input; a withdrawn input is deleted and never seen. |
| §4.7 Only public events and services | Scheduler uses `sessions`, Settings' typed operations, `pluginFiles` (extended) and its own `scheduleJobs`; Notifications listens to its public event without a dependency. |
| §4.8 Requirements to mechanism owners | Named process events across languages are raised in §9; the offline-node behaviour relies on #5 ([ExternalAgent §9](ExternalAgent.md#9-requirements-raised-to-mechanism-owners)); the Agents and Settings supplements are in [overview Appendix B](../overview.md#appendix-b-supplements-to-existing-plugins). |
| §4.9 Only facts later decisions use enter the Timeline | Scheduler writes no Timeline event; jobs stay in Settings, runs in process storage and the run outcome is a process event notice. |
| §1 Capability and App Server boundary | Jobs and runs are process-level, not a Capability; storage is the `pluginFiles` plugin, not an App Server service. |
| ADR 002 D1 | Jobs span Sessions (process part); the tool belongs to a Session (Session part). |
| ADR 002 D2 | No wiring module: Rpc collects `managementMethods`, ToolRegistry collects `toolContribution`, listeners subscribe to the event by name. |
| ADR 002 D3 | Agents and Scheduler reloads propagate by scope and are accepted and documented (§1). |
| ADR 002 D4 | Jobs and the run limit are business data in the Settings business source `schedule`, read at use time; nothing structural changes. |
| ADR 002 D5 | `scheduleJobs`, the tool, the methods and the event are named services with data shapes; Rust forms are in Appendix G. |

## 9. Requirements raised to mechanism owners

### 9.1 Named process events across languages

| Item | Content |
| --- | --- |
| Blocked by | ADR 002 D5 and principles §4.7: the run-outcome event must be public and language-neutral, but a process event today is a typed Rust `EventKey<T>`; a TypeScript Scheduler cannot emit it and a TypeScript Notifications cannot listen to it by name. |
| Existing mechanisms considered | A Timeline event: rejected, the outcome is process-level and has no branch to live on (§4.1, time-bus). A named service called by Scheduler: rejected, it makes Scheduler depend on its listeners (D2). rutis event forwarding between nodes: it forwards typed events and does not define a name-and-data form. |
| Generic capability | Emit and listen to a process event identified by a name with serializable data, from any language, with the same live, non-resumable delivery as typed process events; no business meaning. |
| Sequence, batch, interrupt, reload | No Timeline sequence or batch is involved; delivery is live; a listener unloaded or reloading misses events, the emitter is unaffected; no reload follows from listening. |
| Owner and issue | rutis (event bus and rutis-bridge runtimes); no issue yet, to be filed with [#4](https://github.com/arcships/dim/issues/4) as the related exposure work. |

## Appendix A. Interface details

### A.1 Records

| Record | Fields |
| --- | --- |
| Job | `id`, `name`, `schedule` (A.4), `prompt` (text ≤ 32 KiB), `cwd`, `resource_node` (`None` = control node), `model` (`None` = the profile's model), `agent_id` (optional; `None` = Agents' default profile), `mode` (`create` \| `reuse`), `session_id` (bound by `reuse` on first run), `policy` (optional preset, narrowing only), `policy_cap` (captured at save), `enabled`, `max_run_minutes` (default 60, A7), `revision`, `created_by` (`user` \| `tool:<session_id>`) |
| Run | `run_id`, `job_id`, `window` (instant or `manual`), `state`, `session_id`, `input_event_id`, `submission_id`, timestamps, `outcome` (`completed` \| `failed` \| `cancelled` with reason), `detail` |

Run states: `skipped` (overlap, capacity, disabled, node unavailable, policy), `missed`, `launching`, `submitted`, `running`, terminal `completed`/`failed`/`cancelled`/`timed_out`/`lost`. History keeps the last 100 runs per job (A8). Job writes go through Settings' typed write operation with `revision` checks, so tool and management saves never overwrite each other silently; run records use atomic replace under the storage lock.

### A.2 Tool `schedule`

`toolContribution` key `schedule`, registered by `SchedulerSession` only in Sessions whose configuration has `interactive: true` and no member role (A9), so run Sessions and members never see it and a run cannot schedule further runs. `declaration(): sync -> ToolDeclaration` returns the parameters below with risk `persistent_configuration` for all actions (A10); `execute(ToolCall): async -> ToolResult` calls the process part's `scheduleJobs` service. Cancellation is the consumer dropping the call.

`scheduleJobs`, process scope, no key, defined here: `call(JobRequest): async -> JobReply`, where `JobRequest = {action, job_id?, job?, enabled?, limit?, caller: {sessionId, cwd, resourceNode}}` uses the parameters below and `JobReply = {content: string, records: JSON}` or an error `{code}` from Appendix C. Cancellation is the consumer dropping the call; a save already persisted stays.

| Parameter | Type | Rule |
| --- | --- | --- |
| `action` | `list` \| `get` \| `create` \| `update` \| `delete` \| `set_enabled` \| `run_now` \| `history` | Required |
| `job_id` | string | All but `list`, `create` |
| `job` | object: `name`, `schedule`, `prompt`, `cwd?`, `model?`, `mode?`, `policy?`, `max_run_minutes?` | `create`, `update` (partial) |
| `enabled` | bool | `set_enabled` |
| `limit` | integer ≤ 50 | `history` |

`cwd` and `resource_node` default to the calling Session's configuration; the tool cannot target another resource node (A11). On `create`/`update`, `policy_cap` is the calling Session's current policy, resolved by Agents (A15). Result `content`: a short text with `job_id`, next window, or the list; `structured_content` holds records. Validation errors are error results.

### A.3 Management methods

`managementMethods` key `schedule`: `describe(): async -> MethodDecl[]` lists the methods below (`mutating` for all but `list` and `history`); `call(MethodCall): async -> MethodReply` dispatches them. Parameters and results are JSON objects; errors are codes from Appendix C.

| Method | Parameters | Result |
| --- | --- | --- |
| `schedule.list` | `{}` | Jobs with `next_window`, last run |
| `schedule.save` | `{job, revision?}` | Saved job (`policy_cap` = host configuration maximum); stale `revision` → `schedule_conflict` |
| `schedule.delete` | `{job_id}` | Removes the job (Settings) and its history (`pluginFiles`); an active run is not stopped (A14) |
| `schedule.set_enabled` | `{job_id, enabled}` | Pause/resume |
| `schedule.run_now` | `{job_id}` | `run_id` or the skip reason |
| `schedule.history` | `{job_id, limit?}` | Runs, newest first |

While Scheduler reloads, Rpc answers `service_reloading`; when it is not enabled, `unknown_method`.

### A.4 Schedule formats

| Kind | Form | Rule |
| --- | --- | --- |
| `cron` | `{cron: "m h dom mon dow", tz?: IANA}` | Five fields with lists, ranges, steps; `tz` defaults to the control node's zone; DST-gap times skipped, repeated times fire once |
| `every` | `{every_minutes: n}` | `n` ≥ 1; windows at `anchor + k·n`, anchor = last enable time |
| `at` | `{at: RFC 3339 instant}` | One window; after it fires or is missed the job is disabled |

Granularity one minute (A1). A window fires only if observed within 60 s of its instant (A2); otherwise one `missed` record collapses the stretch. The timer evaluates wall clock on every tick and on wake, so suspended time is never caught up.

### A.5 Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Trigger | Timer task computes the next window per enabled job; at a window applies overlap, capacity (A13), enablement and policy rules; persists `launching` with a fresh `run_id` |
| Session | `create`: `sessions.launch {agent?, cwd, resourceNode?, model?, policy?, starter?}` → `{sessionId}`; `agent` and `model` are passed only when the job sets them: absent `agent` = Agents' default profile, a request `model` overrides the profile's model; Scheduler resolves neither; Agents records `createdBy = {kind: "launch", source: "scheduler"}` from the declared source. For a tool-created job `starter` is the creating Session (A15), else absent. `reuse`: the bound `session_id` (an unloaded Session is restored by the call); the first run launches and binds `session_id` |
| Write | `sessions.input {sessionId, branchId: "main", mode: "queue", submissionId: "scheduler:<run_id>", rawBlocks: [marker {kind: "scheduled-run", attrs: [job, run], body: job name}], content: [prompt text]}` → `{inputId}` stored as `input_event_id`; persist `submitted` |
| Consumption | InputQueue appends `input/applied`; Loop appends `input/consumed { input_id }` and drives the turn |
| Completion | Tracker task: `sessions.idle {sessionId, branchId: "main"}` → cutoff `C`; `sessions.history {through: C}` pages; `running` once `input/consumed { input_id = input_event_id }` exists; ends at the first `turn/end` after it; `input/deleted` → `cancelled: withdrawn` (unless the tracker's own timeout withdraw wrote it: `timed_out`) |
| Timeout | Not consumed: `sessions.withdraw` of the own input; `deleted` → `timed_out`, `notHeld` → wait for `input/consumed`, then as consumed; consumed: `sessions.stop {sessionId, branchId: "main"}` (turn scope), `timed_out` when `turn/end` is read |
| Recheck | Before writing, re-read the job from Settings and the run from storage (enabled, revision unchanged, run still `launching`) |
| Reload or restart | Session parts reload with Scheduler; an in-flight `schedule` call fails with its turn. The new generation settles: `launching` → `sessions.lookup` by `submissionId`: accepted → `submitted`, unknown → `lost` (never resubmitted); `submitted`/`running` resume tracking; passed windows `missed` |
| Run outcome | After persisting a terminal state, publish `scheduled_run_outcomes` with the A.6 data; nothing is replayed after reload or restart |

### A.6 Run-outcome event (public contract)

Process event named `scheduled_run_outcomes`; listeners subscribe by name and form no dependency on Scheduler. Emitting and listening by name from a TypeScript or Rust part is the requirement in §9. The listener that reports a run is the Notifications instance of the run Session (`session_id`), which sends `schedule.ended` ([Notifications](Notifications.md)). Data:

| Field | Type |
| --- | --- |
| `job_id`, `job_name`, `run_id` | string |
| `session_id` | string, optional |
| `state` | `completed` \| `failed` \| `cancelled` \| `timed_out` \| `lost` |
| `reason` | string, optional; fixed reason text, never Session content |
| `ended_at` | RFC 3339 instant |

Emitted once per run after its terminal record is persisted (A16, A17). Delivery is live and non-resumable; a listener that is not loaded misses the notice. In particular, when the run Session is unloaded at the time of the notice, no `schedule.ended` is sent and the notice is lost; this is accepted, and `schedule.history` is the durable record (A18). The notice carries no prompt or Session content.

## Appendix B. Full conflict table

| Scenario | Sequence order | Outcome | Owner |
| --- | --- | --- | --- |
| Previous run active when a window fires | — | `skipped: overlap`; no input (A4) | Scheduler |
| `run_now` while a run is active | — | Rejected `skipped: overlap` | Scheduler |
| Reused Session busy | prior facts < run input | Written as Queue; consumed after the running turn (A5) | InputQueue, Loop |
| Reused Session: concurrent user Steer/Queue/Append | Timeline order | Applied by mode in sequence order; run tracked by its own `input_id` | InputQueue |
| User interrupts the run's turn | consumed < interrupt < `turn/end` | `cancelled` | Loop |
| Interrupt before the run input | interrupt < input | Input starts a turn | Loop |
| Input applied, interrupt before consumption | applied < interrupt | Stays pending; consumed by the next turn; run `submitted` | Loop |
| User withdraws the run input while held | input < `input/withdraw`, no `input/applied` | `input/deleted`; run `cancelled: withdrawn` | InputQueue, Scheduler |
| User withdraws after apply | `input/applied` < `input/withdraw` | `notHeld`; run continues | InputQueue |
| Timeout withdraw races apply | `input/applied` < `input/withdraw` | `notHeld`; `stop` after `input/consumed`, then `timed_out` | InputQueue, Scheduler |
| Branch close or Session closed before the write | — | `failed: session_missing`; reuse binding kept (A6) | Scheduler |
| Session closed while the run is active | — | History read fails; `failed` | Scheduler |
| Compaction during the run | — | No effect: `input/consumed` and `turn/end` stay in local history | — |
| Child branch created in the run Session | — | Ignored; runs use `main` only | Scheduler |
| Run Session idle-unloaded with input pending | — | The tracker's open `sessions.idle` call keeps it loaded; after restart the next `sessions` call restores it | Agents |
| Agents reload mid-run | — | Scheduler reloads with it; runs kept; tracking resumes from history | Scheduler |
| Scheduler reload or restart mid-run | — | A.5 settlement; never resubmitted | Scheduler |
| Scheduler reload during a `schedule` call | — | The calling Session part reloads; that turn fails (D3, accepted) | Loop |
| Daemon down over a window | — | `missed`; no catch-up | Scheduler |
| Resource node offline at trigger, `create` | — | A launch refused for the unavailable node → `skipped: node_unavailable`, not retried (A6); a launched Session runs and its node-service calls fail `node_unavailable` (row below) | Agents, Scheduler |
| Resource node offline, `reuse` or mid-run | — | Input commits; the run Session stays loaded (no reload, no Pending); its calls on node services fail `node_unavailable` and the turn ends on its own outcome; timeout applies (requirement on [#5](https://github.com/arcships/dim/issues/5)) | Session's node services, Scheduler |
| Reuse Session policy above the job's effective policy | — | `skipped: policy` | Scheduler |
| Pending approval in a run Session | `approval/request` | Waits for `approval.respond` from any client or the timeout | Approval; Scheduler |
| Duplicate/late `schedule.save` | revision | Stale revision rejected | Scheduler |
| `schedule` call interrupted after save | save < interrupt | Saved job stays; tool result cancelled | ToolExecutor; Scheduler |
| Client detach | — | No effect | — |

## Appendix C. Errors and recovery

| Case | Result |
| --- | --- |
| Invalid job (schedule, prompt size, unknown node, policy above cap) | `schedule_invalid` or error tool result |
| Launch failure | Run `failed`; window not retried |
| Uncertain write (reload, publication unknown) | Run stays `launching`; settled by lookup |
| Storage unreadable or unknown format version | Scheduler fails to apply (no silent reset) |
| Process exit | Timers stop; settlement at next start |

Error codes: `schedule_invalid`, `schedule_conflict`, `schedule_not_found`, `service_reloading`.

## Appendix D. Acceptance

With a controllable clock, through management methods, the `schedule` tool and the notice:

1. Each schedule kind yields the expected windows, including DST gap/overlap and `at` disabling itself.
2. A due `create` job yields a non-interactive Session whose first input carries the marker and prompt; run `completed`; one notice.
3. Overlap: a window during an active run is `skipped: overlap`.
4. `reuse` with a busy Session: input queued behind the running turn.
5. User interrupt → `cancelled`; timeout while held → `input/deleted` and `timed_out`; timeout after apply → `notHeld`, then `stop`; after consumption → `stop/interrupt` and `timed_out`.
6. User withdraws the run input while held → `input/deleted` and `cancelled: withdrawn`.
7. Clock jump → one `missed` record, no runs.
8. Restart with a `launching` run: accepted resumes, unknown → `lost`; no duplicate input.
9. Agents reload mid-run: Scheduler reloads, the run completes and is tracked to its end.
10. Policy: tool-created job under WorkspaceWrite runs with at most WorkspaceWrite; management job defaults to ReadOnly.
11. `schedule` absent in non-interactive and member Sessions; asked under WorkspaceWrite, allowed under FullAccess.
12. Node unavailable at trigger: `skipped: node_unavailable`.

## Appendix E. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Formats `cron`, `every`, `at`; one-minute granularity | Feature list ("recurrence"); judgment | Formats change |
| A2 | 60 s grace for a window | Judgment | Other missed detection |
| A4 | Overlap: skip the new window | Judgment | Queue or parallel runs |
| A5 | Reused busy Session: enqueue | Judgment | Busy windows skipped |
| A6 | Failures never retry or fall back to `create` | Judgment | Fallback policy |
| A7 | `max_run_minutes` default 60 | Judgment | Unbounded runs |
| A8 | Last 100 runs per job | Judgment | Retention setting |
| A9 | Tool only for interactive leader Sessions | [Overview §6.2](../overview.md#62-constraints) constraint 15; judgment | Runs could create jobs |
| A10 | One risk category for all actions | [Overview §6.2](../overview.md#62-constraints) constraint 7 | Split read-only tool |
| A11 | Tool-created jobs use the caller's resource node | Judgment (overview §6.3) | Other nodes allowed |
| A13 | Max concurrent runs = 4, a business setting in the Settings business source `schedule`, read at each window | Judgment; ADR 002 D4 | Other limit |
| A14 | Deleting a job does not stop its active run | Judgment | Delete stops the run |
| A15 | The cap of a tool-created job is captured at save | Judgment within E2 | Re-resolve per run |
| A16 | Run outcomes are a live, non-resumable notice | [Overview §5.5](../overview.md#55-scheduled-run-through-agents); judgment | Persistent outbox per listener |
| A17 | Only runs that reached `launching` produce a notice | Judgment | Listeners see every window |
| A18 | `schedule.ended` is delivered only by the run Session's Notifications instance; lost if that Session is unloaded | User decision (process-level delivery not added) | A process-level listener delivers every notice |

Decided: run policy (E2), FullAccess allows `persistent_configuration` (E1), daemon liveness (E5), jobs in a Settings business source and run history in `pluginFiles` (extended) (user decision), optional `LaunchRequest.agent` (user decision), withdrawal of own inputs, scope ([ADR 002](../../adr/002-plugin-scope.md)).

## Appendix F. Open questions

| Question | Description |
| --- | --- |
| Liveness wording | Feature list says the interactive terminal must be alive; this design keeps "daemon alive" (E5); recorded, not open |

## Appendix G. Rust binding

One binding of the neutral contracts above, for a Rust implementation; a TypeScript implementation uses the same names and data through its rutis-bridge runtime.

| Neutral | Rust |
| --- | --- |
| `toolContribution` key `schedule` | Session-scoped `ToolContribution` registered in `SchedulerSession`'s `provides`; `declaration()` / `execute(ToolCall, cancel)` |
| `managementMethods` key `schedule` | Looked up by Rpc per call, not a dependency of Rpc ([methods Appendix F](../contracts/session-boundary.md#appendix-f-rust-binding)) |
| `sessions.launch` | `AgentIndex::launch(LaunchRequest) -> SessionUse` |
| `sessions.input` / `history` / `idle` / `stop` / `withdraw` | `AgentIndex::acquire(id)` lease, then `Session::input`, `local_page`, `idle`, `stop`, `service::<InputQueue>(branch).withdraw` |
| `scheduled_run_outcomes` | `pub fn scheduled_run_outcomes() -> EventKey<RunOutcome>` in Scheduler's public types crate; `emit(host_ctx, &key, Arc::new(outcome))`, listeners `on(host_ctx, &key, listener)` |
| `scheduleJobs` | Process-scoped named service provided by `Scheduler`, consumed by `SchedulerSession` |
| Process storage | `pluginFiles` (extended) scope opened in `apply` |
| Timer task | Spawned in `ctx.effect` |

```rust
pub struct RunOutcome {
    pub job_id: String, pub job_name: String, pub run_id: String,
    pub session_id: Option<String>,
    pub state: RunEnd,               // Completed | Failed | Cancelled | TimedOut | Lost
    pub reason: Option<String>,
    pub ended_at: Timestamp,
}
```
