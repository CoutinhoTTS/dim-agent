# ShellInput

Status: Draft. Overall design: [overview §5.3](../overview.md#53-shellinput). Phase: P1. Language: Rust.

Executes `!command` and `!!command` typed by the user, at the RPC boundary, on the Session's resource node. The result of `!command` (command and bounded output) is written as Append input so it enters context with the next model request; the output of `!!command` goes only to the originating client and the diagnostics channel. ShellInput is a process plugin with no events; its methods are contributed to Management.

## 1. User flows

1. Idle Session, the user types `!git status`. The CLI calls `session.shell.run {mode: "record"}`. ShellInput starts the configured shell on the Session's resource node in the Session's working directory, streams output to the CLI, and on exit writes one Append input `<dim-shell-input kind="command" exit="0">$ git status ...</dim-shell-input>`. No turn starts. The next prompt the user sends starts a turn whose first model request contains the command output before the prompt.
2. A turn is running and the user types `!ls build`. The command runs in parallel with the turn; its Append input is consumed with the next model request of the running turn (after the current tool batch result, if any).
3. The user types `!!top -l 1`. Output is shown in the terminal only; nothing is written to the Session.
4. A command hangs. The user presses `Ctrl+C`; the CLI calls `session.shell.stop` for that run, not `session.stop`. The process tree is terminated; for `!` the partial output is recorded with `status="stopped"` (A6).
5. The user presses `Ctrl+C` to stop the model turn while `!make` runs. The turn is interrupted; the command keeps running and is recorded when it ends.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Running the command on the Session's resource node with that node's isolation | Command parsing (`!`/`!!` prefixes, history) in the client |
| Output capture, bounds, encoding, streaming to the originating client | Admission: user-initiated, no Approval (overview §5.3) |
| Writing the `!` result as Append input through session-access | Deciding when the input is consumed (InputQueue, Loop) |
| Run registry: query and stop of its own runs | Persistent shell state (`cd`, variables) between runs (A3) |

## 3. Scope and dependencies

| Part | Scope | Injects | Provides |
| --- | --- | --- | --- |
| `ShellInputPlugin` | Process (App Server) | `Arc<SessionAccess>`, `Arc<SessionResources>`, `Arc<SessionEvents>`, the process diagnostics channel | Management contribution `shell-input` with `session.shell.run`, `session.shell.stop`, `session.shell.list` |

**Contracts depended on**

| Contract | Owner | Status | Use |
| --- | --- | --- | --- |
| `SessionAccess` (`source("shell-input")`, `acquire`, `SessionLease::input`, `lookup`) | [session-access](../contracts/session-access.md) | P0 contract | Append input; Session pinning during a run |
| `SessionResources::resolve(session_id) -> SessionResourceBinding { node: Option<String>, cwd: String, host_platform: Arc<HostPlatform>, file_storage: Arc<FileStorage>, isolation: IsolationConfig }` | Agents (creation configuration) | Proposed here as part of the overview §7.1 FileStorage/HostPlatform/WorkspaceTools supplement and #5 (§14) | Session-bound HostPlatform, cwd, node isolation |
| `HostPlatform::spawn(ProcessSpec)`, `ProcessTree::{stdout, stderr, wait, terminate}`, `shell: ShellStatus` | HostPlatform | Existing; isolation parameter is the overview §7.1 supplement | Process start and tree termination |
| Shell argv construction of the `exec` tool | WorkspaceTools ([exec.rs](../../../crates/dim-agent-capabilities/src/workspace_tools/exec.rs)) | Existing internal function; to be exposed as a HostPlatform helper (A2) | Same shell semantics as `exec` |
| `SessionEvents` (`SessionNotice::Ended`) | [session-broadcast-subscription](../contracts/session-broadcast-subscription.md) | P0 contract | Learn that a Session with a running command closed (filter: no event types, notices only) |
| Management method contribution | Management | Overview §5.16 (contract defined by Management) | Method registration; per-call progress sink (A9) |

**Contracts defined here**: the `session.shell.*` management methods, the source marker `dim-shell-input` (kind `command`) and the `submission_id` prefix `shell-input:`. ShellInput never reads Session state or another plugin's internals; the values it needs (node, cwd, isolation) come from the `SessionResources` contract, fixed at Session creation.

## 4. Events

None (overview §6 "`shell/*`"). A run is not in-flight work of a branch: it is started by the user at the RPC boundary and is not settled by branch interrupt. The only later-relevant outcome of `!command` is the input, which is the existing `input/message`. `!!command` affects nothing later. Run records (command, exit code, duration, bytes, truncation) go to the diagnostics channel.

## 5. State

| State | Storage | Rebuild |
| --- | --- | --- |
| Runs: `run_id → {session_id, branch_id, mode, command, started_at, ProcessTree, cancel, lease}` | Memory, owned by the plugin generation through `ctx.effect` | Not rebuilt: a restart or reload terminates every run (§7) |
| Recorded results | `input/message` in the Session Timeline | Session restore |

## 6. Interfaces

### 6.1 Management methods

Mounted wherever Management is mounted (local RPC entry by default, overview §3 constraint 8).

| Method | Parameters | Result |
| --- | --- | --- |
| `session.shell.run` | `{sessionId, branchId?: "main", runId, command, mode: "record" \| "private"}` | On exit: `{runId, status: "completed" \| "failed" \| "stopped", exitCode?: number, signal?: string, output: string, truncated: bool, durationMs, recorded?: {sequence, submissionId} \| null}`. Progress: `{runId, stream: "stdout" \| "stderr", text}` chunks on the call's progress sink to the calling connection only |
| `session.shell.stop` | `{runId}` | `{stopped: bool}`; idempotent; `false` when the run already ended |
| `session.shell.list` | `{sessionId?}` | `[{runId, sessionId, branchId, mode, command, startedAt}]` of running runs |

| Error (`appError`) | Condition |
| --- | --- |
| `shell_unavailable` | `HostPlatform.shell` is `Unavailable` on the resource node (reason forwarded) |
| `shell_busy` | Another run of the same Session is running (A4) |
| `duplicate_run` | `runId` already used in this process generation |
| `invalid_params` | Empty command, command > 64 KiB, `runId` not `[A-Za-z0-9-]{1,64}` |
| `session_closed` / `branch_missing` | From `SessionAccess` at start |
| `node_unavailable` | `SessionResources` cannot resolve the node's services (offline, Pending) |
| `service_reloading` | ShellInput or a dependency is reloading |

`runId` is generated by the client so `stop` can be sent before the `run` reply arrives.

### 6.2 Execution

| Item | Rule |
| --- | --- |
| Node and services | `SessionResources::resolve(session_id)`; all host access through the returned `host_platform` |
| Shell | `HostPlatform.shell` of that node, argv built as for `exec` (A2) |
| Working directory | `binding.cwd` (Session cwd); a fresh process per run, no state carried (A3) |
| Environment | The node's service environment (`HostPlatform.env`), never the client's; plus `DIM_SESSION_ID` (A5) |
| stdin | Null (existing `spawn`); interactive programs see EOF |
| Isolation | `binding.isolation` applied by HostPlatform (overview §7.1); a Permission preset never relaxes it |
| Timeout | None; ended by `session.shell.stop`, Session close, plugin unload or process exit |
| Output | stdout and stderr interleaved in arrival order; decoded as `exec` does (invalid bytes as `<0xNN>`) |
| Bounds | Model-visible and reply `output`: the last 2000 lines within 50 KiB, the `exec` preview rule ([exec.rs](../../../crates/dim-agent-capabilities/src/workspace_tools/exec.rs) `truncate_preview`) (A7); progress chunks are not bounded in total but rate-coalesced (A9) |

### 6.3 Recorded input (`mode: "record"`)

Written once after the process ended, through `SessionAccess.source("shell-input")`:

```rust
lease.input(branch_id, SourceInput {
    submission_id: Some(format!("shell-input:{run_id}")),
    marker: Marker { kind: "command".into(),
        attrs: vec![("status", status), ("exit", code_or_signal), ("truncated", bool)],
        body: format!("$ {command}\n{bounded_output_without_terminal_escapes}"), key: None },
    blocks: vec![], user: None, mode: InputMode::Append,
})
```

ANSI/OSC escape sequences are removed from the model-visible body (A8); the reply keeps them for the terminal. No `key`: each run is a one-time observation, not standing context.

### 6.4 CLI entry (contract for the CLI layer)

`!cmd` → `session.shell.run {mode: "record"}`; `!!cmd` → `{mode: "private"}`; progress chunks are printed as they arrive; the reply prints the status line. `Ctrl+C` while a run is in progress sends `session.shell.stop {runId}` first; a second `Ctrl+C` falls through to the normal turn stop. Without Management on the entry (remote clients) the CLI reports that shell input is unavailable.

## 7. Timeline and capability mechanics

| Step | Mechanics |
| --- | --- |
| Start | Validate; reject `shell_busy`/`duplicate_run`; `SessionAccess::acquire(session_id)` (restores an unloaded Session) and keep the `SessionLease` for the run, so the Session is not unloaded while the result may still be written (A10); resolve the binding; `spawn`; register the run with its `CancellationToken` and `ProcessTree` in the generation's tracker; return control to the RPC task, which awaits the run |
| Run | A task reads both pipes into a bounded tail buffer and forwards chunks to the progress sink; `select!` on process exit, `cancel` (stop, Session close, unload) |
| Stop | `cancel` → `ProcessTree::terminate` (releases the whole tree) → status `stopped` with partial output |
| Write (`record`) | After resources are released: `lease.input(...)` → `Session::input` → `Branch::input` → `TimelineContext::input` → `Timeline::append` (boundary `validate`, no registration callback). Committed event returned as `recorded.sequence` |
| Uncertain write | `Reloading` or `PublicationUnknown`: `lease.lookup(branch, "shell-input:<runId>")` after recovery; `Accepted` → done, `Unknown` → write once more; never re-run the command |
| Consumption | Not ShellInput's: InputQueue applies Append (`input/applied {mode: append}`) per its Dispatcher subscription; Loop consumes it at the next model request of a running turn (`input/consumed`); while idle it stays pending until a driving input starts a turn |
| StateCell | None owned. The input is in InputQueue's `StateCell<InputSnapshot>` until consumed |
| Recheck | None: the write decision depends on no Timeline fact; ordering against interrupts, compaction and other inputs is decided by the input owners by sequence |
| Session close | `SessionNotice::Ended { reason: Closed }` for a Session with a run cancels that run (A11); no input is written. `Ended { Unloaded }` cannot occur while the lease pins the Session |
| Reload / restart | Unload disposer cancels all runs, terminates their trees, waits for them, and does not write inputs (the generation is going away; a half-done write is resolved by `lookup` only if the client retries). Restart: trees die with the daemon (`ProcessTree` drop kills the tree); nothing to settle because nothing was started on the Timeline |

## 8. Timing and conflicts

`X` = the recorded `input/message` (Append).

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Run during a turn, result before the next request | X < `step/model-start` | Consumed with that request, before its other new inputs by sequence | Loop |
| Result while a tool batch runs | X < `step/tool-result` | Consumed after the batch result, at the next request of the turn | Loop |
| Result while idle | X, no turn | Pending; consumed when the next Queue/Steer starts a turn; never starts one | InputQueue, Loop |
| Turn `stop/interrupt` before X | I < X | Run unaffected; X pending until the next turn | Loop |
| X applied, then interrupt before consumption | X applied < I | Stays pending; consumed by the next turn | Loop |
| X consumed by a turn that is then interrupted | consumed < I | Stays in history at its consumed position | Loop |
| Branch-scoped interrupt or branch close during the run | close < X | Run continues; write fails `BranchMissing`; reply `recorded: null`; output still returned | Runtime facade; ShellInput |
| Session close during the run | — | Run cancelled, tree terminated, nothing written | ShellInput |
| Concurrent user Steer/Queue and X | Timeline order | Each applied by its mode; X never reorders them | InputQueue |
| Concurrent `!` runs in one Session | — | Second rejected `shell_busy` (A4) | ShellInput |
| Compaction between X and consumption | X < compaction < consumed | X unaffected; placed after the summary at its consumed position | ContextPreparation |
| Compaction after consumption | consumed < compaction | Survives only through the summary (not keyed) | ContextPreparation |
| Child branch cut between X and consumption | X < cut < consumed | Child does not adopt X; parent consumes it | InputQueue (child) |
| Child branch cut after consumption | consumed < cut | Child inherits it in its prefix | History projection |
| `!!` run | — | No Timeline effect | — |
| ShellInput reload or process restart mid-run | — | Run terminated; no input; client gets `service_reloading` or a closed connection | ShellInput |
| Reload after commit, reply lost | X committed | Client retry with the same `runId` returns `duplicate_run` in the same generation; after restart the client looks up `shell-input:<runId>` via `session.input_lookup` before rerunning (A12) | Client |
| Session unloaded at start | — | `acquire` restores it, lease pins it for the run | AgentIndex |
| Resource node offline at start | — | `node_unavailable`; nothing started | SessionResources |
| Resource node lost mid-run | — | Pipes close or `wait` fails: status `failed`, reason node lost; for `record`, the partial output is written with `status="failed"` (A6) | ShellInput |
| Duplicate or late `stop` | — | `{stopped: false}` | ShellInput |
| Client detach or disconnect | — | Run continues and is recorded (`record`) or discarded (`private`); progress is dropped (A13) | ShellInput |

## 9. Context mapping

| Mode | What the model sees | Form and placement |
| --- | --- | --- |
| `!command` | `<dim-shell-input kind="command" status="completed" exit="0" truncated="false">` + `$ command` + bounded output | One raw block (user-role message) at the input's consumed position (`input/consumed`): the next model request of a running turn, or the first request of the next turn |
| `!!command` | Nothing | — |

No tool declaration, prompt source or key. After compaction the result survives only through the summary. Child branches inherit it only if consumed before the cut. Non-interactive Sessions: ShellInput is user-initiated and has no model-visible effect unless a client runs `!` there; the mapping is the same. The model never sees `runId`, `submission_id`, stderr/stdout separation, durations, diagnostics or terminal escapes.

## 10. Errors and recovery

| Case | Result |
| --- | --- |
| Spawn fails | Reply `status: "failed"` with the reason; `record` writes nothing (no output exists) |
| Non-zero exit | `status: "completed"`, `exitCode` set; recorded normally |
| Write rejected (`SessionClosed`, `BranchMissing`, `Rejected`) | Reply carries output and `recorded: null` with the error |
| Uncertain write | `lookup` then single retry (§7) |
| Unload, restart | Runs terminated, never re-executed |

## 11. Security

`!` and `!!` execute arbitrary commands as the service user on the resource node without Approval (overview §5.3): authority comes from Management being mounted on the entry; exporting Management grants host administration (overview §3.1). Isolation configured for the node applies (host-local configuration, never from Timeline). The command text and output are written to the Timeline for `!` and replayed to the model; the CLI warns nothing extra, users must avoid printing secrets. Environment is the service's, never forwarded from clients. Diagnostics records omit output bodies beyond the bounded tail (A14).

## 12. Acceptance

1. `!echo hi` while idle: Append input with the marker; no turn starts; the next prompt's first request contains it before the prompt.
2. `!` during a running turn with a tool batch: consumed after the batch result in the same turn.
3. `!!`: output returned, no Timeline change.
4. Turn stop during a run does not stop it; `session.shell.stop` terminates the tree (verified by process absence) and records `stopped`.
5. Output bounds: 10 MiB output yields the last 2000 lines within 50 KiB and `truncated="true"`.
6. Branch close during a run: `recorded: null`; Session close: tree terminated.
7. Reload and restart mid-run: tree terminated, nothing written, no re-execution.
8. Remote entry without Management: method unknown.
9. Interrupt rows of §8 through `session.stop` and `session.history`.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Methods are `session.shell.run/stop/list` with `mode` | Overview §5.3 (`run`, `stop`); `list` and `mode` are judgment | Method set changes |
| A2 | Shell and argv are those of `exec` | Judgment (one shell configuration) | ShellInput builds its own argv |
| A3 | Each run is a fresh process; no persistent shell | Judgment | A per-Session shell would need lifecycle and state |
| A4 | One running command per Session | Judgment | Concurrent runs, inputs in completion order |
| A5 | `DIM_SESSION_ID` is added to the environment | Judgment | Commands cannot identify the Session |
| A6 | Stopped or failed `!` runs record partial output with their status | Judgment | Nothing recorded unless completed |
| A7 | `exec` preview bounds (2000 lines, 50 KiB) | Existing `exec` constants; reuse is judgment | Separate limit |
| A8 | Terminal escapes stripped from model-visible text | Judgment | Escapes reach the model |
| A9 | Management supports a per-call progress sink to the calling connection | Judgment; overview §5.16 silent | Output only at completion |
| A10 | The run holds a `SessionLease` | Session-access §6; judgment | Session may unload; write uses `acquire` |
| A11 | Session close cancels its runs | Judgment; overview §5.3 names only stop and daemon exit | Runs outlive the Session and are discarded |
| A12 | Clients deduplicate by `runId` via `session.input_lookup` | Session-access §6 | Possible duplicate recorded input |
| A13 | Client disconnect does not stop a run | Lifecycle §5 ("connection close keeps Session work"); judgment | Disconnect stops runs |
| A14 | Diagnostics records carry only metadata and the bounded tail | Judgment | Full output in diagnostics |

## 14. Open questions

| Question | Description |
| --- | --- |
| `SessionResources` contract | Not yet in overview §7.1; needed by ShellInput, Attachments and Hooks to reach Session-bound services from process plugins without reading Session state |
| Remote node tree termination | Whether an imported HostPlatform kills process trees when the link drops (#5) |
| Overview §5.3 "terminated when the daemon exits" | Also terminated on ShellInput reload and Session close here (A11) |
| Progress delivery | Depends on the Management contribution contract (A9) |
