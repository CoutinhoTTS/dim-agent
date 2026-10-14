# AcpGateway

Status: Draft. Overall design: [overview §5.15](../overview.md#515-acpgateway). Phase: P3. Language: TypeScript first as a standalone program, then merged into the Rust CLI ([overview §11.3](../overview.md#113-assignment)).

`dimcode acp` serves the Agent Client Protocol (ACP) over standard input and output so that editors can drive dim. It is a protocol adapter in the CLI process: it connects to the daemon over local RPC exactly like the CLI, maps each ACP session to a dim Session, each ACP prompt to a `session.submit`, Session events to ACP `session/update` notifications, and dim approval requests to ACP permission requests. It opens no execution path in the daemon and writes nothing except through existing Session methods.

## 1. User flows

1. An editor launches `dimcode acp`, sends `initialize`, then `session/new {cwd}`. The gateway creates a dim Session in that directory and attaches to it. The editor sends `session/prompt`; text streams back as `agent_message_chunk`, tool calls as `tool_call`/`tool_call_update`; the prompt response arrives with `stopReason: "end_turn"` when the turn ends.
2. The model calls `write`; the Session's policy asks. The editor receives `session/request_permission` with Allow/Deny; the user picks Allow; the tool runs and its update follows.
3. The user presses stop in the editor (`session/cancel`). The running turn is stopped; the prompt response is `cancelled`.
4. The editor reopens a previous conversation with `session/load {sessionId}`. The gateway attaches and replays the main branch history as updates, then the editor continues prompting.
5. The editor opens two ACP sessions on the same connection and prompts both; each streams independently.
6. The same dim Session is attached in the terminal CLI. Turns started there stream to the editor too; the editor's prompt waits behind them (Queue).

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| ACP agent side over stdio: lifecycle, sessions, prompt turns, cancellation, permission requests | Turn execution, approval decisions, input adoption (daemon capabilities) |
| Mapping ACP content to submissions and Session events to ACP updates | Attachments' image validation and text attachments (Attachments, through RPC) |
| One local RPC connection, attachment and reconnection | ACP client side (ExternalAgent) |
| Conformance with the shared ACP schema and suite | Editor UI |

## 3. Scope and dependencies

**Contracts depended on**: the App Server local RPC protocol v3 (`hello`, `session.create`, `session.attach`, `session.detach`, `session.submit`, `session.input_lookup`, `session.history`, `session.stop`, `session.permission.update`, `approval.respond`, notifications `session.event`, `session.state`, `session.model.progress`, `approval.request`, `approval.cancelled`, `session.closed`; [protocol.rs](../../../crates/dim-app-server/src/protocol.rs), [tree_protocol.rs](../../../crates/dim-app-server/src/tree_protocol.rs)); `attachment.put` ([Attachments §6.2](Attachments.md)); the InputQueue withdrawal Session method (overview §7.1 InputQueue change, optional); the CLI's daemon discovery. **Contracts defined here**: the ACP↔dim mapping of this document and the `dimcode acp` entry. The ACP schema and conformance suite are shared with [ExternalAgent §6.5](ExternalAgent.md#65-shared-acp-schema-and-conformance); the gateway is the agent side.

| Part | Scope | Uses | Provides | Status |
| --- | --- | --- | --- | --- |
| `dimcode acp` | CLI process (not a daemon plugin) | Local RPC client | ACP agent on stdio | Proposed |

The gateway depends on public RPC only; no daemon plugin knows it exists, and replacing it with another ACP implementation over the same RPC changes nothing in the daemon.

## 4. Events

None. Inputs, stops and approval answers are written by their owners behind existing RPC methods (`input/message`, `stop/interrupt`, `approval/respond`). The gateway's session map and prompt state are process memory of the CLI process and affect no branch.

## 5. State

| State | Owner | Rebuild |
| --- | --- | --- |
| ACP session ↔ dim Session map (identity: ACP `sessionId` = dim `session_id`) | Gateway, memory | Re-established by `session/load` |
| Per session: in-flight prompt `{acp_request_id, submission_id, input_event_id, consuming_turn}`, open turn id, last seen sequence of `main`, streamed text offset per request | Gateway, memory | From attachment notifications and `session.history` pages after reconnect |
| Pending permission requests `approval request_id → ACP request` | Gateway, memory | Lost on gateway exit; approvals stay pending in dim |

## 6. Interfaces

### 6.1 CLI entry

`dimcode acp [--agent <agent_id>]`: connects to (or starts) the daemon like other CLI commands (A1); stdout carries only ACP JSON-RPC; diagnostics go to stderr and the CLI log. `--agent` selects the Agent profile for `session/new` (default: the default Agent).

### 6.2 ACP methods (client → agent)

| ACP | dim | Notes |
| --- | --- | --- |
| `initialize` | `hello {protocolVersion: 3, clientKind: other}` once per process | Reply: `agentCapabilities { loadSession: true, promptCapabilities { image: true, audio: false, embeddedContext: true } }`, `authMethods: []` |
| `authenticate` | — | Not offered; the local RPC transport authenticates (A2) |
| `session/new {cwd, mcpServers}` | `session.create {cwd, agentId?}`, then `session.attach` | Returns `sessionId` = dim session id; `modes` = dim permission policies, current = the Session's (A3). `mcpServers` are not connected (§14) |
| `session/load {sessionId, cwd, mcpServers}` | `session.attach`, then `session.history` pages of `main` from 0 | History replayed as updates (§6.4) before the response; `cwd` different from the Session's → `invalid_params`. Loading an already-open session returns the same mapping |
| `session/prompt {sessionId, prompt}` | `session.submit {sessionId, branchId: "main", submissionId: <uuid>, user, mode: "queue"}` | Response when the consuming turn ends (§7); a second prompt while one is in flight → error `prompt_in_progress` (A4) |
| `session/cancel {sessionId}` | Before consumption: withdraw the input (InputQueue method) if available; after: `session.stop {sessionId, branchId: "main"}` | Then the prompt responds `cancelled` (§7) |
| `session/set_mode {sessionId, modeId}` | `session.permission.update` | (A3) |

The gateway never calls `fs/*` or `terminal/*` on the client: dim works on its own resource node (A5).

### 6.3 Prompt content → `user`

| ACP `ContentBlock` | `UserInput.content` |
| --- | --- |
| `text` | `Text` |
| `image {data, mimeType}` | `Image` with base64 source; Attachments validates or rejects at the submission boundary |
| `resource` (embedded text) ≤ 32 KiB | `Text` `"<uri>\n<text>"` (A6) |
| `resource` larger | `attachment.put`, then a text reference to the returned path (A6) |
| `resource_link {uri}` | `Text` line `"@<uri>"` (A6) |
| `audio`, binary `resource` | Rejected: `unsupported_content`; nothing submitted |

### 6.4 Updates (agent → client `session/update`)

Sources are the attachment notifications of the mapped Session. Only `main` branch facts produce message updates (A7).

| dim | ACP update |
| --- | --- |
| `session.model.progress` text and `step/text` (per request, by Unicode scalar offset) | `agent_message_chunk` with the not-yet-sent suffix only |
| `step/thinking` | `agent_thought_chunk` |
| `input/message` with user content not submitted by this gateway session | `user_message_chunk` (A8) |
| `step/tool-use-start` | `tool_call {toolCallId: call_id, title: name, kind, status: pending}` |
| `step/tool-use` | `tool_call_update {rawInput, locations}` |
| `step/tool-start` | `tool_call_update {status: in_progress}` |
| `step/tool-result` (each call) | `tool_call_update {status: completed \| failed, content: [text of ToolResult.content, bounded 16 KiB]}` |
| Raw-block-only inputs, `exec/*`, `collaboration/*`, `context/*`, `branch/*` | Not mapped (A8) |

`kind` comes from the tool name: `read`→`read`; `write`,`edit`→`edit`; `glob`,`grep`→`search`; `exec`→`execute`; `web_fetch`,`web_search`→`fetch`; otherwise `other` (A9). `locations` uses a `path` input field when present.

### 6.5 Permission

`approval.request {sessionId, branchId, requestId, source: Tool{call_id,..}, call{name, input}, reason}` from any branch of the Session → `session/request_permission {sessionId, toolCall: {toolCallId: call_id, title: name, kind, rawInput: input}, options: [{optionId: "allow", kind: "allow_once"}, {optionId: "deny", kind: "reject_once"}]}`. `selected allow` → `approval.respond {decision: allow}`; `selected deny` → `deny`; `cancelled` → no response (A10). No `*_always` option: dim approval has no persistent rule here.

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Prompt write | `session.submit` → Rpc submission filters (Attachments, …) → `SessionLease::relay` → `Session::input` → `Branch::input` → boundary `validate` → `Timeline::append` of `input/message` (Queue). The reply carries the committed event; the gateway records `input_event_id` |
| Adoption | InputQueue (Dispatcher) appends `input/applied`; Loop appends `turn/start`, `input/consumed {input_id}` and runs the turn. The gateway sees these as `session.event` notifications in sequence order per branch |
| Prompt end | The consuming turn is the turn open on `main` at the sequence of `input/consumed {input_event_id}` (tracked from `turn/start`/`turn/end` order). Its `turn/end` decides: `Completed` → `end_turn`; `Cancelled` → `cancelled`; `Failed` → JSON-RPC error `turn_failed` with the error text |
| Not consumed | If a `stop/interrupt` follows the input on `main` and `session.state` reports idle without consumption, the prompt responds `cancelled`; the input is withdrawn when the withdrawal method exists, else it stays pending and joins the next turn (A11) |
| Cancel | `session/cancel` before consumption: withdraw (or §A11); after consumption: `session.stop` writes turn-scoped `stop/interrupt` through the Session facade and waits for settlement; the prompt ends by the `turn/end` rule |
| Approval | `approval.respond` → Approval service validates at the boundary and writes `approval/respond`; Approval consumes it by sequence on the branch Dispatcher |
| State and recheck | No `StateCell`, no Dispatcher. Every decision uses notifications in per-branch sequence order or `session.history` pages; wall clock is never used |
| Daemon reload | RPC calls return `service_reloading`; the gateway retries with the same `submissionId` after `session.input_lookup` (`accepted` → do not resubmit) |
| Daemon restart or lost connection | Reconnect, re-`attach` each mapped Session, read `session.history` after the last seen sequence to catch up; an in-flight prompt is resolved from those facts (its turn was settled by the restart) |
| Gateway exit (stdin EOF) | `session.detach` for each Session; running turns continue; nothing is stopped (detach is not close) |

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Cancel before submit reply | — | Wait for the reply, then cancel as below | Gateway |
| Cancel after submit, before consumption | input < cancel's effect | Withdrawn (or pending, A11); prompt `cancelled` | InputQueue, Gateway |
| Cancel after consumption | consumed < `stop/interrupt` | Turn ends `Cancelled`; prompt `cancelled`; text already sent stays | Loop |
| Turn completes before the stop lands | `turn/end` < `stop/interrupt` | Prompt `end_turn`; the stop covers no turn | Loop |
| Other client stops the turn | `stop/interrupt` | Prompt `cancelled` | Loop |
| Other client's Queue input pending | Timeline order | Editor's prompt runs in a later turn, or the same turn consumes both; the prompt ends with the turn that consumed it | InputQueue, Loop |
| Steer from another client during the prompt | consumed in the same turn | Same prompt continues; updates stream; the user content is mirrored | Loop |
| Tool batch not settled | — | `tool_call_update` completed only on the batch's single `step/tool-result` | ToolExecutor |
| Approval answered by another client | `approval/respond` earlier | Editor's later answer is rejected by `approval.respond`; ignored | Approval |
| Approval cancelled by stop while the editor asks | `approval/cancelled` | ACP request cannot be retracted; the later answer is ignored | Approval |
| Compaction during the prompt | — | No update; streaming unaffected | — |
| Child branch created by the model | — | Its facts produce no message updates; its approvals are forwarded | Gateway (A7) |
| Branch-scoped interrupt / Session close | `session.closed` | In-flight prompt `cancelled`; later calls `session_not_found` | Gateway |
| Session unloaded by idle while attached | — | Attachment keeps it loaded (existing attach rule) | AgentIndex |
| Resource node offline | capabilities Pending | Prompt waits; `turn/end` decides as usual | rutis lifecycle |
| Daemon reload mid-submit | — | `input_lookup` decides resubmission | Gateway |
| Daemon restart mid-turn | turn settled `Failed` by reload | Prompt error `turn_failed` after catch-up | Loop |
| Duplicate `session/prompt` | — | `prompt_in_progress` | Gateway |
| Two ACP sessions load the same dim Session | — | Same mapping; one in-flight prompt | Gateway |
| Editor disconnects | — | Detach; turn continues; pending approvals wait for another client | Connection |

## 9. Context mapping

The gateway places only user content: what the editor sends becomes the `user` content of a Queue input, rendered as one user message at its consumed position (§6.3 for each block kind). It writes no raw blocks, keys, prompt sources or tool declarations. The input survives compaction only through the summary; child branches see it if consumed before their cut. ACP modes change the permission policy, which is a host setting outside the Timeline and never model text. The model never sees ACP identities, `mcpServers`, client capabilities, updates or diagnostics.

## 10. Errors and recovery

| Error (`error.data.code`) | When |
| --- | --- |
| `session_not_found` | Unknown or closed session id |
| `prompt_in_progress` | Second concurrent prompt on one ACP session |
| `unsupported_content` | Audio or binary resources; image rejected by Attachments (`attachment_*` code passed through) |
| `turn_failed` | The consuming turn ended `Failed` |
| `daemon_unavailable` | Connection lost and not re-established within 30 s (A12) |
| `invalid_params` | ACP parameter validation, `cwd` mismatch on load |

Parameters are validated at the stdio boundary against the shared ACP schema. Gateway exit settles nothing in the daemon; there is nothing to settle.

## 11. Security

The gateway has the authority of a local CLI client: the local socket's ownership checks apply, and it calls only Session methods plus `session.create`. It does not call management methods, so editors cannot change host configuration through ACP. Permission answers go through `approval.respond`, where Approval rechecks current policy; `allow_always` is never offered. Client-supplied `mcpServers` are not started, so an editor cannot make the daemon run commands. Embedded resources become user content; secrets in them are the user's content and follow Timeline rules.

## 12. Acceptance

Flow tests driving `dimcode acp` over stdio against a mock-model daemon, plus the shared ACP conformance suite (agent side):

1. initialize, new, prompt → streamed chunks without duplication across progress and committed text; `end_turn`.
2. Tool call lifecycle updates; permission request allow and deny; approval answered by another client.
3. Cancel before consumption (with and without withdrawal) and after consumption; stop racing turn end.
4. `session/load` replays history in order; multiple ACP sessions on one connection.
5. Daemon restart mid-prompt: catch-up from history, `turn_failed`; reload mid-submit with `input_lookup`.
6. Images and embedded resources; audio rejected.
7. Editor EOF detaches without stopping the turn.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | `dimcode acp` connects to or starts the daemon like other CLI commands | Overview §5.15 ("like the CLI"); start behavior is judgment | Requires a running daemon |
| A2 | No ACP authentication methods | Judgment (local transport) | Add an auth method |
| A3 | ACP modes map to dim permission policies | Feature list (three policy levels); judgment | No modes |
| A4 | One in-flight prompt per ACP session | Judgment (ACP prompt turns are sequential) | Queue prompts |
| A5 | Client `fs`/`terminal` capabilities are not used | Overview §3.1 (resource node services) | Editor-buffer reads via the client |
| A6 | Resource conversions and the 32 KiB threshold | Judgment; Attachments for large text | Other renderings |
| A7 | Message updates only from `main`; approvals from all branches | Judgment (ACP has no branches) | Branch-aware updates |
| A8 | Mirror other clients' user content; do not mirror raw-block inputs | Judgment | Show runtime inputs too |
| A9 | Tool kind by name table | Judgment | Kind from a tool metadata contract |
| A10 | A client `cancelled` permission outcome sends no answer | ACP cancel semantics; judgment | Answer `deny` |
| A11 | Without withdrawal, an unconsumed prompt cancelled before its turn stays pending | Interrupt contract (inputs applied before an interrupt stay pending) | Requires the InputQueue withdrawal method |
| A12 | Reconnect window 30 s | Judgment | Other bound |

## 14. Open questions

| Question | Description |
| --- | --- |
| Client MCP servers | ACP expects agents to connect `mcpServers` given in `session/new`; dim's MCP configuration has its own authority and host-local approval. This design ignores them, which is a conformance deviation |
| Withdrawal method | Name and availability of the InputQueue withdrawal Session method (overview §7.1) |
| "ACP Session display" | The feature-list setting is not defined; presumably a CLI setting outside this adapter |
