# AcpGateway

Status: Draft. Overall design: [overview §4 Plugin catalog](../overview.md#4-plugin-catalog); scope: [ADR 002](../../adr/002-plugin-scope.md) (CLI process). Phase: P3. Language: TypeScript first as a standalone program, then merged into the Rust CLI ([overview Appendix C](../overview.md#appendix-c-implementation-language)).

> **Before-callback paths withdrawn.** The before-callback behaviour this document describes (delaying the commit of `input/message`, adding blocks to it, or withdrawing it in its own batch) is withdrawn with Hooks ([#21](https://github.com/arcships/dim/issues/21)); before callbacks only transform a draft synchronously. These passages are removed in the P3 design review.

`dimcode acp` serves the Agent Client Protocol (ACP) over standard input and output so that editors can drive dim. It is a protocol adapter in the CLI process: it connects to the daemon over local RPC exactly like the CLI, maps each ACP session to a non-interactive dim Session, each ACP prompt to a `session.submit`, Session events to ACP `session/update` notifications, and dim approval requests to ACP permission requests. It opens no execution path in the daemon, writes nothing except through Rpc's built-in Session methods and `sessionMethods` services (dispatched by Rpc through the Session), and declares no event. The outcome of a prompt removed before it runs is decided from InputQueue's result fact `input/deleted`: when the effective `input/withdraw` was in the input's own batch (a before callback, for example a Hooks `user_prompt_submit` rule, [Hooks](Hooks.md)) it is the ACP stop reason `refusal`, with the optional `reason` of that `input/withdraw` shown as text; in every other case (the gateway's own cancel, or another client's withdraw) it is `cancelled`. Its only contracts are the RPC wire protocol and the ACP schema, both language-neutral data, so it has no Rust binding appendix ([ADR 002 D5](../../adr/002-plugin-scope.md#d5-contracts-are-language-neutral)).

## 1. Structure

```mermaid
flowchart LR
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
  classDef contract stroke-dasharray:4 3
  Editor["Editor (ACP client)"]
  subgraph cli["CLI process"]
    GW["dimcode acp (AcpGateway)"]:::new
    Schema["Shared ACP schema and conformance suite"]:::contract
  end
  subgraph daemon["Daemon (App Server)"]
    Rpc["Rpc: local RPC protocol v3"]:::contract
    Agents["Agents: session.create with interactive false"]
    IQ["InputQueue: session.input.withdraw (sessionMethods)"]
    Appr["Approval: approval.respond"]
    Att["Attachments: attachment.put (sessionMethods key attachment)"]
    TLD["Session Timeline: input/message (before callbacks may delay its commit)"]
  end
  Editor <-->|"ACP JSON-RPC over stdio"| GW
  GW --- Schema
  GW -->|"local socket"| Rpc
  Rpc --> Agents
  Rpc --> IQ
  Rpc --> Appr
  Rpc --> Att
  Rpc -->|"session.submit"| TLD
```

The gateway depends on public RPC only; no daemon plugin knows it exists, and replacing it with another ACP implementation over the same RPC changes nothing in the daemon. The schema and suite are shared with [ExternalAgent](ExternalAgent.md) (client side); no runtime code is shared.

| Part | Scope | Depends on | Provides |
| --- | --- | --- | --- |
| `dimcode acp` | CLI process (not a daemon plugin) | Local RPC protocol v3 ([protocol.rs](../../../crates/dim-app-server/src/protocol.rs), [tree_protocol.rs](../../../crates/dim-app-server/src/tree_protocol.rs)): `hello`, `session.create` (with `interactive: false`, Agents supplement), `session.attach`/`detach`, `session.submit`, `session.input_lookup`, `session.history`, `session.stop`, `session.permission.update`, `approval.respond`; `session.input.withdraw` (InputQueue, `sessionMethods`) and `attachment.put` ([Attachments](Attachments.md), `sessionMethods` key `attachment`) ([session-boundary](../contracts/session-boundary.md)); the `input/withdraw {input_id, reason?}` and `input/deleted` data contracts ([session access A.4](../contracts/session-access.md#a4-withdrawal)); notifications `session.event`, `session.state`, `session.model.progress`, `approval.request`, `approval.cancelled`, `session.closed`; CLI daemon discovery | ACP agent on stdio; the ACP↔dim mapping of this document |

## 2. Data and state

```mermaid
flowchart TB
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
  classDef contract stroke-dasharray:4 3
  Map["ACP sessionId = dim session_id (gateway memory)"]:::new
  Prompt["In-flight prompt: acp request id, submission_id, input_event_id, cancel requested, consuming turn"]:::new
  Perm["Open permission asks: approval_id to ACP request"]:::new
  Cursor["Last seen sequence of main, streamed text offset per request"]:::new
  TL["dim Timeline of the Session (daemon)"]:::contract
  TL -->|"session.event in sequence order"| Prompt
  TL -->|"session.history pages after reconnect"| Cursor
  TL -->|"approval.request / approval.cancelled"| Perm
  Map --> Prompt
```

All gateway state is CLI process memory derived from notifications and `session.history`; it affects no branch. After a lost connection the gateway re-attaches and rebuilds from history after the last seen sequence; open permission asks lost on gateway exit leave the approvals pending in dim for another client.

## 3. Flows

### 3.1 New session and prompt

```mermaid
sequenceDiagram
  participant E as Editor
  participant G as Gateway
  participant R as Rpc
  participant TL as Session Timeline (main)
  E->>G: session/new {cwd, mcpServers}
  G->>R: session.create {cwd, agentId?, interactive: false}, then session.attach
  G-->>E: sessionId, modes (mcpServers ignored)
  E->>G: session/prompt {prompt}
  G->>R: session.submit {branchId main, submissionId, user, mode queue}
  R->>TL: validate, append input/message (before callbacks may add blocks or withdraw it)
  R-->>G: committed event: input_event_id
  alt withdrawn in its own batch (before callback)
    TL-->>G: input/withdraw {input_id, reason}, then input/deleted
    G-->>E: agent_message_chunk with the reason, prompt response refusal
  else withdrawn later by another client
    TL-->>G: input/withdraw in a later batch, then input/deleted
    G-->>E: prompt response cancelled
  else
  TL-->>G: input/applied, turn/start, input/consumed {input_event_id}
  TL-->>G: progress, step/text, step/tool-use-*, step/tool-result
  G-->>E: session/update chunks and tool_call updates
  TL-->>G: turn/end of the consuming turn
  G-->>E: prompt response end_turn, cancelled, or error turn_failed
  end
```

A prompt whose `input/deleted` arrives ends at once. If the effective `input/withdraw` was in the input's own batch (only a before callback can put it there, for example a Hook rule), the response is `refusal` and the reason, if any, is sent as an `agent_message_chunk`; if it came in a later batch (the gateway's own cancel or another client's withdraw), the response is `cancelled`. The input is in history only as deleted, no turn runs, and the ACP session stays usable. The gateway decides on `input/deleted`, reading the withdraw's batch and `reason` only to choose between the two; it never acts on the withdraw command alone. The consuming turn of an admitted prompt is the turn open on `main` at the sequence of `input/consumed {input_event_id}`; its `turn/end` decides the response (`Completed` → `end_turn`, `Cancelled` → `cancelled`, `Failed` → `turn_failed`). Every decision uses notifications in per-branch sequence order or `session.history` pages, never wall clock.

### 3.2 Cancel

```mermaid
sequenceDiagram
  participant E as Editor
  participant G as Gateway
  participant R as Rpc
  participant TL as Session Timeline (main)
  E->>G: session/cancel
  alt input/applied not yet seen
    G->>R: session.input.withdraw {sessionId, main, inputId}
    R->>TL: input/withdraw, InputQueue writes input/deleted (still held)
    G-->>E: prompt response cancelled
  else applied, or withdraw replied notHeld
    G->>R: session.stop {sessionId, main}
    R->>TL: stop/interrupt (turn scope) through the Session facade
    TL-->>G: turn/end of the consuming turn
    G-->>E: prompt response cancelled
  end
```

Withdrawal is tried first because the deleted input never enters model context; once InputQueue has applied the input (`input/applied`) the command has no effect and the reply is `notHeld`, and the gateway falls back to `session.stop`. After `session/cancel` the prompt always responds `cancelled` (ACP rule), whatever the turn's outcome; pending ACP permission requests of the Session are answered `cancelled` by the client side.

### 3.3 Permission

`approval.request {sessionId, branchId, requestId, source, call {name, input}, reason}` from any branch of the Session → `session/request_permission {sessionId, toolCall: {toolCallId: call_id or requestId, title: name, kind, rawInput: input}, options: [{optionId: "allow", kind: "allow_once"}, {optionId: "deny", kind: "reject_once"}]}`. Selected `allow` → `approval.respond {decision: allow}`; `deny` → `deny`; outcome `cancelled` → no answer (A10). Approval validates the answer at its boundary, writes `approval/respond`, consumes it by sequence and rechecks the current policy. No `*_always` option is offered.

### 3.4 Load and multi-client

`session/load {sessionId, cwd}` attaches and replays `main` history as updates before responding (`cwd` mismatch → `invalid_params`). Several ACP sessions share one connection and stream independently. A Session attached in the terminal CLI too streams the CLI's turns to the editor; the editor's Queue prompt waits behind them.

## 4. Timing and conflicts

```mermaid
sequenceDiagram
  participant G as Gateway
  participant IQ as InputQueue
  participant L as Loop
  participant TL as Timeline (main)
  Note over TL: input/message at seq n (editor prompt)
  alt withdraw lands first
    G->>IQ: session.input.withdraw
    IQ->>TL: input/withdraw at n+1, input/deleted at n+2
    Note over G: prompt cancelled, never applied
  else adoption lands first
    IQ->>TL: input/applied at n+1
    G->>IQ: session.input.withdraw: input/withdraw, no effect, notHeld
    L->>TL: turn/start, input/consumed
    G->>TL: session.stop writes stop/interrupt through the Session facade
    L->>TL: turn/end Cancelled
    Note over G: prompt cancelled after turn/end
  end
```

Withdraw versus adoption is decided by InputQueue on Timeline order; the gateway never guesses from timing. The narrow case "adopted, stop lands before consumption" leaves the input pending per the interrupt contract (Appendix B).

| Scenario | Order | Outcome | Owner |
| --- | --- | --- | --- |
| Another client withdraws the editor's input | `input/withdraw` in a later batch than the input < `input/deleted` of `input_event_id` | Prompt `cancelled` | InputQueue, Gateway |
| Turn completes before the stop lands | `turn/end` < `stop/interrupt` | Prompt `cancelled` (cancel was requested); the stop covers no turn | Loop, Gateway |
| Model asks a question in a loaded interactive Session | `question/asked` | Shown as a `tool_call` update "waiting for an answer in another dim client" (A13); the prompt waits; `session/cancel` interrupts and Questions writes `question/cancelled` | Questions, Gateway |
| Approval answered by another client | `approval/respond` earlier | The editor's later answer is rejected; ignored | Approval |
| Daemon restart mid-turn | turn settled `Failed` | Catch-up from history; `turn_failed` (or `cancelled` if cancel was requested) | Loop |
| A Hooks `user_prompt_submit` rule withdraws the prompt | input, `input/withdraw {reason}` in one batch < `input/deleted` | Reason as `agent_message_chunk`; prompt response `refusal`; no in-flight prompt remains | Hooks, InputQueue, Gateway |

All other rows: [Appendix B](#appendix-b-full-conflict-table).

## 5. Context mapping

| Source | Model sees | Placement |
| --- | --- | --- |
| ACP prompt `text` | `Text` user content | One user message at the Queue input's consumed position |
| ACP `image` | `Image` (base64); whether the model accepts images is handled by ModelProvider when it builds the request | Same message |
| Embedded text `resource` ≤ 32 KiB | `Text` `"<uri>\n<text>"` (A6) | Same message |
| Larger `resource` | `attachment.put`, then a text reference to the returned path (A6) | Same message |
| `resource_link` | `Text` line `"@<uri>"` (A6) | Same message |
| `session/set_mode` | Nothing: `permission/changed` is Session configuration, not model text | — |

The gateway writes no raw blocks, keys, prompt sources or tool declarations; a before callback may add its own attributed raw blocks (Hooks `user_prompt_submit` context) to the same input, placed before the gateway's user content. Its inputs survive compaction only through the summary; child branches see them if consumed before their cut. ACP Sessions are non-interactive, so `ask_user` is declared but answers that no one is available to answer; otherwise the context is the same as any Queue input. The model never sees ACP identities, `mcpServers`, client capabilities, updates or diagnostics.

## 6. Interfaces (summary)

| Interface | Kind | Purpose |
| --- | --- | --- |
| `dimcode acp [--agent <agent_id>]` | CLI entry | Start the ACP agent on stdio ([A.1](#a1-cli-entry)) |
| ACP methods `initialize`, `session/new`, `session/load`, `session/prompt`, `session/cancel`, `session/set_mode` | ACP (agent side) | Mapped to RPC ([A.2](#a2-acp-methods)) |
| `session/update`, `session/request_permission` | ACP (agent → client) | Updates and permission asks ([A.3](#a3-prompt-content-and-updates)) |
| ACP schema and conformance suite | Shared with ExternalAgent | Boundary validation and tests |

## 7. Security

The gateway has the authority of a local CLI client: the local socket's ownership checks apply, and it calls only Session methods plus `session.create`, never management methods, so editors cannot change host configuration (isolation, plugins). ACP modes change the Session's permission preset, which is Session configuration (`permission/changed`, decisions E4), bounded by what a local client may already do and copied with the facts on fork ([#26](https://github.com/arcships/dim/issues/26)). Permission answers go through `approval.respond`, where Approval rechecks current policy; `allow_always` is never offered. Client-supplied `mcpServers` are never started, so an editor cannot make the daemon run commands (documented ACP conformance deviation, E13). Embedded resources become user content and follow Timeline rules.

## 8. Rule check

Rules: [principles §1 and §4](../../architecture/rust-agent/principles.md), [ADR 002](../../adr/002-plugin-scope.md) D1–D5.

| Rule | How this document complies |
| --- | --- |
| §4.1 Command events for state owned by others | Cancellation is the command `input/withdraw` through InputQueue's `session.input.withdraw`, or `stop/interrupt` through `session.stop`; owners write the results, and a withdraw with no effect writes nothing and replies `notHeld`. |
| §4.2 No withdrawal after handover | After `input/applied` the gateway no longer withdraws and uses Loop's own command `session.stop`. |
| §4.3 Intervene before a write only through before callbacks | The gateway registers no before callback; it checks ACP content only at its own stdio boundary as a client, and Rpc keeps only parameter-shape checks. |
| §4.4 Core mechanisms carry no business meaning | The gateway asks no change of Timeline or any core mechanism; it uses only the generic optional `interactive` parameter of `session.create` ([session access A.3](../contracts/session-access.md#a3-sessionconfig-fields)), which carries no ACP behaviour. |
| §4.5 Read the whole batch before deciding | The gateway decides a prompt's outcome only from result facts (`input/deleted`, `input/consumed`, `turn/end`) received in sequence order, after the batch that carries them. |
| §4.6 Model context only from handed-over events | The gateway's prompt is ordinary user content that enters context only once consumed; a deleted prompt never does. |
| §4.7 Plugins interact only through public events and services | The gateway decides from InputQueue's `input/deleted`, reads only the public `reason` field of `input/withdraw` for display, and depends on no plugin-internal state or Hooks-specific event. |
| §4.8 Generic requirements to mechanism owners | Not applicable: no mechanism change is needed. |
| §4.9 Only facts later decisions use enter the Timeline | The gateway declares no event; it writes only the user's input and the commands `input/withdraw` and `stop/interrupt`, and its own state stays in CLI process memory. |
| §1 Capability and App Server boundary | The gateway is a CLI-process client, not a Capability or a daemon plugin; it needs only the optional `interactive` parameter of `session.create` ([session access A.3](../contracts/session-access.md#a3-sessionconfig-fields)). |
| ADR 002 D1 Scope follows the owner | Its state is per CLI process and per ACP connection, so it lives in the CLI process. |
| ADR 002 D2 No wiring modules | No wiring module; the gateway only calls public RPC. |
| ADR 002 D3 Dependencies follow scope | It has no in-process dependencies; daemon reloads appear as `service_reloading` and are retried. |
| ADR 002 D4 Structural and business configuration | Not applicable: its only option is the `--agent` command-line flag. |
| ADR 002 D5 Language-neutral contracts | Its contracts are the RPC wire protocol and the ACP schema, both language-neutral data. |

## 9. Requirements raised to mechanism owners

None. The optional `interactive` parameter of `session.create` is defined in [session access A.3](../contracts/session-access.md#a3-sessionconfig-fields).

## Appendix A. Interface details

### A.1 CLI entry

`dimcode acp [--agent <agent_id>]` connects to or starts the daemon like other CLI commands (A1); stdout carries only ACP JSON-RPC; diagnostics go to stderr and the CLI log. `--agent` selects the Agent profile for `session/new` (default: the default Agent).

### A.2 ACP methods

| ACP | dim | Notes |
| --- | --- | --- |
| `initialize` | `hello {protocolVersion: 3}` once per process | Reply: `agentCapabilities { loadSession: true, promptCapabilities { image: true, audio: false, embeddedContext: true } }`, `authMethods: []` (A2) |
| `authenticate` | — | Not offered; the local transport authenticates |
| `session/new {cwd, mcpServers}` | `session.create {cwd, agentId?, interactive: false}`, then `session.attach` | `sessionId` = dim session id; `modes` = permission presets, current = the Session's (A3); `mcpServers` ignored (E13) |
| `session/load {sessionId, cwd, mcpServers}` | `session.attach`, then `session.history` of `main` from 0 | Replay before the response; `cwd` mismatch → `invalid_params`; an already-open session returns the same mapping; the Session keeps its own `interactive` fact |
| `session/prompt {sessionId, prompt}` | `session.submit {sessionId, branchId: "main", submissionId: <uuid>, user, mode: "queue"}` | Response when the consuming turn ends; `input/deleted` → at once `refusal` if the withdraw was in the input's own batch, else `cancelled`; a second prompt while one is in flight → `prompt_in_progress` (A4) |
| `session/cancel {sessionId}` | `session.input.withdraw {sessionId, branchId: "main", inputId: input_event_id}` if still held (no `input/applied` seen), else (or if the reply is `notHeld`) `session.stop {sessionId, branchId: "main"}` | Prompt responds `cancelled` |
| `session/set_mode {sessionId, modeId}` | `session.permission.update` | (A3) |

The gateway never calls client `fs/*` or `terminal/*`: dim works on its own resource node (A5).

### A.3 Prompt content and updates

`audio` and binary `resource` blocks are rejected with `unsupported_content`; nothing is submitted. `image` blocks are checked by the gateway as a client against the shared schema (media type, base64 data) before `session.submit`; Rpc checks only the parameter shape. Updates come from the attachment notifications of the mapped Session; only `main` produces message updates (A7):

| dim | ACP update |
| --- | --- |
| `session.model.progress` text and `step/text` (per request, Unicode scalar offset) | `agent_message_chunk` with the not-yet-sent suffix only |
| `step/thinking` | `agent_thought_chunk` |
| `input/message` with user content not submitted by this gateway session | `user_message_chunk` (A8) |
| `step/tool-use-start` | `tool_call {toolCallId: call_id, title: name, kind, status: pending}` |
| `step/tool-use` | `tool_call_update {rawInput, locations}` |
| `step/tool-start` | `tool_call_update {status: in_progress}` |
| `step/tool-result` (each call, published with the whole batch) | `tool_call_update {status: completed or failed, content: text of ToolResult.content bounded 16 KiB}` |
| `question/asked` (interactive Sessions only) | `tool_call_update` with text "waiting for an answer in another dim client" (A13) |
| Raw-block-only inputs, `exec/*`, `collaboration/*`, `context/*`, `branch/*`, `input/withdraw`, `input/deleted` | No message update (A8) |

`kind` from the tool name: `read`→`read`; `write`, `edit`→`edit`; `glob`, `grep`→`search`; `exec`→`execute`; `web_fetch`, `web_search`→`fetch`; else `other` (A9). `locations` uses a `path` input field when present.

### A.4 Mechanics per step

| Step | Path |
| --- | --- |
| Prompt write | `session.submit` → `Session::input` (user content, no blocks): `validate`, then `input/message` appended; a pending before callback may delay the commit, add blocks, or append `input/withdraw {reason}` to the batch; the reply carries the committed event |
| Adoption | InputQueue (Dispatcher) appends `input/applied`; Loop appends `turn/start`, `input/consumed` and runs the turn |
| Withdraw | `session.input.withdraw` (InputQueue's `sessionMethods`) writes `input/withdraw`; InputQueue consumes it in order and writes `input/deleted` if it still holds the input unapplied, else nothing (`notHeld`) |
| Stop | `session.stop` writes turn-scoped `stop/interrupt` through the Session facade and waits for settlement |
| Approval | `approval.respond` → Approval validates, writes `approval/respond`, consumes it by sequence |
| State | No `StateCell`, no Dispatcher; gateway memory only |
| Daemon reload | Calls return `service_reloading` (a `sessionMethods` service reloading with its Session plugin, or a structural reload); the gateway retries with the same `submissionId` after `session.input_lookup` (`accepted` → no resubmission) |
| Restart or lost connection | Reconnect, re-attach each mapped Session, read `session.history` after the last seen sequence; an in-flight prompt is resolved from those facts |
| Gateway exit (stdin EOF) | `session.detach` for each Session; running turns continue; nothing is stopped |

## Appendix B. Full conflict table

| Scenario | Order | Outcome | Owner |
| --- | --- | --- | --- |
| Cancel before the submit reply | — | Wait for the reply, then cancel | Gateway |
| Cancel before adoption | `input/withdraw` < `input/applied` | `input/deleted`; `cancelled`; nothing in history but the deleted input | InputQueue |
| Cancel after adoption, before consumption | `input/applied` < `input/withdraw` | No effect, `notHeld`; `session.stop`; the consuming turn ends `Cancelled` → `cancelled` | InputQueue, Loop |
| Adopted, stop lands before `turn/start` | `stop/interrupt` < `input/consumed` | Prompt `cancelled` once `session.stop` returns with the input unconsumed; the input stays pending (interrupt contract) and runs with the next driving input, mirrored as `user_message_chunk` (A11) | Loop, Gateway |
| Cancel after consumption | consumed < `stop/interrupt` | Turn `Cancelled`; `cancelled`; sent text stays | Loop |
| Turn completes before the stop | `turn/end` < `stop/interrupt` | `cancelled` | Gateway |
| Other client withdraws the input | `input/withdraw` in a later batch < `input/deleted` | `cancelled` | InputQueue |
| Other client stops the turn | `stop/interrupt` | `cancelled` | Loop |
| Other client's Queue input pending | Timeline order | The editor's prompt ends with the turn that consumed it | InputQueue, Loop |
| Steer from another client | consumed in the same turn | Prompt continues; content mirrored | Loop |
| Tool batch not settled | — | Completed updates only when the batch's results are published | ToolExecutor |
| Approval answered elsewhere | earlier `approval/respond` | Later editor answer rejected, ignored | Approval |
| Approval cancelled by stop while the editor asks | `approval/cancelled` | ACP request cannot be retracted; later answer ignored | Approval |
| Question in a loaded interactive Session | `question/asked` | Waits for another client; cancel interrupts → `question/cancelled` (A13) | Questions |
| ACP-created Session | — | Non-interactive: `ask_user` answers that no one is available to answer | Questions |
| Compaction during the prompt | — | No update; streaming unaffected | — |
| Child branch created by the model | — | No message updates; its approvals forwarded | Gateway (A7) |
| Session closed | `session.closed` | In-flight prompt `cancelled`; later `session_not_found` | Gateway |
| Session idle-unload while attached | — | Attachment keeps it loaded | AgentIndex |
| Resource node offline | — | The Session stays loaded (no reload, no Pending); calls on its node services fail `node_unavailable` and appear as failed tool results; the prompt ends when its `turn/end` decides (requirement on [#5](https://github.com/arcships/dim/issues/5)) | Session's node services, Loop |
| Hook withdraws the prompt | input, `input/withdraw {reason}` in one batch < `input/deleted` | Reason chunk; `refusal` | Hooks, InputQueue, Gateway |
| Daemon reload mid-submit | — | `input_lookup` decides resubmission | Gateway |
| Daemon restart mid-turn | turn settled `Failed` | `turn_failed` after catch-up | Loop |
| Duplicate `session/prompt` | — | `prompt_in_progress` | Gateway |
| Two ACP sessions load one dim Session | — | Same mapping; one in-flight prompt | Gateway |
| Editor disconnects | — | Detach (not close); turn continues; approvals wait for another client | Connection |

## Appendix C. Errors and recovery

| Error (`error.data.code`) | When |
| --- | --- |
| `session_not_found` | Unknown or closed session id |
| `prompt_in_progress` | Second concurrent prompt on one ACP session |
| `unsupported_content` | Audio or binary resources, or an image that fails the gateway's own content check at the stdio boundary |
| `turn_failed` | The consuming turn ended `Failed` and no cancel was requested |
| `daemon_unavailable` | Connection lost and not re-established within 30 s (A12) |
| `invalid_params` | ACP parameter validation, `cwd` mismatch on load |

Parameters are validated at the stdio boundary against the shared schema. Gateway exit settles nothing in the daemon.

## Appendix D. Acceptance

Flow tests driving `dimcode acp` over stdio against a mock-model daemon, plus the shared conformance suite (agent side):

1. initialize, new, prompt → chunks without duplication across progress and committed text; `end_turn`; the created Session is non-interactive, and `ask_user` answers that no one is available to answer.
2. Tool call updates; permission allow and deny; approval answered by another client.
3. Cancel before adoption (deleted), after adoption (`notHeld`, stop), after consumption; stop racing turn end; another client's withdrawal.
4. `session/load` replays history in order; a loaded interactive Session's question shows the waiting update and cancel ends it.
5. Daemon restart mid-prompt: catch-up, `turn_failed`; reload mid-submit with `input_lookup`.
6. Images and embedded resources; audio rejected; `mcpServers` ignored.
7. A denying Hooks `user_prompt_submit` rule makes `session/prompt` end with `refusal` after an `agent_message_chunk` carrying the `input/withdraw` reason; a withdrawal by another client ends `cancelled` without a reason chunk; a withdrawal after the gateway's own cancel ends `cancelled`; the next prompt on the same ACP session works.
8. Editor EOF detaches without stopping the turn.

## Appendix E. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | `dimcode acp` connects to or starts the daemon like other CLI commands | Overview §4; start behavior is judgment | Requires a running daemon |
| A2 | No ACP authentication methods | Judgment (local transport) | Add an auth method |
| A3 | ACP modes map to dim permission presets | Feature list (policy levels); judgment | No modes |
| A4 | One in-flight prompt per ACP session | Judgment | Queue prompts |
| A5 | Client `fs`/`terminal` capabilities unused | Overview §6.3 | Editor-buffer reads via the client |
| A6 | Resource conversions and 32 KiB threshold | Judgment | Other renderings |
| A7 | Message updates only from `main`; approvals from all branches | Judgment (ACP has no branches) | Branch-aware updates |
| A8 | Mirror other clients' user content; not raw-block inputs | Judgment | Show runtime inputs |
| A9 | Tool kind by name table | Judgment | Kind from tool metadata |
| A10 | Client `cancelled` permission outcome sends no answer | ACP cancel semantics; judgment | Answer `deny` |
| A11 | An input adopted before the stop stays pending and runs later | Interrupt contract | Requires withdrawal of adopted inputs |
| A12 | Reconnect window 30 s | Judgment | Other bound |
| A13 | Questions in loaded interactive Sessions are shown as waiting, not answered over ACP | Decisions E13; ACP has no question request | Map to an ACP elicitation if one is standardized |
| A14 | Deferred: no client label. `session.create` has no `client` parameter and `SessionConfig` no `createdBy` until something decides with them; ACP passes only `interactive: false` | [session access A.3](../contracts/session-access.md#a3-sessionconfig-fields) | — |
| A15 | Deferred with A14: "ACP Session display" (feature list, `/settings`) is a CLI display preference owned by the CLI and is not defined while there is no client label; if the CLI later needs a filter, it can use `interactive`; nothing in this adapter | Feature list names only the setting | Defined elsewhere; adapter unchanged |

## Appendix F. Open questions

| Question | Description |
| --- | --- |
| Client label | Deferred (A14): added to `session.create` when something decides with it |
| "ACP Session display" | Deferred (A15); confirm with the CLI settings design when it is needed |
