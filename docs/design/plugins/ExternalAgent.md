# ExternalAgent

Status: Draft. Overall design: [overview §5.10](../overview.md#510-externalagent). Phase: P3. Language: TypeScript first ([overview §11.3](../overview.md#113-assignment)).

ExternalAgent makes locally installed coding agents (Claude Code, Codex, Cursor, OpenCode, ZCode and similar) usable as a kind of model connection. A Session whose model is an external Agent runs through Loop and ModelProvider as usual; ExternalAgent is the ModelHost provider registered for its own connection kind and turns each dim model request into one external turn, then turns the external agent's stream back into model output chunks. External agents get no dim tools: they use their own. Delegation works through Collaboration because each enabled external agent is also an Agent profile. ExternalAgent writes no Timeline event.

## 1. User flows

1. The user runs `/external-agents`; the list shows, per node, the detected agents, version, protocol (ACP or CLI) and state. The user enables Claude Code on the local node.
2. The user starts a Session with the profile "Claude Code (local)" and types a request. Loop prepares the request; ModelProvider calls the ExternalAgent provider; the bridge opens an external conversation, sends the history and the request, and streams the reply. The user sees text, the external agent's tool activity as thinking, and the turn end.
3. The user interrupts. The external turn is cancelled (ACP `session/cancel`, or the CLI process is signalled); text already received stays; the turn ends cancelled. The next input continues the same external conversation.
4. A leader Session asks the `agent` tool to request work from the "Codex (build-box)" Agent. Collaboration creates a member Session with that profile; the bridge runs Codex on node `build-box` in the leader's workspace; the result returns as Collaboration's result.
5. The user switches a native Session to an external agent in the middle of a conversation, or forks a branch. The first external call on that branch initializes a new external conversation from the branch's visible history.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Per-node detection, enablement and configuration of external agents | Routing by connection kind (ModelHost/Aimux), connection resolution (Provider) |
| External Agent connection entries and Agent profiles (through their owners' contracts) | Loop, request preparation, output commit, retry policy (Loop, ModelProvider) |
| Bridging: conversation identity and delivery cursor per Session + branch, history initialization, request mapping, stream mapping, cancellation | Collaboration semantics (delegation reuses Collaboration unchanged) |
| ACP client and non-interactive CLI adapters; process lifetime on the agent's node | Isolation backends (HostPlatform); the agents' own accounts and permissions |

## 3. Scope and dependencies

**Contracts depended on**: the ModelHost provider registration by connection kind (overview §7.2 Aimux supplement; defined by its consumer, which knows no provider) extended with a call scope (§6.3, proposed); the existing `ModelDriver`/`ModelStream`/`ModelChunk`/`HostError`/`ModelFailureFacts` contract ([host.rs](../../../crates/dim-agent-capabilities/src/host.rs)) and ModelProvider's failure policy ([recovery.rs](../../../crates/dim-agent-capabilities/src/model_provider/recovery.rs)); the Provider external Agent connection type (overview §7.2 Provider supplement); Agents profiles with an external model and no assembled tools (overview §7.2 Agents supplement); [session-access](../contracts/session-access.md) (read only); HostPlatform process launch with isolation on the agent's node (overview §7.1); Settings typed plugin configuration (overview §7.2); [Management](Management.md) contributions. **Contracts defined here**: the connection kind `external-agent` and its connection target, the `ExternalAgentNode` bridge service, the conversation record, the configuration keys `external_agents.*`, the `external_agents.*` methods, and the ACP schema and conformance suite shared with [AcpGateway](AcpGateway.md) (§6.5).

| Part | Scope | Injects | Provides | Status |
| --- | --- | --- | --- | --- |
| `ExternalAgent` | Process, control node | `Arc<SessionAccess>`; ModelHost provider registry; Provider connection directory; Agents profile registration; Settings; `ExternalAgentNode` of each configured node (`When`, by node) | `ModelDriver` registered for kind `external-agent`; `ManagementContribution` `external_agents` | Proposed |
| `ExternalAgentBridge` | Process, one row per node: local row, `peer:<node>/ExternalAgentBridge` elsewhere ([overview §3.1](../overview.md#31-multiple-nodes)) | that node's `HostPlatform` | `ExternalAgentNode`, exported over the rutis link | Proposed |

ModelHost and Aimux never name this plugin; Provider stores connection entries of kind `external-agent` without interpreting their target; Agents stores the profiles. ExternalAgent names no internal type of those owners.

## 4. Events

None. Every fact of an external call is an existing ModelProvider fact written from the chunks the bridge yields: `step/text`, `step/thinking`, `model/usage`, `step/model-output-end`. Delegation uses Collaboration's existing facts ([overview §6](../overview.md#6-new-timeline-events), `external_agent/*` row). Conversation identity and cursor do not affect any branch state machine; they are bridge state in process storage.

## 5. State

| State | Owner | Rebuild |
| --- | --- | --- |
| Per-node catalog (detected agents, version, protocol) | `ExternalAgentBridge`, memory | Re-detected on `external_agents.list {refresh: true}` and at load (A1) |
| Configuration: enabled, executable, args, env credential references, permission answer, reasoning mapping | Settings loader editable layer (`external_agents.<node>.<agent>.*`) | Settings |
| Connection entries and Agent profiles | Provider and Agents (written through their contracts on enable/disable) | Their owners |
| Conversation records | `ExternalAgent`, process storage | Loaded at `apply`; `in_flight` records become `uncertain` (§7) |
| Live external processes and ACP connections | `ExternalAgentBridge` | Not rebuilt; reopened on demand (§6.4) |

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

## 6. Interfaces

### 6.1 Connection and profile

Enabling `(node, agent)` writes a Provider connection entry `external-agent:<node>:<agent>` with one model `default` (A2) and connection target `{node, agent}`, and an Agent profile `{ name: "<Agent> (<node>)", system_prompt: "", tools: [], model: { provider_id: "external-agent:<node>:<agent>", model_id: "default" } }`. Disabling removes both; existing Sessions keep their adopted model and fail their next call with `external_agent_disabled`. Agents created from these profiles are listed by `AgentIndex` like any Agent, so Collaboration's `agent` tool can request them.

### 6.2 Management methods (contribution `external_agents`)

| Method | Params | Result | `mutating` |
| --- | --- | --- | --- |
| `external_agents.list` | `refresh?` | Per node: agent, detected executable, version, protocol, enabled, configuration (references only) | no |
| `external_agents.set_enabled` | `node`, `agent`, `enabled` | New state | yes |
| `external_agents.configure` | `node`, `agent`, `executable?`, `args?`, `env?: {name: credential reference}`, `permission?: reject\|allow_once`, `reasoning?` | New configuration | yes |

Detected but never enabled agents are off (A3).

### 6.3 Provider registration and call scope

```rust
// Registration contract owned by ModelHost/Aimux (overview §7.2); this plugin registers kind "external-agent".
// Proposed extension: the router passes the call scope ModelProvider already knows.
pub struct ModelCallScope {
    pub session_id: String, pub branch_id: String,   // SessionIdentity, TimelineIdentity
    pub turn_id: String, pub request_id: String,      // ModelCallData
    pub call_sequence: u64,                           // sequence of the consumed model/call event
    pub cwd: String, pub resource_node: Option<String>, // SessionConfig
}
fn invoke(&self, request: Arc<ModelRequest>, target: ConnectionTarget, scope: ModelCallScope,
          cancel: CancellationToken) -> HostFuture<'_, ModelStream>;
```

`ModelDriver::invoke` today receives only `request`, `connection` and `cancel`, and `ModelRequest.metadata` carries no branch; the scope is required for branch isolation and the cursor (§14).

### 6.4 Bridge service and adapters

```rust
/// Process service on each node; exported over the link for peer nodes.
trait ExternalAgentNode {
    async fn detect(&self) -> Vec<Detected>;                          // agent, executable, version, protocol
    async fn turn(&self, spec: TurnSpec, cancel: CancellationToken) -> Result<TurnStream, BridgeError>;
}
struct TurnSpec { agent: String, config: AgentConfig, cwd: String, conversation: Conversation, prompt: Vec<PromptBlock> }
enum Conversation { New, Continue { external_id: String } }
enum TurnUpdate { Accepted { external_id: String }, Text(String), Thought(String),
                  ToolActivity { id: String, title: String, status: String },
                  Permission { title: String, answered: String }, Usage(Usage), End(EndReason) }
```

| Adapter | Open | Turn | Cancel |
| --- | --- | --- | --- |
| ACP client | Spawn the agent's ACP command through the node's HostPlatform (cwd, isolation); `initialize` with `clientCapabilities` fs and terminal off; `session/new {cwd, mcpServers: []}`, or `session/load` for a known `external_id` when the agent declares `loadSession` | `session/prompt {sessionId, prompt}`; `session/update` notifications stream; the prompt response carries `stopReason`. `Accepted` is yielded when the request is written | `session/cancel` notification, then drain until the prompt response; after a grace period (A4) kill the process |
| Non-interactive CLI | Per turn spawn the adapter's command line with the conversation id when continuing | Prompt on stdin, structured output parsed per adapter. `Accepted` after spawn and complete stdin write | Interrupt signal, grace (A4), then kill |

One ACP process serves one conversation and stays alive between turns while used; it is closed after 10 minutes idle (A5). Adapter definitions (detection probe, protocol, launch arguments, structured-output parser, reasoning flag) are data inside this plugin (A6). Permission requests from the external agent (`session/request_permission`) are answered from configuration: `reject` (default) selects a `reject_once` option, `allow_once` selects an `allow_once` option; `*_always` is never selected (A7, overview §13).

### 6.5 Shared ACP schema and conformance

The ACP schema (pinned protocol version, generated types per language) and a transcript-based conformance suite are shared with AcpGateway; this plugin is the client side. Fixtures cover initialize, new/load, prompt with text and image, streamed updates, cancel during a prompt, a permission request during cancel, and process exit mid-prompt. No runtime code is shared ([overview §5.15](../overview.md#515-acpgateway)).

## 7. Timeline and capability mechanics

ExternalAgent appends nothing and subscribes to no Dispatcher. The call path, per request:

| Step | Path |
| --- | --- |
| Request | Loop writes `input/consumed`* and `step/model-start` (`Timeline::append`), then `context/prepare`; ContextPreparation writes `model/call` (`Timeline::append`, its own batch); ModelProvider consumes `model/call` on its Dispatcher, registers the call task and returns |
| Resolve | In the task: `ContextPreparation::validate`, then Provider resolves `request.provider_id` to `{kind: external-agent, target}`; the router calls this plugin's `invoke` with the scope |
| Read | `SessionAccess::source("external-agent").get(session_id)` (loaded: it is running this call); `local_page(branch_id, after, limit, through = call_sequence)`. `call_sequence` is dispatched, so published, and a batch boundary |
| Decide | From the record and the pages only (§7.1). No in-memory state of another plugin is read |
| Write-ahead | Persist `in_flight { request_id }` before the turn is sent; on `Accepted`, persist `cursor = call_sequence`, `last_request_id`, `scan_from`, `external_id`. An attempt that ends before `Accepted` restores the previous record |
| Output | The returned `ModelStream` yields chunks (§7.2); ModelProvider's `consume_output` commits them (`Timeline::append`) and ModelProvider writes `step/model-output-end` with `append_batch_registered` (closing started calls first; there are none). `StateCell<ModelProgress>` shows uncommitted text and thinking |
| End | On the external end or a confirmed cancel, persist `ready`; on kill or lost connection after `Accepted`, persist `uncertain` |
| Reload | ExternalAgent or bridge unload cancels its calls' tokens and kills processes before the disposer returns; ModelProvider settles each call by its outcome. The registry withdraws the kind while Pending, so a call in that window fails before acceptance |
| Restart | ModelProvider's rebuild fails unended `model/call`s without invoking ([recovery](../../architecture/rust-agent/plugins/ModelProvider.md)); `apply` marks every `in_flight` record `uncertain`. Nothing is re-sent |

### 7.1 Delivery decision

Given record `R` (or none), request `Q` and scope `S`:

1. Scan local events of `S.branch_id` in `(R.scan_from, S.call_sequence]` (with no usable record, scan the whole local branch). Collect `input/message` events, `input/consumed` events with sequence `> R.cursor`, and output events (`step/text`, `step/thinking`, `step/tool-use-start`) with sequence `> R.cursor`.
2. **Initialize** when: no record; `R.target` differs from the connection; `R.state = uncertain`; or an output event in the range belongs to a request other than `R.last_request_id` (another model answered on this branch). Initialization opens `Conversation::New` and sends one prompt: a transcript of `Q.messages` (§9), followed by the instruction to answer the last user message.
3. **Continue** otherwise: open `Continue { external_id }` and send the consumed inputs of the range in consumption order, each rendered from its `input/message` (§9).
4. New `scan_from` = the lowest sequence of a collected `input/message` not consumed by `S.call_sequence`, else `S.call_sequence`.
5. An empty delta on continue fails the call before acceptance with `external_agent_no_input` (A8).

`Q.messages` is the branch's projected history, which on a child branch is the ancestor prefix up to the cut plus local events; a child therefore initializes from its own history and never touches the parent's conversation. A fork creates a new Session id and initializes likewise.

### 7.2 Stream mapping

| External | `ModelChunk` |
| --- | --- |
| `Accepted` | `MessageStart { message_id: "<external_id>:<request_id>" }` (does not close the replay window) |
| `agent_message_chunk` / CLI text | `Text(delta)`; a thinking block open before it is closed first with `ThinkingEnd { thinking }` |
| `agent_thought_chunk` / CLI reasoning | `Thinking(delta)`; an open text block is closed first with `TextEnd` |
| `tool_call`, `tool_call_update` (status change) / CLI tool events | Part of the thinking block: one line `"[tool] <title>: <status>"` as `Thinking(delta)` (A9). Never `ToolStart`: the request offers no tools, and ModelProvider rejects unoffered calls |
| Permission answered | Thinking line `"[permission] <title>: <answer>"` |
| `plan`, `available_commands_update`, `current_mode_update`, replayed history during `session/load` | Not forwarded; diagnostics only |
| Usage reported by the adapter | `Usage` (A10) |
| End `end_turn` | Close the open block, then `MessageStop { reason: EndTurn }` |
| End `max_tokens` | `MessageStop { reason: MaxTokens }` (ModelProvider fails the request) |
| End `max_turn_requests`, `refusal`, CLI non-zero exit | `HostError::Failed`, non-transient (A11) |
| End `cancelled` (after our cancel) | Stream ends without `MessageStop`; ModelProvider reports `Cancelled` |

`ModelChunk::Content` is not emitted; history is built from `step/text` and `step/thinking`.

### 7.3 Request fields

| Field | Handling |
| --- | --- |
| `messages` | Initialization only: rendered as the transcript. Continue uses the Timeline delta |
| User content, raw block text | Forwarded (through the transcript or the delta); images as ACP image blocks when the agent declares `promptCapabilities.image`, else the text `[image omitted]` |
| `system`, `tools`, `temperature`, `max_tokens`, `top_p`, `top_k`, `stop_sequences`, `provider_options`, `metadata` | Not forwarded |
| `config.reasoning` | Mapped only through the adapter's reasoning flag when configured; else ignored |
| `capability_snapshot` | Not forwarded; ContextPreparation uses it for budget as for any model |

### 7.4 Retry safety

Failures before `Accepted` that may be retried (spawn failure, connection refused, peer link down) return `HostError::Failed` with `ModelFailureFacts { origin: Transport, transient: true }`. Every other failure (not installed, authentication required, protocol error, any failure after `Accepted`) carries `transient: false` or no facts. ModelProvider's `RecoveryState::decide` stops on non-transient errors, and on any error once a non-`MessageStart` chunk was received; so a turn the external agent may have started is never sent twice. No `ModelFailurePolicy` and no credential recovery are supplied. Because the record is unchanged before `Accepted`, a retried attempt computes the same delta.

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Turn `stop/interrupt` before the `model/call` is consumed | interrupt < `model/call` | ModelProvider completes `Cancelled` without invoking; no external effect | ModelProvider |
| Interrupt before `Accepted` | — | Attempt token cancelled; turn not sent; record unchanged | Bridge, ModelProvider |
| Interrupt after `Accepted` | `model/call` < interrupt | Cancel sent; updates until the prompt response are still committed; stream ends; `Cancelled`; record `ready`, cursor advanced (the external conversation holds the input) | Bridge, ModelProvider |
| Cancel not acknowledged within grace | — | Process killed; `Cancelled`; record `uncertain`; next call initializes | Bridge |
| Steer consumed before any visible chunk | `input/applied` (Steer) during the call | ModelProvider replaces the request (`replaced by steer`), cancels as above; next request in the same turn delivers only the Steer if accepted, else the earlier inputs too | ModelProvider (gate), Bridge (cursor) |
| Steer after visible output (text, thinking, tool activity) | — | Not replaced; Loop continues the turn after this output with a new request delivering the Steer | Loop |
| Concurrent Queue/Append inputs | Timeline order | Delivered when consumed, in consumption order | Loop, Bridge |
| Tool batch not settled | — | No dim tool calls exist in these requests | — |
| Compaction before a call | `context/compaction` < `model/call` | Continue: no effect (delta from facts). Initialize: transcript includes the summary | ContextPreparation, Bridge |
| Child branch cut | cut anywhere | Child has no record → initializes from its own history | Bridge |
| Branch-scoped interrupt or branch close mid-call | branch interrupt | Same as interrupt; record kept until retention (A12) | ModelProvider |
| Model switched to native and back | foreign outputs after cursor | Re-initialization from the full history | Bridge (§7.1) |
| Pre-acceptance transient failure | — | Retried by ModelProvider with the same delta | ModelProvider |
| ExternalAgent/bridge reload mid-call | — | Process killed; `Failed`; record `uncertain` | Bridge, ModelProvider |
| Process restart mid-call | `model/call` without end | Settled by ModelProvider rebuild; record `uncertain` | ModelProvider, ExternalAgent |
| Session unload and restore | — | Records persist; ACP process closed by idle; next call reopens with `session/load` or initializes | Bridge |
| Agent's node offline | bridge row Pending | Call fails before acceptance (transient, connection) and is retried within ModelProvider's bound, then `Failed` | rutis lifecycle, ModelProvider |
| Session resource node differs from the agent's node | — | Fails before acceptance, non-transient, `external_agent_wrong_node` (A13) | Bridge |
| Duplicate `set_enabled` | — | Idempotent | ExternalAgent |
| Client detach | — | No effect | — |

## 9. Context mapping

**What the external agent receives.** Initialization: one text prompt `<dim-history>` containing `Q.messages` in order, each as `[user]`, `[assistant]`, `[tool call <name>] <input JSON>`, `[tool result] <content text>`, with images as `[image]` and thinking omitted (A14), then `</dim-history>` and the instruction. Continue: per consumed input, its raw block texts verbatim, then its user content (text and images). Keyed blocks are delivered once at consumption like any raw block; "in effect" and retirement have no meaning in the external conversation, and a re-initialization sends only what the projection holds. It never receives dim's system prompt, prompt sources, tool declarations, other plugins' events or diagnostics.

**What dim's history holds.** The external reply is committed as `step/text`; tool activity and reasoning as `step/thinking`. These enter branch history like any model output: Collaboration's result and Workflow's stage result read the last request's text, so tool activity does not pollute results. An interrupted output keeps its text and drops thinking (existing projection rule). Compaction summarizes them like other output; child branches inherit them up to the cut; a later native model on the branch sees them as assistant messages. Non-interactive Sessions behave identically.

## 10. Errors and recovery

| Code | Before acceptance | Meaning |
| --- | --- | --- |
| `external_agent_not_installed` | yes, non-transient | Executable not found on the node |
| `external_agent_auth_required` | yes, non-transient | Agent requires its own login; presentation tells the user which command to run |
| `external_agent_launch_failed` | yes, transient | Spawn or connection failure |
| `external_agent_disabled`, `external_agent_wrong_node`, `external_agent_no_input` | yes, non-transient | §6.1, §8, §7.1 |
| `external_agent_protocol` | either | Malformed ACP or CLI output; process killed; record `uncertain` if after acceptance |
| `external_agent_exited` | no | Process exited mid-turn; record `uncertain` |

Errors surface as `ModelOutputOutcome::Failed`; text already committed stays. External conversation continuation failures re-initialize (overview §13, A15).

## 11. Security

External agents run with their own accounts and their own tools; dim's Approval and Permission do not admit their actions. Processes start only through the HostPlatform of the agent's node, with that node's isolation configuration; replay or fork never widens it. Permission requests are never answered `always`. Credentials are passed as environment values resolved from references at launch and never written to records, Timeline, diagnostics or replies. Enabling an agent is a host configuration change available only through Management; the model cannot enable agents. Executables are resolved on the node, never from project files.

## 12. Acceptance

Flow tests through public interfaces with a scripted ACP agent and a scripted CLI on the local node:

1. First call initializes with the transcript; second call delivers only the new input; an Append input consumed with the next request is delivered with it.
2. Text, thought and tool activity map to `step/text` and one `step/thinking` block; no `step/tool-use-start` is written.
3. Interrupt before and after acceptance; cancel ignored past grace → `uncertain` → next call initializes.
4. Steer before visible output replaces the request and delivers only the Steer; after output, the Steer becomes a new request in the same turn.
5. Transient pre-acceptance failure is retried; failure after acceptance is not.
6. Child branch and native↔external switch initialize; parent conversation untouched.
7. Restart with an `in_flight` record → `uncertain`; no resend.
8. Collaboration request to an external profile returns the external reply as the result.
9. Shared ACP conformance suite passes on the client side.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Detection on load and on explicit refresh, not by watching | Judgment | Background polling |
| A2 | One model `default` per external agent | Judgment | Per-agent model lists |
| A3 | Detected agents are disabled until enabled | Judgment (they spend the user's own accounts) | Default on when installed |
| A4 | Cancel grace 10 s | Judgment | Other bound |
| A5 | One ACP process per conversation, closed after 10 min idle | Judgment | Shared process with multiple ACP sessions |
| A6 | Adapter definitions are data inside the plugin | Overview §11.3 ("CLIs differ widely") | Per-agent plugins |
| A7 | External permission requests answered by configuration, default `reject`, never `always` | Overview §13 open question; default is judgment | Mapping to dim approval |
| A8 | An empty delta fails the call | Judgment (no Loop path produces it for these Sessions) | Send a continuation prompt |
| A9 | External tool activity is rendered as thinking lines | Judgment; ModelProvider rejects unoffered tool calls | Text, or diagnostics only |
| A10 | Usage only when the adapter reports it | Judgment | — |
| A11 | `max_turn_requests` and `refusal` fail the request | Judgment | Map to `EndTurn` |
| A12 | Conversation records kept 30 days after last use | Judgment | Other retention, or deletion on Session close via subscription |
| A13 | An external agent is usable only by Sessions whose resource node is its node | Overview §3.1 (resources on the Session's node); judgment | Cross-node working directories |
| A14 | Transcript format and omission of thinking | Overview §5.10 lists user input, replies, tool results; format is judgment | Another rendering |
| A15 | Uncertain or failed continuation re-initializes from history | Overview §13 open question | Explicit failure |

## 14. Open questions

| Question | Description |
| --- | --- |
| Call scope | `ModelDriver::invoke` lacks session, branch, call sequence and cwd; ModelProvider must pass `ModelCallScope` to registered providers. Overview §7 registers no ModelProvider/Aimux supplement for this |
| External Agent permissions | Overview §13; this design answers by configuration |
| External conversation continuation | Overview §13; this design re-initializes |
| ACP `session/load` | Whether continuing after a process restart via `session/load` is preferred over re-initialization when the record is `ready` |
