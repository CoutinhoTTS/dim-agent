# Hooks

Status: Draft. Overall design: [overview §5.7](../overview.md#57-hooks). Phase: P2 (user and project sources); plugin source after #3. Language: Rust.

Hooks is an adapter that maps command Hook configuration onto existing mechanisms; it adds no extension mechanism and no Timeline event. User-submit and Session-start Hooks run at the RPC boundary, pre-tool Hooks implement the admission check contract that Approval defines, post-tool, turn-end and notification Hooks run after the live [Session broadcast subscription](../contracts/session-broadcast-subscription.md), and injected output enters context only as input through [session-access](../contracts/session-access.md). Every command runs on the Session's resource node through the Session-bound HostPlatform.

## 1. User flows

1. The user adds a `pre_tool_use` Hook matching `write|edit` in the user configuration. The model calls `edit`; Approval admits by policy, then calls the admission check; Hooks runs the command with the call as JSON on stdin; the command exits 2 with a reason on stderr; the call's `ToolResult.content` is the denial with that reason; the other calls of the batch run normally.
2. A repository ships `.dim/hooks.json` (A2). `hooks.catalog` lists its Hooks as `approval: required`; none runs. The user reviews and calls `hooks.approve` with the shown token; from the next execution point on they run. Editing the file changes the fingerprint and they stop running until re-approved.
3. A `user_prompt_submit` Hook prints `{"context": "branch is release/1.4"}`; the submitted input carries that text as a raw block before the user's content. Another Hook exits 2; `session.submit` fails with `hook_rejected` and nothing is written.
4. A `session_start` Hook prints environment facts; `session.create` returns after it finished, and the facts stay in context as a keyed block, surviving compaction.
5. A `post_tool_use` Hook runs a formatter after `write`; its stdout context is appended for the next model request. A `turn_end` Hook prints `{"follow_up": "tests failed: ..."}`; Hooks writes a Queue input that starts one more turn, up to a limit (A14).
6. `hooks.disable_all` stops every Hook immediately; `dimcode hooks` and `/hooks` show and manage the catalog.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Hook configuration format, catalog from user, project and plugin sources | Approval/Permission policy, approval requests (Approval) |
| Host-local execution approval of project and plugin Hooks | OS isolation mechanics (HostPlatform supplement) |
| Matching, process execution, stdin/stdout/exit-code protocol, timeouts | Plugin installation and plugin-content discovery (PluginManager) |
| Implementing the admission check contract and the submission filter contract | Ordering of other submission filters or admission checks (their owners) |
| Writing Hook output as input through session-access | Replay of missed observation Hooks (never done) |
| Management methods contributed to Management | In-process Hook needs: plugins use `before` or the subscription directly |

## 3. Scope and dependencies

Single process plugin `HooksPlugin`. No Session or branch part: all Session values arrive as call parameters or through public interfaces (overview §3 constraint 5).

**Contracts depended on** (none names Hooks; any provider satisfying them may replace another):

| Contract | Owner | Status | Use |
| --- | --- | --- | --- |
| `AdmissionCheck` (§6.4) | Approval (consumer) | Proposed; overview §7.1 Approval supplement | Pre-tool Hooks |
| `SubmissionFilter`, `SessionStartObserver` (§6.5) | Rpc (consumer) | Proposed here as an Rpc supplement; not yet registered in overview §7.2 (§14) | User-submit and Session-start Hooks |
| `SessionEvents` | [session-broadcast-subscription](../contracts/session-broadcast-subscription.md) | P0 contract | Observation triggers |
| `SessionAccess` | [session-access](../contracts/session-access.md) | P0 contract | Fixed-prefix reads, input writes |
| `SessionResources::resolve(session_id) -> SessionResourceBinding { node, cwd, host_platform, isolation, .. }` | Agents | Proposed with the overview §7.1 FileStorage/HostPlatform/WorkspaceTools supplement (shared with ShellInput, Attachments) | Session-bound HostPlatform (`spawn`, `ProcessTree`) and isolation for command execution |
| Host-local approval store | Settings | Overview §7.2 supplement ("host-local approvals keep their own approval files") | Grants |
| Typed Settings operations, loader editable layer | Settings (#3) | Overview §7.2 | User Hooks, disable flags |
| Typed plugin-content source, content type `hooks` | PluginManager | Overview §5.12; `When`, absent until #3 | Plugin Hooks |
| Management method contribution | Management | Overview §5.16 | `hooks.*` |

**Contracts defined here**: the Hook configuration format (§6.1), the command protocol (§6.2), the `hooks.*` management methods (§6.6), the source marker `dim-hooks` and key namespace `hooks:` (session-access conventions).

| Part | Scope | Injects | Provides |
| --- | --- | --- | --- |
| `HooksPlugin` | Process | `SessionEvents`, `SessionAccess`, `SessionResources`, Settings services, `FileStorage`, `When<PluginContentSource>` | `AdmissionCheck` under name `hooks`, `SubmissionFilter` under name `hooks`, `SessionStartObserver` under name `hooks`, management contribution `hooks` |

## 4. Events

None (overview §6 "`hook/*`"). Each outcome is already recorded as another owner's fact or is not a fact:

| Outcome | Where it is recorded |
| --- | --- |
| Pre-tool deny | The call's error result inside `step/tool-result`, written by ToolExecutor from Approval's admission error |
| User-submit reject | Absence of the input; error returned to the RPC caller |
| Injected context, follow-up | `input/message` written through session-access (source `hooks`) |
| Execution records (start, exit code, duration, stderr excerpt) | Process diagnostics channel |

No later behavior depends on a Hook run except through these facts, so a Hook event would be derivable or irrelevant (time bus rule 3).

## 5. State

| State | Storage | Rebuild |
| --- | --- | --- |
| Catalog: parsed Hooks per source with id, event, matcher, command, fingerprint, validity | Memory | Recomputed from sources on `apply` and on source change (Settings watch, project file read per execution point (A6), plugin-content source version) |
| User Hooks and `disable_all`, per-Hook enable overrides | Loader editable layer via typed Settings operations (A3, A9) | Read by Settings |
| Execution grants | Host-local approval file `hooks-approvals.json` (A7) | Read per execution, never cached as authority |
| Follow-up counters | None; derived from the prefix (§7) | — |
| Running Hook processes | Memory, `ctx.effect` | Not rebuilt; terminated on unload |

No `StateCell`; nothing is rebuilt from Timeline.

## 6. Interfaces

### 6.1 Configuration format

One JSON document per source, the same shape for user configuration (stored under the `hooks` plugin configuration key), project file `.dim/hooks.json` (A2) and plugin content of type `hooks`:

```json
{
  "version": 1,
  "hooks": [
    { "id": "guard-writes", "event": "pre_tool_use", "tools": ["write", "edit", "mcp__fs__*"],
      "command": "./scripts/guard.sh", "timeout_ms": 10000, "on_error": "deny", "enabled": true }
  ]
}
```

| Field | Type | Rule |
| --- | --- | --- |
| `version` | `1` | Other values make the whole document invalid (A1) |
| `id` | string `[a-z0-9][a-z0-9_-]{0,63}` | Unique within the source; identity of grants and overrides |
| `event` | `session_start` \| `user_prompt_submit` \| `pre_tool_use` \| `post_tool_use` \| `turn_end` \| `notification` | Required (A4) |
| `tools` | string array of name patterns | Only for `pre_tool_use`/`post_tool_use`; `*` matches any run of characters, otherwise exact and case-sensitive; omitted = all tools (A5) |
| `kinds` | array of `approval` \| `question` | Only for `notification`; omitted = both (A15) |
| `command` | string | Run by the resource node's resolved execution shell with the Session `cwd` as working directory (A8) |
| `timeout_ms` | integer 100..=600000 | Default 60000 (A10) |
| `on_error` | `allow` \| `deny` | Only for blocking events; default `allow` (A11) |
| `enabled` | bool | Default `true` |

Invalid entries are listed as `invalid` in the catalog and never run; other entries of the same document are unaffected. No `env` field: credentials appear in configuration only as references, and Hooks resolves none (A8). Execution order for one event: user, then project, then plugin sources (plugins by id), then document order (A12).

### 6.2 Command protocol

| Channel | Content |
| --- | --- |
| stdin | One UTF-8 JSON object, then EOF: `{version: 1, event, hook_id, source, session_id, branch_id, cwd, resource_node, ...event fields}` |
| Environment | HostPlatform's filtered base environment plus `DIM_HOOK_EVENT`, `DIM_SESSION_ID`, `DIM_PROJECT_DIR` (A8) |
| Exit 0 | Success. stdout empty, or one JSON object (`decision`, `reason`, `context`, `follow_up`); non-JSON stdout counts as `context` for `session_start` and `user_prompt_submit`, otherwise ignored (A13) |
| Exit 2 | Block: for `pre_tool_use` and `user_prompt_submit` a deny with stderr (first 2 KiB) as reason; for other events treated as an error |
| Other exit, signal, timeout, spawn failure, stdout over 64 KiB | Error: diagnostics; blocking events apply `on_error` |

Event fields: `user_prompt_submit {prompt: text of the user content, images: count}`; `pre_tool_use {turn_id, request_id, call_id, tool, input, risk}`; `post_tool_use {turn_id, call_id, tool, input, result: {text, is_error}}`; `turn_end {turn_id, outcome: completed|failed|cancelled, reason}`; `notification {kind, approval_id | call_id, tool?}`; `session_start {}`. Outputs are size-bounded: `context` and `follow_up` each at most 16 KiB after UTF-8 truncation (A13).

### 6.3 Outcomes

| Event | Allow / success | Deny | `context` | `follow_up` |
| --- | --- | --- | --- | --- |
| `session_start` | — | Not supported (exit 2 = error) | Keyed raw block, Append (§9) | Ignored |
| `user_prompt_submit` | Submission continues | Submission rejected, nothing written | Raw block in the same input | Ignored |
| `pre_tool_use` | Next check; call admitted when all allow | `AdmissionVerdict::Deny`; error tool result | Ignored (A16) | Ignored |
| `post_tool_use` | — | Not supported | Append input after the batch | Ignored |
| `turn_end` | — | Not supported | Ignored | Queue input starting one turn, bounded (A14) |
| `notification` | — | Not supported | Ignored | Ignored |

Hooks never widen authority: no outcome admits a call that Permission or the user denied, and no output edits tool input.

### 6.4 Admission check (implemented; defined by Approval)

Approval owns this extension point; it knows no implementer. Proposed shape, registered as overview §7.1 Approval supplement:

```rust
/// Process service; key TypeKey::keyed_dynamic::<AdmissionCheck>(name); injected by
/// Approval as When<ProcessContributions<AdmissionCheck>>.
pub struct AdmissionCheck(pub Arc<dyn AdmissionChecker>);
pub trait AdmissionChecker: Send + Sync + 'static {
    fn check(&self, request: AdmissionRequest, cancel: CancellationToken)
        -> HostFuture<'_, AdmissionVerdict>;
}
pub struct AdmissionRequest {
    pub session_id: String, pub cwd: String, pub resource_node: Option<String>,
    pub interactive: bool, pub branch_id: String,
    pub turn_id: String, pub request_id: String, pub call_id: String,
    pub tool: String, pub input: JsonObject, pub risk: Option<String>,
}
pub enum AdmissionVerdict { Allow, Deny { reason: String }, Cancelled }
```

Approval's rules (its decision, stated here as the contract Hooks relies on): checks run after Permission returns Allow, or after an Ask was answered Allow and rechecked; checks run sequentially in registration order and the first `Deny` ends admission; the last cancellation check happens after the checks; a check that panics or whose provider is unavailable denies (A17). Hooks' implementation: run the matching `pre_tool_use` Hooks in order (§6.1); a block or an error with `on_error: deny` returns `Deny { reason: "Hook <source>/<id>: <reason>" }`; on `cancel` it terminates the process tree, awaits its release and returns `Cancelled`.

### 6.5 Submission boundary (implemented; defined by Rpc)

Proposed Rpc supplement (§14). Rpc calls registered filters sequentially in registration order inside `session.submit`, before `SessionAccess::relay`; Attachments registers before Hooks so Hooks sees final content (A18):

```rust
pub struct SubmissionFilter(pub Arc<dyn SubmissionFiltering>);   // keyed_dynamic by name
pub trait SubmissionFiltering: Send + Sync + 'static {
    fn filter(&self, ctx: SubmissionContext, draft: SubmissionDraft, cancel: CancellationToken)
        -> HostFuture<'_, Result<SubmissionDraft, SubmissionRejected>>;
}
pub struct SubmissionContext { pub session_id: String, pub branch_id: String, pub cwd: String,
    pub resource_node: Option<String>, pub model_accepts_images: Option<bool> }
pub struct SubmissionDraft { pub user: UserInput, pub blocks: Vec<SourceBlock>, pub mode: InputMode }
pub struct SubmissionRejected { pub filter: String, pub reason: String }   // RPC error `hook_rejected` for Hooks
pub struct SessionStartObserver(pub Arc<dyn SessionStarting>);
pub trait SessionStarting: Send + Sync + 'static {   // awaited by session.create before replying
    fn started(&self, lease: SessionLease, ctx: SubmissionContext, cancel: CancellationToken) -> HostFuture<'_, ()>;
}
```

### 6.6 Management methods (contributed to Management)

| Method | Parameters | Result |
| --- | --- | --- |
| `hooks.catalog` | `{cwd?, node?}` | Hooks per source: `id`, `source`, `event`, matcher, `command`, `enabled`, `approval` (`notRequired` \| `required` \| `approved`), `approvalToken`, `invalid?`; global `disabled` |
| `hooks.approve` | `{cwd, node?, source, id, approvalToken}` | Persists the grant; stale token → `hooks_settings_conflict` |
| `hooks.revoke` | `{cwd, node?, source, id}` | Removes the grant (A19) |
| `hooks.set_enabled` | `{source, id, cwd?, enabled}` | User source: edits the entry; project/plugin source: host-local override (A9) |
| `hooks.disable_all` | `{disabled: bool}` | Global switch in the loader editable layer |

CLI: `dimcode hooks` and `/hooks` call these methods. No Session methods, no tools.

## 7. Timeline and capability mechanics

| Path | Mechanics |
| --- | --- |
| Pre-tool | ToolExecutor consumes `step/tool-call` on its Dispatcher, registers the batch task and returns; per call it appends `step/tool-start` (`Timeline::append`) and awaits `Approval::admit`, which (after its own `append_registered` request path, if Ask) awaits the checks. Hooks runs outside every commit lock and Dispatcher handler. Deny/cancel return as admission errors; ToolExecutor writes the single batch `step/tool-result` (terminal fact of the initiator). Hooks writes nothing |
| User submit | Rpc filter chain, then `SessionAccess::relay` → `Session::input` → `TimelineContext::input` → `Timeline::append` (no registration callback). InputQueue's Dispatcher subscription appends `input/applied`; Loop appends `input/consumed` |
| Session start | `session.create` awaits observers after the Session exists, before replying; Hooks writes one Append `input` with its marker and keyed blocks through the lease |
| Observation | `SessionEvents::subscribe` with `types = {step/tool-result, turn/end, approval/request, question/asked}` (A15); the stream task (registered with `ctx.effect`) enqueues work per `(session, branch)` and returns; one worker per branch runs Hooks serially in sequence order (A20) |
| Fixed prefix | A worker calls `lease.prefix(branch, at_least = event.sequence)`; `post_tool_use` resolves tool names and inputs from the `step/tool-call` named by `call_event_id`; never reads capability state |
| Recheck before write | Before writing, the worker reads the prefix again and writes only if: the branch exists; for `post_tool_use` context, no `stop/interrupt` after the tool result; for `turn_end` follow-up, no `stop/interrupt` and no `input/message` after the `turn/end`, and fewer than N consecutive `dim-hooks kind="follow-up"` inputs since the last input with another source or user content (A14). Facts committed after that read precede the write in sequence and are resolved by InputQueue and Loop |
| Writes | `SourceAccess` bound to source `hooks`; `submission_id = hooks:<uuid>`; on `Reloading`/`PublicationUnknown`, `lookup` before one retry |
| Reload | Hooks reload: running processes are terminated in the `ctx.effect` disposer before it returns. Approval (injector of `AdmissionCheck`) reloads with its dependency, so in-flight batches settle as reload in ToolExecutor; Rpc reloads its filter chain (`service_reloading`); the subscription ends and the new generation subscribes from now on, skipping the window |
| Restart | Same as reload; no Hook is re-run; pending follow-ups are lost |

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Turn `stop/interrupt` while a pre-tool Hook runs | `step/tool-start` < interrupt | Token cancelled; process tree terminated and released; `Cancelled`; call result "Tool execution cancelled"; Hook output discarded even if it finished | Approval (cancel), ToolExecutor (result) |
| Interrupt before the batch reaches admission | interrupt < `step/tool-call` consumption | Not admitted; no Hook runs | ToolExecutor |
| Hook allowed, interrupt before start | checks done < interrupt | Approval's last cancel check settles cancelled | Approval |
| Branch-scoped interrupt or branch close during a pre-tool Hook | branch interrupt | As turn interrupt; branch close also closes Approval | Approval, ToolExecutor |
| Pre-tool Hook denies one call of a batch | — | That call's error result; other calls run; one `step/tool-result` after the whole batch | ToolExecutor |
| `post_tool_use` for a batch | `step/tool-result` | Fires only after the whole batch; one run per matching call; no partial results exist | Hooks on the broadcast |
| Post-tool context vs following interrupt | tool-result < interrupt < write | Recheck sees the interrupt: not written | Hooks |
| Turn-end follow-up vs user input | `turn/end` < user input < Hook write | Recheck sees the input: not written | Hooks |
| Follow-up written, then interrupt | follow-up < interrupt | Per session-access: consumed by the next turn or cancelled with the turn | Loop |
| Concurrent Steer/Queue/Append | Timeline order | Hook inputs applied by mode in sequence order | InputQueue |
| User-submit Hook and a concurrent submit | — | Each submission filtered independently; written in commit order | Rpc, Timeline |
| Compaction between context write and consumption | input < compaction < consumed | Placed at its consumed position after the summary | ContextPreparation |
| Compaction after consumption | — | Session-start keyed blocks replayed; other Hook inputs only via summary | History projection |
| Child branch cut between Hook input and its consumption | input < cut < consumed | Child does not adopt it; Hook does not re-run for the child | InputQueue (child) |
| Observation event missed across Hooks reload or daemon restart | dispatch during the window | Never replayed; no Hook runs | Hooks |
| Subscription `Gap` | — | Dropped events' Hooks never run; diagnostic recorded | Hooks |
| Duplicate delivery | — | Impossible within a subscription; a new generation receives only new events; workers also skip a `(session, branch, sequence)` already handled in this generation | `SessionEvents`, Hooks |
| Duplicate or retried `session.submit` | two calls | Hooks run per call (A21); the client's retry after an uncertain outcome is the client's (`session.input_lookup`) | Rpc, client |
| Duplicate `hooks.approve` | — | Idempotent for the same token; stale token rejected | Hooks |
| Approval revoked or file edited while a Hook runs | — | The running process finishes; later executions are denied | Hooks |
| Capability reload of Approval mid-check | — | Token cancelled; Hook terminated; batch settled as reload | ToolExecutor |
| Session unload during an observation worker | — | Lease pins the Session for the read and write; `get` returns none afterwards and the work is dropped | AgentIndex, Hooks |
| Session restore | `Loaded {restored: true}` | `session_start` does not run | Hooks |
| Resource node offline | HostPlatform import Pending | Session tools Pending, so pre-tool Hooks are not reached; observation Hooks fail to spawn (diagnostics); blocking events apply `on_error` | rutis lifecycle, Hooks |
| Client detach | — | No effect; a pending `session.submit` still completes its filter chain | — |

## 9. Context mapping

| Source | Form | Placement | Compaction | Child branches |
| --- | --- | --- | --- | --- |
| `session_start` context | One Append input: marker `<dim-hooks kind="session-start">` plus one raw block per Hook with key `hooks:session-start/<source>/<id>` | Next model request (Append) | Latest block per key replayed | Inherited while live at the cut |
| `user_prompt_submit` context | Raw blocks `<dim-hooks kind="user-prompt-submit" hook="...">` in the relayed user input, before user content | Consumed position of that input | Summary only | Inherited if consumed before the cut |
| `pre_tool_use` deny | Text of the error `ToolResult.content` | Call's position in the batch result | Summary only | Inherited before the cut |
| `post_tool_use` context | Append input, marker `kind="post-tool" call_id tool` | Next model request of a turn | Summary only | As inputs |
| `turn_end` follow-up | Queue input, marker `kind="follow-up"` | Start of the turn it drives | Summary only | As inputs |
| `notification` | Nothing | — | — | — |

Non-interactive Sessions: identical, except that Session-start and user-submit Hooks run only for `session.create`/`session.submit` (A22). The model never sees Hook configuration, commands, exit codes, stderr (except a deny reason), approvals, diagnostics or other plugins' events.

## 10. Errors and recovery

| Case | Result |
| --- | --- |
| Timeout | Process tree terminated; error; `on_error` for blocking events |
| Unapproved project/plugin Hook | Skipped as not runnable; diagnostic; never treated as an error that denies |
| Corrupt approval file | Fail closed: project/plugin Hooks do not run (A7) |
| Invalid document | Catalog marks it invalid; no Hook of that document runs |
| Write rejected (`BranchMissing`, `SessionClosed`) | Output dropped; diagnostic |
| Cancellation | Terminate tree, await release, then return (interrupt rule) |
| Process exit | `ctx.effect` disposer terminates running Hooks; nothing settled later |

Error codes: `hook_rejected` (submit), `hooks_settings_conflict`, `hooks_invalid`, `service_reloading`.

## 11. Security

User Hooks run without approval (the user configured them). Project and plugin Hooks run only with a host-local grant keyed by digest(node, canonical workspace, source, plugin id, Hook id) and a fingerprint `sha256("dim/hooks-execution/v1", key, entry without enabled)`, following the MCP precedent (ADR 001 trust); reading configuration grants nothing; replay, fork and restore never restore grants. Grants, overrides and isolation configuration never enter Timeline. Commands run on the Session's resource node with that node's isolation configuration applied by HostPlatform; a Hook is not admitted by Permission (it is not a tool). Approving configuration does not pin script bytes. stdin may contain user prompts and tool input; diagnostics record only Hook id, exit status, duration and a bounded stderr excerpt.

## 12. Acceptance

Through public interfaces (management methods, `session.create`/`session.submit`, tools, events):

1. Each exit-code and stdout case of §6.2 for each event, including timeout and `on_error`.
2. Pre-tool deny in a three-call batch: one error result, others executed, single `step/tool-result`.
3. Interrupt during a pre-tool Hook: process terminated before the result; result cancelled.
4. Project Hook: not run before approval; runs after; stops after the file changes; corrupt store fails closed.
5. Session-start context survives compaction; not re-run on restore.
6. Post-tool context after an interrupt is not written; turn-end follow-up stops at the limit and after user input.
7. Hooks reload and daemon restart: observation events in the window produce no runs and no duplicates.
8. Resource node offline: observation Hook spawn failure recorded; no Timeline write.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Versioned JSON document per source | Judgment | Format changes; semantics unchanged |
| A2 | Project file is `.dim/hooks.json` | Judgment | Path changes |
| A3 | User Hooks are Hooks plugin configuration in the loader editable layer | Overview §1, §7.2 (#3) | A separate user file with its own authority |
| A4 | Six events: session start, user submit, pre/post tool, turn end, notification | Overview §5.7 timing points; names are judgment | Event names change |
| A5 | Tool name patterns with `*` glob only | Edge-case reference; judgment | Regex matching |
| A6 | Project file is re-read at each execution point (no watcher) | ADR 001 MCP precedent | A watcher would be needed |
| A7 | Grants in `hooks-approvals.json`, format 1, same rules as MCP grants | ADR 001; overview §7.2 | Different store, same binding |
| A8 | Command runs via the node's shell in Session cwd with base environment plus three variables; no `env` field | Judgment; overview §9 credentials | Env configuration with references |
| A9 | Enable overrides for project/plugin Hooks are host-local | Judgment ("write by source") | Only user Hooks can be disabled individually |
| A10 | Default timeout 60 s, range 0.1–600 s | Judgment | Values change |
| A11 | Default `on_error: allow` (fail open) | Edge-case reference; judgment | Broken guards block work by default |
| A12 | Order: user, project, plugin, document order | Judgment | Other ordering |
| A13 | stdout JSON object or plain text, 64 KiB stdout, 16 KiB per output | Judgment | Limits change |
| A14 | `turn_end` follow-up starts a turn, at most 3 consecutive | Edge-case reference; Goal precedent (overview §5.2) | No follow-up support |
| A15 | Notification Hooks trigger on `approval/request` and `question/asked` (latter only when Questions is present) | Overview §5.11 attention cases; judgment | Different triggers |
| A16 | Pre-tool Hooks cannot inject context or modify input | Judgment (never widen authority) | Contract needs input rewrite rules |
| A17 | Approval treats an unavailable or panicking check as deny | Judgment (fail closed at admission) | Approval's choice |
| A18 | Attachments filter runs before Hooks | Coordination with Attachments; judgment | Hooks see raw attachments |
| A19 | `hooks.revoke` exists in addition to the overview's four methods | MCP precedent | Revocation via file edit only |
| A20 | Observation Hooks run serially per branch | Judgment | Injected order may differ from event order |
| A21 | Submit Hooks run per `session.submit` call, retries included | Judgment | Rpc must dedupe by `submission_id` first |
| A22 | Session-start Hooks run only for `session.create`, not for Sessions created by Agents for Scheduler, Workflow or Collaboration | Overview §5.7 ("at the RPC boundary") | An Agents creation observer is needed |

## 14. Open questions

| Question | Description |
| --- | --- |
| Rpc submission boundary | `SubmissionFilter`/`SessionStartObserver` are not registered in overview §7.2; they should be added as an Rpc supplement (shared with Attachments) |
| Relay blocks attribution | session-access `relay` has no marker; Hooks' blocks inside a relayed input are recognizable by element name only |
| Reload blast radius | A Hooks reload reloads Approval in every branch, failing running turns (same as process-tool-contribution) |
| Remote resource nodes | `SessionResources` is proposed (Agents); importing a remote node's HostPlatform depends on #5 |
| Plugin source | Depends on #3 and PluginManager's content-source contract |
