# ExternalAgent

Status: Draft. Overall design: [overview §B.10](../overview.md#b10-externalagent); scope: [ADR 002](../../adr/002-plugin-scope.md) (process). Phase: P3. Language: TypeScript first ([overview §F.3](../overview.md#f3-assignment)).

ExternalAgent makes locally installed coding agents (Claude Code, Codex, Cursor, OpenCode, ZCode and similar) usable as a kind of model connection. A Session whose model is an external agent runs through Loop and ModelProvider as usual: ExternalAgent registers the connection kind `external-agent` with Aimux's ModelHost (an Aimux supplement), turns each dim model request into one external turn and turns the external stream back into model output chunks. External agents use their own tools; their permission requests become ordinary dim approvals in the Session that runs the call. ExternalAgent writes no Timeline event of its own.

## 1. Structure

```mermaid
flowchart LR
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
  subgraph proc["Process, control node"]
    EA["ExternalAgent"]:::new
    MM["ManagementMethods: external_agents.*"]:::new
    Aimux["Aimux ModelHost (connection kinds, supplement)"]
    Rpc["Rpc method table"]
    Store["Process storage: records, external_agents config"]
    Agents["Agents: profiles, AgentIndex"]
    Settings["Settings: credential resolver, typed loader operations"]
    Provider["Provider: connection entries"]
  end
  subgraph node["Process, each node with an enabled agent"]
    Bridge["ExternalAgentBridge row"]:::new
    HP["HostPlatform (launch, isolation)"]
  end
  subgraph branch["Branch of the calling Session"]
    MP["ModelProvider (ModelCallScope)"]
    Appr["Approval"]
  end
  EA -->|"register kind external-agent"| Aimux
  EA --> MM
  Rpc -->|"Contributions of ManagementMethods"| MM
  MP -->|"invoke with ModelCallScope"| Aimux
  Aimux -->|"routes kind"| EA
  EA -->|"call-bound approval in ModelCallScope"| Appr
  EA -->|"history reads"| Agents
  EA --> Store
  EA --> Settings
  EA -->|"connection entries"| Provider
  EA -->|"profiles"| Agents
  Bridge -->|"attaches its node"| EA
  Bridge --> HP
```

External agent connections, records and per-node processes are shared across Sessions, so ExternalAgent is a process plugin ([ADR 002](../../adr/002-plugin-scope.md) D1). It depends on Aimux, Agents, Provider and Settings at the same scope. Registering the kind with ModelHost is an Aimux supplement: ModelHost routes requests whose resolved connection has kind `external-agent` to ExternalAgent. Dependencies follow scope (D3): an Aimux, Agents, Provider or Settings reload reloads ExternalAgent and its bridges, failing their in-flight external calls; this rare structural cost is accepted and documented. ModelProvider does not depend on ExternalAgent: while it is not loaded, requests for the kind fail before acceptance (transient) and other kinds are unaffected. Each `ExternalAgentBridge` is a structural loader row bound to one node, present only while at least one agent is enabled on that node; it depends on ExternalAgent and its node's HostPlatform and attaches its node's `ExternalAgentNode` to ExternalAgent in `apply`. ExternalAgent adds and removes bridge rows through typed Settings operations when enablement changes (D4). One offline node fails only the calls targeting it. Approval is reached only through the call-bound approval that ModelProvider passes with the call.

| Part | Scope | Depends on | Provides |
| --- | --- | --- | --- |
| `ExternalAgent` | Process, control node | Aimux (ModelHost kind registration), Agents (profile registration, `AgentIndex` for history reads), Provider connection-entry operations, Settings (credential resolver, typed loader operations for bridge rows), App Server process storage (records and `external_agents.*` business configuration) | Kind `external-agent`; `ManagementMethods` entry `external_agents.*` collected by Rpc; attachment point for bridges |
| `ExternalAgentBridge` | Process, one structural row per node with an enabled agent: local row, `peer:<node>/ExternalAgentBridge` elsewhere ([overview §6.3](../overview.md#63-multiple-nodes)) | ExternalAgent, that node's `HostPlatform` | `ExternalAgentNode` for its node, exported over the link |

**Contracts depended on**: the Aimux supplement (ModelHost routes a registered connection kind; `ModelCallScope`, §A.2); two further supplements: **ModelProvider** passes a call-bound approval in `ModelCallScope`, and **Approval** accepts a `Host`-source admission while a model step is open, reusing `approval/request`, `approval/respond` and `approval/cancelled` (no new event); the existing `ModelDriver`/`ModelStream`/`ModelChunk`/`HostError`/`ModelFailureFacts` contract ([host.rs](../../../crates/dim-agent-capabilities/src/host.rs)) and ModelProvider's failure policy ([recovery.rs](../../../crates/dim-agent-capabilities/src/model_provider/recovery.rs)); Approval's `admit` ([Approval](../../architecture/rust-agent/plugins/Approval.md)); the Collaboration supplement "member approval forwarded to the leader" (decisions E6); the Session public interface ([session-access](../contracts/session-access.md)); management-level methods in Rpc's method table ([session-boundary](../contracts/session-boundary.md)); App Server process storage; Settings. **Contracts defined here**: the connection kind `external-agent` and its target `{node, agent}`; `ExternalAgentNode`; the conversation record; the approval call shape `external_agent.<kind>`; business configuration `external_agents.*`; methods `external_agents.*`; the ACP schema and conformance suite shared with [AcpGateway](AcpGateway.md) (client side). Provider, Agents, Approval and Collaboration never name this plugin.

## 2. Data and state

```mermaid
flowchart TB
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
  classDef contract stroke-dasharray:4 3
  Cfg["Business configuration external_agents.node.agent.* (process storage)"]:::new
  Row["Loader row peer:node/ExternalAgentBridge (structural)"]
  Conn["Provider connection entry external-agent:node:agent"]
  Prof["Agents profile: Agent (node), no tools"]
  Rec["ConversationRecord per Session + branch (process storage)"]:::new
  Cat["Per-node catalog (bridge memory)"]:::new
  Live["Live ACP connections and CLI processes (bridge)"]:::new
  TL["Branch Timeline: model/call, step/*, input/*, approval/*"]:::contract
  Cfg -->|"reconciled on apply and set_enabled"| Conn
  Cfg -->|"reconciled"| Prof
  Cfg -->|"row present while an agent is enabled on the node"| Row
  TL -->|"local pages read through AgentIndex and the Session"| Rec
  Rec -->|"selects Continue or New"| Live
  Cat -->|"external_agents.list"| Cfg
```

The `external_agents.*` entries are business configuration ([ADR 002](../../adr/002-plugin-scope.md) D4) kept in ExternalAgent's process storage, read at use time; a change is a data update plus notification, never a reload. They are the source of truth for enablement; connection entries, profiles and bridge rows are reconciled from them through their owners' typed operations, so they are rebuildable. Only the bridge rows are structural, and adding or removing one reloads only that bridge. Conversation records hold only the delivery cursor and external identity; every delivery decision is recomputed from Timeline pages plus the record. The bridge holds nothing that must survive.

```text
ConversationRecord {
  key: (session_id, branch_id),
  target: { node, agent },
  external_id: String,          // ACP sessionId or CLI conversation id
  cursor: u64,                  // sequence of the model/call of the last accepted call on this branch
  scan_from: u64,               // lowest local sequence that may hold an input/message not yet consumed
  last_request_id: String,
  state: ready | in_flight { request_id } | uncertain,
}
```

Records live in App Server process storage under this plugin's root (atomic replace, format version). At `apply`, every `in_flight` record becomes `uncertain`; records are kept 30 days after last use (A12).

## 3. Flows

### 3.1 Model call (initialize or continue)

```mermaid
sequenceDiagram
  participant TL as Branch Timeline
  participant MP as ModelProvider
  participant MH as Aimux ModelHost
  participant EA as ExternalAgent
  participant SA as Calling Session (via AgentIndex)
  participant BR as Bridge (agent node)
  TL->>MP: model/call (Dispatcher handler registers call task, returns)
  MP->>MH: invoke(request, connection, ModelCallScope, cancel)
  MH->>EA: route kind external-agent
  EA->>SA: local_page(branch, after scan_from, through call_sequence)
  EA->>EA: decide New or Continue (record + pages only)
  EA->>EA: persist in_flight (process storage)
  EA->>BR: turn(TurnSpec) on the attached bridge of the target node
  BR-->>EA: Accepted, Text, Thought, ToolActivity, End
  EA->>EA: persist cursor, external_id, ready
  EA-->>MP: ModelStream chunks
  MP->>TL: step/text and step/thinking (append), then step/model-output-end (append_batch_registered)
```

The decision reads only facts at or before `call_sequence` (dispatched, so published, and a batch boundary) and the record; it never reads another plugin's memory. The record is written ahead of the external turn, so a restart can tell that a turn may have started.

Delivery decision, given record `R`, request `Q`, scope `S`:

1. Scan local events of `S.branch` in `(R.scan_from, S.call_sequence]` (whole local branch without a usable record): `input/message`, `input/consumed` with sequence `> R.cursor`, output events (`step/text`, `step/thinking`, `step/tool-use-start`) with sequence `> R.cursor`.
2. **Initialize** when there is no record, `R.target` differs, `R.state = uncertain`, or an output in the range belongs to a request other than `R.last_request_id` (another model answered). Opens `New` and sends a transcript of `Q.messages` (§5) plus the instruction to answer the last user message.
3. **Continue** otherwise: `Continue { external_id }` with the consumed inputs of the range in consumption order.
4. New `scan_from` = lowest sequence of a collected `input/message` not consumed by `S.call_sequence`, else `S.call_sequence`. An empty delta fails before acceptance with `external_agent_no_input` (A8).

`Q.messages` on a child branch is the ancestor prefix up to the cut plus local events, so a child initializes from its own history and never touches the parent's conversation; a fork (new Session id) does the same.

### 3.2 External permission request

```mermaid
sequenceDiagram
  participant BR as Bridge (ACP client)
  participant EA as ExternalAgent
  participant AP as Approval (calling branch)
  participant TL as Branch Timeline
  participant U as Leader model or user
  BR->>EA: PermissionAsked(ask_id, title, kind, options)
  EA->>AP: CallApproval admit (from ModelCallScope), after a thinking line
  AP->>TL: Ask only: approval/request (append_registered), step still open
  U->>AP: approval.respond (leader via Collaboration E6, else user)
  TL->>AP: approval/respond (Dispatcher, by sequence), recheck policy
  AP-->>EA: Ok, denied or cancelled
  EA->>BR: allow_once, reject_once or cancelled
```

The permission request is an ordinary call admission in the Session running the external call (for a delegated member, the member Session), through the two supplements of §1: Permission's preset decides first, and only `Ask` writes `approval/request`. Forwarding to a leader is Collaboration's rule (E6); ExternalAgent only sees the outcome. A thinking line is emitted before asking, so the call already has visible output while it waits (§4).

Mapping: ACP `toolCall.kind` → call name `external_agent.<kind>` and risk: `read`, `search` → `read_filesystem`; `edit`, `delete`, `move` → `write_filesystem`; `execute` → `execute_process`; `fetch` → `network`; others → unclassified (A7). `admit` `Ok` selects an `allow_once` option; denial selects `reject_once`; when the needed once-option is absent, or the ask was cancelled, the bridge answers outcome `cancelled`. `*_always` is never selected. Non-interactive CLI adapters have no request channel: their launch arguments put the agent in its own non-auto-approving mode, so a request it cannot ask is refused by the agent (A16).

### 3.3 Delegation and interrupt

1. A leader Session whose resource node is `build-box` asks the `agent` tool for "Codex (build-box)". Collaboration creates a member Session with that profile; the member inherits the leader's resource node `build-box`, so the bridge runs Codex on `build-box` in the member's cwd. A leader on another node gets `external_agent_wrong_node` before acceptance (A13).
2. Interrupt: ModelProvider cancels the call token; the bridge sends ACP `session/cancel` (or signals the CLI), answers any pending permission request `cancelled`, drains to the prompt response, and after 10 s grace (A4) kills the process. Text already received stays; the next input continues the same conversation if the cancel was acknowledged.

## 4. Timing and conflicts

```mermaid
sequenceDiagram
  participant TL as Branch Timeline
  participant MP as ModelProvider
  participant AP as Approval
  participant EA as ExternalAgent
  participant BR as Bridge
  Note over TL: approval/request at seq n, call waiting
  alt approval/respond at seq n+1 before stop/interrupt at n+2
    TL->>AP: respond consumed first: admit returns Ok after recheck
    EA->>BR: allow_once
    TL->>MP: stop/interrupt: cancel call
    EA->>BR: session/cancel, drain, Cancelled
  else stop/interrupt at n+1 before any respond
    TL->>MP: interrupt consumed: call token cancelled
    AP->>TL: approval/cancelled (Approval, initiator of the request)
    EA->>BR: answer cancelled, then session/cancel
    Note over AP: later approval.respond returns Unknown
  end
```

Answer versus interrupt is decided by Dispatcher order of sequence numbers on the calling branch; the request is always ended by Approval. The external agent learns only the decision that won.

| Scenario | Order | Outcome | Owner |
| --- | --- | --- | --- |
| Approval pending, no answer for a long time | — | No dim timeout; the call stays open; the user or leader can interrupt (A17) | Approval, Collaboration |
| External agent withdraws its request or ends its prompt while dim asks | bridge `PermissionWithdrawn` | ExternalAgent cancels the ask token; Approval writes `approval/cancelled`; a late answer returns Unknown | ExternalAgent, Approval |
| Leader unavailable (unloaded, turn ended, its request to the member stopped) | per Collaboration | Collaboration's supplement decides delivery or fallback to the user; stopping the member request interrupts the member turn (row above) | Collaboration |
| Steer while an approval is pending | `input/applied` after the thinking line | Not a replacement (visible output exists); Loop delivers the Steer in the next request | Loop |
| ExternalAgent reload with a pending approval | — | Call gets "provider unavailable" (not `capability reloading`); ask cancelled; process killed; record `uncertain`; bridges reload with it | ExternalAgent, ModelProvider |
| Agent node offline | entry enabled, that node's bridge not attached | Calls for that node fail before acceptance `node_unavailable` (transient); other nodes unaffected | ExternalAgent |
| Process restart mid-call | `model/call` and `approval/request` unended | Approval recovery writes `approval/cancelled`; ModelProvider fails the call without invoking; record `uncertain`; nothing re-sent | Approval, ModelProvider |

All other rows: [Appendix B](#appendix-b-full-conflict-table).

## 5. Context mapping

| Audience | Sees | Form |
| --- | --- | --- |
| External agent, initialize | `Q.messages` as one text prompt `<dim-history>`: `[user]`, `[assistant]`, `[tool call <name>] <input JSON>`, `[tool result] <content text>`, images as `[image]`, thinking omitted (A14); then the instruction | ACP text block or CLI stdin |
| External agent, continue | Per consumed input: raw block texts verbatim, then user content (images as ACP image blocks when the agent declares `promptCapabilities.image`, else `[image omitted]`) | Prompt blocks |
| External agent, never | dim system prompt, prompt sources, tool declarations, sampling parameters, `metadata`, other plugins' events, diagnostics; `config.reasoning` only through an adapter reasoning flag | — |
| Calling Session's history | External reply as `step/text`; tool activity and permission lines as `step/thinking` (`[tool] <title>: <status>`, `[permission] <title>: <state>`) | Model output facts |
| Leader model (member approval) | The approval's `call` (`external_agent.<kind>`, `{agent, title, kind, raw_input}` bounded to 4 KiB, A18) as Collaboration renders it in the `agent` tool | Collaboration's tool result |
| User | `approval.request` with the same call shape | Client UI |

Keyed blocks are delivered once at consumption; "in effect" and retirement have no meaning externally, and a re-initialization sends only what the projection holds. Committed external output enters history like any model output: compaction summarizes it, children inherit it up to the cut, a later native model sees it as assistant messages, and an interrupted output keeps text and drops thinking. Collaboration's and Workflow's results read the last request's text, so tool activity never pollutes results. Non-interactive Sessions behave identically; the approval mapping is the same because approvals are not `ask_user`.

## 6. Interfaces (summary)

| Interface | Kind | Purpose |
| --- | --- | --- |
| `external_agents.list` / `set_enabled` / `configure` | Methods, level `management` | Per-node detection, enablement, configuration ([A.1](#a1-methods-level-management)) |
| Kind `external-agent`, target `{node, agent}` | Registered with Aimux's ModelHost (Aimux supplement) | Model calls for external profiles ([A.2](#a2-provider-registration-and-call-scope)) |
| `ExternalAgentNode` | Provided by each bridge row, attached to ExternalAgent | `detect`, `turn` with answerable permission asks ([A.3](#a3-bridge-service-and-adapters)) |
| Approval call `external_agent.<kind>` | Approval admission (Host source) | External permission requests |
| ACP schema and conformance suite | Shared with AcpGateway | Client side ([A.5](#a5-shared-acp-schema)) |

## 7. Security

External agents run with their own accounts and tools; dim admits their actions only where they ask, through the calling Session's policy and Approval, and cannot stop actions the agent takes without asking: the node's HostPlatform isolation is the enforcement boundary, and replay or fork never widens it. `*_always` is never selected. Credentials are environment values resolved from references by Settings' resolver at launch and never written to records, Timeline, diagnostics or replies. Enabling an agent is host configuration through `management`-level methods only; the model cannot enable agents. Executables are resolved on the node, never from project files.

## Appendix A. Interface details

### A.1 Methods (level `management`)

| Method | Params | Result | `mutating` |
| --- | --- | --- | --- |
| `external_agents.list` | `refresh?` | Per node: agent, executable, version, protocol (ACP or CLI), enabled, configuration (references only); offline nodes marked `node_unavailable` | no |
| `external_agents.set_enabled` | `node`, `agent`, `enabled` | New state | yes |
| `external_agents.configure` | `node`, `agent`, `executable?`, `args?`, `env?: {name: credential reference}`, `reasoning?` | New configuration | yes |

Detection runs at load and on `refresh` (A1); detected agents stay disabled until enabled (A3). `set_enabled` and `configure` update the business configuration in process storage and notify; they never reload ExternalAgent. Enabling the first agent on a node adds that node's bridge row, disabling the last removes it. Enabling `(node, agent)` reconciles, through Provider's typed connection-entry operation, the entry `external-agent:<node>:<agent>` with one model `default` (A2) and target `{node, agent}`, and, through Agents' profile registration, a profile `{ name: "<Agent> (<node>)", system_prompt: "", tools: [], model: { provider_id: "external-agent:<node>:<agent>", model_id: "default" } }`. Disabling removes both; existing Sessions keep their adopted model and fail their next call with `external_agent_disabled`. Profiles are listed by `AgentIndex` like any Agent, so Collaboration's `agent` tool can request them.

### A.2 Provider registration and call scope

```rust
// Aimux supplement: ModelHost routes a registered connection kind to its provider;
// ExternalAgent registers kind "external-agent" in apply and unregisters in its ctx.effect.
fn invoke(&self, request: Arc<ModelRequest>, target: ConnectionTarget, scope: ModelCallScope,
          cancel: CancellationToken) -> HostFuture<'_, ModelStream>;
// ModelCallScope (ModelProvider supplement): call identity (session_id, branch_id, turn_id,
// request_id, call_sequence), the SessionConfig subset {cwd, resource_node, interactive,
// collaboration_role}, and the call-bound approval:
trait CallApproval {   // bound by ModelProvider to its branch's Approval
    async fn admit(&self, operation_id: String, call: ApprovalCallData, risk: Option<String>,
                   cancel: CancellationToken) -> Result<(), HostError>;
}
```

The call-bound approval forwards to the existing `Approval::admit(AdmitCall { source: BranchSource::Host { operation_id }, call }, risk, cancel)`; `operation_id = "external-agent:<request_id>:<ask_n>"`. Approval supplement: such an admission is accepted while `step/model-start` is open; its facts are the ordinary approval facts, with no model-visible effect. When a request's adopted Provider connection entry `external-agent:<node>:<agent>` exists but the kind is not registered, ModelHost fails it before acceptance (transient); other kinds are unaffected.

### A.3 Bridge service and adapters

```rust
trait ExternalAgentNode {   // per node; provided by the bridge row, attached to ExternalAgent in its apply
    async fn detect(&self) -> Vec<Detected>;   // agent, executable, version, protocol
    async fn turn(&self, spec: TurnSpec, cancel: CancellationToken) -> Result<TurnStream, BridgeError>;
}
struct TurnSpec { agent: String, config: AgentConfig, cwd: String, conversation: Conversation, prompt: Vec<PromptBlock> }
enum Conversation { New, Continue { external_id: String } }
enum TurnUpdate { Accepted { external_id: String }, Text(String), Thought(String),
                  ToolActivity { id: String, title: String, status: String },
                  PermissionAsked { ask_id: String, title: String, kind: String,
                                    raw_input: Option<JsonObject>, options: Vec<PermissionOption> },
                  PermissionWithdrawn { ask_id: String }, Usage(Usage), End(EndReason) }
impl TurnStream { fn answer(&self, ask_id: &str, answer: PermissionAnswer); } // Allow | Reject | Cancelled
```

| Adapter | Open | Turn | Cancel |
| --- | --- | --- | --- |
| ACP client | Spawn the ACP command through the node's HostPlatform (cwd, isolation); `initialize` with fs and terminal client capabilities off; `session/new {cwd, mcpServers: []}`, or `session/load` for a known `external_id` when `loadSession` is declared | `session/prompt`; `session/update` streams; `session/request_permission` → `PermissionAsked`; `Accepted` once the request is written | `session/cancel`, answer pending permission requests `cancelled`, drain; kill after grace (A4) |
| Non-interactive CLI | Per turn spawn with the conversation id when continuing; restrictive permission flags (A16) | Prompt on stdin, structured output parsed per adapter; `Accepted` after spawn and complete stdin write | Interrupt signal, grace, kill |

One ACP process per conversation, closed after 10 min idle (A5). Adapter definitions are data inside this plugin (A6).

### A.4 Stream mapping and request fields

| External | `ModelChunk` |
| --- | --- |
| `Accepted` | `MessageStart { message_id: "<external_id>:<request_id>" }` |
| Message chunk / CLI text | `Text(delta)`; an open thinking block is closed first |
| Thought chunk / CLI reasoning | `Thinking(delta)`; an open text block is closed first with `TextEnd` |
| `tool_call`, `tool_call_update` status change | `Thinking("[tool] <title>: <status>")` (A9); never `ToolStart` (no tools offered; ModelProvider rejects unoffered calls) |
| Permission asked / decided | `Thinking("[permission] <title>: asked")`, then `allowed`, `denied` or `cancelled` |
| `plan`, `available_commands_update`, `current_mode_update`, history replayed by `session/load` | Not forwarded; diagnostics |
| Usage reported by the adapter | `Usage` (A10) |
| End `end_turn` | Close open block, `MessageStop { reason: EndTurn }` |
| End `max_tokens` | `MessageStop { reason: MaxTokens }` |
| End `max_turn_requests`, `refusal`, CLI non-zero exit | `HostError::Failed`, non-transient (A11) |
| End `cancelled` after our cancel | Stream ends without `MessageStop`; ModelProvider reports `Cancelled` |

`ModelChunk::Content` is not emitted. Request fields: `messages` only at initialization; `system`, `tools`, sampling fields, `provider_options`, `metadata`, `capability_snapshot` not forwarded.

Retry safety: failures before `Accepted` that may be retried (spawn failure, connection refused, `node_unavailable`) are `HostError::Failed` with `ModelFailureFacts { origin: Transport, transient: true }`; every other failure is non-transient. ModelProvider stops on non-transient errors and once a non-`MessageStart` chunk arrived, so a turn the agent may have started is never sent twice. The record is unchanged before `Accepted`, so a retry computes the same delta.

### A.5 Shared ACP schema

Pinned protocol version, generated types per language, transcript-based conformance suite shared with AcpGateway; fixtures cover initialize, new/load, prompt with text and image, streamed updates, cancel during a prompt, a permission request answered allow, reject and cancelled, a permission request during cancel, and process exit mid-prompt. No runtime code is shared ([overview §B.15](../overview.md#b15-acpgateway)).

### A.6 Mechanics per step

| Step | Path |
| --- | --- |
| Request | Loop writes `input/consumed`* and `step/model-start`; ContextPreparation writes `model/call` (`Timeline::append`); ModelProvider consumes it on its Dispatcher, registers the call task, returns |
| Resolve | In the task: Provider resolves `{kind: external-agent, target}`; ModelHost routes to the registered provider with `ModelCallScope` |
| Read | `AgentIndex` get of the calling Session, then `Session::local_page(branch_id, after, limit, through = call_sequence)`; a Session that cannot be read fails before acceptance, transient |
| Write-ahead | Persist `in_flight` before sending; on `Accepted` persist cursor, `last_request_id`, `scan_from`, `external_id`; an attempt ending before `Accepted` restores the record |
| Output | ModelProvider commits chunks (`Timeline::append`) and writes `step/model-output-end` with `append_batch_registered`; `StateCell<ModelProgress>` shows uncommitted text |
| Approval | Approval `append_registered` of `approval/request` (waiter registered in the commit lock); `approval/respond` consumed on the branch Dispatcher; Approval writes `approval/cancelled` on every other end |
| End | External end or confirmed cancel → `ready`; kill or lost link after `Accepted` → `uncertain` |
| Reload | The `ctx.effect` disposer cancels the plugin's call tokens (asks cancelled first), kills processes, then unregisters the kind; each call ends with a provider-unavailable failure, never `CAPABILITY_RELOADING`; bridges reload with ExternalAgent (they depend on it) |
| Restart | ModelProvider rebuild fails unended `model/call`s; Approval rebuild cancels unmatched requests; `apply` marks `in_flight` records `uncertain` |

ExternalAgent itself appends nothing, subscribes to no Dispatcher and owns no `StateCell`.

## Appendix B. Full conflict table

| Scenario | Order | Outcome | Owner |
| --- | --- | --- | --- |
| Turn interrupt before `model/call` consumed | interrupt < `model/call` | `Cancelled` without invoking | ModelProvider |
| Interrupt before `Accepted` | — | Attempt cancelled; nothing sent; record unchanged | Bridge, ModelProvider |
| Interrupt after `Accepted` | `model/call` < interrupt | Cancel sent; updates until the prompt response committed; `Cancelled`; record `ready`, cursor advanced | Bridge, ModelProvider |
| Cancel not acknowledged within grace | — | Killed; `Cancelled`; `uncertain`; next call initializes | Bridge |
| `approval/respond` before interrupt | respond < interrupt | Decision delivered; then cancel as above | Approval |
| Interrupt before `approval/respond` | interrupt < respond | `approval/cancelled`; ask answered `cancelled`; late respond Unknown | Approval |
| Policy changed while asking | `permission/changed` < respond | Recheck after allow; `Deny` → `approval/cancelled`, ask answered reject | Approval |
| Preset decides without asking | — | `Allow`/`Deny` returned at once; no event | Permission |
| Several asks in one call | — | Independent approvals in arrival order | Approval |
| Permission request arriving during cancel drain | — | Answered `cancelled` without asking | Bridge |
| External agent withdraws its request | `PermissionWithdrawn` | Ask cancelled; `approval/cancelled` | ExternalAgent, Approval |
| Leader unavailable for a member approval | — | Collaboration supplement decides; ExternalAgent waits for a fact | Collaboration |
| No answer, no interrupt | — | Waits indefinitely (A17) | — |
| Steer before any visible chunk | `input/applied` (Steer) during call | ModelProvider replaces the request, cancels; next request delivers the Steer (and earlier inputs if not accepted) | ModelProvider, Bridge |
| Steer after visible output (incl. permission line) | — | Not replaced; new request in the same turn | Loop |
| `input/withdrawn` of a queued input | withdrawn before consumption | Never consumed, so never delivered; `scan_from` passes it once consumption moves past | InputQueue, Bridge |
| Concurrent Queue/Append inputs | Timeline order | Delivered in consumption order | Loop, Bridge |
| Tool batch not settled | — | No dim tool calls in these requests | — |
| Compaction before a call | `context/compaction` < `model/call` | Continue unaffected; initialize includes the summary | ContextPreparation, Bridge |
| Child branch cut, including between `approval/request` and respond | any | Child has no record and initializes; Approval tracks local requests only, so the parent's request stays the parent's | Bridge, Approval |
| Branch interrupt or close mid-call | branch interrupt | As interrupt; Approval close cancels asks | ModelProvider, Approval |
| Native model in between | foreign outputs after cursor | Re-initialize | Bridge |
| Pre-acceptance transient failure | — | Retried with the same delta | ModelProvider |
| ExternalAgent reload mid-call | — | Provider-unavailable failure; killed; `uncertain`; ModelProvider not reloaded | ExternalAgent |
| Agents, Aimux, Provider or Settings reload | — | ExternalAgent and its bridges reload (D3, accepted); in-flight external calls fail as on its own reload | ExternalAgent |
| Process restart mid-call | unended facts | Settled by rebuilds; `uncertain`; nothing re-sent | ModelProvider, Approval |
| Session unload and restore | — | Records persist; ACP process closed by idle; reopen with `session/load` or initialize | Bridge |
| Agent node offline | bridge not attached | `node_unavailable` transient before acceptance, retried within bound, then `Failed`; other nodes unaffected | ExternalAgent |
| Link lost after `Accepted` | — | Non-transient failure; asks cancelled; `uncertain` | Bridge |
| Session resource node differs from agent node | — | `external_agent_wrong_node`, non-transient (A13) | ExternalAgent |
| Duplicate `set_enabled` | — | Idempotent | ExternalAgent |
| Client detach | — | No effect; pending approvals wait for another client or the leader | — |

## Appendix C. Errors and recovery

| Code | Before acceptance | Meaning |
| --- | --- | --- |
| `external_agent_not_installed` | yes, non-transient | Executable not found on the node |
| `external_agent_auth_required` | yes, non-transient | Agent needs its own login; presentation names the command |
| `external_agent_launch_failed` | yes, transient | Spawn or connection failure |
| `node_unavailable` | yes, transient | Agent enabled on the node but its bridge is not attached |
| `external_agent_disabled`, `external_agent_wrong_node`, `external_agent_no_input` | yes, non-transient | A.1, §3.3, §3.1 |
| `external_agent_protocol` | either | Malformed output; killed; `uncertain` if after acceptance |
| `external_agent_exited` | no | Process exited or link lost mid-turn; `uncertain` |
| provider unavailable | either | ExternalAgent unloading; asks cancelled first |

Errors surface as `ModelOutputOutcome::Failed`; committed text stays. Continuation failure or an `uncertain` record re-initializes (A15).

## Appendix D. Acceptance

Flow tests through public interfaces with a scripted ACP agent and a scripted CLI:

1. First call initializes; second delivers only the new input; an Append input is delivered with its consuming request; a withdrawn Queue input is never delivered.
2. Text, thought and tool activity map to `step/text` and one `step/thinking` block; no `step/tool-use-start`.
3. Interrupt before and after acceptance; ignored cancel → `uncertain` → next call initializes.
4. Steer before visible output replaces the request; after output (including a permission line) it becomes a new request.
5. Transient pre-acceptance failure retried; post-acceptance failure not.
6. Child branch and native↔external switch initialize; parent conversation untouched.
7. Restart with `in_flight` record and a pending approval → `uncertain`, `approval/cancelled`; no resend.
8. Collaboration request to an external profile returns the external reply; a leader on another node gets `external_agent_wrong_node`.
9. Permission: WorkspaceWrite `execute` → `approval/request`; allow → `allow_once`, deny → `reject_once`; ReadOnly `execute` → reject without event; FullAccess → allow without event; no `allow_once` option → `cancelled`.
10. Member Session: the approval reaches the leader through Collaboration and the leader's decision is delivered; without a leader the user answers via `approval.request`.
11. Interrupt racing `approval.respond` in both orders; external withdrawal cancels the approval; ExternalAgent reload cancels it without reloading ModelProvider.
12. One offline peer node fails only its calls; other nodes and native connections continue.
13. Shared ACP conformance suite passes on the client side.

## Appendix E. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Detection on load and explicit refresh, not by watching | Judgment | Background polling |
| A2 | One model `default` per external agent | Judgment | Per-agent model lists |
| A3 | Detected agents are disabled until enabled | Judgment (they spend the user's accounts) | Default on |
| A4 | Cancel grace 10 s | Judgment | Other bound |
| A5 | One ACP process per conversation, closed after 10 min idle | Judgment | Shared process |
| A6 | Adapter definitions are data inside the plugin | Overview §F.3 | Per-agent plugins |
| A7 | ACP tool kind → call name `external_agent.<kind>` and risk category mapping | Decisions E6 (ordinary approval); mapping is judgment | Single unclassified category (always asks under WorkspaceWrite) |
| A8 | An empty delta fails the call | Judgment | Send a continuation prompt |
| A9 | External tool activity rendered as thinking lines | Judgment | Text, or diagnostics only |
| A10 | Usage only when the adapter reports it | Judgment | — |
| A11 | `max_turn_requests` and `refusal` fail the request | Judgment | Map to `EndTurn` |
| A12 | Records kept 30 days after last use | Judgment | Other retention or deletion on Session close |
| A13 | An external agent serves only Sessions whose resource node is its node | Overview §6.3; judgment | Cross-node working directories |
| A14 | Transcript format; thinking omitted | Overview §B.10 lists the content; format is judgment | Another rendering |
| A15 | Uncertain or failed continuation re-initializes from history | Overview §9 (still open) | Explicit failure |
| A16 | CLI adapters launch in the agent's own non-auto-approving mode; they cannot ask dim | Judgment (no request channel) | Per-CLI permission bridges |
| A17 | No timeout on an external permission approval | Approval has none; Questions precedent (E24) | Timeout answering reject |
| A18 | Approval call input `{agent, title, kind, raw_input}` with `raw_input` bounded to 4 KiB | Judgment | Other shape |

## Appendix F. Open questions

| Question | Description |
| --- | --- |
| Supplement cost | The call-bound approval makes ModelProvider inject `B:Approval`, so an Approval reload reloads ModelProvider (rare, accepted per D3) |
| External conversation continuation | Overview §9; this design re-initializes (A15) |
| Agents profile registration | The shape of Agents' profile registration and Provider's connection-entry operations (direct dependencies, ADR 002 D3) is not yet defined |
| ACP `session/load` | Whether a `ready` record after a process restart should continue via `session/load` rather than initialize |
