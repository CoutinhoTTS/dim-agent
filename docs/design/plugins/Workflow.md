# Workflow

Status: Draft. Overall design: [overview §5.9](../overview.md#59-workflow). Phase: P2. Language: TypeScript first ([overview §11.3](../overview.md#113-assignment)).

Workflow is a process plugin that owns workflow definitions and runs. A run advances a fixed list of steps; each stage runs in its own Session, either a new non-interactive Session launched through Agents or a Collaboration member of the Session that started the run. The model starts, queries and stops runs with the process-level `workflow` tool; operators use the `workflow.*` management methods. When a run ends, its result is reported to the starting branch as Steer input through [session-access](../contracts/session-access.md). Workflow writes no Timeline event.

## 1. User flows

1. The user runs `/workflow`, picks `review-and-fix`, supplies `target=src/parser`, and starts it. The CLI calls `workflow.start` with the current Session and branch as report target. Stage `analyze` runs in a new non-interactive Session; then `fix-a` and `fix-b` run in parallel with `analyze`'s result in their prompts; then `summarize`. `workflow.status` shows stages, their Session ids, logs and results. When the run completes, a report input arrives on the user's branch and the model answers from it.
2. The model calls `workflow {action: "start", name: "review-and-fix", params: {...}}`. The tool result returns `run_id` at once; the model continues or calls `wait`. The report Steer input ends the `wait`.
3. The user runs `/workflow:save` to store a definition in the user or project source.
4. A project definition was edited after approval. `start` fails with `approval_required`; the user reviews it and calls `workflow.approve`; `start` succeeds.
5. The user stops a run. Running stage Sessions receive a turn-scoped stop, pending stages never start, the run ends `stopped`, and no report input is written.
6. The daemon restarts during a run. The run is settled `interrupted`; the stage that was running is recorded `lost`; nothing is re-executed. A report saying so is delivered to the report target.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Definition format, validation, sources and host-local approval checks | Session creation and non-interactive configuration (Agents supplement) |
| Runs: state machine, stage advancement, parameters and stage inputs, logs, results, stop | Member creation and Team rules (Collaboration) |
| `workflow` tool, `workflow.*` management methods | Turn execution, approvals and tools inside stage Sessions (their own capabilities) |
| Report input to the report target | Model selection policy (Provider), external Agent bridging (ExternalAgent) |

## 3. Scope and dependencies

**Contracts depended on**: [process-level tool contribution](../contracts/process-tool-contribution.md) (`ProcessToolSource`); [session-access](../contracts/session-access.md) (`SessionAccess`, `SourceAccess`, `SessionLease`, `SourceInput`, `Lookup`); the Agents launch contract (overview §7.2 Agents supplement; same shape as [Scheduler §6.4](Scheduler.md#64-session-launch-depended-on-agents-supplement)); a Collaboration member-establishment contract (proposed here, §14); Provider model selection by purpose (`ModelSelectionHost`, overview §7.2 Provider supplement); Settings document sources and host-local approvals (overview §7.2 Settings supplement); the [Management](Management.md) contribution contract (`ManagementContribution`); PluginManager's typed plugin-content sources (overview §5.12, optional). **Contracts defined here**: the definition format `dim.workflow/v1`, the `workflow` tool, the `workflow.*` methods, the source name `workflow` with marker kinds `stage` and `report`, the purpose key `workflow.stage`, and the run record. Workflow names no other plugin's internal type; any provider of these contracts can replace another.

| Part | Scope | Injects | Provides | Status |
| --- | --- | --- | --- | --- |
| `WorkflowPlugin` | Process (control node) | `Arc<SessionAccess>`; Agents launch service; `When<CollaborationMembers>`; `ModelSelectionHost`; Settings workflow sources and approvals; process storage; `When<PluginContentSources>` | `ProcessToolSource` named `workflow`; `ManagementContribution` named `workflow` | Proposed |

Only one part exists: runs belong to the process, not to the starting branch ([overview §5.9](../overview.md#59-workflow)), so no Session or branch part is created ([overview §3.2](../overview.md#32-capabilities-and-process-plugins)). Without `CollaborationMembers` the `member` runner is rejected at validation; without plugin-content sources only user and project definitions exist.

## 4. Events

None. A run's start is the tool result (or the management reply); its outcome reaches the model as the report input, which is an existing `input/message`. Stage work is ordinary input and turns inside stage Sessions. Run, stage and log state are process storage ([overview §6](../overview.md#6-new-timeline-events), `workflow/*` row). No later branch behavior depends on a fact that these existing events do not already carry.

## 5. State

| State | Owner | Rebuild |
| --- | --- | --- |
| Definitions | Settings sources (files); plugin-content sources | Read on every `list`/`start`; never cached across calls (A1) |
| Approvals | Settings host-local approval store | Read per check (§11) |
| Run records: definition snapshot and fingerprint, params, report target, run state, steps, stage records, logs, pending report | `WorkflowPlugin`, process storage | Loaded at `apply`; non-terminal runs are settled `interrupted` (§7) |
| Stage Session facts | Each stage Session's Timeline | Read through session-access; never copied except the bounded result text |

Run record (persisted on every transition, before the external effect it guards):

```text
Run { run_id, name, source: user|project|plugin, fingerprint, definition (snapshot),
      params, cwd, resource_node, report_to: Option<{session_id, branch_id}>,
      started_by: tool{session_id, branch_id, call_id} | management,
      state: running | stopping | completed | failed | stopped | interrupted,
      stages: [Stage], log: [LogEntry] (bounded, A9), report: none | pending{submission_id} | delivered | undeliverable{reason} }
Stage { id, step, state: waiting | launching | running | completed | failed | stopped | skipped | lost,
        session_id?, agent_id?, submission_id?, input_event_id?, result_text?, reason? }
```

## 6. Interfaces

### 6.1 Definition format `dim.workflow/v1`

YAML (A2). Files: `<home>/workflows/<name>.yaml` (user source), `<workspace>/.dimcode/workflows/<name>.yaml` (project source) (A3); plugin packages declare the same documents as plugin content.

```yaml
schema: dim.workflow/v1
name: review-and-fix                 # [a-z0-9-]{1,64}; unique per source
description: Analyze, fix in parallel, summarize
parameters:
  - { name: target, description: Path to review, required: true }
  - { name: style, default: concise }
steps:
  - stage: { id: analyze, prompt: "Analyze {{params.target}}." }
  - parallel:
      - { id: fix-a, agent: <agent id>, prompt: "Fix part A.\n{{stages.analyze.result}}" }
      - { id: fix-b, runner: member, prompt: "Fix part B.\n{{stages.analyze.result}}" }
  - stage: { id: summarize, model: { provider: p, model: m }, prompt: "Summarize: {{stages.fix-a.result}} {{stages.fix-b.result}}" }
```

| Field | Rule |
| --- | --- |
| `steps` | 1–32 steps executed in order; a step is one `stage` or one `parallel` group of 2–8 stages (A4) |
| `stage.id` | `[a-z0-9-]{1,32}`, unique in the definition |
| `stage.agent` | Optional Agent id (a native profile or an external Agent profile); default: the Agents default Agent |
| `stage.runner` | `session` (default) or `member`; `member` requires `report_to` and `CollaborationMembers` (§6.4) |
| `stage.model` | Optional explicit model; else the Agent profile's model; else Provider's selection for purpose `workflow.stage` (A5) |
| `stage.prompt` | Text template, ≤ 32 KiB after rendering (A6). Placeholders: `{{params.<name>}}` and `{{stages.<id>.result}}` |
| References | A `stages.<id>` reference must name a stage of an earlier step; same-step and forward references are rejected at validation. Rendering is plain substitution, no expressions or code (A7) |
| Parameters | Supplied values are strings; missing required parameters and unknown names are `invalid_parameters`; defaults fill the rest |

Validation happens at the external boundary (file read, `workflow.save`, `start` params); runs use the validated snapshot.

### 6.2 Tool `workflow`

Process-level contribution `workflow`, `requires: { interactive: true, roles: None }` (A8), risk `session` (existing category, A10).

| Action | Parameters | `ToolResult.content` |
| --- | --- | --- |
| `list` | — | Definitions visible for the Session cwd: name, source, description, parameters, approval state |
| `start` | `name`, `source?` (`project` first, then `user`, then `plugin`), `params` | `{"run_id", "status": "running", "stages": [...]}` and a sentence that the result arrives as a report input |
| `status` | `run_id` | State, per stage state, Session id, bounded result preview, last log entries |
| `stop` | `run_id` | Final state after propagation (§7) |

`start` from the tool sets `report_to = {session_id, branch_id}` from `ProcessToolSession` and `ToolExecutionContext`, `cwd` and `resource_node` from `ProcessToolSession`. The handler persists the run, schedules it and returns; it does not wait for stages. Errors are error results with a code: `definition_not_found`, `invalid_definition`, `invalid_parameters`, `approval_required`, `run_not_found`, `limit_exceeded` (A11).

### 6.3 Management methods (contribution `workflow`)

| Method | Params | Result | `mutating` |
| --- | --- | --- | --- |
| `workflow.list` | `cwd` | Definitions with source, fingerprint, approval state, validation issues | no |
| `workflow.save` | `source: user\|project`, `cwd?`, `document` | `{name, fingerprint}`; written through Settings' source (file authority) | yes |
| `workflow.start` | `name`, `source?`, `cwd`, `resource_node?`, `params`, `report_to?: {session_id, branch_id}` | `{run_id}` | yes |
| `workflow.status` | `run_id?` | One run, or recent runs | no |
| `workflow.stop` | `run_id` | Final run state | yes |
| `workflow.approve`, `workflow.revoke` | `name`, `source`, `cwd` | Approval state bound to node + workspace + fingerprint | yes |

`approve`/`revoke` extend the overview's method list (§14). All use `Interruption::Report`: a run already admitted continues if the caller disconnects.

### 6.4 Depended-on contracts (proposed)

```rust
// Agents supplement (overview §7.2), shape shared with Scheduler §6.4:
async fn launch(&self, config: LaunchConfig) -> Result<SessionLease, LaunchError>;
// LaunchConfig { agent_id, cwd, resource_node, model, interactive: false }

// Collaboration supplement proposed here (§14): establish a member of `leader` without branch facts.
async fn establish(&self, leader_session_id: &str, agent_id: Option<&str>)
    -> Result<MemberRef /* agent_id, session_id */, MemberError>;
```

Workflow drives a member Session exactly like a launched one, through session-access; Collaboration keeps Team rules (single leader, `Busy` when the member already works) and applies its own member configuration (leader cwd and policy).

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Start | Tool handler or management call validates, checks approval, persists `Run{state: running}` and spawns the run task on the plugin's `TaskTracker` (registered with `ctx.effect`), then returns. No Timeline write |
| Stage launch | Recheck in storage: run still `running`, approval still valid for project/plugin sources (A12). Persist `launching`; `launch(...)` or `establish(...)`; persist `session_id`; persist `submission_id = "workflow:<uuid>"` |
| Stage input | `lease.input("main", SourceInput { submission_id, marker: { kind: "stage", attrs: [run, stage], body: definition name }, blocks: [], user: Some(rendered prompt), mode: Queue })` → `Session::input` → `TimelineContext::input` → `Timeline::append`. Persist `running` with `input_event_id` |
| Consumption | The stage Session's InputQueue and Loop consume on their Dispatchers (`input/applied`, `input/consumed`) and run turns. Workflow holds no Dispatcher subscription and no `StateCell` |
| Completion | `lease.idle("main")` returns cutoff `C`; Workflow reads local pages of `main` up to `C` and decides from those facts only: before `input/consumed{input_event_id}` the stage is not started: if a `stop/interrupt` follows the input it is `stopped`, else Workflow waits for `idle` again. After consumption, the last `turn/end` in `(consumed, C]` decides `completed`/`failed`/`stopped`, and the text of that turn's last request is the result (Collaboration's `outcome` rule) |
| Advance | Persist the stage result; when every stage of a step is terminal, either start the next step or end the run. A failed stage stops its running siblings (§7 stop path) and fails the run (A4) |
| Report | On `completed`, `failed` or `interrupted` with `report_to`: persist `report: pending{submission_id}`, then `acquire(session_id)` (restores an unloaded Session) and `lease.input(branch_id, SourceInput { marker: { kind: "report", attrs: [run, status] }, blocks: [summary], mode: Steer })`. Persist `delivered`, or `undeliverable` on `BranchMissing`/`SessionClosed` |
| Stop | Persist `stopping`; cancel launch of waiting stages (`skipped`); for each `running` stage `lease.stop("main", "workflow stopped")` (turn-scoped `stop/interrupt`, waits for settlement); then the completion rule reads the outcome; the run ends `stopped`. No report (explicit owner stop never wakes, [event-driven flows §3](../../architecture/rust-agent/event-driven-flows.md)) |
| Recheck | Every decision is taken from storage plus a fixed prefix read; sequence order inside each stage Session decides races (§8) |
| Reload, restart | `apply` of a new generation loads run records first. Every `running`/`stopping` run becomes `interrupted`; `launching`/`running` stages become `lost`; nothing is relaunched or re-input ([overview common constraint 4](../overview.md#3-integration-methods)). A `pending` report is looked up by `submission_id` (`Accepted` → `delivered`; `Unknown` → delivered now); interrupted runs with `report_to` get a report. On unload, the old generation stops running stage turns (bounded wait, A13) before its disposer returns |

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Starter turn `stop/interrupt` before the `workflow` call is dispatched | interrupt < `step/tool-call` | Call not executed; no run | ToolExecutor |
| Starter turn interrupt while the handler runs | `step/tool-start` < interrupt | Run persisted before return continues; if cancelled before persistence, no run | Workflow (persistence point) |
| Starter turn interrupt after the tool result | result < interrupt | Run continues (not branch detached work) | Workflow |
| Starter branch-scoped interrupt or branch close | close < report write | Run continues; report `undeliverable` (`BranchMissing`) | session-access, Workflow |
| Tool batch not settled when the run ends quickly | report `input/message` < `step/tool-result` | Report is Steer: consumed after the batch result, so the model sees `run_id` first | Loop |
| Concurrent user Steer/Queue on the report branch | Timeline order | Each applied by mode; report is an ordinary Steer | InputQueue |
| Report arrives while the starter branch is idle | — | Steer starts a turn | InputQueue, Loop |
| Compaction between report write and consumption | input < compaction < consumed | Report placed at its consumed position after the summary | ContextPreparation |
| Child branch cut between report write and consumption | input < cut < consumed | Child does not see the report; parent does | InputQueue (child) |
| Stage input written, then a user stops the stage Session before consumption | input < interrupt, no consumed | Stage `stopped`; the input stays pending in that Session (A14) | Workflow (reads prefix) |
| `workflow.stop` races the stage's own `turn/end` | `turn/end` < stop's `stop/interrupt` | Stage `completed` with its result; run still `stopped` | Stage Session order |
| `workflow.stop` twice / after the run ended | — | Idempotent: returns the current state | Workflow (storage) |
| Duplicate `start` calls | — | Two runs; no deduplication (A15) | Workflow |
| Member stage, leader also `request`s that member | member running | Collaboration answers `Busy` to one of them | Collaboration |
| Stage Session's resource node offline | capabilities Pending | `launch` fails at the startup deadline → stage `failed: node_unavailable`; a running stage waits in `idle` until reload settles its turn | Agents, rutis lifecycle |
| Stage Session reloads mid-turn | turn ends `Failed` (reload) | Stage `failed` | Stage Session's Loop |
| Starter Session unloaded at report time | — | `acquire` restores it, then writes | AgentIndex |
| Workflow or Agents reload mid-run | — | Runs `interrupted`, stages `lost`, report delivered by the new generation | Workflow |
| Process restart mid-run | — | Same as reload; stage inputs committed but unconsumed stay in their Sessions | Workflow |
| Report write with uncertain outcome (`Reloading`, `PublicationUnknown`) | — | `lookup(submission_id)` before retry; never two reports | Workflow |
| Project definition edited after start | — | Running run keeps its snapshot; next stage launch rechecks the approval of the snapshot's fingerprint | Workflow |
| Client detach (CLI that started the run) | — | No effect; detach is not a close | — |

## 9. Context mapping

| What the model sees | Form | Placement |
| --- | --- | --- |
| `workflow` declaration | Tool declaration in adopted `context/configured` (interactive Sessions only) | Requests whose prefix holds that configuration |
| Start, status, stop | `ToolResult.content` (text incl. `run_id`) | Tool result position of its batch |
| Report | One raw block, first line `<dim-workflow kind="report" run="..." status="...">`, body: per-stage status, bounded preview of the last step's results (A16), and how to get full results (`workflow` action `status`) | Consumed position of the Steer input; not keyed |
| Stage task (in the stage Session) | Marker raw block `<dim-workflow kind="stage" run="..." stage="...">` then the rendered prompt as user content | First input of the stage Session (or a new input of a member Session) |

The report and stage inputs are unkeyed: after compaction they survive only through the summary. Child branches inherit them only when consumed before the cut. Stage Sessions are non-interactive: no `ask_user`, no `workflow` tool, so runs never nest. The model never sees definitions it did not list, run records, logs, approvals, fingerprints, other plugins' events or diagnostics.

## 10. Errors and recovery

| Case | Result |
| --- | --- |
| Invalid definition file | Listed with issues; `start` → `invalid_definition` |
| Approval missing or stale | `approval_required` at start; `failed: approval_revoked` at a later stage launch |
| `launch`/`establish` fails | Stage `failed` with the reason; run `failed` |
| Stage turn `Failed`/`Cancelled` | Stage `failed`/`stopped`; run `failed` (A4) |
| Session access error while waiting (`SessionClosed`, `BranchMissing`) | Stage `failed: session_closed` |
| Storage write failure | Run task stops before the guarded effect; run left in its last persisted state and settled `interrupted` on next load |
| Unload | Stops stage turns, settles runs `interrupted` (§7) |

Codes are the tool error codes of §6.2; management methods return them as `app_error`.

## 11. Security

Project and plugin definitions are executable content: `start` and every stage launch require a host-local approval bound to node + workspace + fingerprint (SHA-256 of the definition document) ([overview §9](../overview.md#9-cross-cutting-conventions)). Approval is granted only by `workflow.approve` on a Management-mounted entry; the tool, Timeline facts, replay and fork never grant it. User-source definitions need no approval. Stage Sessions run with the Agents non-interactive default policy (A17) or, for members, Collaboration's leader-policy rule; a definition cannot widen either. Stage Sessions run on the run's resource node through that node's resource services. Params and results are Timeline content of stage and report Sessions; definitions must not carry credentials.

## 12. Acceptance

Flow tests through the tool, management methods and session-access reads:

1. Sequential and parallel steps; `{{stages.x.result}}` substitution; forward reference rejected at save.
2. Tool start returns `run_id`; the report Steer ends a `wait` and arrives after the tool batch result.
3. Stop: waiting stages `skipped`, running stage interrupted, run `stopped`, no report; repeated stop idempotent.
4. Stop racing a stage `turn/end` keeps the completed result.
5. Starter turn interrupt does not stop the run; starter branch closed → `undeliverable`.
6. Project definition: unapproved → `approval_required`; approve; edit → next stage `approval_revoked`.
7. Restart and plugin reload mid-run: `interrupted`/`lost`, no relaunch, one report (lookup prevents duplicates).
8. `member` runner: member appears in the leader's Team; `Busy` from a concurrent leader request.
9. Non-interactive stage Sessions have no `workflow` tool.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Definitions are read per call, not cached | Judgment (file authority, overview §1) | A cache with change detection |
| A2 | Definition format is YAML with `schema: dim.workflow/v1` | Judgment | Another syntax; same model |
| A3 | File locations `<home>/workflows/` and `<workspace>/.dimcode/workflows/` | Judgment; overview §5.9 names user and project sources | Paths decided by Settings |
| A4 | Steps are stages or parallel groups (≤ 32 steps, ≤ 8 per group); fail-fast on any stage failure | Feature list ("stages"); limits and policy are judgment | Per-stage failure policies, other limits |
| A5 | Model precedence: stage model > profile model > purpose `workflow.stage` | Overview §5.9 (purpose); precedence is judgment | Other precedence |
| A6 | Rendered prompt ≤ 32 KiB; stage result stored ≤ 64 KiB | Judgment | Other bounds |
| A7 | Templates are plain substitution of params and earlier stage results | Judgment | Expressions would need a sandbox |
| A8 | Tool offered only to interactive Sessions | Overview §5.1 non-interactive rule; prevents nested runs (judgment) | Nested runs need a depth limit |
| A9 | Run log bounded to 500 entries; last 100 runs retained | Judgment | Other retention |
| A10 | Tool risk is the existing `session` category | Judgment | A new category registered with Permission |
| A11 | Concurrency limits: 4 running runs, 4 concurrent stages per run → `limit_exceeded` | Judgment | Other limits |
| A12 | Approval is rechecked before each stage launch | Overview §9; per-step timing follows the MCP precedent (judgment) | Check only at start |
| A13 | Unload waits up to 10 s for stage stops | Judgment | Other bound |
| A14 | A stage stopped before consumption leaves its input pending in the stage Session | Interrupt contract (inputs applied before an interrupt stay pending); judgment not to withdraw | Withdrawal through InputQueue (overview §7.1) |
| A15 | `start` does not deduplicate | Judgment | Idempotency keys |
| A16 | Report preview ≤ 4 KiB | Judgment; precedent `dim-agent-report` preview | Other bound |
| A17 | Launched stage Sessions use the Agents non-interactive default policy | Scheduler A15 alignment; judgment | A per-definition policy field (narrowing only) |

## 14. Open questions

| Question | Description |
| --- | --- |
| Collaboration member contract | Overview §7.2 registers only a model-selection change for Collaboration; "stage members" (overview §3.3) need a public `establish(leader, agent)` service without branch facts. Not yet registered in §7 |
| Management method list | Overview §5.9 lists `workflow.list/save/start/status/stop`; approval of project definitions needs `workflow.approve/revoke` (precedent `hooks.approve`) |
| Agents launch contract | Shape shared with Scheduler; owned by the Agents supplement |
| Report on restart | Whether an `interrupted` run should report at all, or only be visible in `status` |
| Reload blast radius | A Workflow or Agents reload interrupts every running run; a stable run store that resumes observation (not execution) would need an exception to overview common constraint 4 |
