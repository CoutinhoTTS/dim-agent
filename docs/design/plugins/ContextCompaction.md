# ContextCompaction

Status: Draft (for review). Roadmap item F1 ([#72](https://github.com/arcships/dim/issues/72)).
Overview: [Appendix B.1 ContextPreparation row](../overview.md#b1-session-and-branch-capabilities).
Scope: [ADR 002](../../adr/002-plugin-scope.md): Session parts plus supplements to the existing
branch capabilities Loop and ContextPreparation. Nothing in this document is implemented.
The trigger and retention values are provisional and are studied in
[#128](https://github.com/arcships/dim/issues/128).

## 1. Summary

**Compaction** replaces the history before a cut with a shorter replacement and keeps the last
turns verbatim. The replaced range is normally everything since the previous compaction point,
together with the previous replacement. From then on, the branch advances on the new result.

Compaction is part of how the Loop controls the context window. It is a Loop step, like a
model call or a tool batch, and it is not tied to a turn. While a compaction step runs, the
Loop does not advance to its next step. Otherwise the context could overflow during
compaction, and new events would be decided on the old context, inconsistent with the
replacement.

Automatic compaction already runs inside a request's preparation (`context/prepare` →
`context/compaction` → `model/call`), and the Loop waits for that request's terminal
([context_policy.rs](../../../crates/dim-agent-capabilities/src/context_policy.rs)). It never
runs in production, because the App Server binds `ContextModelHost(None)`
([main.rs](../../../crates/dim-app-server/src/main.rs)).

F1 separates three layers (§2.2) and adds:

1. **Model window facts.** `ContextModel` now carries only window facts: context window, output
   budget, threshold and token count. `ProductContextModel` takes them from the request's adopted
   capability snapshot, with a local token estimate.
2. **A strategy port.** A new `CompactionStrategy` port produces the replacement.
   `ProductSummaryStrategy` is the only strategy F1 implements: it summarizes with the request's
   own model, reaching it through the existing `ProviderConnectionResolver` and `ModelHost`.
3. **A general record.** A completed `context/compaction` carries
   `{cursor, replacement: Message[], strategy, …}`. There is one replay rule for every strategy:
   the history up to the cursor is replaced by `replacement`.
4. **One compaction path.** ContextPolicy's compaction step is one function. Automatic
   compaction calls it during preparation, and the Loop's manual step calls it too.
5. **Manual compaction as a Loop step.** `session.context.compact` writes the command
   `context/compact`, and the Loop schedules a compaction step:
   - Inside a turn, the step runs at the next step boundary, before the next request is
     prepared.
   - With no turn running, the step runs at once and writes only `context/compaction`.
6. **Usage query.** `session.context.usage` returns the current budget. It writes nothing.

What does not change: Provider, Aimux, ModelProvider, Timeline, Dispatcher, the event envelope,
rutis and `context/prepare`. F1 adds the event kind `context/compact` and changes
`context/compaction` directly; breaking changes are accepted because the software is
unreleased.

### 1.1 Why it is this small

- **Not AuxiliaryModel.** S4's `auxiliaryModel.complete` returns text only. Compaction needs the
  window facts of the model whose context it compacts
  ([auxiliary-model §1.1](https://github.com/arcships/dim/blob/roadmap/s4-provider-purpose-policy-and-auxiliarymod/docs/design/appserver/auxiliary-model.md)).
- **Not inside Provider.** The capability snapshot travels on the `ModelRequest`. Credentials are
  resolved by `ProviderConnectionResolver`, as ModelProvider resolves them for every turn.
  Execution is `ModelHost`.
- **No second state machine.** The Loop sequences every step. ContextPreparation runs the
  compaction function only inside a request preparation the Loop has already handed to it.

## 2. Structure

```
 Process scope (existing, unchanged)
 +-------------------------------------------------------------------+
 |  Aimux: ModelHost        Provider: ProviderConnectionResolver     |
 +----------------------------------^-------------------^------------+
                                    | inject (hard)     |
 Session scope                      |                   |
 +----------------------------------+-------------------+------------+
 |  ContextModelHost(Some(ProductContextModel))   session value (new)|
 |  ProductSummaryStrategy (new part)                                |
 |    provides S:CompactionStrategyHost(Some(summary strategy))      |
 |  ContextMethods (new part)                                        |
 |    provides sessionMethods["session.context"]: compact, usage     |
 |      compact -> TreeCommands.get(b).service::<AgentLoop>().compact()
 |      usage   -> TreeCommands.get(b).service::<ContextPreparation>().usage()
 +---------------------------------^---------------------------------+
            Rpc (C1) dispatches ---'
 Branch scope (one Timeline, one Dispatcher)
 +-------------------------------------------------------------------+
 |  Loop (supplement): sequences steps                               |
 |    model request -> context/prepare (handoff, unchanged)          |
 |    compaction step (manual) -> ContextPolicy::compact_step        |
 |  ContextPreparation (supplement)                                  |
 |    in preparation: ContextPolicy::prepare -> compact_step         |
 |  ContextPolicy::compact_step: retention rule, strategy.compact,   |
 |    validate, append context/compaction, snapshot                  |
 |  Projection (history.rs): replacement + baseline + entries after  |
 |    the cursor                                                     |
 +-------------------------------------------------------------------+
```

`ProductContextModel` needs no service, so it is a plain Session value. ContextMethods is a
separate part from the strategy because branches inject `CompactionStrategyHost`, and its
provider must not depend on `TreeCommands`, which the branch tree provides.

### 2.1 Where the manual step lives

The Loop runs the manual compaction step itself, in a tracked task, by calling
`ContextPolicy::compact_step`. It does not drive ContextPreparation through an event. The reasons:

- **The Loop already holds what the step needs.** Its `Deps` already inject `Timeline`,
  `HistoryRenderer` and `ContextModelHost`
  ([agent_loop.rs](../../../crates/dim-agent-capabilities/src/agent_loop.rs) `LoopPlugin::Deps`);
  F1 adds `CompactionStrategyHost` beside them. The Loop already builds the request from the
  latest `context/configured` in `start_preparation`.
- **Nothing is handed over.** `context/prepare` hands a request to ContextPreparation and then to
  ModelProvider. A manual step makes no model request, so a handoff would only add a second
  command and a second single-flight slot (ContextPreparation's `current`).
- **No race, by construction.** The Loop starts a manual step only when no request of the branch
  is in flight, and it starts no request while a step runs. ContextPreparation calls the step
  only inside the preparation of a request in flight. So there is one sequencer, one function
  and one fact.

### 2.2 Extension points

| Layer | What it fixes | Where | Changes when a new strategy is added |
| --- | --- | --- | --- |
| Mechanism | When compaction runs, and that the Loop holds while it runs: the automatic trigger in preparation, the manual Loop step, the retention rule, validation, commit | Loop, ContextPolicy | No |
| Contract | The record and its replay: `context/compaction {cursor, replacement, strategy, context_baseline}`; replay puts `replacement`, then the keyed baseline, then the entries after the cursor. It is deterministic, so fork, reload and prompt-cache prefixes see the same messages | `dim-agent-types`, `history.rs` projection | No |
| Strategy | What the replacement is, and the cut within the retention rule | `CompactionStrategyHost` Session value | It is replaced |

**Installing another strategy.** A strategy is installed by replacing the
`CompactionStrategyHost` Session value: the assembly registers another value, or another Session
part that provides it, instead of `ProductSummaryStrategy`.

- ContextPreparation and the Loop depend on the key, not on the strategy's plugin, so no
  optional dependency is added.
- Old records stay replayable because replay never calls the strategy.
- Examples of later strategies: a sliding window with notes, or old tool calls rendered as short
  stubs.

**Not done in F1:** strategies other than the summary strategy, and composing several strategies.

## 3. Data and state

| Item | Owner | Kind |
| --- | --- | --- |
| `context/compact {}` | Loop | New command event (§4.3) |
| `context/compaction` | Written by ContextPolicy for its caller | Existing fact, changed (§4.3) |
| Compaction slot: `{command, running: Option<CancellationToken>}` | Loop | Private. Folded from `context/compact` and `context/compaction`, rebuilt on load (§5.4) |

The Session values hold no state. A resolved connection lives only inside one strategy call.

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
- It then waits on the branch Timeline for the first `context/compaction` whose `command ≥ c`.
- It maps the outcome: `completed` → `{status: "completed", sequence}`;
  `skipped` → `{status: "skipped", reason}`; `failed` → `compaction_failed`;
  `cancelled` → `compaction_cancelled`.

`session.context.usage` returns the values a preparation at the committed head `through` would
write into `context/budget`, plus `thresholdTokens`.

Cancellation follows session-boundary A.1: an unload interrupts the call, and a caller's
disconnect cancels nothing.

### 4.2 Ports (Rust, runtime-internal)

The existing `ContextModel` port is split in two. Each comes from its own Session value.

| Port | Session value | Methods | F1 implementation |
| --- | --- | --- | --- |
| `ContextModel` (narrowed) | `ContextModelHost(Option<Arc<dyn ResolveContextModel>>)` (existing) | `context_window`, `output_tokens`, `threshold_tokens`, `count_tokens`; `compact` moves out | `ProductContextModel` (§5.1) |
| `CompactionStrategy` (new) | `CompactionStrategyHost(Option<Arc<dyn CompactionStrategy>>)` | `compact(input, cancel) -> Completed {cursor, replacement, strategy, diagnostics} \| Skipped {reason, diagnostics} \| Err(HostError)` | `ProductSummaryStrategy` (§5.2) |

Both values are bound explicitly in every profile, as `ContextModelHost` is today. The policy
exists when both are `Some`. One `Some` without the other is an assembly defect that
ContextPreparation reports when it loads.

`CompactionInput` is produced by ContextPolicy:

| Field | Meaning |
| --- | --- |
| `history` | The projected entries after the previous cursor (existing) |
| `checkpoint` | The previous `{cursor, replacement, context_baseline}` (changed from `summary`) |
| `request`, `model` | The request whose context is compacted, and its `ContextModel` window facts (new) |
| `retain_from` | First entry of the oldest retained turn; the cut must lie before it (new; §5.3) |
| `fallback` | `true` under budget recovery: any legal cut is allowed when none lies before `retain_from` (new) |
| `trigger` | `Threshold`, `ProviderOutputBudgetExhausted` or `Manual` (new) |
| estimates, `provider_budget` | Existing |

### 4.3 Events

| Event | Writer | Consumer | Data |
| --- | --- | --- | --- |
| `context/compact` (new) | Loop, through its public write operation `AgentLoop::compact` | Loop, in sequence order | `{}` |
| `context/compaction` (changed) | ContextPolicy, for its caller | Loop (manual step terminal), ContextPolicy (retry reuse), projection, ContextMethods | `{turn_id?, request_id?, command?, through, outcome}` |

`outcome` is one of:

- `completed {cursor, replacement: Message[], strategy: string, diagnostics, context_baseline}`.
  `replacement` replaces `summary: String`, and `strategy` names the producer, for example
  `"summary"`.
- `skipped {reason, diagnostics}`.
- `failed {error, diagnostics?}`.
- `cancelled {reason}` (new), so every manual step ends with exactly one fact.

| Origin | `turn_id` | `request_id` | `command` |
| --- | --- | --- | --- |
| Automatic (threshold, budget recovery) | the request's turn | the request | absent |
| Manual step inside a turn | the turn | absent | sequence of the `context/compact` served |
| Manual step with no turn | absent | absent | sequence of the `context/compact` served |

**Replay rule (every strategy):**

- The projected messages are `replacement`, then the cursor's keyed `context_baseline` blocks,
  then the history entries after `cursor`.
- `replacement` is stored exactly as produced, so replay never calls a strategy and never
  re-renders a wrapper.
- This changes `context_messages` and ContextPolicy's `project` (which today push
  `summary_message(summary)`) and `CompactionCheckpoint` (`replacement` instead of `summary`).
- The history snapshot state is unchanged: it stores `compacted_through`, and the replacement is
  read from the anchoring fact.

Readers update with the struct:

- ContextPolicy's retry filter compares `turn_id` with `Some(turn)`.
- Trace reads optional ids.
- Native storage indexes completions by `through` only.
- The frozen v1 samples are not changed. The v1-to-native conversion maps `summary` to
  `replacement: [summary_message(summary)]` with `strategy: "summary"`, and maps their ids to
  `Some`.

**Dynamic events (decision A).** `context/compact` is added as an `EventKind` variant now. When
#2 lands (plugins register their own event types; Timeline checks structure and type range,
not content), it migrates to dynamic registration owned by the Loop. F1 is #2's first real
consumer. `tree_protocol.rs` classifies `context/compact` as not client-visible.

### 4.4 Rust binding

```rust
// dim-agent-types/src/events.rs
EventKind::ContextCompact(ContextCompactData {})                  // "context/compact"
pub struct ContextCompactionData {
    pub turn_id: Option<String>, pub request_id: Option<String>,
    pub command: Option<u64>, pub through: u64, pub outcome: ContextCompactionResult }
pub enum ContextCompactionResult {
    Completed { cursor: String, replacement: Vec<Message>, strategy: String,
                diagnostics: Box<CompactionDiagnosticsRecord>,
                context_baseline: Vec<ContextBaselineEntry> },
    Skipped { .. }, Failed { .. }, Cancelled { reason: String } }

// dim-agent-capabilities/src/host.rs
pub trait ContextModel { fn context_window(&self) -> f64; fn output_tokens(&self) -> f64;
                         fn threshold_tokens(&self) -> f64;
                         fn count_tokens(&self, r: &ModelRequest) -> f64; }   // compact removed
pub trait CompactionStrategy: Send + Sync + 'static {
    fn compact(&self, input: CompactionInput, cancel: CancellationToken)
        -> HostFuture<'_, StrategyOutcome>; }          // Completed | Skipped
pub struct CompactionStrategyHost(pub Option<Arc<dyn CompactionStrategy>>);
pub struct CompactionCheckpoint { pub cursor: String, pub replacement: Arc<[Message]>,
                                  pub context_baseline: Arc<[ContextBaselineEntry]> }

// dim-agent-capabilities/src/context_policy.rs (crate-internal)
impl ContextPolicy { async fn compact_step(&self, step: CompactionStep, cancel: CancellationToken)
                         -> Result<Arc<AgentEvent>, HostError>; }   // committed fact

// dim-agent-capabilities/src/agent_loop.rs
impl AgentLoop { pub fn compact(&self) -> Result<u64>; }  // appends context/compact
impl PublicService for AgentLoop {}
// dim-agent-capabilities/src/context_preparation.rs
impl ContextPreparation { pub async fn usage(&self, cancel: CancellationToken)
                              -> Result<ContextUsage, ContextOpError>; }
impl PublicService for ContextPreparation {}

// dim-agent-capabilities: context_model.rs, summary_strategy.rs, context_methods.rs (new)
pub struct ProductContextModel;                    // ResolveContextModel, no dependencies
pub struct ProductSummaryStrategyPlugin { scope: SessionScope }
//   Deps = (Arc<ModelHost>, Arc<ProviderConnectionResolver>); provides CompactionStrategyHost
pub struct ContextMethodsPlugin { scope: SessionScope }
//   Deps = InSession<TreeCommands>; SessionMethods::provide(ctx, &scope, "session.context", ..)

// dim-app-server/src/main.rs, product profile only (no --models file):
builder.session_value(Arc::new(ContextModelHost(Some(Arc::new(ProductContextModel)))))
       .session_plugin(ProductSummaryStrategyPlugin::new)
       .session_plugin(ContextMethodsPlugin::new)
       .session_methods("session.context");
// --models and mock profiles bind ContextModelHost(None) and CompactionStrategyHost(None)
```

## 5. Flows

### 5.1 ProductContextModel

`resolve(request)`:

1. It requires `request.capability_snapshot`, valid for the request's model. Without it, it
   returns `InvalidRequest` "Model capabilities are not adopted; select the model again", the
   same condition and text as `ProductResolver`.
2. It returns window facts bound to the request:

| Method | Value |
| --- | --- |
| `context_window` | `snapshot.capabilities.contextWindow`, the catalog value with its existing overrides |
| `output_tokens` | `request.config.maxTokens` if set, else `snapshot.capabilities.maxOutputTokens`; never above the latter |
| `threshold_tokens` | `0.8 × (context_window − output_tokens)`, provisional (#128) |
| `count_tokens` | UTF-8 bytes of the serialized system prompt, messages and tool declarations divided by 4; each image part counts a fixed 1,600 tokens. Provisional (#128). Used for deltas over the provider's `prompt_tokens`, the first request, `validate` and the reduction check |

### 5.2 ProductSummaryStrategy

`compact(input, cancel)`:

1. **Cut.** The latest legal cut before `input.retain_from`. With `input.fallback`, if no such
   cut exists, the latest legal cut. If there is still none: `Skipped {reason:
   "nothing_to_compact"}`, with no model call.
2. **Summary.** It summarizes the previous replacement (from `input.checkpoint`) plus the entries
   up to and including the cut.
3. **Result.** It returns `Completed {cursor: cut, replacement: [summary_message(summary)],
   strategy: "summary", diagnostics}`. The existing `summary_message` wrapper, which marks the
   text as data and escapes it, is applied once here and stored in the record.

How the summary is produced:

- **Request.** One summarizer request per chunk. It uses `input.request`'s provider, model,
  snapshot and config, with `maxTokens = model.output_tokens()`, a fixed summarizer system
  prompt owned by this module, one user message with the transcript, and no tools.
- **Credentials and failures.** The connection comes from `ProviderConnectionResolver` and is
  used only inside this call. The call runs through `invoke_with_recovery`, ModelProvider's
  failure and credential policy. S4 extracts it; if F1 lands first, F1 does the same extraction.
- **Output.** Thinking output is dropped. A tool stop or an empty text is a failure.
- **Transcript.** Role-labelled text: tool calls as name and input, tool results as text, images
  as `[image]`.
- **Chunking.** An entry larger than one call's input budget is truncated in the middle and
  counted in `compactedToolPayloadCount`. A longer transcript is folded in chunks: each call
  gets the running summary and the next chunk.
- **Diagnostics.** The existing record, filled from the run (`reason` and `trigger` from
  `input.trigger`).

### 5.3 One compaction path (ContextPolicy)

`compact_step` is the existing compaction section of `ContextPolicy::prepare`, extracted:

1. **Retention boundary.** `retain_from` is the first entry of the oldest of the last K turns
   on the branch (provisional K = 2, #128), the running turn included. A turn runs from
   `turn/start` to `turn/end`.
   - An entry belongs to a turn by the sequence of its source event in the fixed context.
   - Entries restored from a snapshot's state predate that snapshot's compaction and count as
     older than any retained turn.
2. **Strategy.** It calls `strategy.compact` with `retain_from`, `fallback` (under recovery)
   and `trigger`.
3. **Validation.** For any strategy, it checks that:
   - the cursor passes the existing legal-cut check;
   - the cursor lies before `retain_from`, unless `fallback` applies;
   - the projected request is smaller than before and below the window.

   A failed check turns the outcome into `failed`.
4. **Commit.** It appends `context/compaction` with the caller's ids and `command`, then saves
   the optional snapshot.

Callers:

- **ContextPreparation, inside `prepare`.** At the threshold (once per boundary, as today) or
  under `recovery`. The rest of `prepare` is unchanged.
- **The Loop's manual step.** Trigger `Manual`. The request is built as `start_preparation`
  builds one: the latest `context/configured` settings and system with the projected messages,
  without binding tools.

### 5.4 Manual compaction

**Inside a turn:**

```
      step/model-start {T, R1} ... step/model-output-end {T, R1, completed}
 k    context/compact {}                          writer: Loop (AgentLoop::compact)
      -- Loop consumes k: a request is in flight -> pending
      step/tool-call {T, R1} ... step/tool-result {T, R1}
      -- Loop consumes the tool result (boundary b): pending -> compaction step
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
       a turn now, prepared from the compacted context)
```

**Scheduling rules:**

- **Boundary.** A step boundary is where the Loop would prepare the next request (`next_model`
  without a budget-retry source) or where the turn ends. A pending step starts at the first
  boundary.
  - If the turn ends instead, the step runs right after `turn/end`, with no turn.
  - A budget retry belongs to its request and is not a boundary.
- **Through.** `through` is the sequence of the event whose consumption started the step.
- **Merging.** A command consumed while a step is pending merges into it; the fact references
  the latest merged command. A command consumed while a step runs schedules one more step after
  it.
- **No advance.** While a step runs, `next_model` and `try_start` wait for its fact. Inputs queue
  as usual and are not consumed by the step.

**Steer, interrupt and reload:**

- **Steer.** A driving input applied during a step does not cancel it. It joins the next request,
  or starts a turn after the step.
- **Interrupt.** A `stop/interrupt` cancels a running step and drops a pending one, consistent
  with automatic compaction:
  - A result the strategy already returned is committed (`completed`).
  - Otherwise the fact is `cancelled {reason}`.
  - Inside a turn, the turn then ends `Cancelled` through the existing `next_model` check.
- **Reload.** A consumed `context/compact` without its fact is settled by the new Loop
  generation with `cancelled: "capability reloading"`. The strategy call is not replayed.

### 5.5 What the Loop gains

All in `crates/dim-agent-capabilities/src/agent_loop.rs`:

1. **Subscription.** `context/compact` and `context/compaction` are added. `CompactionStrategyHost`
   is added to `Deps`.
2. **Public write.** `AgentLoop::compact()` appends `context/compact` and returns its sequence.
   `AgentLoop` gets the `PublicService` marker.
3. **Compaction slot**, folded from consumed events. On consuming `context/compact`, the Loop
   marks the step pending, and starts it at once if no request is in flight.
4. **Boundaries.**
   - `next_model` (except with a budget-retry source) and `turn/end` start a pending step instead
     of advancing.
   - `next_model` and `try_start` do nothing while a step runs.
5. **The step task.** It is tracked like the preparation task, builds the request as
   `start_preparation` does, and calls `ContextPolicy::compact_step`.
6. **Step terminal.** On consuming a `context/compaction` with a `command`, the Loop clears the
   slot and resumes: `next_model` inside a turn, `try_start` otherwise.
7. **Interrupt.** A `stop/interrupt` cancels the running step and writes `cancelled` for a
   pending one.
8. **Rebuild.** The slot is folded from events, and an in-flight command is settled with
   `cancelled`.

Unchanged: turn and request lifecycles, input consumption, tool batches, steer of model
requests, budget recovery, `LoopSnapshot` and `context/prepare`.

### 5.6 What ContextPreparation gains

- `ContextPolicy::prepare` calls `compact_step`. ContextPreparation injects
  `CompactionStrategyHost`, and its policy exists when both hosts are `Some`.
- `usage(cancel)` reads the committed head and runs the existing estimate. It writes nothing.

## 6. Errors

`MethodError` codes (`code -32000` unless noted):

| Code | Method | When | Retry |
| --- | --- | --- | --- |
| `-32602` | both | `branchId` missing or not a string | No |
| `session_invalid_input` | both | Unknown branch | No |
| `session_closed` | both | Session or branch closing | No |
| `context_not_configured` | usage | No `context/configured` in the branch's history | After a model is adopted |
| `context_model_failed` | usage | `resolve` failed (no capability snapshot) | After selecting the model again |
| `compaction_failed` | compact | Fact `failed`: strategy error after the shared failure policy, empty summary, a cursor that fails validation, no reduction, no `context/configured`, or no snapshot | User's choice |
| `compaction_cancelled` | compact | Fact `cancelled`: `stop/interrupt` or reload | User's choice |
| `operation_interrupted` | compact | ContextMethods unloads while waiting; the command is still served or settled | User's choice |

Automatic compaction errors are unchanged: a strategy error becomes a `failed` fact and the
request continues uncompacted; under recovery it fails the request.

## 7. Security

- **Credentials stay request-local.** The connection comes from `ProviderConnectionResolver`
  inside one strategy call and goes to `ModelHost`. It is never stored, returned, serialized or
  logged. Provider is unchanged.
- **No new data egress.** The summarizer sends the Session's own history to the provider and model
  that receive it on every turn.
- **The replacement is untrusted model output.** The summary strategy stores it inside the
  existing `summary_message` wrapper, which marks it as data.
- **No secrets in results.** Facts, replies and diagnostics carry counts, ids, replacement
  messages and `HostError` text, which contains no credential values. No request or summary
  content goes to tracing.
- **Authorization.** Rpc authorizes `sessionId` for Session methods (C1).

## 8. Relationship to existing parts

| Part | Relationship |
| --- | --- |
| Loop | Supplement (§5.5). Budget recovery becomes active in the product profile |
| ContextPreparation, ContextPolicy | Supplement (§5.3, §5.6): `compact_step`, retention rule, validation of any strategy, `usage` |
| Projection (`history.rs`), snapshots | One replay rule using `replacement`; snapshot state unchanged |
| `HistoryRenderer` | Render-time projections that run on every request without a record are not compaction. An example is truncating old tool output by a fixed rule. They belong to the existing injectable `HistoryRenderer`, not to a strategy |
| ModelProvider | Unchanged code. `validate` now enforces the window estimate in the product profile |
| Provider, ProductResolver, Aimux | Unchanged; the summary strategy uses `ProviderConnectionResolver` and `ModelHost` |
| AuxiliaryModel (S4) | Not used. Shares `invoke_with_recovery`; whichever lands first extracts it |
| `sessionMethods` (C1) | Required for the two methods. The host values and the strategy do not need C1 |
| Turn consumers (clients, future Goal, Hooks) | Unaffected. A manual compaction outside a turn writes no `turn/*` |
| #2 dynamic events | `context/compact` migrates to dynamic registration; F1 is #2's first real consumer |
| `--models` profile, mock profile | Both hosts `None`: no compaction, and `session.context` is not recorded |
| ManagedProviders plan usage, CLI `/compact` and `/usage` | Not part of F1's acceptance |

## 9. Rule check

| Rule | Answer |
| --- | --- |
| Principles §1 (Capability, App Server boundary) | New parts and supplements live in `dim-agent-capabilities`; the App Server only installs them in its product profile. |
| §1 one event bus | The command and the fact are on the branch Timeline and consumed through its Dispatcher. The method's wait reads that Timeline. |
| §3 dependency direction | New parts depend on core services. ContextPreparation and the Loop depend on the `ContextModelHost` and `CompactionStrategyHost` keys, which every profile binds explicitly. No optional dependency. |
| §3 changing another module (Loop) | Approved change. The need is the Loop's own control of the context window: compaction is a Loop step like a model call or a tool batch, and the Loop must not advance on a context the replacement supersedes. It closes within the Loop: automatic compaction already runs in its request preparation, and the manual step uses the same function under the same sequencer. |
| §3 extension without change (three layers) | The mechanism (Loop step and trigger) and the contract (record and replay) are fixed. The strategy is a replaceable Session value. A new strategy changes neither the Loop, nor the record, nor replay, and replay of old records never calls a strategy (§2.2). |
| §4 rule 1 (command events) | The client asks the owner of progression with `context/compact`. The Loop decides when to serve it, and the result is one `context/compaction` that references the command. |
| §4 rule 2 (handoff) | Model requests are handed over with `context/prepare` as today. A manual step hands nothing over. |
| §4 rule 3 (before callback; handlers) | No before callback. Loop handlers register the step task and return. |
| §4 rule 4 (no business meaning in core) | Timeline, Dispatcher and facades unchanged. |
| §4 rule 5 (atomic batch) | Each new write is one event. |
| §4 rule 6 (context only from handed-off events) | The replacement reaches the model only through the projection of a completed fact. Pending inputs are not consumed by a step (AGENTS rule 10). |
| §4 rule 7 (public events and services) | `TreeCommands`, `Branch::service` on `PublicService`s, `ProviderConnectionResolver`, `ModelHost`, `sessionMethods`. |
| §4 rule 8 (infrastructure unchanged) | Timeline, Dispatcher, envelope, rutis, loader, Agent assembly and facades unchanged. The new event kind moves to #2's dynamic registration later. |
| §4 rule 9 (only needed facts) | `context/compact` is read by the Loop's later decisions and on reload. `context/compaction` is used by projection. Usage writes nothing. |
| ADR 002 D1 (scope) | Window facts, strategy and methods: Session. Command, step and slot: branch, in the Loop. |
| ADR 002 D2 (no wiring modules) | ContextMethods exposes owners' operations through `sessionMethods`. |
| ADR 002 D3 (dependencies) | Hard dependencies on core only. A Provider or Aimux reload propagates to the strategy part (accepted cost). |
| ADR 002 D4 (configuration) | No configuration; provisional constants are code (#128). |
| ADR 002 D5 (language-neutral contracts) | §4.1 and §4.3 are the contracts; §4.4 is one binding. |

**Impact (public content):**

- The `session.context` namespace.
- The command `context/compact`.
- The changed `context/compaction` (optional ids, `command`, `cancelled`, `replacement` and
  `strategy` instead of `summary`).

Internal Rust changes:

- The split ports and `CompactionStrategyHost`.
- `CompactionInput` fields.
- `CompactionCheckpoint.replacement`.
- `PublicService` on `AgentLoop` and `ContextPreparation`.

**Requirements raised to mechanism owners:** none. The already planned #2 will host
`context/compact`.

## 10. Decisions and open questions

| Item | Decision |
| --- | --- |
| Which model compacts | The request's own model, in the summary strategy; a separate model is deferred |
| Trigger and retention | `0.8 × (window − output)`, K = 2 turns, `bytes / 4`, 1,600 tokens per image: provisional, #128 |
| Manual compaction | A Loop step scheduled by `context/compact`, inside or outside a turn, never creating a turn |
| Where the step runs | In the Loop, calling the one ContextPolicy compaction function (§2.1) |
| Record | `replacement` and `strategy` with one replay rule; optional ids, `command`, `cancelled`; changed directly (unreleased) |
| Ports | `ContextModel` (window facts) and `CompactionStrategy` (replacement), each from its own Session value |
| Other strategies, composition | Not done in F1 |
| New event kind | `EventKind` variant now, dynamic registration after #2 |
| `--models` profile | No capability snapshot, no compaction: accepted |
| Provider-reported overflow | Separate Aimux issue, outside F1 |

Open:

1. **Diagnostics per strategy.** `CompactionDiagnosticsRecord` has required summary-oriented
   fields (`summaryChars`, `estimatedSummaryTokens`, …). It fits the only F1 strategy. Should the
   record become strategy-neutral now (common counts plus a strategy-owned object), or when the
   second strategy arrives?

## 11. Acceptance (flow tests)

The tests run a product-profile App Server against a mock HTTP provider with a small context
window (`crates/dim-app-server/tests`), plus Loop and policy flow tests in
`crates/dim-agent-capabilities/tests`.

1. **Automatic.** Past the threshold, the next preparation writes `context/budget` and sends one
   summarizer request. It commits `context/compaction` `completed` with the request's ids, no
   `command`, `strategy: "summary"`, a one-message `replacement`, and the cut before the last
   2 turns. The model request starts with that message and keeps the last 2 turns verbatim.
2. **Budget recovery.** The retry compacts with `trigger: provider_output_budget_exhausted`.
3. **Manual, no turn.** `session.context.compact` writes exactly `context/compact` and then
   `context/compaction {command}`, with no `turn/*`, `step/*` or `model/call`. The reply is
   `completed`.
4. **Manual, inside a turn.** `step/tool-result` → `context/compaction {turn_id, command}` →
   the next request's `step/model-start`, prepared from the compacted context.
5. **Replay and strategy replacement.**
   - A Session reloaded after compaction, and a branch forked after it, project the same messages
     as before.
   - A test strategy bound in place of the summary strategy, returning a two-message
     `replacement`, is committed and replayed by the same rule, with no Loop or policy change.
   - The policy rejects a test strategy whose cursor lies inside the retained turns.
6. **Interrupt and reload.** `session.stop` during a step writes `cancelled` (or `completed` if
   the strategy had returned). A restart during a step writes `cancelled` without a second
   strategy call.
7. **Usage, failure, credentials.**
   - Usage returns the window facts and an estimate, and writes nothing.
   - A failing summarizer gives a `failed` fact.
   - No API key appears in facts, replies or diagnostics.
