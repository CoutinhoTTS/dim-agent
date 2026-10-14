# Modality

Status: Draft. Overall design: [overview §5.14](../overview.md#514-modality). Phase: P3. Language: TypeScript first for provider-executed categories; the OCR executor depends on the OCR runtime component ([overview §11.3](../overview.md#113-assignment)).

Modality is a process plugin that is the single entry for non-conversational multimodal work in all nine categories: image, video, speech, music, audio, 3D asset, motion, realtime and OCR. It reports availability per category, resolves the default model or executor, runs asynchronous tasks (submit, status, wait, cancel) stored in process storage, and holds realtime connections. It writes no Timeline events and contributes no model tool.

## 1. User flows

1. `dimcode modality` lists every category with its operations, availability per dimension (build, platform, runtime resource, adapter, configuration) and the current default model. Categories the build excludes are omitted from the listing and help; invoking one prints the reason (A1).
2. `dimcode modality set-default image <provider>/<model>` writes the default through Provider's purpose policy for the key `modality.image`.
3. `dimcode image generate --prompt "..." --out ./logo.png` submits a task, waits, retrieves the result and writes it to `./logo.png`. `--no-wait` prints the task id and returns.
4. `dimcode modality task list|status|wait|cancel <id>` inspects and controls tasks, including tasks submitted earlier or by another client.
5. `dimcode ocr scan.pdf` runs text recognition through the OCR runtime component on the control node (or `--node <id>`), and prints the text; `--json` prints the structured result.
6. `dimcode realtime --model <m>` opens a realtime connection, sends stdin lines as text frames and prints received events until closed.
7. The daemon restarts while a video task runs on a provider. The new generation resumes polling the provider job if a remote handle was stored; otherwise it marks the task `interrupted`. It never submits the job again.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Category and operation catalog, availability reporting | Conversational models, image understanding (vision models through `read`, overview §4) |
| Purpose keys `modality.*` and their validation before Provider stores policy | Storing model policy and credentials (Provider) |
| Adapter and host-executor extension points; built-in adapters | Provider accounts and connection records |
| Task records, result store, retention, cancellation, recovery | Writing files outside its own result store |
| Realtime connections bound to the opening client connection | Model-visible tools (none; see §9) |
| Management methods and process service for tasks | CLI rendering and output-file writing (CLI) |

## 3. Scope and dependencies

A single process part on the control node. No Session or branch part: Modality never reads Session state and is not driven by Sessions.

**Contracts depended on**

| Contract | Owner | Use | Status |
| --- | --- | --- | --- |
| Model selection by purpose (`ModelSelectionHost` with an opaque purpose key) | Provider ([overview §7.2](../overview.md#72-app-server-process-plugins)) | Resolve and store the default per `modality.*` key | Proposed supplement; the existing `ResolveModelSelection::resolve(explicit, cwd, cancel)` has no purpose |
| Non-conversational model connection | Provider | Resolve `(provider_id, model_id)` to a connection (`kind`, endpoint, headers, credentials) for an adapter | Proposed ([overview §11.3](../overview.md#113-assignment) prerequisite) |
| Process spawning with isolation | HostPlatform of the executing node ([HostPlatform](../../architecture/rust-agent/plugins/HostPlatform.md), isolation supplement in [overview §7.1](../overview.md#71-branch-and-session-capabilities)) | Start the OCR runtime component | Existing spawn; isolation proposed |
| Node import | rutis links (#5) | HostPlatform of a node other than the control node for OCR | Proposed |
| Trusted plugin storage (`pluginFiles`) | App Server ([README](../../../crates/dim-app-server/README.md#trusted-plugin-storage)) | Task records (UTF-8 JSON, atomic replace under lock) | Existing; binary result files need a binary write (proposed, §14) |
| Management method contribution | Management ([overview §5.16](../overview.md#516-management)) | Expose `modality.*` | Proposed (P0) |
| Diagnostics channel | Trace | Progress and adapter logs | Existing |

**Contracts defined here** (owner: Modality): category and operation identifiers; purpose keys `modality.<category>` and `modality.<category>.<operation>` (opaque to Provider); the `ModalityAdapter` and `ModalityHostExecutor` extension points; the `ModalityTasks` process service; the task record and result formats; the `modality.*` management methods. Adapters for further provider kinds may be contributed by other plugins under these contracts (`When<ProcessContributions<ModalityAdapter>>`); Modality knows no provider-specific semantics outside its built-in adapters, and Provider knows nothing about modalities.

| Part | Scope | Injects | Provides |
| --- | --- | --- | --- |
| `Modality` | Process (control node) | Provider purpose selection and connection resolution, `pluginFiles`, control-node HostPlatform, `When` imported HostPlatform per node, `When<ProcessContributions<ModalityAdapter>>`, `When<ModalityHostExecutor>` | `ModalityTasks` service; management contribution `modality.*` |

## 4. Events

None. Tasks and connections are not branch work, are not started by a branch, and affect no later Session behavior; overview §5.14 places tasks and results in process storage. Progress goes to the diagnostics channel. Should a Session consume a result, it does so through ordinary input (a user attaching or referencing the result file), which is already a fact.

## 5. State

| State | Where | Rebuild |
| --- | --- | --- |
| Task records | `modality/tasks/<task_id>/task.json` in Modality's product root on the control node | Read on `apply`; non-terminal records settled per §7 |
| Result files | `modality/tasks/<task_id>/result/` | Referenced by the record; deleted with it |
| `client_task_id` index | Derived from records on `apply`, kept in memory | Rebuilt from records |
| Running work (abort handles, child processes, poll loops) | Memory | Not rebuilt; settled per §7 |
| Realtime connections | Memory | Not rebuilt; closed on unload |
| Availability snapshot | Memory | Recomputed on `apply`, adapter or executor change, and Provider configuration change |
| Default models | Provider (purpose policy) | Provider's own storage |

Task record (stored format version 1; incompatible changes take a new version, [overview §9](../overview.md#9-cross-cutting-conventions)):

| Field | Type |
| --- | --- |
| `version` | `1` |
| `task_id` | string, `mt_<uuid>` |
| `client_task_id` | string or null |
| `revision` | integer, incremented on every write |
| `category`, `operation` | identifiers from §6.1 |
| `executor` | `{ kind: "provider", provider_id, model, adapter }` or `{ kind: "host", executor, node }` |
| `params` | object validated per operation; input files replaced by `{ name, media_type, bytes, sha256 }` and stored under `input/` |
| `state` | `queued`, `running`, `succeeded`, `failed`, `cancelled`, `interrupted` |
| `remote_handle` | string or null; adapter-defined job id, written before `running` is reported |
| `progress` | `{ fraction?: number, message?: string }` or null |
| `result` | `{ files: [{ name, media_type, bytes, sha256 }], data?: object }` or null |
| `error` | `{ code, message }` or null |
| `remote_cancel` | `"cancelled"`, `"unsupported"`, `"already_finished"` or null |
| `created_at`, `updated_at`, `finished_at` | RFC 3339 |

Retention: terminal tasks are deleted with their files after 30 days or when more than 200 terminal tasks exist, oldest first (A2). Non-terminal tasks are never deleted.

## 6. Interfaces

### 6.1 Categories and operations

| Category | CLI | Operations | Input | Result | Executor | Purpose key |
| --- | --- | --- | --- | --- | --- | --- |
| `image` | `image` | `generate`, `edit` (image, optional mask), `enhance` (`upscale`, `restore`, `denoise`, `sharpen`, `remove_background`, `colorize`) | prompt, files | image files | Provider | `modality.image` |
| `video` | `video` | `generate`, `edit`, `enhance` | prompt, files | video file | Provider | `modality.video` |
| `speech` | `speech` | `synthesize`, `transcribe` | text or audio | audio file; or `data: { text, segments: [{ start_ms, end_ms, text }] }` plus `transcript.txt` | Provider | `modality.speech.synthesize`, `modality.speech.transcribe` |
| `music` | `music` | `generate` | prompt | audio file | Provider | `modality.music` |
| `audio` | `audio` | `generate` | prompt | audio file | Provider | `modality.audio` |
| `asset` | `asset` | `generate` | prompt, optional images | 3D file | Provider | `modality.asset` |
| `motion` | `motion` | `generate` | prompt | motion data file | Provider | `modality.motion` |
| `realtime` | `realtime` | `connect` | — | connection (§6.5) | Provider | `modality.realtime` |
| `ocr` | `ocr` | `recognize` | image or PDF | `data: { pages: [{ index, text }] }` plus `text.txt` | Host runtime component | none; executor configured by installation |

`image read` is not Modality (overview §4). Per-operation keys are resolved first, then the category key (A3). Operation parameters beyond the input (size, count, duration, voice, format) are declared by each adapter in the capability report and validated at submission (A4).

### 6.2 Extension points (defined by Modality)

```rust
/// Process service; key TypeKey::keyed_dynamic::<ModalityAdapter>(name).
pub trait ModalityAdapterImpl: Send + Sync + 'static {
    fn connection_kinds(&self) -> Vec<String>;                 // Provider connection kinds it serves
    fn operations(&self, model: &ModelRef) -> Vec<OperationSpec>;  // category, operation, params schema
    fn submit(&self, job: Job, conn: ResolvedConnection, cancel: CancellationToken)
        -> HostFuture<'_, Submitted>;                          // Submitted { remote_handle: Option<String> }
    fn wait(&self, handle: Option<&str>, conn: ResolvedConnection, progress: ProgressSink,
        cancel: CancellationToken) -> HostFuture<'_, AdapterOutput>; // files written to the given dir, data
    fn cancel_remote(&self, handle: &str, conn: ResolvedConnection) -> HostFuture<'_, RemoteCancel>;
    fn realtime(&self, conn: ResolvedConnection, params: JsonObject, cancel: CancellationToken)
        -> HostFuture<'_, Option<RealtimeChannel>>;
}
pub enum RemoteCancel { Cancelled, Unsupported, AlreadyFinished }

/// Process service; key TypeKey::of::<ModalityHostExecutor>(). Host-executed categories.
pub trait ModalityHostExecutorImpl: Send + Sync + 'static {
    fn probe(&self, platform: &HostPlatform) -> HostFuture<'_, Availability>;
    fn run(&self, platform: &HostPlatform, job: Job, cancel: CancellationToken)
        -> HostFuture<'_, AdapterOutput>;
}
```

`wait` with a stored `remote_handle` must work in a new generation; that is what resume after restart uses. An adapter that returns no handle cannot be resumed.

### 6.3 Process service and management methods

`ModalityTasks` (in-process, for other plugins) offers the same operations as the management methods below with typed values. Management methods are contributed to Management and are available where it is mounted (local RPC entry by default):

| Method | Parameters | Result |
| --- | --- | --- |
| `modality.capabilities` | `{}` | `{ categories: [{ category, operations: [{ operation, available, reasons: [{ dimension: "build" \| "platform" \| "runtime" \| "adapter" \| "configuration", detail }], params_schema?, default_model? }] }] }`; every category is present |
| `modality.configure` | `{ category, operation?, model: "<provider>/<model>" \| null }` | Validates that some adapter serves that model for that operation, then writes the policy for the key through Provider; `null` clears it |
| `modality.task.submit` | `{ category, operation, params, files?: [{ name, media_type, data_base64 } \| { node_path }], model?, node?, client_task_id? }` | `{ task }`; with an existing `client_task_id`, the existing task, nothing new started |
| `modality.task.status` | `{ task_id }` | `{ task }` |
| `modality.task.list` | `{ state?, category?, limit? (≤100), after? }` | `{ tasks, next? }`, newest first |
| `modality.task.wait` | `{ task_id, timeout_ms? (≤600000, default 600000) }` | `{ task }` when terminal or at timeout (not an error); returns at once if terminal |
| `modality.task.cancel` | `{ task_id }` | `{ task }`; idempotent; a terminal task is returned unchanged |
| `modality.task.result` | `{ task_id, name, offset, length (≤4 MiB) }` | `{ data_base64, eof }` |
| `modality.realtime.open` / `send` / `close` | §6.5 | §6.5 |

Boundary validation: `files` total at most 100 MiB per submission, inline data at most 25 MiB per file (A5); media types checked against the operation; `node_path` only for host-executed operations and read on the executing node by the component; `node` only for host-executed categories, defaulting to the control node.

Model selection: explicit `model`, else the purpose key policy from Provider, else `configuration` unavailable. The resolved model is fixed in the record at submission.

Concurrency: at most 4 running tasks per category; others stay `queued` in submission order (A6).

### 6.4 CLI entries

| Command | Behavior |
| --- | --- |
| `dimcode modality` | Capability table; `--all` includes build-excluded categories with reasons |
| `dimcode modality set-default <category>[.<operation>] <provider>/<model>` / `--clear` | `modality.configure` |
| `dimcode modality task list\|status\|wait\|cancel` | Task methods; `wait --out <path>` retrieves results |
| `dimcode image generate\|edit\|enhance`, `video`, `speech synthesize\|transcribe`, `music`, `audio`, `asset`, `motion` | Submit; default waits and writes results to `--out` (file or directory) or the current directory; `--no-wait` prints the id; `--json` prints the record |
| `dimcode ocr <file> [--node <id>]` | Submit `ocr.recognize`; prints text |
| `dimcode realtime` | Opens a connection, relays stdin and events |

The CLI reads input files and sends them inline, and writes outputs itself from `modality.task.result`, so Modality never writes outside its own store (A7). The first Ctrl+C during a foreground wait stops waiting and prints the task id; the task continues; `dimcode modality task cancel` cancels it (A8).

### 6.5 Realtime

`modality.realtime.open { model?, params }` resolves the model, calls the adapter's `realtime` and returns `{ connection_id }`. `send { connection_id, frame: { type: "text" | "audio" | "control", data } }` forwards a frame. Events from the provider are pushed as `modality.realtime.event { connection_id, event }` notifications to the RPC connection that opened it. `close { connection_id }` closes it. A connection is bound to its opening RPC connection: it closes when that client disconnects, after 10 minutes without frames in either direction (A9), on Modality reload, and on process exit. It is never stored, never resumed and has no task record.

## 7. Timeline and capability mechanics

Modality has no Timeline path: no append, no Dispatcher subscription, no `StateCell`, no Session reads or writes. Ordering among competing writes is decided by the task record `revision` instead of a Timeline sequence.

| Step | Mechanism |
| --- | --- |
| Submit | Validate at the boundary; write the record `queued` (revision 1) and the input files; return. The scheduler task registered with `ctx.effect` starts it when a slot frees |
| Start | Write `running` with the resolved executor; call `submit`; write `remote_handle` as soon as it is known |
| Progress | Updates `progress` at most once per second (A10); also to diagnostics |
| Terminal | One write under the per-task lock: compare `revision`, write the terminal state with result or error. The first terminal write wins; later outcomes are discarded and their files removed |
| Cancel | Mark cancellation requested in memory, cancel the local token (aborts the HTTP request or terminates the component's process tree through HostPlatform), call `cancel_remote` when a handle exists, release resources, then write `cancelled` (with `remote_cancel: unsupported` when the provider cannot cancel) |
| Reload | The disposer cancels local tokens, terminates OCR processes and closes realtime connections, then writes `interrupted` for host tasks and for provider tasks without a `remote_handle`. Provider tasks with a handle stay `running` for the new generation |
| Startup (new generation or process restart) | Read all records. `queued`: `interrupted` (never started, never started later; A11). `running` host task: `interrupted`. `running` provider task with handle and an adapter able to resume: continue `wait` (observation only, no resubmission; A12). Otherwise `interrupted` with message "did not survive recovery" |
| Configuration change | Availability recomputed; existing tasks keep their resolved executor |

## 8. Timing and conflicts

Modality is not branch work. Turn and branch facts never affect tasks; only the rows below apply.

| Scenario | Order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Turn `stop/interrupt` or branch close in any Session | — | No effect; tasks are not started by branches | — |
| Session unload, restore or close | — | No effect | — |
| Compaction, child branch creation, Steer/Queue/Append | — | No effect; no model-visible output | — |
| Cancel before start | `queued` < cancel | `cancelled` without contacting the executor | Modality |
| Cancel and completion race | lock order | Whichever terminal write holds the lock first; the other is discarded | Modality (revision) |
| Cancel on a terminal task | terminal < cancel | Unchanged record returned | Modality |
| Duplicate `submit` with the same `client_task_id` | second after first | Existing task returned | Modality |
| Duplicate `cancel` | — | Idempotent | Modality |
| `wait` while Modality reloads | wait pending | Call fails `service_reloading`; the client calls `wait` again and sees the settled or resumed state | rutis lifecycle, client |
| Process restart mid-task | `running` record | §7 startup rule; never resubmitted | Modality |
| Retention deletes a task while a client waits for it | — | Terminal tasks only; `wait` already returned; `result` then fails `not_found` | Modality |
| Executing node goes offline before start | imported HostPlatform Pending | Task fails `node_unavailable` | Modality |
| Executing node goes offline mid-OCR | process lost | Task `interrupted` | Modality |
| Client detaches while waiting | — | Task continues; realtime connections of that client close | Modality |
| Provider credentials removed mid-task | — | Adapter call fails; task `failed` with `provider_error` | Modality |
| Default model changed after submission | — | Task keeps its recorded model | Modality |

## 9. Context mapping

No model-visible effect. Modality contributes no tool, prompt source or input, and its records are not Timeline facts. A result reaches the model only when a user puts it into a Session by ordinary means: referencing the result file path (`@`), attaching an image (Attachments), or pasting text; those are input facts handled by their owners. The model never sees task records, availability, default models, progress or diagnostics. Non-interactive Sessions are equally unaffected. A model-visible tool is not part of this design; a possible one is in §14 (A13).

## 10. Errors and recovery

| Code | Meaning |
| --- | --- |
| `invalid_input` | Unknown category or operation, parameter or file violation |
| `unavailable` | Category or operation unavailable; the `reasons` from `modality.capabilities` are included |
| `no_default_model` | No explicit model and no policy for the purpose key |
| `node_unavailable` | Requested node not imported or offline |
| `provider_error` | Adapter or provider failure, message stripped of secrets |
| `executor_error` | OCR component failed or produced invalid output |
| `not_found` | Unknown or deleted task, result file or connection |
| `storage_error` | Record or file write failed; the task is marked `failed` if the record can still be written, otherwise it is settled at the next startup |

Recovery is in §7. Billing side effects of provider jobs that were submitted and then interrupted are not undone; the record states `interrupted` and, if known, the `remote_handle`.

## 11. Security

- Management plane: all `modality.*` methods are management methods because they spend provider credit and run host processes; they are available only where Management is mounted ([overview §3 constraint 8](../overview.md#3-integration-methods)).
- Credentials: resolved by Provider per call; never in records, results, diagnostics or errors.
- OCR runtime component: an installed executable, not project content; started through the executing node's HostPlatform with that node's isolation configuration, read access to the input only, no network (A14). Its path is host configuration, never taken from a request.
- Files: inline inputs are size-checked and stored under the task directory; `node_path` is read by the component on its node with that node's permissions. Modality never writes outside its store.
- Result store files are created with owner-only permissions (`pluginFiles` precedent).

## 12. Acceptance

Flow tests through management methods with a fake adapter (controllable handle, delays, cancel support) and a fake host executor:

1. `modality.capabilities` lists all nine categories, each unavailable one with reasons per dimension; the CLI hides build-excluded categories unless `--all`.
2. `configure` writes the purpose key through Provider; submit without `model` uses it; a task keeps its model after the default changes.
3. Submit, status, list, wait (terminal and timeout), result retrieval in chunks with matching `sha256`.
4. Concurrency limit queues the fifth task of a category.
5. Cancel before start, during run (local abort, remote cancel, unsupported remote cancel), on a terminal task; cancel racing completion yields one terminal state.
6. Duplicate `client_task_id` returns the same task.
7. Reload and restart: host task `interrupted`; provider task with handle resumes and completes without a second `submit`; provider task without handle `interrupted`; `queued` task `interrupted`.
8. OCR on the control node and on an imported node; node offline before start and during run.
9. Realtime: open, send, events reach only the opening client, close on disconnect and on idle timeout.
10. No Timeline event, tool declaration or input appears in any Session during these tests.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Service reports every category; the CLI hides build-excluded ones by default and explains when invoked | Feature list ("show only capabilities supported by the current build"); overview §5.14 ("not silently hidden") | One of the two sources must change |
| A2 | Retention 30 days or 200 terminal tasks | Judgment | Disk use or history length differs |
| A3 | Purpose keys per category, per operation for speech, with per-operation override | Overview §5.14 ("by purpose (category)"); judgment | Speech synthesis and transcription share one default |
| A4 | Operation parameters are declared by adapters and validated at submission | Judgment | A fixed parameter schema per operation |
| A5 | 25 MiB per inline file, 100 MiB per submission | Judgment | Limits change |
| A6 | At most 4 running tasks per category | Judgment | Unbounded concurrency or another limit |
| A7 | The CLI sends inputs inline and writes outputs; Modality writes only its store | Overview §5.14 ("output location specified by the caller or the task result"); judgment | Modality needs write access to caller paths on some node |
| A8 | Ctrl+C during a CLI wait detaches rather than cancels | Judgment | Ctrl+C cancels costly tasks |
| A9 | Realtime idle timeout 10 minutes; connections bound to the opening RPC connection | Judgment | Connections outlive clients |
| A10 | Progress written at most once per second | Judgment | More storage writes |
| A11 | `queued` tasks are not started after restart | Overview §3 constraint 4, extended by judgment | Queued tasks would start after restart |
| A12 | A running provider task with a stored remote handle is resumed by polling after restart | Judgment: polling is observation, not re-execution | All running tasks become `interrupted` on restart |
| A13 | No model-visible tool in this design | Feature list defines only CLI commands; overview §3.2 does not list Modality among tool contributors | A tool contribution must be designed (§14) |
| A14 | The OCR component runs with input read access and no network | Judgment; overview §7.1 isolation | Weaker isolation for the component |

## 14. Open questions

| Question | Description |
| --- | --- |
| Model-visible tool | A process-level tool such as `modality_task` (risk `network`) could let the model generate images or transcribe audio, returning file paths or text in `ToolResult.content`. It would need Modality to write results on the Session's resource node, which a control-node process tool cannot reach ([tool contribution §6](../contracts/process-tool-contribution.md)) |
| Binary result storage | `pluginFiles` supports only UTF-8 text; result files need a binary write in Modality's product root |
| Notifications on management methods | Realtime needs server-pushed notifications bound to the calling RPC connection; Management's method contribution contract must support them |
| Provider catalog by category | Whether Provider's model catalog should tag non-conversational models by category, or adapters remain the only source of which model serves which operation |
| Build vs feature list | Overview §1 says builds constrain availability, not scope; the feature list places seven categories outside the standalone CLI's default scope. This design keeps them in scope and reports them unavailable by build |
