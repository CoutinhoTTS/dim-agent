# ContextCompaction

Status: Draft (for review). Roadmap item F1 ([#72](https://github.com/arcships/dim/issues/72)).
Overview: [Appendix B.1 ContextPreparation row](../overview.md#b1-session-and-branch-capabilities).
Scope: [ADR 002](../../adr/002-plugin-scope.md): two Session parts, plus supplements to the
existing branch capabilities Loop and ContextPreparation. Nothing in this document is
implemented. The trigger and retention values are provisional and are studied in
[#128](https://github.com/arcships/dim/issues/128).

## 1. Summary

**Compaction** summarizes the history before a cut and keeps the last turns verbatim. The
summary range is normally everything since the previous compaction point, together with the
previous summary. From then on, the branch advances on the new result.

Compaction is part of how the Loop controls the context window. It is a Loop step, like a
model call or a tool batch, and it is not tied to a turn. While a compaction step runs, the
Loop does not advance to its next step. Otherwise the context could overflow during
compaction, and new events would be decided on the old context, inconsistent with the summary.

Automatic compaction already runs inside a request's preparation (`context/prepare` →
`context/compaction` → `model/call`), and the Loop waits for that request's terminal
([context_policy.rs](../../../crates/dim-agent-capabilities/src/context_policy.rs)). It never
runs in production, because the App Server binds `ContextModelHost(None)`
([main.rs](../../../crates/dim-app-server/src/main.rs)).

F1 adds:

1. **A production host.** A Session part, `ProductContextModel`, provides
   `ContextModelHost(Some(..))`. It implements the existing port `ResolveContextModel` →
   `ContextModel` ([host.rs](../../../crates/dim-agent-capabilities/src/host.rs)) on two existing
   process services: `ProviderConnectionResolver` (connection and credentials) and `ModelHost`
   (Aimux).
   - The request's own model compacts.
   - Its window and output budget come from the request's adopted capability snapshot.
   - Token counting is a local estimate.
2. **One compaction path and one cut rule.** ContextPolicy's compaction step is a single
   function, used by automatic compaction during preparation and by the Loop's manual step.
   The cut is the legal cut just before the last K turns (provisional K = 2, #128).
3. **Manual compaction as a Loop step.** `session.context.compact` writes the command
   `context/compact`. The Loop consumes it and schedules a compaction step:
   - Inside a turn, the step runs at the next step boundary, before the next request is
     prepared. The turn then continues from the compacted context.
   - With no turn running, the step runs at once. It writes `context/compaction`, and nothing
     else happens: there is no turn.
4. **Usage query.** `session.context.usage` returns the budget the policy would compute now.
   It writes nothing.

Both methods are exposed through `sessionMethods` (namespace `session.context`) by a second
Session part, `ContextMethods`.

What does not change: Provider, Aimux, ModelProvider, Timeline, Dispatcher, the event envelope,
rutis and `context/prepare`. F1 adds one event kind, `context/compact`, and changes
`context/compaction` directly; breaking changes are accepted because the software is
unreleased (§4.3).

### 1.1 Why it is this small

- **Not AuxiliaryModel.** S4's `auxiliaryModel.complete` returns text only. Compaction needs the
  window, the output budget and the token counts of the model whose context it compacts
  ([auxiliary-model §1.1](https://github.com/arcships/dim/blob/roadmap/s4-provider-purpose-policy-and-auxiliarymod/docs/design/appserver/auxiliary-model.md)).
- **Not inside Provider.** The host reaches everything through the generic ports in
  `dim-agent-capabilities::host`:
  - The capability snapshot travels on the `ModelRequest`.
  - Credentials are resolved by `ProviderConnectionResolver`, as ModelProvider resolves them for
    every turn.
  - Execution is `ModelHost`.
- **No second state machine.** The Loop sequences every step. ContextPreparation runs the
  compaction function only inside a request preparation, which the Loop has already handed to
  it.

## 2. Structure

```
 Process scope (existing, unchanged)
 +-------------------------------------------------------------------+
 |  Aimux: ModelHost        Provider: ProviderConnectionResolver     |
 +-----------------^-------------------------------^-----------------+
                   | inject (hard)                 | inject (hard)
 Session scope     |                               |
 +-----------------+-------------------------------+-----------------+
 |  ProductContextModel (new)                                        |
 |    provides S:ContextModelHost(Some(ProductContextResolver))      |
 |  ContextMethods (new)                                             |
 |    provides sessionMethods["session.context"]: compact, usage     |
 |    per call: TreeCommands.get(branchId)                           |
 |      compact -> .service::<AgentLoop>().compact(), then waits for |
 |                 the context/compaction that references it         |
 |      usage   -> .service::<ContextPreparation>().usage()          |
 +---------------------------------^---------------------------------+
            Rpc (C1) dispatches ---'
 Branch scope (one Timeline, one Dispatcher)
 +-------------------------------------------------------------------+
 |  Loop (supplement): sequences steps                               |
 |    model request -> context/prepare (handoff, unchanged)          |
 |    compaction step (manual) -> ContextPolicy::compact_step        |
 |  ContextPreparation (supplement)                                  |
 |    in preparation: ContextPolicy::prepare -> compact_step         |
 |    (threshold, budget recovery)                                   |
 |  ContextPolicy::compact_step: K-turn cut, host.compact, validate, |
 |    append context/compaction, snapshot (one path for all)         |
 +-------------------------------------------------------------------+
```

The two Session parts are separate because their dependencies differ. Branches inject
`ContextModelHost`, so its provider must not depend on `TreeCommands`, which the branch tree
provides.

### 2.1 Where the manual step lives

The Loop runs the manual compaction step itself, in a tracked task, by calling
`ContextPolicy::compact_step`. It does not drive ContextPreparation through an event. The reasons:

- **The Loop already holds what the step needs.** Its `Deps` already inject `Timeline`,
  `HistoryRenderer` and `ContextModelHost`
  ([agent_loop.rs](../../../crates/dim-agent-capabilities/src/agent_loop.rs) `LoopPlugin::Deps`).
  It already reads the fixed context and builds the request from the latest `context/configured`
  in `start_preparation`. ContextPolicy is a crate-internal struct of the same crate.
- **Nothing is handed over.** `context/prepare` exists to hand a request to ContextPreparation
  and then to ModelProvider. A manual step makes no model request, so a handoff to
  ContextPreparation would add a second command and a second single-flight slot
  (ContextPreparation's `current`) for a step that ContextPreparation does not sequence.
- **No race, by construction.** The two callers of the step can never overlap:
  - The Loop starts a manual step only when no request of the branch is in flight, and it starts
    no request while a step runs.
  - ContextPreparation calls the step only inside the preparation of a request in flight.

  So there is one sequencer (the Loop), one compaction function and one fact.

## 3. Data and state

| Item | Owner | Kind |
| --- | --- | --- |
| `context/compact {}` | Loop | New command event (§4.3) |
| `context/compaction` | Written by ContextPolicy for its caller (ContextPreparation or the Loop) | Existing fact, changed: optional `turn_id`, `request_id`, new `command`, new `cancelled` outcome (§4.3) |
| Compaction slot: `{command, running: Option<CancellationToken>}` | Loop | Private. Folded from `context/compact` and `context/compaction` and rebuilt on load (§5.4) |

The hosts hold no state. A resolved connection lives only inside one `compact` call.

## 4. Interfaces

### 4.1 Session methods (language-neutral, ADR 002 D5)

`sessionMethods`, Session scope, key `session.context`, provided by ContextMethods
([session-boundary A.1](../contracts/session-boundary.md#a1-method-contributions)). Both
methods are declared with `interruption: report`, `progress: false`, `notifications: []` and
`reply: direct`.

| Method | Mutating | Request | Reply |
| --- | --- | --- | --- |
| `session.context.compact` | yes | `{sessionId, branchId}` | `{status: "completed", sequence}` or `{status: "skipped", reason}` |
| `session.context.usage` | no | `{sessionId, branchId}` | `{through, contextWindow, outputTokens, thresholdTokens, estimatedInputTokens, baselineSource: "provider_baseline" \| "full_estimate"}` |

`session.context.compact`:

- It writes `context/compact` through `AgentLoop::compact`, which returns the command's
  sequence `c`.
- It then waits on the branch Timeline for the first `context/compaction` whose
  `command ≥ c`. Commands merged into one step (§5.3) are served by that step's fact.
- It maps the outcome: `completed` → `{status: "completed", sequence}` (the fact);
  `skipped` → `{status: "skipped", reason}`; `failed` → `compaction_failed`;
  `cancelled` → `compaction_cancelled`.

`session.context.usage`:

- `through` is the committed head the numbers were computed at.
- The numbers are those a preparation at that head would write into `context/budget`, plus
  `thresholdTokens`.

Cancellation follows session-boundary A.1: an unload interrupts the call, and a caller's
disconnect cancels nothing.

### 4.2 ContextModel port (existing, Rust)

F1 adds two fields to `CompactionInput`. Both are set by ContextPolicy, the producer:

- `cut: String`: the entry up to which the host summarizes (§5.2).
- `trigger: CompactionTrigger`: `Threshold`, `ProviderOutputBudgetExhausted` or `Manual`.

### 4.3 Events

| Event | Writer | Consumer | Data |
| --- | --- | --- | --- |
| `context/compact` (new) | Loop, through its public write operation `AgentLoop::compact` | Loop, in sequence order | `{}` |
| `context/compaction` (changed) | ContextPolicy, for its caller | Loop (manual step terminal), ContextPolicy (retry reuse), projection, ContextMethods | `{turn_id?, request_id?, command?, through, outcome}` |

`context/compaction` field rules:

| Origin | `turn_id` | `request_id` | `command` |
| --- | --- | --- | --- |
| Automatic (threshold, budget recovery) | the request's turn | the request | absent |
| Manual step inside a turn | the turn | absent | sequence of the `context/compact` served |
| Manual step with no turn | absent | absent | sequence of the `context/compact` served |

- `outcome` gains `cancelled {reason}`, so every manual step ends with exactly one fact.
- Automatic compaction keeps its current behaviour on cancellation: no fact, and the request
  ends `Cancelled`.
- The struct is changed directly. Readers update with it: ContextPolicy's retry filter compares
  `turn_id` with `Some(turn)`, and Trace's id extraction becomes optional. Native storage
  indexes completions by `through` only and is unaffected.
- The frozen v1 wire samples are not changed. The v1-to-native conversion maps their required
  ids to `Some`.

**Dynamic events (decision A).** `context/compact` is added as an `EventKind` variant now. When
#2 lands (plugins register their own event types; Timeline checks structure and type range,
not content), it migrates to dynamic registration owned by the Loop. F1 is #2's first real
consumer. `tree_protocol.rs`'s exhaustive `ProductEvent` classification lists `context/compact`
as not client-visible, like `context/compaction`.

### 4.4 Rust binding

```rust
// dim-agent-types/src/events.rs
EventKind::ContextCompact(ContextCompactData {})                  // "context/compact"
pub struct ContextCompactionData {
    pub turn_id: Option<String>, pub request_id: Option<String>,
    pub command: Option<u64>, pub through: u64, pub outcome: ContextCompactionResult }
pub enum ContextCompactionResult { Completed{..}, Skipped{..}, Failed{..},
                                   Cancelled { reason: String } }

// dim-agent-capabilities/src/host.rs (existing port, same crate)
pub struct CompactionInput { /* existing */ pub cut: String, pub trigger: CompactionTrigger }

// dim-agent-capabilities/src/context_policy.rs (crate-internal)
impl ContextPolicy {
    /// The one compaction path: cut, host.compact, validate, append, snapshot.
    async fn compact_step(&self, step: CompactionStep, cancel: CancellationToken)
        -> Result<Arc<AgentEvent>, HostError>;   // the committed context/compaction
}

// dim-agent-capabilities/src/agent_loop.rs (supplement)
impl AgentLoop { pub fn compact(&self) -> Result<u64>; }  // appends context/compact
impl PublicService for AgentLoop {}                          // lib.rs

// dim-agent-capabilities/src/context_preparation.rs (supplement)
impl ContextPreparation {
    pub async fn usage(&self, cancel: CancellationToken) -> Result<ContextUsage, ContextOpError>;
}
impl PublicService for ContextPreparation {}                // lib.rs

// dim-agent-capabilities/src/context_model.rs (new)
pub struct ProductContextModelPlugin { scope: SessionScope }
//   Deps = (Arc<ModelHost>, Arc<ProviderConnectionResolver>)  -- process scope
// dim-agent-capabilities/src/context_methods.rs (new)
pub struct ContextMethodsPlugin { scope: SessionScope }
//   Deps = InSession<TreeCommands>; SessionMethods::provide(ctx, &scope, "session.context", ..)

// dim-app-server/src/main.rs, product profile only (no --models file):
builder.session_plugin(ProductContextModelPlugin::new)
       .session_plugin(ContextMethodsPlugin::new)
       .session_methods("session.context");
// replaces ContextModelHost(None) there; --models and mock profiles keep it
```

## 5. Flows

### 5.1 Production host

`ProductContextResolver::resolve(request)`:

1. It requires `request.capability_snapshot`, valid for the request's model. Without it, it
   returns `InvalidRequest` "Model capabilities are not adopted; select the model again", the
   same condition and text as `ProductResolver`.
2. It returns a `ContextModel` bound to the request:

| Method | Value |
| --- | --- |
| `context_window` | `snapshot.capabilities.contextWindow`, the catalog value with its existing overrides |
| `output_tokens` | `request.config.maxTokens` if set, else `snapshot.capabilities.maxOutputTokens`; never above the latter |
| `threshold_tokens` | `0.8 × (context_window − output_tokens)`, provisional (#128) |
| `count_tokens` | UTF-8 bytes of the serialized system prompt, messages and tool declarations divided by 4; each image part counts a fixed 1,600 tokens. Provisional (#128). Used for deltas over the provider's `prompt_tokens`, the first request, `validate` and the reduction check |
| `compact` | Below |

`compact(input, cancel)` summarizes the previous summary (`input.checkpoint`) plus the history
entries up to and including `input.cut`, and returns
`Completed {cursor: input.cut, summary, diagnostics}`:

- **Request.** One summarizer request per chunk. It uses the request's provider, model, snapshot
  and config, with `maxTokens = output_tokens`, a fixed summarizer system prompt owned by this
  module, one user message with the transcript, and no tools.
- **Credentials and failures.** The connection comes from `ProviderConnectionResolver` and is
  used only inside this call. The call runs through `invoke_with_recovery`, ModelProvider's
  failure and credential policy. S4 extracts it; if F1 lands first, F1 does the same extraction.
- **Output.** Thinking output is dropped. A tool stop or an empty text is a failure.
- **Transcript.** Role-labelled text: tool calls as name and input, tool results as text, images
  as `[image]`.
- **Chunking.** An entry larger than one call's input budget is truncated in the middle and
  counted in `compactedToolPayloadCount`. A longer transcript is folded in chunks: each call
  gets the running summary and the next chunk.
- **Diagnostics.** The required diagnostics fields come from the run (`reason` and `trigger`
  from `input.trigger`).

### 5.2 One compaction path (ContextPolicy)

`compact_step` is the existing compaction section of `ContextPolicy::prepare`, extracted:

1. Choose the cut.
2. Call `host.compact`.
3. Validate the cut, and that the compacted request is smaller and below the window.
4. Append `context/compaction` with the caller's ids and `command`.
5. Save the optional snapshot.

Its callers:

- **ContextPreparation, inside `prepare`.** When the estimate reaches the threshold (once per
  boundary, as today) or under `recovery`. The trigger is `Threshold` or
  `ProviderOutputBudgetExhausted`. Everything else in `prepare` is unchanged: estimate,
  `context/budget`, retry reuse of a compaction with the same `through` and turn, and
  projecting the request on the committed fact.
- **The Loop's manual step.** Trigger `Manual`. The request used for `resolve` and the
  estimate is built as `start_preparation` builds one: the latest `context/configured`
  settings and system, with the projected messages. No tools are bound.

**The cut** is chosen by ContextPolicy for every trigger:

- The retained turns are the last K turns on the branch, the running one included, from
  `turn/start` to `turn/end`.
- An entry belongs to a turn by the sequence of its source event in the fixed context. Entries
  restored from a snapshot's state predate that snapshot's compaction, so they count as older
  than any retained turn.
- The cut is the latest legal cut before the first entry of the oldest retained turn. A legal
  cut is one the existing check accepts: no retained tool result for a removed call, and the
  next entry is not tool context.
- If no such cut exists: under `recovery` the cut falls back to the latest legal cut, so the
  request still fits; otherwise the outcome is `skipped {reason: "nothing_to_compact"}` with no
  model call.

### 5.3 Manual compaction

**Inside a turn:**

```
      step/model-start {T, R1} ... step/model-output-end {T, R1, completed}
 k    context/compact {}                          writer: Loop (AgentLoop::compact)
      -- Loop consumes k: a request is in flight -> pending
      step/tool-call {T, R1} ... step/tool-result {T, R1}
      -- Loop consumes the tool result (boundary b): pending -> compaction step,
         no next request yet
 m    context/compaction {turn_id: T, command: k,  writer: ContextPolicy (Loop's step)
                          through: b, outcome}
      -- Loop consumes m: the step is over -> next_model as usual
      input/consumed* ; step/model-start {T, R2}   Loop (unchanged)
      context/prepare {T, R2, ...} -> model/call   on the compacted context
```

**With no turn running:**

```
 k    context/compact {}                          writer: Loop (AgentLoop::compact)
      -- Loop consumes k: no request in flight -> compaction step at once
 m    context/compaction {command: k, through: k, outcome}   writer: ContextPolicy
      -- Loop consumes m: the step is over -> try_start
      (no turn/start, no turn/end; a driving input applied during the step starts
       a turn now, and its first request is prepared from the compacted context)
```

**Scheduling rules:**

- **Boundary.** A step boundary is a point where the Loop would prepare the next request
  (`next_model` without a budget-retry source) or where the turn ends. A pending step starts at
  the first boundary.
  - If the turn ends instead of continuing, the step runs right after `turn/end`, with no turn.
  - A budget retry is part of its request, so it is not a boundary.
- **Through.** `through` is the sequence of the event whose consumption started the step: the
  `context/compact` itself with no turn, or the boundary event inside a turn.
- **Merging.** A command consumed while a step is pending merges into it; the fact references
  the latest merged command. A command consumed while a step runs schedules one more step after
  it.
- **No advance.** While a step runs, the Loop does not advance: `next_model` and `try_start`
  wait for the step's fact. Inputs queue as usual and are not consumed by the step, so they
  never enter the summarized range.

**Steer, interrupt and reload:**

- **Steer.** A driving input applied during a step does not cancel it. Inside a turn, the next
  request consumes the input. With no turn, the input starts a turn after the step.
- **Interrupt.** A `stop/interrupt` cancels a running step and drops a pending one. This is
  consistent with automatic compaction:
  - A compaction the host already returned is kept and committed as `completed`.
  - Otherwise the host call is cancelled and the fact is `cancelled {reason}`.
  - A dropped pending step is written as `cancelled` immediately.
  - Inside a turn, the turn then ends `Cancelled` through the existing `next_model` check.
- **Reload.** A consumed `context/compact` without its fact was in flight. The new Loop
  generation writes `context/compaction {command, cancelled: "capability reloading"}` and does
  not replay the summarizer call. A rebuilt turn then ends `Failed` as today.

### 5.4 What the Loop gains

All in `crates/dim-agent-capabilities/src/agent_loop.rs`:

1. **Subscription.** `context/compact` and `context/compaction` are added to the existing list.
2. **Public write.** `AgentLoop::compact()` appends `context/compact` and returns its sequence.
   `AgentLoop` gets the `PublicService` marker.
3. **Compaction slot** (`command`, running token), folded from consumed events. On consuming
   `context/compact`, the Loop marks the step pending. If no request is in flight, it starts the
   step at once.
4. **Boundaries.**
   - `next_model` (except with a budget-retry source) and the `turn/end` handler start a pending
     step instead of advancing.
   - `next_model` and `try_start` do nothing while a step runs.
5. **The step task.** It is tracked like the preparation task. It builds the request as
   `start_preparation` does and calls `ContextPolicy::compact_step` (trigger `Manual`, the
   turn's id if a turn is open, `command`). Its handler registers the task and returns.
6. **Step terminal.** On consuming a `context/compaction` with a `command`, the Loop clears the
   slot and resumes: `next_model` inside a turn, `try_start` otherwise.
7. **Interrupt.** A `stop/interrupt` cancels the running token and writes `cancelled` for a
   pending step. Steer leaves the step alone.
8. **Rebuild.** The slot is folded from events, and an in-flight command is settled with a
   `cancelled` fact.

Unchanged: turn and request lifecycles, input consumption, tool batches, steer of model
requests, budget recovery, `LoopSnapshot` and `context/prepare`.

### 5.5 What ContextPreparation gains

- `ContextPolicy::prepare` calls the extracted `compact_step`, with the K-turn cut and the
  trigger. Its preparation, steer, interrupt and reload handling are unchanged.
- `usage(cancel)` reads the committed head, builds the request as in §5.2 and runs the existing
  estimate. It writes nothing and may run during a turn.

## 6. Errors

`MethodError` codes (`code -32000` unless noted):

| Code | Method | When | Retry |
| --- | --- | --- | --- |
| `-32602` | both | `branchId` missing or not a string | No |
| `session_invalid_input` | both | Unknown branch (existing mapping of `AgentError::UnknownBranch`) | No |
| `session_closed` | both | Session or branch closing | No |
| `context_not_configured` | usage | No `context/configured` in the branch's history | After a model is adopted |
| `context_model_failed` | usage | `resolve` failed (no capability snapshot) | After selecting the model again |
| `compaction_failed` | compact | Fact `failed`: host error after the shared failure policy, empty summary, invalid cut, no reduction, no `context/configured`, or no capability snapshot | User's choice |
| `compaction_cancelled` | compact | Fact `cancelled`: `stop/interrupt` or reload | User's choice |
| `operation_interrupted` | compact | ContextMethods unloads while waiting (session-boundary A.1, `report`); the command stays in the Timeline and is still served or settled | User's choice |

Automatic compaction errors are unchanged. A host error becomes a `failed` fact and the request
continues uncompacted; under recovery it fails the request.

## 7. Security

- **Credentials stay request-local.** The connection comes from `ProviderConnectionResolver`
  inside one `compact` call and goes to `ModelHost`. It is never stored, returned, serialized or
  logged. Provider is unchanged.
- **No new data egress.** The summarizer sends the Session's own history to the provider and model
  that receive it on every turn.
- **The summary is untrusted model output.** It enters context only through the existing
  `summary_message`, which marks it as data and escapes it.
- **No secrets in results.** Facts, replies and diagnostics carry counts, ids and `HostError`
  text, which contains no credential values. No request or summary content goes to tracing.
- **Authorization.** Rpc authorizes `sessionId` for Session methods (C1). A connection served
  Session methods may compact, as it may submit input or stop.

## 8. Relationship to existing parts

| Part | Relationship |
| --- | --- |
| Loop | Supplement (§5.4): the compaction step. Budget recovery becomes active in the product profile because `ContextModelHost` is `Some` |
| ContextPreparation, ContextPolicy | Supplement (§5.2, §5.5): extracted `compact_step`, K-turn cut, trigger, `usage` |
| ModelProvider | Unchanged code. `validate` now enforces the window estimate in the product profile |
| Provider, ProductResolver, Aimux | Unchanged; used through `ProviderConnectionResolver` and `ModelHost` |
| AuxiliaryModel (S4) | Not used. Shares `invoke_with_recovery`; whichever lands first extracts it |
| `sessionMethods` (C1) | Required for the two methods. The host part does not need C1 and can land first |
| Turn consumers (clients, future Goal, Hooks) | Unaffected. A manual compaction outside a turn writes no `turn/*`; inside a turn it is one step of that turn |
| #2 dynamic events | `context/compact` migrates to dynamic registration when #2 lands; F1 is its first real consumer |
| Trace, HistoryRenderer, snapshots | Projection unchanged; manual facts are projected and snapshotted like automatic ones; Trace reads optional ids |
| `--models` profile, mock profile | Keep `ContextModelHost(None)`: no compaction, and `session.context` is not recorded |
| ManagedProviders plan usage, CLI `/compact` and `/usage` | Not part of F1's acceptance; the CLI calls the two methods |

## 9. Rule check

| Rule | Answer |
| --- | --- |
| Principles §1 (Capability, App Server boundary) | Two Session Capabilities and two supplements in `dim-agent-capabilities`; the App Server only installs them in its product profile. |
| §1 one event bus | The command and the fact are on the branch Timeline and consumed through its Dispatcher. The method's wait reads that Timeline. No side channel. |
| §3 dependency direction | New parts depend on core services (`ModelHost`, `ProviderConnectionResolver`, `TreeCommands`, Loop, ContextPreparation). Existing parts use them only through the existing `ContextModelHost` key, which is always provided where the part is installed. No optional dependency. |
| §3 changing another module (Loop) | Approved change. The need is the Loop's own control of the context window: compaction is a Loop step like a model call or a tool batch, and the Loop must not advance on a context a summary replaces. It closes within the Loop: automatic compaction already runs in the Loop's request preparation, and the manual step uses the same function under the same sequencer. No other module changes shape for it. |
| §4 rule 1 (command events) | The client asks the owner of progression with `context/compact`. The Loop decides when to serve it, and the result is one `context/compaction` fact that references the command. |
| §4 rule 2 (handoff) | Model requests are handed over with `context/prepare` as today. A manual step hands nothing over. |
| §4 rule 3 (before callback; handlers) | No before callback. Loop handlers register the step task and return. |
| §4 rule 4 (no business meaning in core) | Timeline, Dispatcher and facades unchanged. |
| §4 rule 5 (atomic batch) | Each new write is one event. |
| §4 rule 6 (context only from handed-off events) | The summary reaches the model only through the existing projection of a completed fact. Pending inputs are not consumed by a step (AGENTS rule 10). |
| §4 rule 7 (public events and services) | `TreeCommands`, `Branch::service` on `PublicService`s (`AgentLoop`, `ContextPreparation`), `ProviderConnectionResolver`, `ModelHost`, `sessionMethods`. |
| §4 rule 8 (infrastructure unchanged) | Timeline, Dispatcher, envelope, rutis, loader, Agent assembly and facades unchanged. The new event kind is plugin content in the existing envelope and moves to #2's dynamic registration when that exists. No requirement on rutis. |
| §4 rule 9 (only needed facts) | `context/compact` is read by the Loop's later decisions and on reload. `context/compaction` is used by projection and ends each manual step. Usage writes nothing. |
| ADR 002 D1 (scope) | Host and methods: Session. Command, step and slot: branch, in the Loop. |
| ADR 002 D2 (no wiring modules) | ContextMethods exposes owners' operations through the sanctioned `sessionMethods`; ProductContextModel is the feature's host. |
| ADR 002 D3 (dependencies) | Hard dependencies on core only. A Provider or Aimux reload propagates as it does to ModelProvider (accepted cost). |
| ADR 002 D4 (configuration) | No configuration; provisional constants are code (#128). |
| ADR 002 D5 (language-neutral contracts) | §4.1 and §4.3 are the contracts; §4.4 is one binding. |

**Impact (public content):**

- The `session.context` namespace (`compact`, `usage`).
- The command event `context/compact`.
- The changed `context/compaction` (optional ids, `command`, `cancelled`).

Internal Rust changes: `CompactionInput.cut` and `.trigger`; `PublicService` on `AgentLoop` and
`ContextPreparation`.

**Requirements raised to mechanism owners:** none. #2, already planned, will host
`context/compact` as a dynamic event.

## 10. Decisions and open questions

| Item | Decision |
| --- | --- |
| Which model compacts | The request's own model; a separate compaction model is deferred |
| Trigger and retention | `0.8 × (window − output)`, K = 2 turns, `bytes / 4`, 1,600 tokens per image: provisional, studied in #128 |
| Manual compaction | A Loop step scheduled by the `context/compact` command, inside or outside a turn, never creating a turn |
| Where the step runs | In the Loop, calling the one ContextPolicy compaction function (§2.1) |
| `context/compaction` | Changed directly: optional ids, `command`, `cancelled`; no storage version question (unreleased) |
| New event kind | `EventKind` variant now, dynamic registration after #2 |
| `--models` profile | No capability snapshot, no compaction: accepted |
| Provider-reported overflow | Separate Aimux issue, outside F1 |

Open: none beyond #128.

## 11. Acceptance (flow tests)

The tests run a product-profile App Server against a mock HTTP provider with a small context
window (`crates/dim-app-server/tests`), plus Loop flow tests in
`crates/dim-agent-capabilities/tests`.

1. **Automatic.** Past the threshold, the next preparation writes `context/budget` and sends one
   summarizer request (summarizer prompt, one user message, no tools, the Session's model). It
   commits `context/compaction` `completed` with the request's ids, no `command`,
   `trigger: threshold`, and the cut before the last 2 turns. The model request starts with the
   summary and keeps the last 2 turns verbatim.
2. **Budget recovery.** A request whose estimate reaches the window ends with budget facts. The
   Loop retries once, and the retry compacts with `trigger: provider_output_budget_exhausted`.
3. **Manual, no turn.** `session.context.compact` writes exactly `context/compact` and then
   `context/compaction {command}`, with no `turn/*`, no `step/*` and no `model/call`. The reply
   is `completed` with the fact's sequence. An input submitted during the step starts a turn
   after the fact, and its request starts with the summary.
4. **Manual, inside a turn.** With the command written during a tool batch:
   `step/tool-result` → `context/compaction {turn_id, command}` → `step/model-start` of the next
   request. That request is prepared from the compacted context, and the turn continues.
5. **Interrupt and reload.**
   - `session.stop` during a step writes `cancelled` (or `completed` if the host had returned),
     and the reply matches.
   - Restarting during a step writes `cancelled` with the reloading reason, without a second
     summarizer request.
6. **Usage.** `session.context.usage` returns the catalog window, output budget, threshold and
   an estimate. The estimate drops after a compaction, and no event is written.
7. **Failure and credentials.** A failing summarizer gives a `failed` fact: the automatic path
   continues uncompacted, and manual replies `compaction_failed`. No API key appears in facts,
   replies or diagnostics.
