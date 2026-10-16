# ContextCompaction

Status: Implemented on the F1 branch (approved design; implementation notes in §12). Roadmap item F1 ([#72](https://github.com/arcships/dim/issues/72)).
Overview: [Appendix B.1 ContextPreparation row](../overview.md#b1-session-and-branch-capabilities).
Scope: [ADR 002](../../adr/002-plugin-scope.md): Session parts plus supplements to the existing
branch capabilities Loop and ContextPreparation. §12 records where the implementation refines this text.
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
compaction → `model/call`), and the Loop waits for that request's terminal
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
   `{retain_from, replacement: Message[], strategy, …}`, where `retain_from` is the id of the first
   Timeline event kept verbatim. There is one replay rule for every strategy: everything before
   `retain_from` is replaced by `replacement`.
4. **One compaction path.** ContextPolicy's compaction step is one function. Automatic
   compaction calls it during preparation, and the Loop's manual step calls it too.
5. **Manual compaction as a Loop step.** `session.context.compact` writes the command
   `context/compact`, and the Loop schedules a compaction step:
   - Inside a turn, the step runs at the next step boundary, before the next request is
     prepared.
   - With no turn running, the step runs at once and writes only its compaction facts.
6. **Compaction state as facts.** Every strategy call is bracketed by two Timeline facts: a
   `context/compaction-start` written just before the call, and exactly one `context/compaction`
   result that refers to it. Clients, history and reload settlement read both (§4.3, §5.7).
7. **Usage query.** `session.context.usage` returns the current budget. It writes nothing.

What does not change: Provider, Aimux, ModelProvider, Timeline, Dispatcher, the event envelope,
rutis and `context/prepare`. F1 adds the event kinds `context/compact` and
`context/compaction-start` and changes `context/compaction` directly; breaking changes are accepted because the software is
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
 |  Projection (history.rs): replacement + baseline + entries from   |
 |    retain_from                                                    |
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
| Contract | The record and its replay: `context/compaction {retain_from, replacement, strategy, context_baseline}`, where `retain_from` is a Timeline event id; replay puts `replacement`, then the keyed baseline, then the entries placed by `retain_from` and later events. The record references Timeline events, not rendering details, so another `HistoryRenderer` or strategy still finds the cut. Replay is deterministic, so fork, reload and prompt-cache prefixes see the same messages | `dim-agent-types`, `history.rs` projection | No |
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
| `context/compaction-start` | Written by ContextPolicy for its caller | New fact (§4.3) |
| `context/compaction` | Written by ContextPolicy for its caller, or by the settling owner on reload | Existing fact, changed (§4.3) |
| Compaction slot: `{pending: Option<command>, running: Option<(command, CancellationToken)>}` | Loop | Private. The active command and the latest pending command are kept apart. Folded from commands, starts and results, rebuilt on load (§5.5) |

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
- It then waits on the branch Timeline for the result that **covers** `c`: the first
  `context/compaction` whose `command = c`, or, as long as no `context/compaction-start` with
  `command = c` has been written, the first one whose `command > c`. A step starts only under the
  command it serves, so the second case is a command coalesced into a later pending command
  (§5.4); it is answered by the result of the step that served the later command. A command that
  was started is answered only by its own result, never by a later command's.
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
| `CompactionStrategy` (new) | `CompactionStrategyHost(Option<Arc<dyn CompactionStrategy>>)` | `compact(input, cancel) -> Completed {retain_from, replacement, strategy, diagnostics} \| Skipped {reason, diagnostics} \| Err(HostError)` | `ProductSummaryStrategy` (§5.2) |

Both values are bound explicitly in every profile, as `ContextModelHost` is today. The policy
exists when both are `Some`. One `Some` without the other is an assembly defect that
ContextPreparation reports when it loads.

`CompactionInput` is produced by ContextPolicy:

| Field | Meaning |
| --- | --- |
| `history` | The projected entries from the previous `retain_from` on (existing) |
| `checkpoint` | The previous `{retain_from, replacement, context_baseline}` (changed from `cursor` and `summary`) |
| `request`, `model` | The request whose context is compacted, and its `ContextModel` window facts (new) |
| `retain_limit` | Event id of the first event of the oldest of the last K turns; the strategy's `retain_from` must be at or before it (new; §5.3) |
| `fallback` | `true` under budget recovery: a later legal `retain_from` is allowed when none lies at or before `retain_limit` (new) |
| `trigger` | `Threshold`, `ProviderOutputBudgetExhausted` or `Manual` (new) |
| estimates, `provider_budget` | Existing |

### 4.3 Events

| Event | Writer | Consumer | Data |
| --- | --- | --- | --- |
| `context/compact` (new) | Loop, through its public write operation `AgentLoop::compact` | Loop, in sequence order | `{}` |
| `context/compaction-start` (new) | ContextPolicy in `compact_step`, for its caller (Loop step or ContextPreparation) | Loop and ContextPreparation (rebuild), clients | `{turn_id?, request_id?, command?, through, trigger: "manual" \| "threshold" \| "provider_output_budget_exhausted", strategy}` |
| `context/compaction` (changed) | ContextPolicy, for its caller; on reload, the settling owner | Loop (manual step terminal), ContextPolicy (retry reuse), projection, ContextMethods, clients | `{turn_id?, request_id?, command?, start?, through, outcome}` |

`outcome` is one of:

- `completed {retain_from, replacement: Message[], strategy: string, diagnostics, context_baseline}`.
  `retain_from` (a Timeline event id) replaces the renderer-level `cursor`, `replacement`
  replaces `summary: String`, and `strategy` names the producer, for example `"summary"`.
- `skipped {reason, diagnostics}`.
- `failed {error, diagnostics?}`.
- `cancelled {reason}` (new), so every manual step ends with exactly one fact.

| Origin | `turn_id` | `request_id` | `command` |
| --- | --- | --- | --- |
| Automatic (threshold, budget recovery) | the request's turn | the request | absent |
| Manual step inside a turn | the turn | absent | sequence of the `context/compact` served |
| Manual step with no turn | absent | absent | sequence of the `context/compact` served |

**Start and result pairing.**

- `compact_step` writes `context/compaction-start` immediately before it calls
  `strategy.compact`. A compaction that ends before any strategy call writes no start.
- Every start is followed by exactly one `context/compaction` with `start` = the start's
  sequence, and the same `turn_id`, `request_id`, `command` and `through`. Its outcome is
  `completed`, `skipped`, `failed` or `cancelled`.
- A result without `start` exists only for a manual command that ended before any strategy call:
  - `failed` when no request could be built (no `context/configured`, no capability snapshot);
  - `cancelled` when a pending command was dropped by an interrupt or settled on reload.
- Automatic compaction that never reaches the strategy writes neither fact, as today.
- At most one start per branch is unmatched at any time: the Loop never runs a manual step while
  a request is in flight, and ContextPreparation runs one preparation at a time.
- On reload, an unmatched start means a compaction was in flight, and its owner writes the
  `cancelled` result (§5.8). Automatic compaction now also writes a `cancelled` result when it is
  cancelled after its start (interrupt or steer), instead of writing no fact.

**The cut: `retain_from`.** A completed result names the first Timeline event kept verbatim.
Cuts fall only on event boundaries.

- **Replaced range.** The events from the previous completed compaction's `retain_from`
  (inclusive; or from the branch start) up to this `retain_from` (exclusive), together with the
  previous `replacement`. A reader derives it from the two records alone.
- **Validity.** No tool call started before `retain_from` may have its result at or after it.
  The keyed baseline is the authoritative in-effect state just before `retain_from`, as today.
- **Why an event id and not the old cursor.**
  - The old `cursor` was a `HistoryEntry` id such as `<event id>:block:<i>`, which depends on
    the injectable `HistoryRenderer`'s id scheme. A different renderer would break old records.
  - The record is content that users and other state machines read from history, so it must
    reference Timeline events, not rendering details. Event ids are also stable across a fork's
    inherited prefix.
  - With K-turn retention, the cut is naturally the first event of the earliest retained turn,
    which is an event boundary.
- **Mapping at projection time.** ContextPolicy and `history.rs` map `retain_from` to a position
  in the current rendering: the first rendered entry whose source event is at or after
  `retain_from` in sequence order. For this, `HistoryEntry` carries the id of the event that
  placed it (`source`), so the id scheme of entries no longer matters.

**Replay rule (every strategy):**

- The projected messages are `replacement`, then the keyed `context_baseline` blocks, then the
  history entries placed by `retain_from` and later events.
- `replacement` is stored exactly as produced, so replay never calls a strategy and never
  re-renders a wrapper.
- This changes `context_messages` and ContextPolicy's `project` (which today push
  `summary_message(summary)`) and `CompactionCheckpoint` (`replacement` instead of `summary`).
- The history snapshot state stores `retain_from` instead of the entry-id `compacted_through`,
  and the replacement is read from the anchoring fact. The snapshot interpreter version is
  bumped. Older snapshots are bypassed, which is harmless because snapshots are only a cache.
- History-side cut checks (`validate_cut` and the projection's cursor lookup) compare event ids
  instead of entry ids. The snapshot anchor is checked by the snapshot's owner, not by Timeline
  (below).

**Snapshot anchoring without business content (#130).** Today, Timeline core and both stores
read the content of `context/compaction` to anchor projection snapshots:

- `Timeline::context_with_checkpoint`
  ([timeline.rs](../../../crates/dim-agent-runtime/src/core/timeline.rs)) requires
  `EventKind::ContextCompaction`, compares `data.through`, and matches `Completed { cursor }`
  against `checkpoint.state.compacted_through`.
- `MemoryTimelineStore::save_snapshot`
  ([memory_timeline_store.rs](../../../crates/dim-agent-runtime/src/memory_timeline_store.rs))
  rejects an anchor that is not a completed compaction.
- The SQLite store ([storage/timeline.rs](../../../crates/dim-app-server/src/storage/timeline.rs))
  does the same on save, and its lookup SQL filters `e.kind='context/compaction'` and
  `status='completed'`.

This contradicts the principle that Timeline checks structure and type range, not content, and it
would make every change to the compaction record touch core code. F1 moves the check to its owner:

| Part | Checks after F1 |
| --- | --- |
| Timeline (`context_with_checkpoint`) and both stores | Structure only: the anchor event exists at the snapshot's sequence with the snapshot's event id, the interpreter tag matches, and the sequence is within the cut (plus the existing resume-position checks on the decoded checkpoint). No `EventKind` or outcome matching, and no business kind or status in SQL |
| Snapshot owner (`HistoryRenderer`, whose snapshots ContextPolicy saves) | Anchor validity, through the interpreter callback Timeline already calls. The callback now also receives the anchor event: `decode: fn(&RecoverySnapshot, &AgentEvent) -> Option<ProjectionCheckpoint>`. It accepts only a completed `context/compaction` whose `retain_from` matches its state and whose `through` precedes the snapshot. `None` is a cache miss, as today |
| Saving | The owner saves snapshots only at its own completed compaction, as it does today. The stores no longer enforce the kind |

Behaviour is unchanged for valid snapshots, and an invalid snapshot is a cache miss, which falls
back to full fact replay.

Readers update with the struct:

- ContextPolicy's retry filter compares `turn_id` with `Some(turn)`.
- Trace reads optional ids.
- Native storage indexes completions by `through` only.
- The frozen v1 samples are not changed. They decode only as v1 events; the native record does
  not accept the old `cursor`/`summary` form, and v1 connections no longer receive
  `context/compaction` (§12).

**Dynamic events (decision A).** `context/compact` and `context/compaction-start` are added as
`EventKind` variants now. When #2 lands (plugins register their own event types; Timeline checks
structure and type range, not content), they migrate to dynamic registration. `context/compact`
is owned by the Loop, and `context/compaction-start` by the owner of the compaction facts. F1 is
#2's first real consumer.

**Diagnostics (breaking).** `CompactionDiagnosticsRecord` becomes strategy-neutral. The
summary-oriented required fields move into a strategy-owned object:

```text
{ reason: "auto" | "manual", trigger,           // common
  retainFrom: string, retainLimit?: string,    // Timeline event ids
  estimatedInputTokens, estimatedOutputTokens,  // before and after, from count_tokens
  entriesReplaced, modelCallCount, durationMs,
  strategy: { kind: string, ... } }             // owned by the strategy
// summary strategy: { kind: "summary", summaryChars, estimatedSummaryTokens,
//                     chunkCount, compactedToolPayloadCount, droppedMessageCount }
```

ContextPolicy fills the common fields; the strategy returns its own object. The old flat record
is removed, not kept beside the new one. The frozen v1 samples are not changed and keep their
own flat record type in the v1 module (§12).

**Client visibility.** SessionBroadcast forwards every branch event unfiltered
([session_broadcast.rs](../../../crates/dim-agent-capabilities/src/session_broadcast.rs)), but
Rpc's wire filter `ProductEvent::new` in
[tree_protocol.rs](../../../crates/dim-app-server/src/tree_protocol.rs) drops
`context/compaction` today. F1 makes `context/compact`, `context/compaction-start` and
`context/compaction` visible to protocol v3 connections through `ProductEvent::new`, so a
client sees every state from live events or from history. `context/budget` and
`context/prepare` stay hidden.

### 4.4 Rust binding

```rust
// dim-agent-types/src/events.rs
EventKind::ContextCompact(ContextCompactData {})                  // "context/compact"
EventKind::ContextCompactionStart(ContextCompactionStartData {     // "context/compaction-start"
    turn_id: Option<String>, request_id: Option<String>, command: Option<u64>,
    through: u64, trigger: CompactionTrigger, strategy: String })
pub struct ContextCompactionData {
    pub turn_id: Option<String>, pub request_id: Option<String>,
    pub command: Option<u64>, pub start: Option<u64>, pub through: u64,
    pub outcome: ContextCompactionResult }
pub enum ContextCompactionResult {
    Completed { retain_from: String, replacement: Vec<Message>, strategy: String,
                diagnostics: Box<CompactionDiagnosticsRecord>,
                context_baseline: Vec<ContextBaselineEntry> },
    Skipped { .. }, Failed { .. }, Cancelled { reason: String } }

// dim-agent-capabilities/src/host.rs
pub trait ContextModel { fn context_window(&self) -> f64; fn output_tokens(&self) -> f64;
                         fn threshold_tokens(&self) -> f64;
                         fn count_tokens(&self, r: &ModelRequest) -> f64; }   // compact removed
pub struct HistoryEntry { /* existing */ pub source: String }   // id of the placing event
pub trait CompactionStrategy: Send + Sync + 'static {
    fn name(&self) -> &str;                            // written into the start, e.g. "summary"
    fn compact(&self, input: CompactionInput, cancel: CancellationToken)
        -> HostFuture<'_, StrategyOutcome>; }          // Completed | Skipped
pub struct CompactionStrategyHost(pub Option<Arc<dyn CompactionStrategy>>);
pub struct CompactionCheckpoint { pub retain_from: String, pub replacement: Arc<[Message]>,
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

1. **Cut.** `retain_from` is the latest valid event boundary at or before `input.retain_limit`,
   normally `retain_limit` itself. With `input.fallback`, if none exists, it is the latest valid
   boundary. If there is still none, or nothing lies before it: `Skipped {reason:
   "nothing_to_compact"}`, with no model call.
2. **Summary.** It summarizes the previous replacement (from `input.checkpoint`) plus the entries
   placed by events before `retain_from`.
3. **Result.** It returns `Completed {retain_from, replacement: [summary_message(summary)],
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
- **Diagnostics.** The summary object (`kind: "summary"`, `summaryChars`,
  `estimatedSummaryTokens`, `chunkCount`, `compactedToolPayloadCount`, `droppedMessageCount`);
  ContextPolicy fills the common part.

### 5.3 One compaction path (ContextPolicy)

`compact_step` is the existing compaction section of `ContextPolicy::prepare`, extracted:

1. **Retention limit.** `retain_limit` is the id of the `turn/start` of the oldest of the last K
   turns on the branch (provisional K = 2, #128), the running turn included. If fewer than K
   turns exist, there is no limit before which to compact, and only `fallback` allows a cut.
2. **Start.** It appends `context/compaction-start` with the caller's ids, `command`,
   `through`, `trigger` and `strategy.name()`.
3. **Strategy.** It calls `strategy.compact` with `retain_limit`, `fallback` (under recovery)
   and `trigger`. If it is cancelled before the strategy returns, it appends the `cancelled`
   result for the start and returns `Cancelled`.
4. **Validation.** For any strategy, it checks that:
   - `retain_from` is an event of the fixed context and passes the event-based validity rule
     (§4.3);
   - `retain_from` is at or before `retain_limit`, unless `fallback` applies;
   - the projected request is smaller than before and below the window.

   A failed check turns the outcome into `failed`.
5. **Result.** It appends `context/compaction` with the caller's ids, `command` and `start`, then
   saves the optional snapshot.

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
 s    context/compaction-start {turn_id: T,       writer: ContextPolicy (Loop's step)
        command: k, through: b, trigger: manual, strategy: "summary"}
 m    context/compaction {turn_id: T, command: k,  writer: ContextPolicy (Loop's step)
                          start: s, through: b, outcome}
      -- Loop consumes m: the step is over -> next_model as usual
      input/consumed* ; step/model-start {T, R2}   Loop (unchanged)
      context/prepare {T, R2, ...} -> model/call   on the compacted context
```

**With no turn running:**

```
 k    context/compact {}                          writer: Loop (AgentLoop::compact)
      -- Loop consumes k: no request in flight -> compaction step at once
 s    context/compaction-start {command: k, through: k,      writer: ContextPolicy
                                trigger: manual, strategy: "summary"}
 m    context/compaction {command: k, start: s, through: k, outcome}   writer: ContextPolicy
      -- Loop consumes m: the step is over -> try_start
      (no turn/start, no turn/end; a driving input applied during the step starts
       a turn now, prepared from the compacted context)
```

**Automatic, for comparison (inside a request's preparation):**

```
      step/model-start {T, R} ; context/prepare {T, R, through: p}       Loop
      context/budget {T, R}                                              ContextPreparation
 s    context/compaction-start {turn_id: T, request_id: R, through: p,   ContextPolicy
        trigger: threshold | recovery, strategy: "summary"}
 m    context/compaction {turn_id: T, request_id: R, start: s, through: p, outcome}
      model/call {T, R}                                                  ContextPreparation
```

**Scheduling rules:**

- **Boundary.** A step boundary is where the Loop would prepare the next request (`next_model`
  without a budget-retry source) or where the turn ends. A pending step starts at the first
  boundary.
  - If the turn ends instead, the step runs right after `turn/end`, with no turn.
  - A budget retry belongs to its request and is not a boundary.
- **Through.** `through` is the sequence of the event whose consumption started the step.
- **Active and pending commands.** The slot keeps the running step's command (active) apart from
  the latest command not yet started (pending).
  - A command consumed while a step is pending, or while a step runs, becomes the pending command
    and replaces an earlier pending one (coalescing). The next step serves the latest command; its
    start and result carry that command.
  - A step's result retires only its own command. If a command is pending, it runs as the next
    step at once, before the Loop advances (`next_model` inside a turn, `try_start` otherwise).
  - Example: `compact(c1) → start(c1) → compact(c2) → compact(c3) → result(c1) → start(c3) →
    result(c3)`. `c2` is coalesced into `c3`; its waiter is answered by `result(c3)` (§4.1).
- **No advance.** While a step runs, `next_model` and `try_start` wait for its fact. Inputs queue
  as usual and are not consumed by the step.

**Steer.** A driving input applied during a step does not cancel it. It joins the next request,
or starts a turn after the step. Interrupt, failure, reload and restart are in §5.8.

### 5.5 What the Loop gains

All in `crates/dim-agent-capabilities/src/agent_loop.rs`:

1. **Subscription.** `context/compact`, `context/compaction-start` and `context/compaction` are
   added. `CompactionStrategyHost`
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
6. **Step terminal.** On consuming the `context/compaction` of the running step's command, the
   Loop clears only the active command. A pending command then starts as the next step;
   otherwise the Loop resumes: `next_model` inside a turn, `try_start` otherwise. Results of
   other commands (settlements) do not resume the Loop.
7. **Interrupt.** A `stop/interrupt` cancels the running step, whose result is `cancelled` with
   its `start`, and writes `cancelled` without `start` for a pending command. When both exist,
   both are settled.
8. **Rebuild.** The slot is folded from command, start and result pairs:
   - A manual start without a result: the step was running. The Loop writes the `cancelled`
     result with that `start` and `command`.
   - A command with neither a start nor a covering result: it was pending. The Loop writes a
     `cancelled` result with that `command` and no `start`. When a running and a pending command
     both exist, both are settled. An unloading Loop starts no further step.

Unchanged: turn and request lifecycles, input consumption, tool batches, steer of model
requests, budget recovery, `LoopSnapshot` and `context/prepare`.

### 5.6 What ContextPreparation gains

- `ContextPolicy::prepare` calls `compact_step`. ContextPreparation injects
  `CompactionStrategyHost`, and its policy exists when both hosts are `Some`.
- `rebuild`: an automatic start (no `command`) without a result is settled first with its
  `cancelled` result, and then the open preparation with the existing reloading terminal.
- `usage(cancel)` reads the committed head and runs the existing estimate. It writes nothing.

### 5.7 Compaction state for clients

Every state is a Timeline fact, so a client sees it from live events or from history at any
time:

| State | Facts | Applies to |
| --- | --- | --- |
| Pending | A `context/compact` with no start or result referring to it | Manual |
| Running | A `context/compaction-start` without its result | Manual and automatic |
| Done | `context/compaction` (`completed`, `skipped`, `failed`, `cancelled`) | Manual and automatic |

- A client that connects mid-compaction reads the branch history (`session.history` or the
  attach replay) and sees an unmatched start, so it shows "running" without any extra channel.
- The start carries `trigger` (manual, threshold or recovery) and `strategy`, and the result
  carries the outcome. The client needs no other state.
- The wire filter change is in §4.3.

### 5.8 Failure, interrupt, reload and restart

| Case | Timeline facts | What the Loop does next | What the client sees |
| --- | --- | --- | --- |
| Strategy error or validation failure, manual inside a turn | start, then `context/compaction {start, failed}` | Continues the turn uncompacted (`next_model`). If the next request exceeds the window, the existing budget recovery forces a compaction (`fallback` cut); if that also fails, the request and the turn fail | Running, then `failed`; reply `compaction_failed` |
| Same, manual outside a turn | start, then `failed` | Nothing more (`try_start` if an input is waiting) | Running, then `failed`; reply `compaction_failed` |
| Same, automatic | start, then `failed` | Unchanged: the request continues uncompacted; under recovery the request fails | Running, then `failed` |
| Manual, no request can be built (no configuration or snapshot) | `failed` without `start` | As above | Pending, then `failed` |
| `stop/interrupt` during a running step (manual or automatic) | `cancelled {reason}` for the start, or `completed` if the strategy had already returned | Manual inside a turn: the turn ends `Cancelled` through `next_model`. Automatic: the request ends `Cancelled` as today | Running, then `cancelled` or `completed`; reply to match |
| `stop/interrupt` while a manual command is pending | `cancelled` without `start` | Turn behaviour unchanged | Pending, then `cancelled` |
| Plugin reload | For each unmatched start, the owner writes `cancelled: "capability reloading"` with that `start`: the Loop for a manual start, ContextPreparation for an automatic one (before its existing reloading terminal). For a pending command, the Loop writes `cancelled` without `start` | No replay. A rebuilt turn ends `Failed` as today | Running or pending, then `cancelled`; a waiting reply is `operation_interrupted` or `compaction_cancelled` |
| Process crash or restart mid-compaction | On the next load, the same settlement as a reload, from the same unmatched starts and commands | Same as reload | Running (from history) until the load, then `cancelled` |

**About crashes and restarts:**

- **Atomic facts.** Timeline batches commit atomically, so a start or a result is either fully
  present or absent, never half written. A crash between them leaves an unmatched start, which
  the next load settles.
- **Snapshots are only a cache.** A crash between the fact and the projection snapshot is
  harmless: projection replays from the fact (principles §4 rule 9).
- **Model work is lost.** The strategy's model work in flight is lost. The next automatic trigger,
  or a new `/compact`, compacts again.
- **No resume of a half-finished compaction.** Model calls are not idempotent and are not replayed
  after a reload anywhere in the runtime. The user's intent may also be stale: the history may
  have grown, or the user may no longer want it.

## 6. Errors

`MethodError` codes (`code -32000` unless noted):

| Code | Method | When | Retry |
| --- | --- | --- | --- |
| `-32602` | both | `branchId` missing or not a string | No |
| `session_invalid_input` | both | Unknown branch | No |
| `session_closed` | both | Session or branch closing | No |
| `context_not_configured` | usage | No `context/configured` in the branch's history | After a model is adopted |
| `context_model_failed` | usage | `resolve` failed (no capability snapshot) | After selecting the model again |
| `compaction_failed` | compact | Fact `failed`: strategy error after the shared failure policy, empty summary, a `retain_from` that fails validation, no reduction, no `context/configured`, or no snapshot | User's choice |
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
| Projection (`history.rs`), snapshots | One replay rule using `replacement` and `retain_from`; `HistoryEntry.source`; cut validation and the snapshot state become event-based (interpreter version bump); the decode callback checks the anchor event (#130) |
| Timeline core, memory and SQLite stores | Snapshot anchoring becomes structural only (#130, §4.3) |
| `HistoryRenderer` | Render-time projections that run on every request without a record are not compaction. An example is truncating old tool output by a fixed rule. They belong to the existing injectable `HistoryRenderer`, not to a strategy |
| ModelProvider | Unchanged code. `validate` now enforces the window estimate in the product profile |
| Provider, ProductResolver, Aimux | Unchanged; the summary strategy uses `ProviderConnectionResolver` and `ModelHost` |
| AuxiliaryModel (S4) | Not used. Shares `invoke_with_recovery`; whichever lands first extracts it |
| `sessionMethods` (C1) | Required for the two methods. The host values and the strategy do not need C1 |
| Rpc `ProductEvent` (`tree_protocol.rs`) | Makes `context/compact`, `context/compaction-start` and `context/compaction` visible to v3 connections |
| Turn consumers (clients, future Goal, Hooks) | Unaffected. A manual compaction outside a turn writes no `turn/*` |
| #2 dynamic events | `context/compact` migrates to dynamic registration; F1 is #2's first real consumer |
| `--models` profile, mock profile | Both hosts `None`: no compaction, and `session.context` is not recorded |
| ManagedProviders plan usage, CLI `/compact` and `/usage` | Not part of F1's acceptance |

## 9. Rule check

| Rule | Answer |
| --- | --- |
| Principles §1 (Capability, App Server boundary) | New parts and supplements live in `dim-agent-capabilities`; the App Server only installs them in its product profile. |
| §1 one event bus | The command and the fact are on the branch Timeline and consumed through its Dispatcher. The method's wait reads that Timeline. Clients see the compaction state from the same facts, so there is no side channel. |
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
| §4 rule 8 (infrastructure unchanged) | Dispatcher, envelope, rutis, loader, Agent assembly and facades unchanged. There is one infrastructure change (#130): snapshot anchoring in Timeline and both stores drops its knowledge of `context/compaction` and keeps only structural checks, and the owner's decode callback receives the anchor event. The requirement comes from Timeline itself: it must not know event content (principles §4 rule 4; Timeline checks structure and type range). It closes within Timeline's own contract: a snapshot is a cache with a structural anchor. Valid snapshots behave the same, and an invalid one is a cache miss. The new event kinds move to #2's dynamic registration later. |
| §4 rule 9 (only needed facts) | `context/compact` is read by the Loop's later decisions and on reload. `context/compaction` is used by projection. `context/compaction-start` and the visible result are facts by the maintainer's decision, for two reasons. First, they do not only drive a state machine: they are content that users and other state machines need in order to read the history (a UI connecting at any time must see that a compaction is running or how it ended). Second, reload settlement uses them: an unmatched start means a compaction was in flight. The Loop and ContextPreparation rebuild from start/result pairs, which is more precise than commands alone because it tells a running compaction from a pending one and covers automatic compaction. Usage writes nothing. |
| ADR 002 D1 (scope) | Window facts, strategy and methods: Session. Command, step and slot: branch, in the Loop. |
| ADR 002 D2 (no wiring modules) | ContextMethods exposes owners' operations through `sessionMethods`. |
| ADR 002 D3 (dependencies) | Hard dependencies on core only. A Provider or Aimux reload propagates to the strategy part (accepted cost). |
| ADR 002 D4 (configuration) | No configuration; provisional constants are code (#128). |
| ADR 002 D5 (language-neutral contracts) | §4.1 and §4.3 are the contracts; §4.4 is one binding. |

**Impact (public content):**

- The `session.context` namespace.
- The command `context/compact`.
- The changed `context/compaction` (optional ids, `command`, `start`, `cancelled`,
  `retain_from` instead of `cursor`, `replacement` and `strategy` instead of `summary`,
  strategy-neutral diagnostics).
- The fact `context/compaction-start` and the `start` reference on results.
- `context/compact`, `context/compaction-start` and `context/compaction` become visible to v3
  clients.

Internal Rust changes:

- The split ports and `CompactionStrategyHost`.
- `CompactionInput` fields.
- `CompactionCheckpoint {retain_from, replacement}` and `HistoryEntry.source`.
- `history.rs` cut validation and the cursor lookup become event-based. The snapshot state stores
  `retain_from` (interpreter version bump).
- Snapshot anchoring (#130): `Timeline::context_with_checkpoint` keeps only structural checks,
  and its `decode` callback becomes `fn(&RecoverySnapshot, &AgentEvent)`.
  `MemoryTimelineStore::save_snapshot` and the SQLite store's save and lookup drop the
  compaction kind and status checks.
- `PublicService` on `AgentLoop` and `ContextPreparation`.

**Requirements raised to mechanism owners:** none. The already planned #2 will host
`context/compact` and `context/compaction-start`.

## 10. Decisions and open questions

| Item | Decision |
| --- | --- |
| Which model compacts | The request's own model, in the summary strategy; a separate model is deferred |
| Trigger and retention | `0.8 × (window − output)`, K = 2 turns, `bytes / 4`, 1,600 tokens per image: provisional, #128 |
| Manual compaction | A Loop step scheduled by `context/compact`, inside or outside a turn, never creating a turn |
| Where the step runs | In the Loop, calling the one ContextPolicy compaction function (§2.1) |
| Record | `retain_from` (Timeline event id), `replacement` and `strategy` with one replay rule; optional ids, `command`, `cancelled`; changed directly (unreleased) |
| Ports | `ContextModel` (window facts) and `CompactionStrategy` (replacement), each from its own Session value |
| Other strategies, composition | Not done in F1 |
| New event kinds | `context/compact` and `context/compaction-start` as `EventKind` variants now, dynamic registration after #2 |
| `--models` profile | No capability snapshot, no compaction: accepted |
| Provider-reported overflow | Separate Aimux issue, outside F1 |
| Diagnostics | Strategy-neutral now: common counts plus a strategy-owned object (breaking) |
| Compaction state | Facts: pending = command without start, running = start without result, done = result (§5.7) |
| Half-finished compaction | Never resumed; settled as `cancelled` (§5.8) |

Open: none beyond #128.

## 11. Acceptance (flow tests)

The tests run a product-profile App Server against a mock HTTP provider with a small context
window (`crates/dim-app-server/tests`), plus Loop and policy flow tests in
`crates/dim-agent-capabilities/tests`.

1. **Automatic.** Past the threshold, the next preparation writes `context/budget`, then
   `context/compaction-start {trigger: threshold}`, and sends one summarizer request. It commits
   `context/compaction` `completed` with the request's ids, `start`, no `command`, `strategy: "summary"`, a one-message `replacement`, and `retain_from` = the
   `turn/start` of the second-to-last turn. The model request starts with that message and keeps the last 2 turns verbatim.
2. **Budget recovery.** The retry compacts with `trigger: provider_output_budget_exhausted`.
3. **Manual, no turn.** `session.context.compact` writes exactly `context/compact`,
   `context/compaction-start {command}` and `context/compaction {command, start}`, with no
   `turn/*`, `step/*` or `model/call`. The reply is
   `completed`.
4. **Manual, inside a turn.** `step/tool-result` → `context/compaction-start {turn_id, command}`
   → `context/compaction {turn_id, command, start}` →
   the next request's `step/model-start`, prepared from the compacted context.
5. **Replay and strategy replacement.**
   - A Session reloaded after compaction, and a branch forked after it, project the same messages
     as before.
   - A test strategy bound in place of the summary strategy, returning a two-message
     `replacement`, is committed and replayed by the same rule, with no Loop or policy change.
   - The policy rejects a test strategy whose `retain_from` lies inside the retained turns, or
     splits a tool call from its result.
   - A Session compacted with the default `HistoryRenderer` and replayed with a different
     renderer (another entry id scheme) still finds the cut from `retain_from` and projects the
     same retained events.
   - Snapshot anchoring (#130): both stores save and return a snapshot anchored at any existing
     event with the matching id and sequence. The owner's decode callback ignores a snapshot whose
     anchor is not a completed compaction, or whose `retain_from` does not match its state, and
     the context falls back to fact replay with the same result.
6. **Interrupt, reload and restart.**
   - `session.stop` during a step writes `cancelled` (or `completed` if the strategy had
     returned).
   - A plugin reload during a step writes `cancelled: capability reloading` without a second
     strategy call.
   - Killing the App Server process while the summarizer request is held open, then starting it
     again on the same home, leaves an unmatched start and no partial result. The next load
     writes `cancelled: capability reloading` with that `start`. The same holds for an automatic
     compaction. The Session projects the history as
     before the compaction, and a new `/compact` compacts normally.
   - A crash after the fact but before the snapshot save projects the compacted history from the
     fact.
   - A manual compaction failing inside a turn lets the turn continue. When the next request
     exceeds the window, budget recovery compacts.
7. **Two commands and a running step.**
   - Normal path: `compact(c1) → start(c1) → compact(c2) → compact(c3) → result(c1)`, then the
     step for `c3` runs before the Loop advances, with its own start and result. There is no start
     for `c2`; the method replies to `c1` with `result(c1)` and to `c2` and `c3` with `result(c3)`.
   - `stop/interrupt` with `c1` running and `c2` pending writes `cancelled` with the start for
     `c1` and `cancelled` without a start for `c2`; no further step starts.
   - A plugin reload with `c1` running and `c2` pending settles both as
     `cancelled: capability reloading`, with no further strategy call.
8. **Compaction state for clients.**
   - A v3 client attached to the Session receives `context/compact`, `context/compaction-start`
     (`trigger: manual`) and `context/compaction` live.
   - A client that connects while the summarizer request is held open sees "running" from the
     branch history (an unmatched start).
   - An automatic compaction shows `trigger: threshold` with the request ids.
   - Across every test above (success, skip, failure, interrupt, reload, crash), each
     `context/compaction-start` has exactly one `context/compaction` with its `start`, and no
     result carries a `start` that does not exist.

## 12. Implementation notes

The implementation follows this design. Where the code needed a detail the text did not
fix, the smallest choice was made and is recorded here.

| Item | Implementation |
| --- | --- |
| Start trigger values | `trigger` uses the existing `CompactionTrigger`: `manual`, `threshold`, `provider_output_budget_exhausted` (§4.3 said `recovery`; §4.2 and acceptance 2 already used the latter). |
| Legal cuts | `CompactionInput` also carries `cuts`: every legal cut in order (`retain_from`, the first kept entry, and whether it is within `retain_limit`), computed by ContextPolicy from the event-based validity rule. Candidates are turn starts, events that place entries, and the boundary's last event (which replaces everything). `CompactionInput::cut()` returns the latest cut within the limit or, under `fallback`, the latest cut. A strategy may return any `retain_from`; the policy still validates it. |
| Adoption | `compact_step` validates the strategy's result before it appends the fact, then adopts the committed fact as Timeline middleware left it: the projection, the reduction check and the snapshot use the committed `retain_from`, `replacement` and baseline. A committed cut that cannot be projected fails the preparation, as a committed cursor did before. |
| Diagnostics | `retainFrom` is absent when the strategy found no cut (`skipped`), and `estimatedOutputTokens` is absent when nothing was replaced. |
| v1 | There is no v1-to-native conversion in production code: the frozen samples decode as v1 events only, and `v1::EventRef` has no form for `context/compact`, `context/compaction-start` or the changed `context/compaction`, so v1 connections no longer receive compaction facts. The old flat diagnostics type moved into the v1 module. |
| Loop subscription | The Loop subscribes to `context/compact` and `context/compaction`; starts are folded only at rebuild, where they tell a running step from a pending command. |
| Two commands | Review fix: the slot keeps the active and the pending command apart, a result retires only its own command, a pending command runs as the next step before the Loop advances, and interrupt or reload settle both. The method's waiter uses the covering rule of §4.1 (acceptance 7). |
| Snapshot anchor | Timeline keeps the structural checks of §4.3. The old `resume ≤ through` check needed the anchor's content; it is replaced by the structural "resume precedes the anchor on the anchor's branch", and the owner's decode checks `through` before the anchor. |
| Native storage example | The record example in [native-storage-format](../appserver/native-storage-format.md) uses the changed record and the v3 interpreter. |
| Acceptance | Acceptance 1, 3, 6 (crash and restart) and 8 run in the binary flow test `transport_flow/compaction.rs`. Acceptance 2, 4, 5 and the reload, interrupt and failure cases of 6 run in `dim-agent-capabilities/tests/context_flow` (`manual_compaction.rs` and the existing compaction tests). The crash between fact and snapshot is covered by the snapshot-failure test in `compaction_events.rs`. |
