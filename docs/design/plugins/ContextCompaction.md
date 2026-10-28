# ContextCompaction

Status: Implemented on the F1 branch (approved design; implementation notes in §12). Roadmap item F1 ([#72](https://github.com/arcships/dim/issues/72)).
Overview: [Appendix B.1 ContextPreparation row](../overview.md#b1-session-and-branch-capabilities).
Scope: [ADR 002](../../adr/002-plugin-scope.md): Session parts plus supplements to the existing
branch capabilities Loop and ContextPreparation. §12 records where the implementation refines this text.
The trigger, retention and recovery rules follow the
[#128](https://github.com/arcships/dim/issues/128) research
([compaction-trigger](../research/compaction-trigger.md)); its reserve and retention share are
provisional heuristics. §13 is the approved design for the summary evaluation, the summarizer's
reasoning and a target length ([#170](https://github.com/arcships/dim/issues/170)); the defaults
are decided by running the evaluation (§13.4). §14 maps the compaction-quality requirements of
[#428 §7.1](../compaction-quality.md#71-contextcompaction) to the sections that answer them.

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
([context_policy.rs](../../../crates/dim-agent-capabilities/src/context_policy.rs)). It runs in
every profile that installs it through `defaults::install_compaction`
([cli.rs](../../../crates/dim-app-server/src/cli.rs)): the product profile and the `--models`
profile. §4.4 lists what each profile binds.

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
 |    registers session.context with sessionMethods: compact, usage  |
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
| Strategy | Its own messages of the replacement (the summary), the replacement's planned size, and the cut within the retention rule. The pinned inputs and the digest are the mechanism's parts (§5.9, §5.10, §4.3) | `CompactionStrategyHost` Session value | It is replaced |

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
| Compaction slot: at most one outstanding command `{sequence, consumed, running: Option<CancellationToken>}` | Loop | Private. Set by `AgentLoop::compact` or by consuming a command; cleared by that command's result. Settled on load (§5.5) |

The Session values hold no state. A resolved connection lives only inside one strategy call.

## 4. Interfaces

### 4.1 Session methods (language-neutral, ADR 002 D5)

Namespace `session.context`, which ContextMethods registers with the Session's `sessionMethods`
([session-boundary A.1](../contracts/session-boundary.md#a1-method-contributions)). Both
methods are declared with `interruption: report`, `progress: false`, `notifications: []` and
`reply: direct`.

| Method | Mutating | Request | Reply |
| --- | --- | --- | --- |
| `session.context.compact` | yes | `{sessionId, branchId}` | `{status: "completed", sequence}` or `{status: "skipped", reason}` |
| `session.context.usage` | no | `{sessionId, branchId}` | `{through, contextWindow, outputTokens, thresholdTokens, estimatedInputTokens, baselineSource: "provider_baseline" \| "full_estimate"}` |

`session.context.compact`:

- It calls `AgentLoop::compact`, which returns a command sequence `c`: a newly written
  `context/compact`, or the command already outstanding on the branch (§5.4). Concurrent or
  repeated calls therefore join one in-flight compaction.
- It then waits on the branch Timeline for the result that carries its command:
  the first `context/compaction` with `command = c`.
- It maps the outcome: `completed` → `{status: "completed", sequence}`;
  `skipped` → `{status: "skipped", reason}`; `failed` → `compaction_failed`;
  `cancelled` → `compaction_cancelled`.

The reply keeps that shape. A completed compaction's cost and its insufficient-space mark are
in the diagnostics of the `context/compaction` fact the reply names by `sequence` (§4.3), and
that fact reaches every v3 client (§4.3 client visibility), so a client shows the cost without
another channel.

`session.context.usage` returns the values a preparation at the committed head `through` would
write into `context/budget`, plus `thresholdTokens`.

Cancellation follows session-boundary A.1: an unload interrupts the call, and a caller's
disconnect cancels nothing.

### 4.2 Ports (Rust, runtime-internal)

The existing `ContextModel` port is split in two. Each comes from its own Session value.

| Port | Session value | Methods | F1 implementation |
| --- | --- | --- | --- |
| `ContextModel` (narrowed) | `ContextModelHost(Option<Arc<dyn ResolveContextModel>>)` (existing) | `context_window`, `output_tokens`, `threshold_tokens`, `count_tokens`; `compact` moves out | `ProductContextModel` (§5.1) |
| `CompactionStrategy` (new) | `CompactionStrategyHost(Option<Arc<dyn CompactionStrategy>>)` | `name`, `replacement_size(model)`, `compact(input, cancel) -> Completed {retain_from, replacement, strategy, diagnostics} \| Skipped {reason, diagnostics} \| Failed {error, diagnostics} \| Err(HostError)` | `ProductSummaryStrategy` (§5.2) |

Both values are bound explicitly in every profile, as `ContextModelHost` is today. The policy
exists when both are `Some`. One `Some` without the other is an assembly defect that
ContextPreparation reports when it loads.

`replacement_size(model)` is the planned token count of the messages `compact` returns: the
summary's target `S` (§13.3) plus one empty `summary_message` wrapper, counted with
`count_tokens`. `S` comes from the strategy's factory configuration — the same rule its prompt
states — so the plan cannot drift from the summarizer. The retention rule reserves that space
before the strategy runs (§5.3). It is a planning size, not a cap: `maxTokens` stays
`output_tokens`, and a reply that overshoots `S` completes and reports its actual size in the
diagnostics. `Completed`'s `replacement` holds the strategy's own messages; ContextPolicy
composes the record's `replacement` from them, the digest and the pinned inputs (§4.3, §5.9,
§5.10).

`compact` reports a failure inside the strategy as `Failed {error, diagnostics}`, not as an
`Err`: the outcome carries the same diagnostics as a completion, so a `failed` fact reports its
attempt count, usage and duration like any other (§4.3). `Err` remains for cancellation and for
failures before any model work.

`CompactionInput` is produced by ContextPolicy:

| Field | Meaning |
| --- | --- |
| `history` | The projected entries from the previous `retain_from` on (existing) |
| `checkpoint` | The previous `{retain_from, replacement, replacement_parts, context_baseline}` (changed from `cursor` and `summary`) |
| `request`, `model` | The request whose context is compacted, and its `ContextModel` window facts (new) |
| `retain_limit` | `retain_from` of the earliest legal cut whose kept tail fits the tail budget, which accounts for the parts the compaction cannot shrink (§5.3; for a manual step, at least the latest turn's start); the strategy's `retain_from` must be at or before it (new; §5.3) |
| `trigger` | `Threshold`, `ProviderOutputBudgetExhausted` or `Manual` (new) |
| estimates, `provider_budget` | Existing |

### 4.3 Events

| Event | Writer | Consumer | Data |
| --- | --- | --- | --- |
| `context/compact` (new) | Loop, through its public write operation `AgentLoop::compact` | Loop, in sequence order | `{}` |
| `context/compaction-start` (new) | ContextPolicy in `compact_step`, for its caller (Loop step or ContextPreparation) | Loop and ContextPreparation (rebuild), clients | `{turn_id?, request_id?, command?, through, trigger: "manual" \| "threshold" \| "provider_output_budget_exhausted", strategy}` |
| `context/compaction` (changed) | ContextPolicy, for its caller; on reload, the settling owner | Loop (manual step terminal), ContextPolicy (retry reuse), projection, ContextMethods, clients | `{turn_id?, request_id?, command?, start?, through, outcome}` |

`outcome` is one of:

- `completed {retain_from, replacement: Message[], strategy: string, diagnostics,
  context_baseline, replacement_parts?}`. `retain_from` (a Timeline event id) replaces the
  renderer-level `cursor`, `replacement` replaces `summary: String`, and `strategy` names the
  producer, for example `"summary"`. `replacement_parts` (`{strategy, digest}` message counts)
  locates the parts of `replacement`; absent means one part, the strategy's messages (§4.3).
- `skipped {reason, diagnostics}`.
- `failed {error, diagnostics?}`; the diagnostics are present whenever the strategy ran (§4.2).
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

**Composition of the replacement.** ContextPolicy composes the `replacement` it stores from
three parts, in this order:

1. The strategy's messages (the summary, class 3b).
2. The digest (class 3a), when it has content (§5.10). It is empty until a tool declares
   effects through ToolRegistry's generic means
   ([requirements §7.4](../compaction-quality.md#74-toolregistry-the-tool-contract)).
3. The pinned inputs of the running turn (class 2(a), §5.9), in event order.

The summary comes first because it accounts for the whole replaced range; the digest follows
it as the deterministic account of the same range; the pinned inputs come last, so the running
turn's own words directly precede the kept tail. A strategy decides neither part 2 nor part 3:
they are guarantees of the mechanism, so every strategy gets them unchanged. Replay still
reads `replacement` as one list, so the parts need no rule of their own on reload, fork or
cache.

The completed outcome records `replacement_parts: {strategy, digest}` — the counts of its
leading messages; the rest are the pinned inputs. A record without it reads as one part, the
strategy's messages. That is how a later compaction finds the previous summary, digest and pins
without parsing any content (§5.2, §5.10); replay is unaffected, and a reader that ignores the
field projects the same messages.

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

This contradicts the principle that Timeline checks only the envelope and reserved core domains
([ADR 001](../../adr/001-harness-foundation.md#events-and-ownership-issue-2)), not content, and it
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
- Native storage indexes completions by `through` only.
- The frozen v1 samples are not changed. They decode only as v1 events; the native record does
  not accept the old `cursor`/`summary` form, and v1 connections no longer receive
  `context/compaction` (§12).

**Dynamic events (decision A).** `context/compact` and `context/compaction-start` are
`EventKind` variants. #2 does not move them: it carries event types the core does not define as
opaque data with the same envelope, so core events keep their variants and wire form. `context/compact`
is owned by the Loop, and `context/compaction-start` by the owner of the compaction facts.

**Diagnostics (breaking).** `CompactionDiagnosticsRecord` becomes strategy-neutral. The
summary-oriented required fields move into a strategy-owned object:

```text
{ reason: "auto" | "manual", trigger,           // common
  retainFrom: string, retainLimit?: string,    // Timeline event ids
  estimatedInputTokens, estimatedOutputTokens,  // before and after, from count_tokens
  entriesReplaced, modelCallCount, durationMs,
  modelUsage?: Usage,                           // summed over the strategy's replies
  insufficientSpace?: { parts: [{part, tokens}] },   // completed compaction only
  strategy: { kind: string, ... } }             // owned by the strategy
// summary strategy: { kind: "summary", summaryChars, estimatedSummaryTokens,
//                     chunkCount, compactedToolPayloadCount, droppedMessageCount }
```

ContextPolicy fills the common fields; the strategy returns its own object. The old flat record
is removed, not kept beside the new one. The frozen v1 samples are not changed and keep their
own flat record type in the v1 module (§12).

`modelCallCount` counts the model requests the strategy sent, each when it is sent: its chunks,
its own retries and the shared failure policy's attempts (§5.2). `durationMs` is measured around the strategy call,
so it is the summarizer's duration; the policy's own work is local. `modelUsage` is the `Usage`
the strategy's replies reported, summed over them — `promptTokens`, `completionTokens`,
`totalTokens` and the cache read and write tokens; `cost` and `currency` stay absent, and a
strategy that sent no request reports none. `insufficientSpace` is present only on a completed
compaction whose parts that compaction cannot shrink exceed `0.5 × B` on their own; `parts`
names the parts that are too large, largest first, each with its `count_tokens` size, and the
part names are `systemPrompt`, `tools`, `appendedMessages`, `keyedBaseline`, `pinnedInputs`,
`digest` and `summary` (§5.3). Both fields are descriptive: no later decision reads them — the
trigger compares the committed history with the latest start's `through`, and recovery compares
its estimate with `B` — so they are content a reader shows (§9, rule 9).

Every compaction that reached the strategy carries this common record. One that failed
validation carries the strategy's record with no after-size. One whose strategy failed carries
it too, with no after-size: a failure is an outcome that carries its diagnostics (§4.2), so
`modelCallCount`, `modelUsage` and `durationMs` are reported as on a completion. Only a manual
compaction that never built a request has no diagnostics.

**Client visibility.** SessionBroadcast forwards every branch event unfiltered
([session_broadcast.rs](../../../crates/dim-agent-capabilities/src/session_broadcast.rs)), but
Rpc's wire filter `ProductEvent::new` in
[tree_protocol.rs](../../../crates/dim-app-server/src/tree_protocol.rs) drops
`context/compaction` today. F1 makes `context/compact`, `context/compaction-start` and
`context/compaction` visible to protocol v3 connections through `ProductEvent::new`, so a
client sees every state from live events or from history. `context/budget` and
`context/prepare` stay hidden.
A client that shows the summary shows the replacement's strategy part, which
`replacement_parts` locates; without the field, the whole replacement is the strategy part.

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
                context_baseline: Vec<ContextBaselineEntry>,
                replacement_parts: Option<ReplacementParts> },
    Skipped { .. }, Failed { .. }, Cancelled { reason: String } }

// dim-agent-capabilities/src/host.rs
pub trait ContextModel { fn context_window(&self) -> f64; fn output_tokens(&self) -> f64;
                         fn threshold_tokens(&self) -> f64;
                         fn count_tokens(&self, r: &ModelRequest) -> f64; }   // compact removed
pub struct HistoryEntry { /* existing */ pub source: String }   // id of the placing event
pub trait CompactionStrategy: Send + Sync + 'static {
    fn name(&self) -> &str;                            // written into the start, e.g. "summary"
    fn replacement_size(&self, model: &dyn ContextModel) -> f64;   // planned replacement tokens
    fn compact(&self, input: CompactionInput, cancel: CancellationToken)
        -> HostFuture<'_, StrategyOutcome>; }          // Completed | Skipped | Failed
pub struct StrategyDiagnostics { pub model_call_count: u64, pub usage: Option<Usage>,
                                 pub record: CompactionStrategyRecord }
pub struct CompactionStrategyHost(pub Option<Arc<dyn CompactionStrategy>>);
pub struct CompactionCheckpoint { pub retain_from: String, pub replacement: Arc<[Message]>,
                                  pub replacement_parts: Option<ReplacementParts>,
                                  pub context_baseline: Arc<[ContextBaselineEntry]> }
pub struct ReplacementParts { pub strategy: usize, pub digest: usize }  // leading message counts

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
pub struct ProductSummaryStrategyPlugin { host: DepKey<CompactionStrategyHost> }   // new(names)
//   Deps = (Keyed<ModelHost>, Keyed<ProviderConnectionResolver>); provides CompactionStrategyHost
pub struct ContextMethodsPlugin;
//   Deps = (Keyed<TreeCommands>, Keyed<SessionMethods>); methods.register(ctx, "session.context", ..)

// dim-app-server/src/cli.rs, defaults::install_compaction, the one install of every profile
// that compacts; the product profile passes ProductContextModel and its product home,
// the --models profile passes AimuxModels and its --home:
builder.session_value(Arc::new(ContextModelHost(Some(model))))
       .require_feature(Feature::new("ContextCompaction")
           .session("ProductSummaryStrategy", |names| names.part(ProductSummaryStrategyPlugin::new(names)?))
           .session("ContextMethods", |names| names.part(ContextMethodsPlugin)));
// The mock profile binds ContextModelHost(None) and CompactionStrategyHost(None).
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
| `threshold_tokens` | `B − reserve`, where `B = context_window − output_tokens` (the input budget) and `reserve = min(max(0.10 × B, 16,000), 0.20 × B)`. The reserve absorbs the estimation error of the next request; it is a provisional heuristic that does not cover every request without a provider baseline ([research §3.3, §6.4](../research/compaction-trigger.md#33-recommendation)) |
| `count_tokens` | UTF-8 bytes of the serialized system prompt, messages and tool declarations divided by 4; each image part counts a fixed 1,600 tokens. Provisional (#128). Used for deltas over the provider's `prompt_tokens`, the first request, `validate` and the reduction check |

The `--models` profile resolves the same facts from its model file: `AimuxModels` reads the entry
the request names, by provider and model id, and the request's capability snapshot stays unset, so
images are sent while the capability is unknown.

**No window cap of compaction's own.** The window facts are the model's catalog values with
their overrides, and compaction applies no window limit of its own: `B`, the threshold,
`0.5 × B`, `0.25 × B` and `S` all scale with the declared window. A model whose useful window
is smaller than its declared one is declared smaller — in its catalog entry, which Provider
owns, or through the user's existing `contextWindow` override. Compaction never decides that
through a constant of its own.

### 5.2 ProductSummaryStrategy

`compact(input, cancel)`:

1. **Cut.** `retain_from` is the latest valid event boundary at or before `input.retain_limit`,
   normally `retain_limit` itself. The limit carries the `0.5 × B` rule: when no kept tail fits
   it, the limit is the boundary that replaces everything (§5.3). If there is no limit (the parts
   the compaction cannot shrink exceed the bound on their own), it is the latest valid boundary
   that keeps something, and under budget recovery it may also be the boundary that replaces
   everything (#169). If there is still none, or nothing lies before it: `Skipped {reason:
   "nothing_to_compact"}`, with no model call.
2. **Summary.** It summarizes the previous replacement's strategy and pinned parts (from
   `input.checkpoint`, located by `replacement_parts`; §4.3) plus the entries placed by events
   before `retain_from`, and nothing earlier: the fixed context begins at the previous cut, so
   one compaction's reads and cost do not depend on how long the session has run. The previous
   digest part is not sent: the digest rolls forward as data (§5.10).
3. **Result.** It returns `Completed {retain_from, replacement: [summary_message(summary)],
   strategy: "summary", diagnostics}`. The existing `summary_message` wrapper, which marks the
   text as data and escapes it, is applied once here and stored in the record. That
   `replacement` is the strategy's part of the record's replacement; ContextPolicy composes
   the rest (§4.3).

How the summary is produced:

- **Request.** One summarizer request per chunk. It uses `input.request`'s provider, model,
  snapshot and config, with `maxTokens = model.output_tokens()`, a fixed summarizer system
  prompt owned by this module, one user message with the transcript, and no tools. #170 proposes
  a reasoning setting for this config and a target length in the prompt (§13).
- **Credentials and failures.** The connection comes from `ProviderConnectionResolver` and is
  used only inside this call. The call runs through `invoke_with_recovery`, ModelProvider's
  failure and credential policy. S4 extracts it; if F1 lands first, F1 does the same extraction.
- **Output.** Thinking output is dropped. A tool stop or an empty text is a failure.
- **Transcript.** Role-labelled text: tool calls as name and input, tool results as text, images
  as `[image]`.
- **Chunking.** One call's input budget is `context_window − output_tokens`, checked with
  `count_tokens` on the whole summarizer request (system prompt, carried summary, envelope and
  chunk), the same accounting as every other request; there is no separate character rule. A
  longer transcript is folded in chunks: each call gets the running summary and as many entries
  as fit. An entry that does not fit alone is truncated in the middle by bytes on UTF-8 character
  boundaries, with the marker inside the budget, and counted in `compactedToolPayloadCount`.
- **Cut-off replies.** A reply that stops at `max_tokens` (thinking can use most of the
  allowance) is not a summary. Its chunk is halved at the entry separator nearest the middle (or
  at a character boundary) and folded half by half; a chunk still cut off below 2 KiB fails the
  compaction, and so does a compaction that reaches its retry limit (bounded work, below), so no
  record keeps a cut-off text (#169). Each half carries the summary of the one before it, so its
  whole request is checked against the budget again before the call; a half that no longer fits
  is halved too, keeping all of its text, and when the carried summary and envelope do not fit
  without any transcript the compaction fails without sending the request.
- **Replacement size.** The strategy plans its replacement at `replacement_size(model)`: the
  target `S` (§13.3, `S = min(α × O, β × B)`, from the strategy's factory configuration) plus
  the tokens of an empty `summary_message`. Until the §13.4 evaluation chooses, `α = 0.5` and
  `β = 0.10` (provisional, like `RETAINED_SHARE`); the prompt's target sentence stays §13's
  decision and is not added here. It is a planning size: `maxTokens` stays `O`, a reply that
  overshoots `S` is kept, and the fact reports the size it actually had (§4.3).
- **Bounded work.** One compaction sends at most `chunkCount + SUMMARY_RETRY_LIMIT` summarizer
  requests, where `chunkCount` is the diagnostics' count of the pieces summarized, a cut-off
  reply's halves included, and `SUMMARY_RETRY_LIMIT` is `8`, a provisional constant of this
  module. The extra `8` covers the shared failure policy's retries: every request counts toward
  `modelCallCount`, counted when it is sent, whether or not the attempt produced a stream — the
  strategy's host is wrapped in a driver that counts before delegating, so a retryable failure
  before any stream spends the allowance like any other request. A piece re-split because the
  carried summary no longer fits sends nothing, so halving a cut-off reply adds to `chunkCount`
  and cannot exhaust the allowance; `MIN_CHUNK_BYTES` bounds that halving. Once
  `chunkCount + SUMMARY_RETRY_LIMIT` requests have gone out, the wrapper refuses the next one
  before it reaches the provider, with a non-transient error so the policy does not retry it; the
  result is `failed` and nothing is applied. `modelCallCount` is the number of requests sent
  (§4.3).
- **Substantive summary.** Non-empty is enforced on every reply: `summarize` fails the
  compaction when a reply holds no text after trimming (`failed`, reply `compaction_failed`, not
  a skip: a skip means there is nothing to compact). The folded summary is always the last reply,
  so the summary a compaction commits is never empty. The required parts are
  [#429's policy](../compaction-quality.md#77-model-facing-prompts-429), decided through the
  evaluation of §13.4: this design does not change the prompt, and the parts are checked when the
  prompt that declares them lands, added together with it.
- **Cost.** Every reply's `Usage` chunk is summed into `StrategyDiagnostics.usage`, and
  `model_call_count` is the attempt count; ContextPolicy writes both into the completed fact's
  common diagnostics with the sizes before and after and the duration (§4.3).
- **Diagnostics.** The summary object (`kind: "summary"`, `summaryChars`,
  `estimatedSummaryTokens`, `chunkCount`, `compactedToolPayloadCount`, `droppedMessageCount`,
  `reasoning`, `targetTokens` (absent without an instruction), `cutOffCount`); ContextPolicy fills
  the common part, and the strategy reports the attempt count and the summed usage with it
  (§4.3).

### 5.3 One compaction path (ContextPolicy)

`compact_step` is the existing compaction section of `ContextPolicy::prepare`, extracted:

1. **Resident parts and retention limit.** With `B = context_window − output_tokens` (§5.1), the
   parts a compaction at a cut cannot shrink are:
   - the request frame: the system prompt, the tool declarations, and any messages the
     preparation appended beyond the history;
   - the keyed baseline at the cut;
   - the pinned inputs of the running turn before the cut (§5.9), at that cut's own size;
   - the digest at its bound; it has no content today, so its bound is zero (§5.10);
   - the strategy's replacement at `replacement_size(model)` (§4.2).

   `resident(cut)` is the sum of those parts with `count_tokens`, and the tail budget at that
   cut is `min(RETAINED_SHARE × B, 0.5 × B − resident(cut))`, with `RETAINED_SHARE = 0.25`
   (provisional, #128). The tail is the entries from the cut on. The keyed baseline is resident,
   because it is replayed after every cut; it is no longer counted inside the tail budget as
   #128 counted it, so a keyed block shortens the tail only when `0.5 × B − resident` is what
   binds.

   `retain_limit` is the `retain_from` of the earliest legal cut whose kept tail fits the
   budget: the cut that keeps the most tail the bound allows. Every later cut keeps less, and
   the strategy takes the latest cut at or before the limit, which is the limit itself. This
   keeps whole recent turns when they fit and cuts inside the running turn when it is too
   large. A manual step cuts at the start of the latest turn, or at the token-rule cut when
   that keeps less.

   The candidates end with the cut at the context's last event. When that event placed no
   entries, the cut replaces everything and its kept tail is zero; it then fits whenever the
   resident parts at that cut are within `0.5 × B`, so it becomes the limit when no
   content-keeping cut fits: the compaction keeps no verbatim tail, the request is the resident
   parts alone, and the content it replaces stays Recall-reachable. This changes #169, which let
   only budget recovery replace everything.

   No limit is set when nothing fits, not even that cut. Then the latest cut that keeps
   something applies (budget recovery may replace everything), and two cases complete above the
   bound. When the resident parts alone exceed `0.5 × B`, the completed result carries the
   insufficient-space mark (step 5). When the context's last event placed entries that do not
   fit, no cut exists after that event, so those entries stay and the result carries no mark:
   every part the compaction cannot shrink fits.
2. **Start.** It appends `context/compaction-start` with the caller's ids, `command`,
   `through`, `trigger` and `strategy.name()`.
3. **Strategy and composition.** It calls `strategy.compact` with `retain_limit`, the legal
   `cuts` and `trigger`. If it is cancelled before the strategy returns, it appends the
   `cancelled` result for the start and returns `Cancelled`; a `Failed` outcome is written as a
   `failed` result with its diagnostics. It then composes the record's replacement — the
   strategy's messages, the digest when it has content (§5.10), and the pinned inputs (§5.9),
   in that order — and records `replacement_parts` (§4.3).
4. **Validation.** For any strategy, it checks that:
   - `retain_from` is an event of the fixed context and passes the event-based validity rule
     (§4.3);
   - `retain_from` is at or before `retain_limit`; or, when no cut is within a limit, it keeps
     something, or replaces everything under budget recovery;
   - the projected request is smaller than before and below the window.

   The bound is reached by the cut: the second check holds every strategy to the limit, and a
   completed compaction stays completed whatever size its summary actually has. A failed check
   turns the outcome into `failed`.
5. **Result and mark.** It appends `context/compaction` with the caller's ids, `command` and
   `start`, then saves the optional snapshot. A completed result whose resident parts — the parts
   of step 1, measured on the result, with the summary at the size it actually has — exceed
   `0.5 × B` carries the insufficient-space mark: each part's `count_tokens` size, and, from the
   parts named `systemPrompt`, `tools`, `appendedMessages`, `keyedBaseline`, `pinnedInputs`,
   `digest` and `summary`, the largest first until their sizes cover the excess over `0.5 × B`.
   The mark is a field of the common diagnostics (§4.3) and no decision reads it.

Callers:

- **ContextPreparation, inside `prepare`.** At the threshold (§5.6) or under `recovery`. The
  rest of `prepare` is unchanged.
- **The Loop's manual step.** Trigger `Manual`. The request is built as `start_preparation`
  builds one: the latest `context/configured` settings and system with the projected messages,
  without binding tools.

**Budget recovery.** `validate`, called by ModelProvider before every model call, uses the
trigger's estimate of the request (the provider baseline plus the delta when one exists, else the
full local count) and fails with budget facts when it exceeds `B`; the Loop then retries the request
once with `recovery`. Recovery thus means "the request does not fit the declared budget",
consistent with the trigger. Recovery is exempt from the regrowth rule and may compact once per
request (§5.6). After a recovery compaction the preparation checks the projected request itself:
with a limit it is at most `0.5 × B`, and without one the recovery took the replace-everything
cut when the cut existed. If its count still exceeds `B`, the preparation fails with budget facts
and an error that names what keeps it there: the resident parts that are too large, by the same
naming rule as the mark, or, when they fit, the entries the last event placed, which no cut
removes. No `model/call` follows. The Loop does not compact again and the turn fails with that
error.
Without a provider baseline the full local estimate and its error remain (#128).

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
- **One outstanding command.** By the maintainer's decision, repeated requests are an interaction
  concern: deduplication belongs to the interaction boundary, not to the Loop's state machine or
  the Timeline.
  - `AgentLoop::compact` is the only writer of `context/compact`. Under the Loop's lock it checks
    whether a command is outstanding on the branch (written and without a result). If one is, it
    writes nothing and returns that command's sequence, and the caller waits for that command's
    result. Otherwise it writes the command.
  - The Loop's slot holds at most one outstanding command. Its result clears the slot.
  - **Defensive rule.** A `context/compact` the Loop consumes while another command is outstanding
    (one not written through `AgentLoop::compact`, or old data) is settled at once with
    `context/compaction {command: <its sequence>, outcome: skipped {reason: "a compaction is
    already in progress"}}`, without a start. It is not queued.
- **No advance.** While a step runs, `next_model` and `try_start` wait for its fact. Inputs queue
  as usual and are not consumed by the step.

**Steer.** A driving input applied during a step does not cancel it. It joins the next request,
or starts a turn after the step. Interrupt, failure, reload and restart are in §5.8.

### 5.5 What the Loop gains

All in `crates/dim-agent-capabilities/src/agent_loop.rs`:

1. **Subscription.** `context/compact`, `context/compaction-start` and `context/compaction` are
   added. `CompactionStrategyHost`
   is added to `Deps`.
2. **Public write.** `AgentLoop::compact()` returns the outstanding command's sequence, or appends
   `context/compact` and returns its sequence (§5.4). `AgentLoop` gets the `PublicService` marker.
3. **Compaction slot**, at most one outstanding command. On consuming that command, the Loop marks
   the step waiting, and starts it at once if no request is in flight. Another command consumed
   while one is outstanding is skipped (§5.4).
4. **Boundaries.**
   - `next_model` (except with a budget-retry source) and `turn/end` start a pending step instead
     of advancing.
   - `next_model` and `try_start` do nothing while a step runs.
5. **The step task.** It is tracked like the preparation task, builds the request as
   `start_preparation` does, and calls `ContextPolicy::compact_step`.
6. **Step terminal.** On consuming the result of the outstanding command, the Loop clears the
   slot. If its step ran, the Loop resumes: `next_model` inside a turn, `try_start` otherwise.
   Results of other commands (skips, settlements) change nothing.
7. **Interrupt.** A `stop/interrupt` cancels the running step, whose result is `cancelled` with
   its `start`, or writes `cancelled` without `start` for a waiting command. The command is
   settled once.
8. **Rebuild.** The slot is folded from command, start and result pairs:
   - A manual start without a result: the step was running. The Loop writes the `cancelled`
     result with that `start` and `command`.
   - A command with neither a start nor a result: it was waiting. The Loop writes a `cancelled`
     result with that `command` and no `start`. Each command is settled once. An unloading Loop
     starts no further step.

Unchanged: turn and request lifecycles, input consumption, tool batches, steer of model
requests, budget recovery, `LoopSnapshot` and `context/prepare`.

### 5.6 What ContextPreparation gains

- `ContextPolicy::prepare` calls `compact_step`. ContextPreparation injects
  `CompactionStrategyHost`, and its policy exists when both hosts are `Some`.
- **Trigger.** The automatic trigger runs in `prepare` only when both hold:
  - the estimate reaches `threshold_tokens` (existing);
  - at least `0.25 × B` of new history has accumulated since the branch's latest compaction
    attempt. New history is the entries placed by events after that attempt's
    `context/compaction-start` `through`, counted with `count_tokens` each and summed. The
    attempt is the latest start of any trigger and any outcome; a branch with no start has all
    its history new.

  This replaces the `context/budget` once-per-boundary check: at the same boundary no new
  history has accumulated, so the second rule already stops a repeat. A skipped or failed
  automatic compaction therefore never runs again on the same history, and a manual compaction
  moves the anchor like an automatic one. Budget recovery is exempt from both rules, which is
  how a request that does not fit is still compacted.
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
| Strategy error or validation failure, manual inside a turn | start, then `context/compaction {start, failed}` | Continues the turn uncompacted (`next_model`). If the next request exceeds the input budget, the existing budget recovery forces a compaction (fallback cut); if that also fails, the request and the turn fail | Running, then `failed`; reply `compaction_failed` |
| Same, manual outside a turn | start, then `failed` | Nothing more (`try_start` if an input is waiting) | Running, then `failed`; reply `compaction_failed` |
| Same, automatic | start, then `failed` | Unchanged: the request continues uncompacted; under recovery the request fails | Running, then `failed` |
| Recovery compaction completed, the request still exceeds `B` | start, then `completed`; no further `model/call` | The preparation fails with the budget error that names what keeps the request above `B` (§5.3); the Loop does not compact again, so the turn fails with it | The compaction shows `completed`, with its mark when its own parts cannot shrink; the turn shows the budget error |
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

### 5.9 Class 2(a): the running turn's inputs

When the cut falls inside the running turn, that turn's inputs would leave the context with the
rest of the replaced range. The replacement carries them verbatim.

- **Where it lives.** ContextPolicy, not the strategy. The rule needs only public facts
  (`input/message`, `input/consumed` and the turn's `turn/start`), it is the same for every
  strategy, and a strategy must not be able to drop it. The summary also covers the same entries,
  because they lie in the replaced range; the pin repeats them because a summary is lossy and the
  running turn's own words are what must not be lost.
- **When.** The step has a running turn — `turn_id` is `Some`, which an automatic compaction
  always has and a manual step has only inside a turn — and the cut is later than that turn's
  `turn/start`. A cut at the turn's start or in an earlier turn pins nothing.
- **Which inputs.** Every `input/consumed` event of that turn before `retain_from`, in event order,
  with the `input/message` it names. Inputs consumed at or after the cut are already in the kept
  tail and are not repeated. When the input message lies before the fixed context, it is one the
  projection checkpoint still held unconsumed, and that state supplies it; without a checkpoint
  the fixed context holds every event, so a missing snapshot costs an unread prefix, not the
  content.
- **Which parts.** The input's user content and every `InputBlock::Raw` that declares no `key`,
  each as the message the renderer places for it. That rendering is the projection's own rule,
  shared with the renderer, so the pinned text is exactly what the model saw. A block that
  declares a key is excluded whether or not it retires: the keyed baseline replays the key's live
  value, and a retirement is the key's absence. An input is told apart by its events and these
  public fields only; no marker inside an input's text is parsed, so a plugin that writes keyed
  content (TimelineTree's branch context, for example) is excluded by its key and never by its
  text.
- **Budget.** The pinned messages total at most `0.1 × B`, counted with `count_tokens`. When all of
  them fit they are pinned verbatim; when they do not, each input keeps its head and tail within
  an equal share of the budget, with a marker naming its `input/message` event id — the handle
  Recall opens for that input ([Recall §4](Recall.md#4-handles-time-and-event-id)) — and a share
  too small for the marker keeps the marker alone. Beyond the limit a single input therefore keeps
  its head and tail plus its event id.
- **Placement.** The last messages of the replacement (§4.3), after the strategy's messages and the
  digest, so the running turn's words directly precede the kept tail.
- **Bound.** The pinned messages are resident in the `0.5 × B` bound (§5.3), and they are stored in
  the record's `replacement`, so replay needs no rule of its own.
- **A later compaction.** The pins are part of the previous replacement (§4.3), so they are not
  in `history`; the next compaction passes them to the summarizer as entries and pins only the
  running turn again. Pins of a turn that has since ended are summarized like any other entry
  and never re-pinned.

### 5.10 Class 3a: the digest

The digest is the deterministic account of what tools did: the files they wrote, the commands they
ran with their exit status, and nothing else.

- **Generated by the mechanism, from facts.** ContextPolicy builds it from the declarations in the
  events it replaces and rolls it forward from the previous replacement's digest part
  (`input.checkpoint.replacement_parts`, §4.3), which the policy owns and reads directly; the model
  neither writes nor chooses it, and the part is never sent to the summarizer. It reads no tool's
  input schema: a tool's effects reach it only through the declaration its owner provides.
- **In the replacement.** One message, wrapped as data like the summary, after the strategy's
  messages and before the pinned inputs (§4.3).
- **Empty until the declaration exists.** No tool declares effects today; the means is
  [ToolRegistry's](../compaction-quality.md#74-toolregistry-the-tool-contract), and the
  requirements raise it there. Until it exists the digest has no content and the replacement
  carries no digest message. A tool's result text in the class 3b summary is not the digest, and
  nothing derives one from tool output or from a tool's schema.

## 6. Errors

`MethodError` codes (`code -32000` unless noted):

| Code | Method | When | Retry |
| --- | --- | --- | --- |
| `-32602` | both | `branchId` missing or not a string | No |
| `session_invalid_input` | both | Unknown branch | No |
| `session_closed` | both | Session or branch closing | No |
| `context_not_configured` | usage | No `context/configured` in the branch's history | After a model is adopted |
| `context_model_failed` | usage | `resolve` failed (no capability snapshot) | After selecting the model again |
| `compaction_failed` | compact | Fact `failed`: strategy error after the shared failure policy, a reply with no text, a `retain_from` that fails validation, a compaction that reaches its summarizer retry limit, no reduction, no `context/configured`, or no snapshot | User's choice |
| `compaction_cancelled` | compact | Fact `cancelled`: `stop/interrupt` or reload | User's choice |
| `operation_interrupted` | compact | ContextMethods unloads while waiting; the command is still served or settled | User's choice |

Automatic compaction errors are unchanged: a strategy error becomes a `failed` fact and the
request continues uncompacted; under recovery it fails the request.

A request that still exceeds `B` after its recovery compaction fails as a turn failure
(`ModelOutputEnd failed`) whose error names what keeps it above `B`: the resident parts that are
too large, or the entries the last event placed (§5.3). It is not a method error and no second
compaction runs.

A `failed` fact carries the same cost diagnostics as a completed one whenever the strategy ran
(§4.3).

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
| #2 dynamic events | No change: both events stay `EventKind` variants with the same wire form |
| `--models` profile | The same strategy and `session.context` methods; window facts come from the model entry, and the request's capability snapshot stays unset |
| Mock profile | Both hosts `None`: no compaction, and `session.context` is not recorded |
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
| §4 rule 8 (infrastructure unchanged) | Dispatcher, envelope, rutis, loader, Agent assembly and facades unchanged. There is one infrastructure change (#130): snapshot anchoring in Timeline and both stores drops its knowledge of `context/compaction` and keeps only structural checks, and the owner's decode callback receives the anchor event. The requirement comes from Timeline itself: it must not know event content (principles §4 rule 4; Timeline checks only the envelope and reserved core domains, ADR 001). It closes within Timeline's own contract: a snapshot is a cache with a structural anchor. Valid snapshots behave the same, and an invalid one is a cache miss. |
| §4 rule 9 (only needed facts) | `context/compact` is read by the Loop's later decisions and on reload. `context/compaction` is used by projection. `context/compaction-start` and the visible result are facts by the maintainer's decision, for two reasons. First, they do not only drive a state machine: they are content that users and other state machines need in order to read the history (a UI connecting at any time must see that a compaction is running or how it ended). Second, reload settlement uses them: an unmatched start means a compaction was in flight. The Loop and ContextPreparation rebuild from start/result pairs, which is more precise than commands alone because it tells a running compaction from a pending one and covers automatic compaction. Usage writes nothing. The completed result's `modelUsage` and `insufficientSpace` are descriptive fields of that required fact: the trigger reads the latest start's `through`, recovery reads the budget, and only a client reads the cost and the mark. |
| ADR 002 D1 (scope) | Window facts, strategy and methods: Session. Command, step and slot: branch, in the Loop. The pinning, the digest, the retention rule and the mark are branch-scope ContextPolicy work: they read the branch's events and compose its record. |
| ADR 002 D2 (no wiring modules) | ContextMethods exposes owners' operations through `sessionMethods`. |
| ADR 002 D3 (dependencies) | Hard dependencies on core only. A Provider or Aimux reload propagates to the strategy part (accepted cost). |
| ADR 002 D4 (configuration) | No configuration; provisional constants are code (#128). |
| ADR 002 D5 (language-neutral contracts) | §4.1 and §4.3 are the contracts; §4.4 is one binding. |

**The compaction-quality requirements.** §14 maps the requirements of
[#428 §7.1](../compaction-quality.md#71-contextcompaction) to their sections. They add no event,
Session value or method: the bound, the mark, the regrowth rule, the pinning and the digest are
parts of the existing compaction path, the replacement and its record. That a skipped or failed
automatic compaction never runs again on the same history follows from regrowth (§5.6); it is not
a separate rule. The strategy port gains one read-only method, `replacement_size` (§4.2), which
names no service and adds no dependency; rule 8 is unaffected, and each write is still one event
(rule 5).

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

**Requirements raised to mechanism owners:** none.

## 10. Decisions and open questions

| Item | Decision |
| --- | --- |
| Which model compacts | The request's own model, in the summary strategy; a separate model is deferred |
| Trigger and retention | #128: trigger at `B − min(max(0.10 × B, 16,000), 0.20 × B)`; keep the longest tail that fits `min(0.25 × B, 0.5 × B − resident)` (§5.3), and replace everything when no kept tail fits the bound; manual steps keep at most the latest turn; `validate` compares the trigger's estimate with `B`; `bytes / 4` and 1,600 tokens per image unchanged. The reserve and share are provisional heuristics |
| Manual compaction | A Loop step scheduled by `context/compact`, inside or outside a turn, never creating a turn |
| Where the step runs | In the Loop, calling the one ContextPolicy compaction function (§2.1) |
| Record | `retain_from` (Timeline event id), `replacement` and `strategy` with one replay rule; optional ids, `command`, `cancelled`; changed directly (unreleased) |
| Ports | `ContextModel` (window facts) and `CompactionStrategy` (replacement), each from its own Session value |
| Other strategies, composition | Not done in F1 |
| New event kinds | `context/compact` and `context/compaction-start` as `EventKind` variants |
| `--models` profile | Window facts from the model entry; the capability snapshot stays unset |
| Provider-reported overflow | Separate Aimux issue, outside F1 |
| Diagnostics | Strategy-neutral: common counts, the summarizer's summed usage and the insufficient-space mark, plus a strategy-owned object (breaking) |
| Compaction state | Facts: pending = command without start, running = start without result, done = result (§5.7) |
| Half-finished compaction | Never resumed; settled as `cancelled` (§5.8) |
| Summarizer reasoning (#170) | Decided by the evaluation (§13.4): `summaryReasoning: inherit \| off \| low` in `compaction.json`, or a constant if one value wins on every model class. Until then `inherit` (today's behaviour) |
| Target summary length (#170) | Decided by the evaluation (§13.4): `S = min(α × O, β × B)` stated in the prompt, or no instruction. Until then no instruction, and the planning size uses α = 0.5 and β = 0.10 (provisional) |
| Window limits | None of compaction's own: the window facts are the model's catalog values with their overrides, and `B`, the threshold, `0.5 × B`, `0.25 × B` and `S` scale with the declared window. A smaller useful window is declared smaller in the catalog (Provider) or by the `contextWindow` override (§5.1) |
| The `0.5 × B` bound | The tail budget is `min(0.25 × B, 0.5 × B − resident)` with the replacement planned at `replacement_size` (§13.3's `S` at the provisional α = 0.5, β = 0.10); when no kept tail fits, the limit is the cut that replaces everything; the bound is reached by the cut, so an overshooting summary still completes (§5.3). The `0.5` share is provisional (#428) |
| Insufficient space | A descriptive `insufficientSpace` object in the common diagnostics of a completed compaction, naming the largest resident parts; no decision reads it (§4.3, §5.3) |
| Regrowth | Automatic threshold compaction needs `0.25 × B` of entries after the latest compaction attempt's `through`; recovery is exempt and runs once per request (§5.6, §5.3) |
| Recovery failure | A request still over `B` after its recovery compaction fails with an error naming what keeps it there — the resident parts, or the entries the last event placed — and is not compacted again (§5.3, §6) |
| Class 2(a) pinning | In ContextPolicy, not the strategy: the running turn's inputs before the cut, keyed blocks excluded, at most `0.1 × B` (provisional, #428), head and tail with the `input/message` id (§5.9) |
| Class 3a digest | One data message in the replacement, after the strategy's messages and before the pinned inputs; empty until ToolRegistry declares effects (§5.10) |
| Summarizer work | At most `chunkCount + 8` requests per compaction, each counted when sent; `modelCallCount` and `durationMs` report them, and `modelUsage` sums their usage (§5.2, §4.3) |
| Substantive summary | Non-empty is enforced on every reply, so a committed summary is never empty; the parts are checked when [#429's prompt](../compaction-quality.md#77-model-facing-prompts-429) declares them, added with it (§5.2) |

Open: the #128 follow-ups outside F1 (a summarizer request that reuses the conversation prefix,
Anthropic cache breakpoints, provider-reported overflow mapped to recovery) and running the
summary evaluation of #170 (§13), whose results set the two rows above.

## 11. Acceptance (flow tests)

The tests run a product-profile App Server against a mock HTTP provider with a small context
window, and the same App Server on a `--models` file whose entry declares a small window
(`crates/dim-app-server/tests`), plus Loop and policy flow tests in
`crates/dim-agent-capabilities/tests`.

1. **Automatic.** Past the threshold, and with `0.25 × B` of new history since the previous
   compaction, the next preparation writes `context/budget`, then
   `context/compaction-start {trigger: threshold}`, and sends one summarizer request. It commits
   `context/compaction` `completed` with the request's ids, `start`, no `command`, `strategy: "summary"`, a one-message `replacement`, and `retain_from` = the
   earliest legal cut whose kept tail fits `min(0.25 × B, 0.5 × B − resident)` (§5.3). The model
   request starts with that message and keeps that tail verbatim. The product window facts give
   the reserve's cap, floor and ratio (32k/8k → 19,200; 128k/8k → 104,000; 1M/128k → 784,800).
   Many small turns after a large one are kept beyond two turns; a keyed block replayed after the
   cut keeps fewer of them than the same block unkeyed when the `0.5 × B` term is what binds.
2. **Budget recovery.** The retry compacts with `trigger: provider_output_budget_exhausted`.
   With a provider baseline, a request whose local count fits the window but whose baseline
   estimate exceeds `B` recovers; one whose baseline estimate fits does not.
3. **Manual, no turn.** `session.context.compact` writes exactly `context/compact`,
   `context/compaction-start {command}` and `context/compaction {command, start}`, with no
   `turn/*`, `step/*` or `model/call`. The reply is
   `completed`. On a two-turn session the cut is the latest turn's start, or a cut inside that
   turn when its input alone exceeds the tail budget.
4. **Manual, inside a turn.** `step/tool-result` → `context/compaction-start {turn_id, command}`
   → `context/compaction {turn_id, command, start}` →
   the next request's `step/model-start`, prepared from the compacted context.
5. **Replay and strategy replacement.**
   - A Session reloaded after compaction, and a branch forked after it, project the same messages
     as before.
   - A test strategy bound in place of the summary strategy, returning a two-message
     `replacement`, is committed and replayed by the same rule, with no Loop or policy change.
   - The policy rejects a test strategy whose `retain_from` lies inside the retained tail, or
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
     exceeds the input budget, budget recovery compacts.
7. **One outstanding command.**
   - Repeated or concurrent `session.context.compact` calls while a command is outstanding: the
     Timeline holds exactly one `context/compact`, one start and one result, and every caller
     receives the same reply.
   - A `context/compact` consumed while another is outstanding (written without
     `AgentLoop::compact`) gets `skipped: "a compaction is already in progress"` at once, with no
     start, and the outstanding compaction completes normally.
   - `stop/interrupt` or a plugin reload with one outstanding command settles it `cancelled` once.
8. **Compaction state for clients.**
   - A v3 client attached to the Session receives `context/compact`, `context/compaction-start`
     (`trigger: manual`) and `context/compaction` live.
   - A client that connects while the summarizer request is held open sees "running" from the
     branch history (an unmatched start).
   - An automatic compaction shows `trigger: threshold` with the request ids.
   - Across every test above (success, skip, failure, interrupt, reload, crash), each
     `context/compaction-start` has exactly one `context/compaction` with its `start`, and no
     result carries a `start` that does not exist.
9. **The `0.5 × B` bound.** With a large-output model (200,000 / 64,000), a 12,000-token frame
   and no running turn, the first request after the compaction counts at most `0.5 × B` with
   `count_tokens`, and its tail is the budget `min(0.25 × B, 0.5 × B − resident)` — 34,000
   tokens with the provisional target. With a frame that takes most of the bound, the compaction
   keeps fewer turns than the same history with a small frame. When no kept tail fits, the
   compaction replaces everything, so the first request is the resident parts, with the running
   turn's inputs pinned and no verbatim tail. A summary that overshoots its planned size
   completes, and its after-size is in the diagnostics.
10. **The insufficient-space mark.** On 32,000 / 8,000 a 12,000-token frame, with the summary at
    its planned size, already exceeds `0.5 × B`: the compaction completes and its diagnostics
    carry `insufficientSpace` naming the frame's parts and the summary, and not a small one. On
    200,000 / 64,000 with the same frame the compaction stays below the bound and carries no
    mark.
11. **Regrowth.** After a completed automatic compaction, a preparation at the threshold with
    less than `0.25 × B` of new history writes no `context/compaction-start`; once `0.25 × B` of
    new history has accumulated it does. Because no new history has accumulated, a skipped or
    failed automatic compaction is not retried on the same history, and a manual compaction runs
    on it.
12. **One recovery compaction.** Under recovery the compaction runs whatever the regrowth rule
    says. When the recovered request still exceeds `B`, the turn fails with the error that names
    what keeps it there, no second compaction runs for that request, and no `model/call` follows
    the recovery one.
13. **Class 2(a) pinning.** A cut inside the running turn carries that turn's consumed inputs
    verbatim, in event order, as the model saw them, including a steer that joined the turn; a
    keyed block, retiring or not, is not repeated; an input consumed after the cut is not
    duplicated; a turn whose inputs exceed `0.1 × B` pins each input as its head and tail with
    its `input/message` id; a cut at the turn's start or in an earlier turn pins nothing.
14. **Linear rolling.** A compaction over a history several times the window sends summarizer
    requests that carry the previous replacement's summary and pinned parts and the newly cut
    entries only: no entry placed before the previous cut appears, and the previous digest part
    is not sent but rolled forward into the new replacement.
15. **Bounded work.** A summarizer that cuts off every reply records `failed` after at most
    `chunks + 8` requests, and one that cuts off once per chunk completes with one request per
    chunk plus its retries. A summarizer that fails before any stream has its retries counted by
    the same allowance: a compaction that can no longer fold within it records `failed` with
    `modelCallCount` equal to the chunks folded plus `8`, and the provider saw exactly that many
    requests. Every failure path records one `context/compaction` for its start, and a failed or
    cancelled compaction leaves the projection as it was.
16. **Cost.** A completed compaction's diagnostics carry `modelCallCount`, `durationMs`,
    `modelUsage` with the input, output and cache tokens summed over its requests, and the sizes
    before and after; a compaction that sent no request carries no `modelUsage`. A compaction
    that fails inside the strategy after its requests carries the same fields on its `failed`
    fact.
17. **The substantive summary.** A reply that is empty or whitespace-only fails the compaction:
    the fact is `failed`, the reply is `compaction_failed`, and nothing is applied. The folded
    summary is the last reply, so it is never empty; the parts are checked when the prompt that
    declares them lands.
18. **The digest.** A completed replacement carries no digest message while no tool declares
    effects, and a record written before the digest exists still replays unchanged.

## 12. Implementation notes

The implementation follows this design. Where the code needed a detail the text did not
fix, the smallest choice was made and is recorded here.

| Item | Implementation |
| --- | --- |
| Start trigger values | `trigger` uses the existing `CompactionTrigger`: `manual`, `threshold`, `provider_output_budget_exhausted` (§4.3 said `recovery`; §4.2 and acceptance 2 already used the latter). |
| Legal cuts | `CompactionInput` also carries `cuts`: every legal cut in order (`retain_from`, the first kept entry, and whether it is within `retain_limit`), computed by ContextPolicy from the event-based validity rule. Candidates are turn starts, events that place entries, and the boundary's last event (which replaces everything). `CompactionInput::cut()` returns the latest cut within the limit or, when none is, the latest cut that keeps something (any cut under budget recovery). A strategy may return any `retain_from`; the policy still validates it. |
| Adoption | `compact_step` validates the strategy's result before it appends the fact, then adopts the committed fact as Timeline middleware left it: the projection, the reduction check and the snapshot use the committed `retain_from`, `replacement` and baseline. A committed cut that cannot be projected fails the preparation, as a committed cursor did before. |
| Diagnostics | `retainFrom` is absent when the strategy found no cut (`skipped`), and `estimatedOutputTokens` is absent when nothing was replaced. |
| v1 | There is no v1-to-native conversion in production code: the frozen samples decode as v1 events only, and `v1::EventRef` has no form for `context/compact`, `context/compaction-start` or the changed `context/compaction`, so v1 connections no longer receive compaction facts. The old flat diagnostics type moved into the v1 module. |
| Loop subscription | The Loop subscribes to `context/compact` and `context/compaction`; starts are folded only at rebuild, where they tell a running step from a pending command. |
| One outstanding command | Maintainer's decision during review: repeated requests are deduplicated at the interaction boundary. `AgentLoop::compact` joins an outstanding command instead of writing another, the slot holds at most one command, a command consumed while another is outstanding is skipped, and the waiter matches its own command id (§4.1, §5.4, acceptance 7). This replaces an earlier active/pending split, so commands can no longer interleave with a running step. |
| Snapshot anchor | Timeline keeps the structural checks of §4.3. The old `resume ≤ through` check needed the anchor's content; it is replaced by the structural "resume precedes the anchor on the anchor's branch", and the owner's decode checks `through` before the anchor. |
| Native storage example | The record example in [native-storage-format](../appserver/native-storage-format.md) uses the changed record and the v3 interpreter. |
| Summarizer budget | Review fix: chunk packing and truncation count the whole summarizer request with the window facts' `count_tokens` against `context_window − output_tokens` (§5.2), so CJK-heavy history no longer exceeds the window; `product_summary.rs` checks every summarizer request. |
| Acceptance | Acceptance 1, 3, 6 (crash and restart) and 8 run in the binary flow test `transport_flow/compaction.rs`. Acceptance 2, 4, 5 and the reload, interrupt and failure cases of 6 run in `dim-agent-capabilities/tests/context_flow` (`manual_compaction.rs` and the existing compaction tests). The crash between fact and snapshot is covered by the snapshot-failure test in `compaction_events.rs`. |
| Trigger, retention and recovery (#128) | `ContextPolicy` keeps the estimate of the request its `prepare` returned (a full count after a completed compaction), so `validate` checks the same request without reading the context again; a request it did not prepare reads the context once. Each kept entry is counted once with `count_tokens` and summed, so the per-message separators of a joint count are left out. The trigger values are in `transport_flow/compaction.rs` and `product_summary.rs`; retention and recovery in `context_flow/retention.rs`; the manual cut in `manual_compaction.rs`. |
| Dead zone and cut-off summaries | #169: the retention fallback no longer depends on budget recovery, so a single long turn compacts at its latest cut that keeps something; only recovery may replace everything. `fallback` is removed from `CompactionInput`. §5.3 changes that last clause: the replace-everything cut becomes the limit when no kept tail fits the bound, for every trigger. A summarizer reply stopped at `max_tokens` is retried in halves and fails at the smallest chunk instead of being kept. |

## 13. Summary quality: reasoning and target length (#170)

Status: approved design for [#170](https://github.com/arcships/dim/issues/170). It designs the
evaluation and the two knobs; the defaults are decided by running the evaluation (§13.4) and are
recorded in §10 when it has run. Until then the default behaviour (§5.2) is unchanged: the
summarizer inherits the conversation's reasoning and its prompt states no length.

### 13.1 What is decided here and what is not

The summary strategy has two open choices. Both are prompt and configuration content of the
strategy layer (§2.2), not mechanism, so neither changes the Loop, ContextPolicy, the record or
replay ([research §5.4](../research/compaction-trigger.md#54-what-the-reserve-is-for-and-the-summarizers-generation-length-code-maintainer-review)).

| Choice | Today [Code] | Why it is open |
| --- | --- | --- |
| Reasoning of summarizer requests | `summary_request` copies `input.request.config` and only sets `maxTokens = output_tokens` ([summary_strategy.rs](../../../crates/dim-agent-capabilities/src/summary_strategy.rs)), so a thinking conversation gets a thinking summarizer | Thinking shares `max_tokens` with the summary text and its length is not controllable on every model; since #169 a reply stopped at `max_tokens` is halved and retried, so thinking costs calls and latency, not correctness. Whether thinking improves summaries is unmeasured |
| Target length | The fixed `SYSTEM` prompt says "concise" and states no length | A stated length may bound the replacement that every later request carries, and may cut `max_tokens` stops, or it may drop facts. That is a prompt-quality question |

Out of scope: a separate summarizer model (§10), summarizer prefix reuse and cache breakpoints
(#128 follow-ups), and any change to chunking, the cut-off rule, retention or the trigger.

### 13.2 `summaryReasoning`

**Contract.**

| Value | Summarizer request config |
| --- | --- |
| `inherit` | The conversation request's config, as today |
| `off` | The conversation request's config with reasoning effort `none`, mapped as Provider maps it for a conversation |
| `low` | The conversation request's config with reasoning effort `low`, mapped the same way |

- The value applies to every summarizer request of a compaction, read once when the compaction
  starts. `maxTokens = output_tokens` is unchanged in every case.
- `off` and `low` apply where the model allows it. Where Provider's mapping rejects the effort for
  this model and protocol (no reasoning support, no reasoning-off declaration for the protocol,
  `low` not among the model's effort options, Bedrock, the local development connection), the
  request uses `inherit`. This is the business meaning of the setting: a preference for the
  summarizer, not a requirement that can fail a compaction. A model without reasoning already
  behaves as `off` under `inherit`.
- `low` does not look at the conversation's own reasoning: it asks for effort `low` even when the
  conversation runs with reasoning off. Comparing a request's effective reasoning across protocols
  would need a second interpretation of provider options, which only Provider has.
- The value applied is written into the summary diagnostics object as `reasoning` (§13.5), so a
  record shows what the summarizer actually did. A compaction that sends no summarizer request
  records the requested value.

**Where it lives.** Business configuration of the summary strategy (ADR 002 D4): the user file
`compaction.json` in the product home, `{"summaryReasoning": "inherit" | "off" | "low"}`, unknown
members rejected.

- `ProductSummaryStrategyPlugin`'s factory captures the product home path, and
  `SummaryStrategy::new` resolves it at load with `FileStorage::register_root`, which returns the
  root `serve` already registered for that home (idempotent by path, so no new root is opened);
  the product profile is assembled in `cli.rs`, before `serve` computes the root list, so the
  factory cannot carry that root's index. The strategy then reads `compaction.json` under that
  root through `FileStorage` at the start of each compaction. A change takes effect at the next
  compaction without a reload. A missing file means the default (§13.4).
- An unreadable or invalid file fails the compaction (`failed`, reply `compaction_failed`) with a
  message that names `compaction.json`, as an invalid `exec-shell.json` fails the command that
  needs it. The file is the user's explicit choice; silently using another value would hide it.
- User scope only. A project file is not offered: summarizer reasoning is a cost and quality
  preference of the user, not of a repository.
- It is not a declared Settings source (those are for owners not written in Rust) and not a
  `BusinessDocument` with management methods: no client edits it in #170. When a client needs to,
  it becomes a `BusinessDocument` with its owner's methods, with the same document.
- The mock profile does not install the strategy, so it has no file. The `--models` profile
  installs it with its `--home`, the home its storage already uses, so its `compaction.json` sits
  there.

**How it maps through Provider.** The effort mapping already exists:
`ModelPreferences::configure` ([preferences.rs](../../../crates/dim-app-server/src/provider/preferences.rs))
turns `reasoningEffort` into config fields per protocol. For `none` it uses the model's
`reasoning.off` declaration (`thinking_type: disabled`, `thinking_budget: 0`,
`enable_thinking: false`), or a native `none` effort when the model declares one; otherwise it
rejects the effort. It is reached today only through `ProviderService::configuration` and
`apply_preferences` in the App Server.

The strategy lives in `dim-agent-capabilities` and cannot call `ProviderService`, and a part that
only adapts Provider for the strategy would be a wiring module (ADR 002 D2). So:

1. The reasoning-effort branch of `ModelPreferences::configure` moves, unchanged in behaviour, to a
   pure function next to `ModelProtocol` in `dim-agent-capabilities`:
   `reasoning_config(base: &ModelConfig, snapshot: &ModelCapabilitySnapshot, protocol: ModelProtocol, effort: &str) -> Result<ModelConfig, &'static str>`.
   It reads `snapshot.capabilities.reasoning` and `snapshot.reasoning`, which are exactly the
   catalog fields `configure` reads today (`ProviderService::configuration` copies
   `model.capabilities` and `model.metadata.reasoning` into the snapshot). `configure` calls it.
   Provider's behaviour and errors stay the same; its existing flow tests cover the move.
2. The strategy resolves the chunk's connection as today, takes `protocol` from the resolved HTTP
   connection, and applies `reasoning_config` to the copied config before invoking with that
   connection. Connection resolution reads the request's provider, model and snapshot, not its
   config [Code: `ProductResolver`], so the connection resolved for the inherited config is the
   connection for the mapped one. `Err` means `inherit` (above).

### 13.3 Target length

**Instruction.** When a target applies, the system prompt ends with one sentence:
"Keep the summary under about S tokens; write less when the transcript needs less." The same
sentence applies to every fold, so the carried summary stays bounded while chunks are folded.

**Deriving S.** With `O = model.output_tokens()` (the allowance the summarizer request gets as
`maxTokens`) and `B = context_window − O` (the input budget):

`S = min(α × O, β × B)`, rounded down to a multiple of 100.

- `α × O` leaves `(1 − α) × O` of the allowance for thinking and for overshoot, so a summary that
  follows the instruction does not reach `max_tokens`.
- `β × B` bounds what the replacement takes from every later request. After a compaction the
  context is about `S` plus the retained tail (at most `0.25 × B`, §5.3), so `β = 0.10` keeps it
  near `0.35 × B`, well below the trigger.
- S is a soft instruction. `maxTokens` stays `O`: a hard limit at S would turn every overshoot into
  a cut-off reply, which is halved and retried (#169).

Candidates evaluated (§13.4): `α ∈ {0.25, 0.5}`, `β ∈ {0.05, 0.10}`, and no instruction.

| Window / output | B | S(0.25, 0.05) | S(0.25, 0.10) | S(0.5, 0.05) | S(0.5, 0.10) |
| --- | --- | --- | --- | --- | --- |
| 32,000 / 8,000 | 24,000 | 1,200 | 2,000 | 1,200 | 2,400 |
| 128,000 / 16,000 | 112,000 | 4,000 | 4,000 | 5,600 | 8,000 |
| 200,000 / 64,000 | 136,000 | 6,800 | 13,600 | 6,800 | 13,600 |

The chosen rule is a code constant of the strategy (ADR 002 D4: provisional constants are code),
passed as factory configuration of `ProductSummaryStrategyPlugin` so that the evaluation can run
every candidate through the shipped strategy. It is not user configuration.

### 13.4 Evaluation

**Where it lives.** Not in CI: it calls real models, is nondeterministic and costs money, and
AGENTS.md keeps automated tests for flows and contracts. It is an `xtask` command beside
`measure`, which already runs with an optional real model configuration:

```bash
cargo run --locked -p xtask -- summary-eval --home DIR --model PROVIDER/MODEL --judge PROVIDER/MODEL --runs 3 --out DIR [--reasoning inherit|off|low]
```

- Without `--reasoning` the runner runs phase 1: the three reasoning arms (`inherit`, `off`,
  `low`) with no target instruction. `--reasoning VALUE` runs phase 2 instead: the no-instruction
  arm plus the four `(α, β)` arms, with `VALUE` being the reasoning value phase 1 chose for the
  model.
- `--home` is required and must be an isolated product home in which the operator has configured
  the providers; the runner never uses `~/.dimcode` or `~/.dimcode-alpha`. Credentials stay in that home and never enter
  the repository, the cases or the output.
- The runner serves the product profile on that home (as `graph` does), builds the conversation
  request from `ProviderService::configuration` for `--model` with default preferences (what a
  user gets), and resolves `ProductContextModel` on it.
- For each case and arm it constructs the product summary strategy from the process's `ModelHost`
  and `ProviderConnectionResolver`, with the arm's target rule as factory configuration and the
  arm's `compaction.json` written into the home, and calls `compact` directly with a
  `CompactionInput` built from the case. Every arm therefore runs the shipped request building,
  chunking, cut-off handling and reasoning mapping; only the two knobs differ.
- Output (JSON per run plus a Markdown table) goes to `--out`, outside the repository. The
  decision and its table are committed as a research note
  (`docs/design/research/summary-eval.md`, EN + zh) when the evaluation has run.

**Cases.** JSON files in `xtask/summary-eval/cases/`, one per history:

```json
{ "id": "long-tool-output",
  "description": "Debugging a failing flow test with a 60 KB cargo test log",
  "contextWindow": null,
  "earlierSummary": null,
  "messages": [ /* dim-agent-types Message values, as in ModelRequest.messages */ ],
  "retainFrom": 14,
  "probes": [
    { "id": "test", "kind": "exact", "text": "transport_flow::compaction::manual_inside_turn" },
    { "id": "cause", "kind": "fact", "text": "The failure is a missing start reference on the cancelled result" },
    { "id": "old-plan", "kind": "absent", "text": "The plan to change the retention share is still current" } ] }
```

- `messages` use the existing `Message` type, parsed at the runner's boundary; no new history
  type. The runner gives message `i` the source `m<i>`, and the only cut is `retainFrom` (checked
  not to separate a tool call from its result), so `retain_limit` is that cut.
- `earlierSummary`, when present, becomes the checkpoint's `replacement`
  (`[summary_message(text)]`), so the merge path is evaluated.
- `contextWindow`, when present, narrows the window facts for that case (a smaller window is always
  legal), so the folding path runs on an affordable history. It also bounds the case's output
  allowance to a quarter of that window (`config.max_tokens = min(output, contextWindow / 4)`), so
  the input budget stays positive on a model whose max output is large; each run's JSON records
  the effective window and the effective output. A case whose input budget would not be positive
  is refused before any model call.
- Content is public: built from this repository's own code, documents and command output, with
  paths under `/work/dim`. No real user session, no secret, no home path.

| Case | What it exercises | Probes include |
| --- | --- | --- |
| `long-tool-output` | One tool result of tens of KB with the key line in the middle; middle truncation | failing test name and error line (exact), the cause and the fix decided |
| `cjk` | A session conducted in Chinese against `docs/zh` | constraints stated in Chinese, the reply-language preference, identifiers kept verbatim |
| `code` | A refactor with file contents and diffs | changed function signatures and file paths (exact), what remains to do |
| `instructions` | Constraints given over several turns, one later revised | the revised constraint (fact), the superseded one not presented as current (absent) |
| `merge` | An earlier summary plus a new transcript | facts from both |
| `folding` | A history several times the narrowed window | facts from the first chunk still present after the last fold |
| `injection` | Tool output that tells the reader to drop instructions or write something | the summary records it as data and does not follow it (absent) |

**Judging.**

- `exact` probes are checked mechanically: the text appears verbatim in the summary, after
  whitespace normalization.
- `fact` and `absent` probes are judged by `--judge`, a different model family from the
  summarizer where possible, at temperature 0 when the model accepts it. The judge sees the
  summary and the probe list only, not the transcript, and answers per probe `present`, `absent`
  or `contradicted` with a supporting quote. A `fact` scores when `present`; an `absent` probe is
  a violation when `present`. The judge prompt is part of the runner.
- Before recording a decision, the operator reads the lowest-scoring summaries of each arm and
  corrects judge errors by hand; corrections are listed in the research note.

Metrics per model and arm, over cases and runs: fact recall `F`, exact recall `X`, violations `V`,
cut-off replies `C` (`cutOffCount`), failed compactions, median summary tokens `T`
(`estimatedSummaryTokens`) and median `durationMs`; for target arms also the ratio `T / S`. Runs
whose applied reasoning (`reasoning`) differs from the arm's value are counted as `fellBack`, so
an arm the model rejects is not compared against itself (§13.2). A probe the judge did not answer
is counted as `unjudged`, in a column of its own for the operator to correct by hand. A failed
compaction has no summary: its `fact` and `exact` probes count as missed and its `absent` probes
count as not violated, without a judge call, so a failure lowers `F` and `X` and adds nothing to
`V`.

**Models.** At least one model per reasoning class the catalog offers: budget thinking with a
`thinking: disabled` off declaration (Anthropic), effort-based reasoning (OpenAI Responses),
an OpenAI-compatible model with `enable_thinking` or `thinking.type` (a model strong in Chinese),
and one model without reasoning as a control.

**Protocol.** Two phases, so that the arms stay few:

1. Reasoning: arms `inherit`, `off`, `low`, no target; every case, `--runs` times.
2. Target: with phase 1's reasoning value for each class, arms no instruction and the four
   `(α, β)` candidates.

**Decision criteria** (`δ = 0.05`):

- **Reasoning default.** For a class, an arm passes when `F ≥ F_inherit − δ`, `X ≥ X_inherit − δ`,
  `V ≤ V_inherit` and its failures are no more than `inherit`'s. The default is the first of
  `off`, `low`, `inherit` that passes on every class: the cheapest value that loses no measured
  quality.
- **Keep the setting or not.** If on every class the default is also the best arm (no other arm
  has `F` more than `δ` above it), the setting is not shipped: the value becomes a strategy
  constant and `compaction.json` is not introduced (ablation). Otherwise the setting ships with
  that default, and the note says which classes benefit from another value.
- **Target length.** An arm passes when `F ≥ F_best − δ`, `X ≥ X_best − δ` and `V ≤ V_best`,
  where best is the best phase 2 arm, including no instruction. If no instruction passes and its
  `C` and failures are no higher than those of every passing target arm, no instruction is kept.
  Otherwise the passing target arm with the fewest `C` plus failures is kept, ties broken by the
  smaller median `T`. A kept target must be followed: the 90th percentile of `T / S` is at most
  1.5; if not, the instruction does not control length and is not kept.
- The result, the arms' table and any hand corrections go into the research note; §10 records the
  defaults and §13 becomes the implemented contract.

### 13.5 Changes when implemented

| Part | Change |
| --- | --- |
| `summary_strategy.rs` | Reads `compaction.json` once per compaction (if the setting ships); maps the config through `reasoning_config`; appends the target sentence when the rule gives one; the strategy value gets a public constructor taking `ModelHost`, `ProviderConnectionResolver`, its factory configuration and the optional `FileStorage` handle the plugin injects when a home is configured, for the runner |
| `ProductSummaryStrategyPlugin` | Factory configuration: the `compaction.json` location and the target rule |
| Provider (`preferences.rs`) | The effort branch moves to `reasoning_config`; behaviour unchanged |
| Summary diagnostics object | Adds `reasoning` (the value applied: `inherit`, `off`, `low`), `targetTokens` (S, absent without an instruction) and `cutOffCount` (replies stopped at `max_tokens`; today they are only visible inside `chunkCount`). Additive fields of the strategy-owned object in `context/compaction`; no format version change |
| `xtask` | `summary-eval` command and `xtask/summary-eval/cases/`; `serve_home`, the product assembly on an existing home that `serve_product` (`graph`) and the runner share |
| App Server product profile | Passes the home's `compaction.json` location to the strategy factory; `RunningServer::service` hands a served profile's process services to `xtask`, so the runner reaches `ProviderService`, `ModelHost`, `ProviderConnectionResolver` and `FileStorage` |

No event, port, Session value, Loop, ContextPolicy, record or replay change.
`summary_reasoning_comes_from_the_compaction_file` in `transport_flow/compaction.rs` covers the
reader and the mapping: with `compaction.json` holding `off`, the summarizer request's config
carries the model's reasoning-off fields while the conversation's request keeps its reasoning,
and the record's diagnostics show `reasoning: "off"`; an invalid file fails a manual compaction
with `compaction_failed`. The target rule needs no test of its own: it is prompt text, measured
by the evaluation.

### 13.6 Rule check (for the #170 changes)

| Rule | Answer |
| --- | --- |
| Principles §1, one event bus | No new event or state; the knobs change only the content of the summarizer request inside one strategy call. |
| §3 dependency direction | The strategy gains no dependency: `FileStorage` is read as HostPlatform reads it, and `reasoning_config` is a function in its own crate. |
| §3 changing another module (Provider) | Provider's effort mapping moves into a shared pure function. The need is that the summarizer must apply the same mapping to a request without a wiring part (D2); the function keeps Provider's behaviour and Provider remains its only author. |
| §3 extension without change | Strategy layer only (§2.2): another strategy is unaffected, old records replay unchanged. |
| §4 rules 1–7 | Not applicable: no command, handoff, handler, core, batch, context path or public service is added. The replacement still reaches the model only through the completed fact's projection. |
| §4 rule 8 | No infrastructure change. |
| §4 rule 9 | No new fact; the new diagnostics fields are content of the existing fact that users read to understand a compaction. |
| ADR 002 D1 | Session part, unchanged scope. |
| ADR 002 D2 | No adapter part: the mapping is a function, the file is read by its owner. |
| ADR 002 D3 | Unchanged. |
| ADR 002 D4 | `summaryReasoning` is business configuration in the owner's file, read at use time, no reload. The target rule is a provisional constant in code. |
| ADR 002 D5 | The contract is the file's JSON shape (§13.2) and the diagnostics fields (§13.5); `reasoning_config` is a Rust-internal binding. |

**Requirements raised to mechanism owners:** none. The Provider change in the table is a change
to a product module, not to infrastructure.

### 13.7 Ablation review

Considered and removed:

- **A port or adapter part for the mapping** (a Session value bound by the App Server that calls
  `apply_preferences`): it would only connect Provider to the strategy (D2), and it would re-read
  the catalog instead of using the adopted snapshot. Replaced by the moved pure function.
- **A Settings source or management method** for `summaryReasoning`: no client edits it in #170;
  the file read at use time has the same document and can become a `BusinessDocument` later.
- **A project-scope file**: no repository-level need.
- **`low` as "at most low"** (keeping off when the conversation is off): needs a second reading of
  provider options; `low` is a plain request instead.
- **A hard `maxTokens = S`**: turns overshoot into cut-off retries.
- **A per-model or per-class default table** in the catalog: the criteria pick one default and
  drop the setting when one value wins everywhere.
- **A continuation probe** (asking a model for the next step from summary plus tail): the
  `fact` probes already cover what remains to be done.
- **A CI quality test**: nondeterministic and paid; the cases are data parsed by the runner.

Kept, with their purpose: the fallback to `inherit` (the setting's meaning, "where the model
allows"); `reasoning`, `targetTokens` and `cutOffCount` in diagnostics (the evaluation reads them
from the outcome, and a user reads from the record what the summarizer did; `cutOffCount` is the
cost thinking imposes, which is otherwise hidden in `chunkCount`); the factory configuration for
the target rule (it lets the evaluation run every candidate through the shipped strategy without
an evaluation-only code path).

## 14. Compaction quality requirements (#428 §7.1)

The requirements of [#428 §7.1](../compaction-quality.md#71-contextcompaction) are answered in
the sections that own each rule; this section is the map and states no rule of its own.

| Requirement | Design |
| --- | --- |
| The `0.5 × B` bound, and the tail cut later when the bound needs it | §5.3 steps 1 and 4, §4.2 `replacement_size` |
| The insufficient-space mark | §5.3 step 5, §4.3 diagnostics |
| Regrowth; that a skipped or failed automatic compaction never runs again on the same history follows from it | §5.6, §5.3 |
| One recovery compaction, then an error naming the resident part | §5.3 budget recovery, §5.8, §6 |
| Class 2(a) pinning | §5.9, §4.3 composition |
| Linear rolling | §5.2: the summarizer reads the previous replacement and the newly cut entries, unchanged |
| Bounded work | §5.2 bounded work |
| Self-reported cost | §4.1, §4.3, §5.2 cost |
| The substantive summary: non-empty on every reply, the parts checked with their prompt | §5.2 substantive summary |
| The class 3b state summary, the task or goal in force first | §5.2 substantive summary; the prompt and its declared parts stay [#429/#170's](../compaction-quality.md#77-model-facing-prompts-429) |
| The class 3a digest | §5.10, §4.3 composition |
| The stale §1 sentence | §1 |

**Requirements raised to mechanism owners:** none. The digest depends on the tool-effect
declaration that
[requirements §7.4](../compaction-quality.md#74-toolregistry-the-tool-contract) raises to
ToolRegistry; this design reads no tool's schema, and the digest stays empty until that means
exists.
