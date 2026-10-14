# Workflow

Status: Draft. Overall design: [overview §B.9](../overview.md#b9-workflow). Phase: P2. Language: TypeScript first ([overview §F.3](../overview.md#f3-assignment)).

Workflow is a process plugin that owns workflow definitions and runs. A run advances a fixed list of steps; each stage runs in its own Session, either a new non-interactive Session launched through Agents or a Collaboration member of the starting Session created with Collaboration's `establish`. The model starts, queries and stops runs with the process-level `workflow` tool; operators use `workflow.*` management methods. When a run ends, its result is reported once to the starting branch as Steer input through [session-access](../contracts/session-access.md). Workflow writes no Timeline event.

## 1. Structure

```mermaid
flowchart LR
  subgraph WF["WorkflowPlugin (process, control node)"]
    Defs["Definition loader and validator"]
    Runner["Run engine and stage runners"]
    Tool["workflow tool handler"]
    Mgmt["workflow.* methods"]
  end
  ToolReg["Process tool registry (ToolRegistry)"]
  MgmtReg["Method contribution, level management (session-boundary)"]
  Launch["Agents launch (non-interactive, policy cap)"]
  Estab["Collaboration establish"]
  Access["SessionAccess facade"]
  Purpose["Provider model selection, purpose workflow.stage"]
  Store["Process storage (App Server)"]
  Approvals["Host-local approval service (Settings)"]
  Sources["Settings user and project sources"]
  Contents["PluginContents type workflows"]
  Tool --> ToolReg
  Mgmt --> MgmtReg
  Runner --> Launch
  Runner -.->|When| Estab
  Runner --> Access
  Runner --> Purpose
  Runner --> Store
  Defs --> Sources
  Defs --> Approvals
  Defs -.->|When| Contents
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
  classDef contract stroke-dasharray:4 3
  class Defs,Runner,Tool,Mgmt new
  class ToolReg,MgmtReg,Launch,Estab,Access,Purpose,Store,Approvals,Sources,Contents contract
```

Runs belong to the process, not to the starting branch, so there is no Session or branch part. Workflow drives every stage Session, member or not, only through the `SessionAccess` facade, so an Agents or Collaboration reload returns `Reloading` to Workflow instead of interrupting runs.

| Part | Scope | Depends on | Provides |
| --- | --- | --- | --- |
| `WorkflowPlugin` | Process (control node) | `SessionAccess` (`config`, acquire, input, `withdraw`, idle, stop, lookup, pages), Agents launch, `When<Collaboration establish>`, Provider purpose selection, App Server process storage, Settings sources and host-local approval service, `When<PluginContents>` (type key `workflows`) | Mount instances: process tool set `workflow` ([process-tool-contribution](../contracts/process-tool-contribution.md)), methods `workflow.*` at level `management` ([session-boundary](../contracts/session-boundary.md)) |

**Contracts depended on**: [extension-registry](../contracts/extension-registry.md) (mount pattern), process-tool-contribution, [session-access](../contracts/session-access.md), Agents launch (overview §C.2; shape in [Scheduler A.5](Scheduler.md#a5-agents-launch-depended-on)), Collaboration `establish(leader, agent)` (Collaboration supplement), Provider purpose policy (overview §C.2), [session-boundary](../contracts/session-boundary.md) (method contribution), App Server process storage (R8), Settings sources and host-local approval service (R11), `PluginContents` (optional). **Contracts defined here**: definition format `dim.workflow/v1`, the `workflow` tool, the `workflow.*` methods, source name `workflow` (marker kinds `stage` and `report`), purpose key `workflow.stage`, content type key `workflows`, the run record.

## 2. Data and state

```mermaid
classDiagram
  class Definition {
    schema dim.workflow/v1
    name, description, parameters
    steps: stage or parallel group
    policy optional
  }
  class Run {
    run_id, name, source, fingerprint, snapshot
    params, cwd, resource_node, policy_cap
    report_to, started_by, state
    report: none, pending, delivered, undeliverable
  }
  class Stage {
    id, step, state, runner
    session_id, agent_id, submission_id
    input_event_id, result_text, reason
  }
  class ProcessStorage {
    workflow root, format version 1
  }
  class StageSessionTimeline {
    stage input, input/consumed, input/withdrawn
    turn/end, stop/interrupt
  }
  class StarterTimeline {
    workflow tool call and result
    report Steer input
  }
  Definition <.. Run : snapshot at start
  Run "1" *-- "1..*" Stage
  ProcessStorage *-- Run
  Stage ..> StageSessionTimeline : decided from fixed reads
  Run ..> StarterTimeline : report written once
```

Definitions are read from their sources per call (A1) and approvals per check; runs live in process storage and are persisted on every transition before the effect they guard. Stage progress is derived from existing facts in each stage Session; only a bounded result text is copied. Nothing is rebuilt from Timeline and Workflow owns no `StateCell`.

## 3. Flows

### 3.1 Start from the tool and run a stage

```mermaid
sequenceDiagram
  participant M as Model (starter branch)
  participant WF as Workflow
  participant ST as Process storage
  participant AG as Agents launch or Collaboration establish
  participant SA as SessionAccess
  participant SS as Stage Session
  M->>WF: workflow start (tool handler)
  WF->>WF: validate, approval check, policy cap from starter
  WF->>ST: persist Run running
  WF-->>M: ToolResult run_id (batch result)
  WF->>ST: persist stage launching
  WF->>AG: launch(non-interactive, capped policy) or establish(leader, agent)
  AG-->>WF: SessionLease
  WF->>SA: input main Queue, marker stage, rendered prompt
  SA->>SS: Session::input then Timeline::append
  WF->>ST: persist stage running with input_event_id
  SS->>SS: Dispatcher: input/applied, input/consumed, turn
  WF->>SA: idle(main) then local pages up to cutoff
  WF->>ST: stage completed or failed, next step
```

The handler persists the run and returns at once; stage advancement runs on the plugin's task tracker. Completion is decided only from the stage Session's facts up to the `idle` cutoff.

### 3.2 Report and stop

```mermaid
sequenceDiagram
  participant WF as Workflow
  participant ST as Process storage
  participant SA as SessionAccess
  participant STB as Starter branch
  alt run completed, failed or interrupted, with report_to
    WF->>ST: report pending(submission_id)
    WF->>SA: acquire(starter) then input Steer, marker report
    SA->>STB: Timeline::append
    WF->>ST: delivered, or undeliverable on BranchMissing or SessionClosed
  else workflow stop
    WF->>ST: stopping
    WF->>SA: withdraw unconsumed stage inputs, stop(main) for running stages
    WF->>ST: stopped, no report
  end
```

An explicit owner stop never wakes the starter ([event-driven flows §3](../../architecture/rust-agent/event-driven-flows.md)). `lookup(submission_id)` before any retry guarantees at most one report.

### 3.3 Policy of a stage

```mermaid
flowchart LR
  Start{"started_by"}
  ToolStart["tool: cap = starter Session policy (resolved by Agents)"]
  MgmtStart["management: cap = host configuration maximum"]
  D1["default = cap"]
  D2["default = read-only"]
  Decl["definition policy (optional, narrowing)"]
  Eff["stage policy = min(declared or default, cap)"]
  Member["member runner: Collaboration leader-policy rule, also capped"]
  Start --> ToolStart --> D1 --> Eff
  Start --> MgmtStart --> D2 --> Eff
  Decl --> Eff
  Eff --> Member
```

A run never widens authority (E2): the `workflow` tool is admitted with the same risk category as Collaboration's `agent` tool, and stage Sessions never exceed the starter's policy.

## 4. Timing and conflicts

```mermaid
sequenceDiagram
  participant WF as Workflow
  participant SA as SessionAccess
  participant AG as Agents
  WF->>SA: idle(main) for a running stage
  AG->>AG: Agents reload window begins
  SA-->>WF: Reloading
  WF->>WF: keep run and stage state, retry after the window
  AG->>AG: new generation takes over
  WF->>SA: idle(main) again, read up to cutoff
  WF->>WF: stage outcome from facts, run continues
```

Agents and Collaboration reloads no longer interrupt runs; only Workflow's own reload or a restart settles runs `interrupted`.

| Scenario | Sequence order | Outcome | Owner |
| --- | --- | --- | --- |
| Starter turn interrupt after the tool result | result < interrupt | Run continues | Workflow |
| Tool batch not settled when the run ends quickly | report < `step/tool-result` | Steer consumed after the batch result; model sees `run_id` first | Loop |
| `workflow.stop` races a stage's own `turn/end` | `turn/end` < `stop/interrupt` | Stage `completed` with result; run `stopped` | Stage Session order |
| Stage input withdrawn (user or stop) before consumption | input < `input/withdrawn` | Stage `stopped`; never consumed | InputQueue, Workflow |
| Workflow reload or restart mid-run | — | Runs `interrupted`, active stages `lost`, nothing relaunched; one report each (E15) | Workflow |
| Leader branch closed with a member stage running | close < report | Member stage continues; report `undeliverable` | Workflow |

Full table: Appendix B.

## 5. Context mapping

| What the model sees | Form | Placement |
| --- | --- | --- |
| `workflow` declaration | Tool declaration in adopted configuration (interactive Sessions only) | Requests whose prefix holds that configuration |
| Start, status, stop | `ToolResult.content` (text incl. `run_id`) | Tool result position of its batch |
| Report | One raw block `<dim-workflow kind="report" run="..." status="...">`; body: per-stage status, bounded preview of the last step's results (A16), how to get full results (`status`) | Consumed position of the Steer input; unkeyed |
| Stage task (stage Session) | Marker `<dim-workflow kind="stage" run="..." stage="...">` then the rendered prompt as user content | First input of a launched Session, or a new input of a member Session |

Report and stage inputs are unkeyed: after compaction they survive only through the summary; child branches inherit them only if consumed before the cut. Stage Sessions are non-interactive (no `ask_user`, no `workflow` tool), so runs never nest. The model never sees unlisted definitions, run records, logs, approvals, fingerprints, other plugins' events or diagnostics.

## 6. Interfaces (summary)

| Interface | Kind | Purpose |
| --- | --- | --- |
| `workflow` (`list`, `start`, `status`, `stop`) | Process tool, risk category of Collaboration's `agent` tool, interactive Sessions | Model runs workflows |
| `workflow.list/save/start/status/stop/approve/revoke` | Methods, level `management` | Operators manage definitions and runs |
| `/workflow`, `/workflow:save` | CLI | Call the management methods |
| Agents `launch`, Collaboration `establish` | Depended on | Create stage Sessions |
| `dim.workflow/v1` | Defined here | Definition format (Appendix A) |

## 7. Security

Project and plugin definitions are executable content: `start` and every stage launch require a grant in the Settings host-local approval service, record `{owner: workflow, node, workspace, source, id: name, fingerprint}`, fingerprint = SHA-256 over the canonical definition document (R11). Grants come only from `workflow.approve` on an entry that mounts the `management` level; the tool, Timeline facts, replay and fork never grant. User definitions need no approval. Stage policy follows 3.3 and a definition can only narrow it. Approvals raised inside a member stage are forwarded to the leader by Collaboration (E6), not by Workflow. Stage Sessions reach host resources only through node services resolved per call by their `resource_node` (`node-services`, [extension-registry](../contracts/extension-registry.md)). Params and results become Timeline content of stage and report Sessions; definitions must not carry credentials.

## Appendix A. Interface details

### A.1 Definition format `dim.workflow/v1`

YAML (A2). Files: user source `<dim home>/workflows/<name>.yaml`, project source `<workspace>/.dimcode/workflows/<name>.yaml` (E11, A3); packages publish the same documents as `PluginContents` type `workflows`.

```yaml
schema: dim.workflow/v1
name: review-and-fix                 # [a-z0-9-]{1,64}; unique per source
description: Analyze, fix in parallel, summarize
policy: workspace_write              # optional; narrowing only
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
| `steps` | 1–32 steps in order; a step is one `stage` or a `parallel` group of 2–8 stages (A4) |
| `stage.id` | `[a-z0-9-]{1,32}`, unique in the definition |
| `stage.agent` | Optional Agent id (native or external profile); default: the Agents default Agent |
| `stage.runner` | `session` (default) or `member`; `member` requires `report_to` and Collaboration `establish` |
| `stage.model` | Optional; else the profile's model; else Provider's selection for purpose `workflow.stage` (A5) |
| `stage.prompt` | Template ≤ 32 KiB after rendering (A6); placeholders `{{params.<name>}}`, `{{stages.<id>.result}}` |
| `policy` | `read_only` \| `workspace_write` \| `full_access`; capped (3.3) |
| References | `stages.<id>` must name a stage of an earlier step; plain substitution only (A7) |
| Parameters | Strings; missing required or unknown names → `invalid_parameters`; defaults fill the rest |

Validation happens at the external boundary (file read, `workflow.save`, `start` params); runs use the validated snapshot.

### A.2 Tool `workflow`

Process tool set `workflow`, `requires: { interactive: true, roles: None }` (A8); risk: the category Collaboration's `agent` tool declares (today none, so unclassified: ReadOnly denies, WorkspaceWrite asks, FullAccess allows; E2).

| Action | Parameters | `ToolResult.content` |
| --- | --- | --- |
| `list` | — | Definitions visible for the caller's cwd: name, source, description, parameters, approval state |
| `start` | `name`, `source?` (`project`, then `user`, then `plugin`), `params` | `{"run_id", "status": "running", "stages": [...]}` and a sentence that the result arrives as a report input |
| `status` | `run_id` | State, per-stage state, Session id, bounded result preview, last log entries |
| `stop` | `run_id` | Final state after propagation |

`start` sets `report_to = {session_id, branch_id}` and `cwd`/`resource_node` from the caller's `SessionConfig` subset in the tool call context, and `policy_cap` from the starter (Agents launch contract). Errors: `definition_not_found`, `invalid_definition`, `invalid_parameters`, `approval_required`, `run_not_found`, `limit_exceeded` (A11).

### A.3 Methods (level `management`)

| Method | Params | Result | `mutating` |
| --- | --- | --- | --- |
| `workflow.list` | `cwd` | Definitions with source, fingerprint, approval state, issues | no |
| `workflow.save` | `source: user\|project`, `cwd?`, `document` | `{name, fingerprint}`, written through Settings' source (file authority) | yes |
| `workflow.start` | `name`, `source?`, `cwd`, `resource_node?`, `params`, `report_to?` | `{run_id}`; `policy_cap` = host configuration maximum, default read-only | yes |
| `workflow.status` | `run_id?` | One run or recent runs | no |
| `workflow.stop` | `run_id` | Final run state | yes |
| `workflow.approve`, `workflow.revoke` | `name`, `source`, `cwd` | Approval state bound to node + workspace + fingerprint | yes |

All use `Interruption::Report`: an admitted run continues if the caller disconnects.

### A.4 Run record

```text
Run { run_id, name, source: user|project|plugin, fingerprint, definition (snapshot),
      params, cwd, resource_node, policy_cap, report_to: Option<{session_id, branch_id}>,
      started_by: tool{session_id, branch_id, call_id} | management,
      state: running | stopping | completed | failed | stopped | interrupted,
      stages: [Stage], log: [LogEntry] (bounded, A9),
      report: none | pending{submission_id} | delivered | undeliverable{reason} }
Stage { id, step, runner, state: waiting | launching | running | completed | failed | stopped | skipped | lost,
        session_id?, agent_id?, submission_id?, input_event_id?, result_text?, reason? }
```

### A.5 Depended-on contracts

| Contract | Owner | Use |
| --- | --- | --- |
| `launch(...) -> SessionLease` with `SessionConfig` values `interactive: false`, `resource_node`, `created_by: Launch { source: "workflow" }` (`SessionOrigin`), and `policy` | Agents supplement | Launched stages; Workflow writes the first input itself |
| `establish(leader_session_id, agent_id) -> MemberRef` | Collaboration supplement | Member stages; Collaboration keeps Team rules (single leader, `Busy`), applies the leader's cwd and policy, and forwards member approvals to the leader |
| `SessionAccess::withdraw(branch, input_id)` | session-access (InputQueue contract) | Withdraw own unconsumed stage inputs on stop |

### A.6 Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Start | Handler or management call validates, checks approval, resolves the policy cap, persists `Run{running}`, spawns the run task on the plugin's `TaskTracker` (`ctx.effect`) and returns. No Timeline write |
| Stage launch | Recheck in storage (run `running`; approval still valid for the snapshot fingerprint, A12). Persist `launching`; `launch` or `establish`; persist `session_id` and `submission_id = workflow:<uuid>` |
| Stage input | `lease.input("main", SourceInput { submission_id, marker: { kind: "stage", attrs: [run, stage], body: definition name }, user: rendered prompt, mode: Queue })` → `Session::input` → `TimelineContext::input` → `Timeline::append`; persist `running` with `input_event_id` |
| Consumption | Stage Session's InputQueue and Loop on their Dispatchers (`input/applied`, `input/consumed`), then turns |
| Completion | `lease.idle("main")` returns cutoff `C`; read local pages up to `C`. Before `input/consumed{input_event_id}`: `input/withdrawn` or a following `stop/interrupt` → `stopped`, else wait again. After consumption: the first `turn/end` after it in `(consumed, C]` decides `completed`/`failed`/`stopped`; that turn's last response text is the result (Collaboration's outcome rule) |
| Advance | Persist the stage result; when all stages of a step are terminal, start the next step or end the run; a failed stage stops running siblings and fails the run (A4) |
| Report | On `completed`, `failed` or `interrupted` with `report_to`: persist `pending{submission_id}`, `acquire`, `lease.input(branch_id, Steer, marker report)`; persist `delivered` or `undeliverable` |
| Stop | Persist `stopping`; waiting stages `skipped`; unconsumed stage inputs withdrawn; running stages `lease.stop("main", "workflow stopped")`; completion rule reads the outcome; run `stopped`, no report |
| Agents or Collaboration reload | Facade calls return `Reloading`; the run task keeps state and retries after the window; `establish` unavailable during the window retries until the startup deadline, then the stage `failed` |
| Own reload, restart | Mounts unregister (in-flight tool call → "tool provider unavailable"; `workflow.*` → `service_reloading`). On unload the old generation stops running stage turns (bounded wait, A13). The new generation loads runs: `running`/`stopping` → `interrupted`; `launching`/`running` stages → `lost`; nothing relaunched. A `pending` report is looked up (`Accepted` → `delivered`, `Unknown` → written now); interrupted runs with `report_to` and `report: none` get one report (E15) |

## Appendix B. Full conflict table

| Scenario | Sequence order | Outcome | Owner |
| --- | --- | --- | --- |
| Starter interrupt before the call is dispatched | interrupt < `step/tool-call` | Not executed; no run | ToolExecutor |
| Starter interrupt while the handler runs | `step/tool-start` < interrupt | Persisted run continues; cancelled before persistence → no run | Workflow |
| Starter interrupt after the tool result | result < interrupt | Run continues | Workflow |
| Starter branch-scoped interrupt or branch close | close < report | Run continues; report `undeliverable` | session-access, Workflow |
| Tool batch not settled when the run ends | report < `step/tool-result` | Steer consumed after the batch result | Loop |
| Concurrent Steer/Queue on the report branch | Timeline order | Applied by mode; report is an ordinary Steer | InputQueue |
| Report arrives while the starter is idle | — | Steer starts a turn | InputQueue, Loop |
| User withdraws the report input | report < `input/withdrawn` | Report counted `delivered`; not rewritten | InputQueue |
| Compaction between report write and consumption | input < compaction < consumed | Placed after the summary | ContextPreparation |
| Child branch cut between report write and consumption | input < cut < consumed | Child does not see it | InputQueue (child) |
| User stops the stage Session before consumption | input < interrupt | Stage `stopped`; input pending in that Session unless withdrawn | Workflow |
| Stage input withdrawn | input < `input/withdrawn` | Stage `stopped` | InputQueue, Workflow |
| `workflow.stop` races the stage's `turn/end` | `turn/end` < `stop/interrupt` | Stage `completed`; run `stopped` | Stage Session order |
| `workflow.stop` races consumption | consumed < withdraw request | Withdraw rejected; `stop` path | InputQueue, Workflow |
| `workflow.stop` twice or after the end | — | Idempotent | Workflow |
| Duplicate `start` | — | Two runs (A15) | Workflow |
| Member stage, leader `request`s the same member | stage input queued, not started | Both inputs queue in the member Session; `Busy` only once a turn runs; the stage result is the first `turn/end` after its own input's consumption | Collaboration, Workflow |
| Leader branch closed with a member stage | close < stage end | Member stage continues (member belongs to the Session); report `undeliverable` | Workflow |
| Leader Session closed with a member stage | — | Member closed by Collaboration → stage `failed: session_closed`; run `failed` | Collaboration, Workflow |
| Collaboration reload | — | Running member stages unaffected (driven through SessionAccess); `establish` retried until deadline | Workflow |
| Agents reload | — | `Reloading`; runs kept; tracking resumes | Workflow |
| Stage resource node offline | capabilities Pending | `launch` fails at deadline → `failed: node_unavailable`; a running stage waits in `idle` | Agents, rutis lifecycle |
| Stage Session capability reload mid-turn | turn ends `Failed` | Stage `failed` | Stage Loop |
| Starter Session unloaded at report time | — | `acquire` restores, then writes | AgentIndex |
| Workflow reload mid-run | — | `interrupted`/`lost`; one report | Workflow |
| Process restart mid-run | — | Same; committed unconsumed stage inputs stay in their Sessions | Workflow |
| Report write uncertain (`Reloading`, `PublicationUnknown`) | — | `lookup` before retry; never two reports | Workflow |
| Project definition edited after start | — | Run keeps its snapshot; next stage launch rechecks the snapshot's fingerprint → `failed: approval_revoked` | Workflow |
| Client detach | — | No effect | — |

## Appendix C. Errors and recovery

| Case | Result |
| --- | --- |
| Invalid definition file | Listed with issues; `start` → `invalid_definition` |
| Approval missing or stale | `approval_required` at start; `failed: approval_revoked` at a later launch |
| Declared policy above the cap | `invalid_definition` at start |
| `launch`/`establish` fails | Stage `failed`; run `failed` |
| Stage turn `Failed`/`Cancelled` | Stage `failed`/`stopped`; run `failed` (A4) |
| `SessionClosed`, `BranchMissing` while waiting | Stage `failed: session_closed` |
| Storage write failure | Run task stops before the guarded effect; settled `interrupted` on next load |

Codes are the tool error codes of A.2; management methods return them as `app_error`.

## Appendix D. Acceptance

Through the tool, management methods and session-access reads:

1. Sequential and parallel steps; result substitution; forward reference rejected at save.
2. Tool start returns `run_id`; the report Steer ends a `wait` and arrives after the batch result.
3. Stop: waiting stages `skipped`, unconsumed input withdrawn, running stage interrupted, run `stopped`, no report; repeated stop idempotent.
4. Stop racing a stage `turn/end` keeps the completed result.
5. Starter turn interrupt does not stop the run; starter branch closed → `undeliverable`.
6. Project definition in `.dimcode/workflows/`: unapproved → `approval_required`; approve; edit → next stage `approval_revoked`.
7. Restart and Workflow reload: `interrupted`/`lost`, no relaunch, exactly one report.
8. Agents reload and Collaboration reload mid-run: run completes normally.
9. Policy: tool start under WorkspaceWrite → stages at most WorkspaceWrite; management start → ReadOnly by default; `workflow` denied under ReadOnly.
10. `member` runner: member appears in the leader's Team; leader request racing a queued stage input handled per Appendix B.
11. Non-interactive stage Sessions have no `workflow` tool.

## Appendix E. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Definitions read per call, not cached | Judgment (file authority) | Cache with change detection |
| A2 | YAML with `schema: dim.workflow/v1` | Judgment | Other syntax |
| A3 | User directory `<dim home>/workflows/` | Judgment; overview §B.9 user source | Path decided by Settings |
| A4 | Stages or parallel groups (≤ 32 steps, ≤ 8 per group); fail-fast | Feature list ("stages"); limits judgment | Per-stage failure policies |
| A5 | Model precedence: stage > profile > purpose `workflow.stage` | Overview §B.9; precedence judgment | Other precedence |
| A6 | Rendered prompt ≤ 32 KiB; stored result ≤ 64 KiB | Judgment | Other bounds |
| A7 | Plain substitution templates | Judgment | Expressions need a sandbox |
| A8 | Tool only for interactive Sessions | Overview §6.2 (interactive Sessions); prevents nesting | Depth limit needed |
| A9 | Log ≤ 500 entries; last 100 runs kept | Judgment | Other retention |
| A11 | 4 running runs, 4 concurrent stages per run | Judgment | Other limits |
| A12 | Approval rechecked before each stage launch | Overview §6; MCP precedent | Check only at start |
| A13 | Unload waits up to 10 s for stage stops | Judgment | Other bound |
| A15 | `start` does not deduplicate | Judgment | Idempotency keys |
| A16 | Report preview ≤ 4 KiB | Judgment; `dim-agent-report` precedent | Other bound |

Decided: tool risk and stage policy cap (E2), `.dimcode/` (E11), report once after restart (E15), member approval forwarding by Collaboration (E6), `establish` (decisions §2), process storage (R8), withdrawal contract (R7), host-local approval service (R11).

## Appendix F. Open questions

| Question | Description |
| --- | --- |
| Overview method list | Overview §B.9 lists `workflow.list/save/start/status/stop`; this design adds `workflow.approve/revoke` (D11) |
| Launch contract | Agents supplement must define `launch` with `policy` and the starter-policy resolution; overview §C.2 still says Agents submits the first input (D17) |
| Collaboration `establish` | Must be registered in overview §C.2 Collaboration row |
