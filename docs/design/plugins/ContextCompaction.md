# ContextCompaction

Status: Draft (for review). Roadmap item F1 ([#72](https://github.com/arcships/dim/issues/72)).
Overview: [Appendix B.1 ContextPreparation row](../overview.md#b1-session-and-branch-capabilities).
Scope: [ADR 002](../../adr/002-plugin-scope.md): two Session parts, plus supplements to the
existing branch capabilities Loop and ContextPreparation. Nothing in this document is
implemented. Provisional values (trigger, retention) are studied in
[#128](https://github.com/arcships/dim/issues/128).

## 1. Summary

**Compaction** summarizes the history before a cut, normally everything since the previous
compaction point together with the previous summary, and keeps the last turns verbatim. From
then on the branch advances on the new result. The Loop does not advance while compaction runs:
compaction is a step of the Loop's progression. Otherwise the context could overflow during
compaction, and new events would be decided on the old context, inconsistent with the summary.

Automatic compaction already follows this. It runs inside a request's preparation step
(`context/prepare` → `context/compaction` → `model/call`) while the Loop waits for the
request's terminal ([context_policy.rs](../../../crates/dim-agent-capabilities/src/context_policy.rs)).
It never runs in production, because the App Server binds `ContextModelHost(None)`
([main.rs](../../../crates/dim-app-server/src/main.rs)). With that binding ContextPreparation has
no policy and Loop has no budget recovery.

F1 adds:

1. **A production host.** A Session part, `ProductContextModel`, provides
   `ContextModelHost(Some(..))`. It implements the existing port `ResolveContextModel` →
   `ContextModel` ([host.rs](../../../crates/dim-agent-capabilities/src/host.rs)) on top of two
   existing process services: `ProviderConnectionResolver` (connection and credentials) and
   `ModelHost` (Aimux, execution).
   - The model that compacts is the request's own model.
   - Its context window and output budget come from the request's adopted capability snapshot.
   - Token counting is a local estimate.
2. **One cut rule for both triggers.** ContextPolicy chooses the cut: the legal cut just
   before the last K turns (provisional K = 2, #128). It summarizes from the previous compaction
   point to that cut, including the previous summary.
3. **Manual compaction as a Loop command.** `session.context.compact` writes the command
   event `context/compact`, which the Loop consumes in order:
   - While idle, the Loop runs a compaction-only turn: one preparation step, no model call.
   - During a turn, the Loop forces compaction in the next request's preparation, before that
     request's model call.
   - Both cases go through ContextPreparation and ContextPolicy, the same path as automatic
     compaction, and the compaction fact carries the Loop's real turn and request ids.
4. **Usage query.** `session.context.usage` returns the budget the policy would compute now. It
   writes nothing.

Both methods are exposed through `sessionMethods` (namespace `session.context`) by a second
Session part, `ContextMethods`.

What F1 does not touch:

- Provider, Aimux, ModelProvider, Timeline, Dispatcher, the event envelope and rutis are
  unchanged.
- F1 adds one command event and one optional field on `context/prepare` (§9, format decision
  in §10).
- No new configuration and no new process plugin.

### 1.1 Why it is this small

- **Not AuxiliaryModel.** S4's `auxiliaryModel.complete` returns text only. Compaction needs the
  window, the output budget and the token counts of the model whose context is being compacted
  ([auxiliary-model §1.1](https://github.com/arcships/dim/blob/roadmap/s4-provider-purpose-policy-and-auxiliarymod/docs/design/appserver/auxiliary-model.md)).
- **Not inside Provider.** The host reaches everything it needs through the generic ports in
  `dim-agent-capabilities::host`:
  - The capability snapshot travels on the `ModelRequest`.
  - Credentials are resolved by `ProviderConnectionResolver`, exactly as ModelProvider resolves
    them for every turn.
  - Execution is `ModelHost`.
- **No second compaction path.** Manual compaction reuses the request preparation step. The
  Loop gains only the command, a pending mark and a compaction-only turn.

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
 |      compact -> .service::<AgentLoop>().compact()                 |
 |      usage   -> .service::<ContextPreparation>().usage()          |
 +---------------------------------^---------------------------------+
            Rpc (C1) dispatches ---'
 Branch scope (one Timeline, one Dispatcher)
 +-------------------------------------------------------------------+
 |  Loop (supplement)                                                |
 |    owns context/compact; pending mark; compaction-only turn;      |
 |    writes context/prepare {compact} as the existing handoff       |
 |        | context/prepare                     ^ context/compaction |
 |        v                                     | step/model-output-end
 |  ContextPreparation (supplement)  ContextPolicy: one compaction   |
 |    path for threshold, recovery and manual; cut before last K     |
 |    turns; writes context/budget, context/compaction, model/call   |
 |  ModelProvider (unchanged): model/call -> ModelHost               |
 +-------------------------------------------------------------------+
```

The two Session parts are separate because their dependencies differ. Branches inject
`ContextModelHost`, so its provider must not depend on `TreeCommands`, which the branch tree
provides. Both parts live in `dim-agent-capabilities`. All new dependencies point from new parts
to existing core services.

## 3. Data and state

| Item | Owner | Kind |
| --- | --- | --- |
| `context/compact {}` | Loop | New command event (§4.3) |
| `context/prepare.compact: "before_model" \| "only"` | Loop writes, ContextPreparation consumes | New optional field of an existing event (§4.3) |
| `context/compaction` | ContextPreparation | Existing fact, existing format; `diagnostics.trigger: manual` for manual compaction |
| Pending compaction: sequence of the latest unserved `context/compact` | Loop | Private, folded from consumed events, rebuilt on load (§5.4) |
| Served compaction: `{command, outcome}` of the last served command | Loop | `StateCell`, folded from the same events; read by ContextMethods to reply |
| Stage `Compacting` of a compaction-only request | Loop | Private request stage, rebuilt from `context/prepare {compact: "only"}` |

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

- It writes `context/compact` through the Loop and replies once the Loop has served that
  command (§5.3).
- `sequence` is the committed `context/compaction` event.
- `skipped` carries the policy's reason, for example `nothing_to_compact` when the history
  has no legal cut before the retained turns.
- Several commands written before one is served are served together, and each caller gets the
  same outcome.

`session.context.usage`:

- `through` is the committed head the numbers were computed at.
- The numbers are those a preparation at that head would write into `context/budget`, plus
  `thresholdTokens`.

Cancellation follows session-boundary A.1: an unload interrupts the call, and a caller's
disconnect cancels nothing. A served command that ended without compaction is an error reply
(§6).

### 4.2 ContextModel port (existing, Rust)

`ContextModel` is an existing runtime host port inside the Rust runtime, not a plugin contract.
F1 adds two fields to its `CompactionInput`. Both are set by ContextPolicy, the producer, so the
host does not decide them:

- `cut: String`: the history entry up to which the host summarizes (§5.2).
- `trigger: CompactionTrigger`: `Threshold`, `ProviderOutputBudgetExhausted` or `Manual`.

### 4.3 Events

| Event | Writer | Consumer | Data |
| --- | --- | --- | --- |
| `context/compact` (new) | Loop, through its public write operation `AgentLoop::compact` | Loop, in sequence order | `{}` |
| `context/prepare` (field added) | Loop | ContextPreparation | Existing data, plus optional `compact: "before_model" \| "only"`, omitted when absent |

Notes on the new event and field:

- `context/compact` is a command to the owner of progression (principles §4 rule 1). Its
  results are the facts the Loop and ContextPreparation write in response (§5.3). It is not
  client-visible: the exhaustive `ProductEvent` classification in `tree_protocol.rs` lists it
  with the other internal events.
- `compact: "before_model"`: compact, then hand the compacted request to ModelProvider
  (`model/call`).
- `compact: "only"`: compact and stop. The committed `context/compaction` fact is the request's
  terminal.

### 4.4 Rust binding

```rust
// dim-agent-types/src/events.rs (additive)
EventKind::ContextCompact(ContextCompactData {})         // "context/compact"
pub struct ContextPrepareData { /* existing */ pub compact: Option<CompactStep> }
pub enum CompactStep { BeforeModel, Only }               // "before_model" | "only"

// dim-agent-capabilities/src/host.rs (existing port, same crate)
pub struct CompactionInput { /* existing */ pub cut: String, pub trigger: CompactionTrigger }

// dim-agent-capabilities/src/agent_loop.rs (supplement)
impl AgentLoop {
    /// Appends context/compact; resolves once the Loop has served it.
    pub async fn compact(&self, cancel: CancellationToken) -> Result<CompactionServed>;
}
pub enum CompactionServed { Completed { sequence: u64 }, Skipped { reason: String },
                            Failed { error: String }, Cancelled { reason: String } }
impl PublicService for AgentLoop {}                       // lib.rs

// dim-agent-capabilities/src/context_preparation.rs (supplement)
impl ContextPreparation {
    pub async fn usage(&self, cancel: CancellationToken) -> Result<ContextUsage, ContextOpError>;
}
impl PublicService for ContextPreparation {}             // lib.rs

// dim-agent-capabilities/src/context_model.rs (new)
pub struct ProductContextModelPlugin { scope: SessionScope }
//   Deps = (Arc<ModelHost>, Arc<ProviderConnectionResolver>)  -- process scope
//   provides scope.key::<ContextModelHost>() = ContextModelHost(Some(ProductContextResolver))
// dim-agent-capabilities/src/context_methods.rs (new)
pub struct ContextMethodsPlugin { scope: SessionScope }
//   Deps = InSession<TreeCommands>; SessionMethods::provide(ctx, &scope, "session.context", ..)

// dim-app-server/src/main.rs, product profile only (no --models file):
builder.session_plugin(ProductContextModelPlugin::new)
       .session_plugin(ContextMethodsPlugin::new)
       .session_methods("session.context");
// replaces .session_value(Arc::new(ContextModelHost(None))) there;
// the --models and mock profiles keep ContextModelHost(None)
```

`SessionMethods` is C1's type in `dim-agent-capabilities::methods`.

## 5. Flows

### 5.1 Production host

`ProductContextResolver::resolve(request)`:

1. It requires `request.capability_snapshot`, valid for the request's model. Without it, it
   returns `InvalidRequest` "Model capabilities are not adopted; select the model again". The
   condition and text are those of `ProductResolver`, which refuses such a request anyway.
2. It returns a `ContextModel` bound to the request:

| Method | Value |
| --- | --- |
| `context_window` | `snapshot.capabilities.contextWindow`, the catalog value with the catalog's existing overrides |
| `output_tokens` | `request.config.maxTokens` if set, else `snapshot.capabilities.maxOutputTokens`; never above the latter |
| `threshold_tokens` | `0.8 × (context_window − output_tokens)`, provisional (#128) |
| `count_tokens` | Local estimate: the UTF-8 bytes of the serialized system prompt, messages and tool declarations divided by 4; each image part counts a fixed 1,600 tokens instead of its encoded bytes. Provisional (#128). The policy uses it only for deltas over the provider's reported `prompt_tokens`, the first request, `validate`, and the reduction check |
| `compact` | Below |

`compact(input, cancel)` summarizes `input.checkpoint`'s summary plus the history entries up to
and including `input.cut`, and returns `Completed {cursor: input.cut, summary, diagnostics}`:

- **Request.** One summarizer request per chunk. It uses the request's provider, model, snapshot
  and config, with `maxTokens = output_tokens`, a fixed summarizer system prompt owned by this
  module, one user message containing the transcript, and no tools.
- **Credentials and failures.** `ProviderConnectionResolver.resolve` returns a request-local
  connection. The call runs through `invoke_with_recovery`
  (`dim-agent-capabilities::model_provider`), the failure and credential policy ModelProvider
  uses. S4 extracts that function; if F1 lands first, F1 does the same extraction.
- **Output.** Thinking output is dropped. A tool stop or an empty text is a failure.
- **Transcript.** Each entry is role-labelled text: tool calls as name and input, tool results as
  their text, images as `[image]`.
- **Chunking.** An entry larger than one call's input budget (window − output − prompt) is
  truncated in the middle and counted in `compactedToolPayloadCount`. A transcript that does not
  fit one call is folded in chunks: each call gets the running summary and the next chunk.
- **Diagnostics.** The required `CompactionDiagnosticsRecord` fields come from the run
  (`reason` and `trigger` from `input.trigger`, `coverage: full`, counts, estimates,
  `durationMs`). Optional fields the host does not compute are omitted.

### 5.2 One compaction path (ContextPolicy)

ContextPolicy's existing `prepare` changes in two places. The rest stays as it is today:
estimate, `context/budget`, checkpoint reuse for a retry, validating that the compacted request
is smaller and below the window, committing `context/compaction`, and the optional snapshot.

**When it compacts.** Compaction is forced when `source.recovery` or `source.compact` is
present. Otherwise it runs once per boundary when the estimate reaches the threshold, as today.
The trigger is `ProviderOutputBudgetExhausted` if `recovery` is present, else `Manual` if
`compact` is present, else `Threshold`.

**The cut.** ContextPolicy, not the host, chooses the cut, so both triggers follow one rule:

- The retained turns are the last K turns on the branch, the running turn included. A turn runs
  from `turn/start` to `turn/end`.
- An entry belongs to a turn by the sequence of its source event in the fixed context. Entries
  restored from a snapshot's state predate the snapshot's anchoring compaction, so they count as
  older than any retained turn.
- The cut is the latest legal cut before the first entry of the oldest retained turn. A legal
  cut is one that passes the existing check: no retained tool result for a removed call, and the
  next entry is not tool context.
- If no such cut exists: under `recovery` the cut falls back to the latest legal cut, so the
  request can still fit; otherwise the outcome is `Skipped {reason: "nothing_to_compact"}`
  without a model call.
- The summary range is therefore from the previous compaction point to the cut, with the
  previous summary. The K turns after the cut stay verbatim, and keyed baseline blocks are
  replayed from the cut's authoritative baseline as today.

**Compaction-only requests.** For `compact: "only"`, the committed `context/compaction` fact
(`Completed`, `Skipped` or `Failed`) is the request's result. ContextPreparation writes no
`model/call` (§5.3).

### 5.3 Manual compaction

The Loop serves a pending `context/compact` with the next request it prepares. If it is idle
and no input drives a turn, it starts a compaction-only turn.

**Idle branch:**

```
 seq  event                                              writer
 n    context/compact {}                                 Loop (AgentLoop::compact, from
                                                           session.context.compact)
      -- Loop consumes n: idle, no driving input -> compaction-only turn
 n+1  turn/start {T}                                     Loop
      -- Loop consumes n+1: no input/consumed, no step/model-start
 n+2  context/prepare {T, R, through: n+1, request,      Loop (handoff to ContextPreparation)
                       compact: "only"}
 n+3  context/budget {T, R, ...}                         ContextPreparation
 n+4  context/compaction {T, R, through: n+1, outcome}   ContextPreparation  <- terminal of R
      -- Loop consumes n+4: pending cleared, served = outcome
 n+5  turn/end {T, completed "compacted"                 Loop
                 | completed "compaction_skipped"
                 | failed <error>}
      -- driving inputs applied since n+1 start the next turn (try_start)
```

If preparation fails or is cancelled before a fact is committed, ContextPreparation writes the
existing terminal `step/model-output-end {T, R, Failed | Cancelled}` instead of n+3/n+4. The
Loop then writes `turn/end` with `failed` or `cancelled`.

**During a turn:**

```
      step/model-start {T, R1} ... step/model-output-end {T, R1, completed}
 k    context/compact {}                                 -- Loop: pending = k
      step/tool-call {T, R1} ... step/tool-result {T, R1}
      input/consumed* ; step/model-start {T, R2}         Loop (unchanged)
      context/prepare {T, R2, through, request,          Loop: pending at write time
                       compact: "before_model"}
      context/budget {T, R2}                             ContextPreparation
      context/compaction {T, R2, outcome}                ContextPreparation
      -- Loop consumes it: pending cleared, served = outcome
      model/call {T, R2}  (on the compacted request)     ContextPreparation -> ModelProvider
```

If the turn ends before another request is prepared, the pending mark is still set when the
Loop goes idle, and it is served by a compaction-only turn as above. A failed manual
compaction in a `before_model` request continues uncompacted, like an automatic one.

**Reply.** `AgentLoop::compact` appends `context/compact` and waits until the Loop's served
state covers that sequence. ContextMethods maps the outcome:

- `Completed` → `{status: "completed", sequence}`
- `Skipped` → `{status: "skipped", reason}`
- `Failed` → `compaction_failed`
- `Cancelled` → `compaction_cancelled`

### 5.4 What the Loop gains

All of it is in `crates/dim-agent-capabilities/src/agent_loop.rs`.

| Area | Change |
| --- | --- |
| Subscription | Adds `context/compact` and `context/compaction` to the existing list |
| Public write | `AgentLoop::compact(cancel)` appends `context/compact` and waits on the served state; `PublicService` so ContextMethods reaches it by branch |
| State | `compaction: Option<u64>`, the pending command; `served: StateCell<..>`; `Stage::Compacting`; `Turn.compaction_only` until the turn's request exists |
| Consuming `context/compact` | Sets pending (a later command replaces the sequence, so several are served together). When idle, calls `try_start` |
| `try_start` | Starts a turn when an input drives one (unchanged) or when compaction is pending. Without a driving input the turn is compaction-only |
| Consuming `turn/start` | For a compaction-only turn, starts the compaction step instead of `next_model`: no `input/consumed`, no `step/model-start`, request stage `Compacting`, and `start_preparation(through = turn/start sequence)` |
| `start_preparation` | Sets `compact: "only"` for a compaction-only request. When compaction is pending at the moment `context/prepare` is written (under the turn lock, where the steer and interrupt checks already happen), sets `compact: "before_model"`. A budget retry copies the source and clears `compact`, since the fact already served it |
| Consuming `context/compaction` | For the request whose `context/prepare` carried `compact`: clears pending and sets served to `Completed {sequence}`, `Skipped` or `Failed`. If the request is `Compacting`: releases it and writes `turn/end`: `completed "compacted"`, `completed "compaction_skipped"`, or `failed` with the fact's error |
| `step/model-output-end` of a `Compacting` request | `Failed` or `Cancelled` ends the turn with the same outcome (no budget recovery) and clears pending, with served `Failed` or `Cancelled` |
| Other terminals of a request that carried `compact` | Any terminal before its fact, except `replaced by steer`, clears pending and serves `Failed` or `Cancelled`. After a steer replacement the mark stays and the next request carries it |
| `steer` | Skips a `Compacting` request: driving inputs stay pending and start the next turn after `turn/end`. The Loop does not advance during compaction |
| Consuming `stop/interrupt` | Clears pending and serves `Cancelled`. A running compaction-only preparation is cancelled by the existing cancel path |
| `rebuild` | Folds `context/compact`, the requests' `compact` fields, `context/compaction` and the clearing terminals into pending and served. `context/prepare {compact: "only"}` rebuilds a `Compacting` request. An open compaction-only turn is marked failed like any rebuilt turn: ContextPreparation settles its open preparation with the reloading terminal, and the Loop ends the turn. A compaction-only turn without its `context/prepare` has nothing outstanding and ends at once |

Not changed: turn and request ids, input consumption, tool batches, budget recovery for model
requests, `LoopSnapshot`. A compaction-only turn is published as `Running`, so inputs that
arrive queue exactly as they do during any turn.

### 5.5 What ContextPreparation gains

| Area | Change |
| --- | --- |
| `context/prepare {compact}` | Passes the source to ContextPolicy (§5.2). For `"only"`: on success no `model/call` is written, because the policy's committed fact is the terminal; `Failed` and `Skipped` outcomes are facts, not errors. Errors before a fact are written as today (`step/model-output-end`) |
| Steer | Unchanged rule. A compaction-only request has no `step/model-start`, so its steer mark never matches and a steer never cancels it |
| `rebuild` | An open `context/prepare {compact: "only"}` is also settled by its own `context/compaction` |
| `usage(cancel)` | Reads the committed head and builds the request as Loop does from the latest `context/configured` settings and system with the projected messages, without binding tools. It resolves the `ContextModel` and runs the existing estimate. Writes nothing; may run during a turn |

## 6. Errors

`MethodError` codes (`code -32000` unless noted):

| Code | Method | When | Retry |
| --- | --- | --- | --- |
| `-32602` | both | `branchId` missing or not a string | No |
| `session_invalid_input` | both | Unknown branch (existing mapping of `AgentError::UnknownBranch`) | No |
| `session_closed` | both | Session or branch closing | No |
| `context_not_configured` | usage | No `context/configured` in the branch's history | After a model is adopted |
| `context_model_failed` | usage | `resolve` failed (no capability snapshot) | After selecting the model again |
| `compaction_failed` | compact | Served `Failed`: a host error after the shared failure policy, an empty summary, an invalid cut, no reduction below the window, or a preparation failure before compaction; message is the error text | User's choice |
| `compaction_cancelled` | compact | Served `Cancelled`: `stop/interrupt`, or the carrying request was cancelled | User's choice |
| `operation_interrupted` | compact | Loop or ContextMethods unloads while the call waits (session-boundary A.1, `report`); the command stays in the Timeline and is folded on reload | User's choice |

Automatic compaction errors are unchanged. A host error becomes a `Failed` outcome in the fact
and the request continues uncompacted; under recovery it fails the request.

## 7. Security

- **Credentials stay request-local.** The connection comes from `ProviderConnectionResolver`
  inside one `compact` call and goes to `ModelHost`, as for turns. It is never stored, returned,
  serialized or logged. Provider is unchanged.
- **No new data egress.** The summarizer sends the Session's own history to the same provider and
  model that receive it on every turn.
- **The summary is untrusted model output.** It enters context only through the existing
  `summary_message`, which marks it as data and escapes it.
- **No secrets in results.** Facts, replies and diagnostics carry counts, ids and `HostError`
  text. That text carries no credential values. No request or summary content goes to tracing.
- **Authorization.** Rpc authorizes `sessionId` for Session methods (C1). A connection served
  Session methods may compact, as it may submit input or stop.

## 8. Relationship to existing parts

| Part | Relationship |
| --- | --- |
| Loop | Supplement (§5.4): command, pending mark, compaction-only turn. Budget recovery becomes active in the product profile because `ContextModelHost` is `Some` |
| ContextPreparation, ContextPolicy | Supplement (§5.2, §5.5): one compaction path for all triggers, the K-turn cut, compaction-only requests, `usage` |
| ModelProvider | Unchanged code. `validate` now enforces the window estimate in the product profile; it never receives a compaction-only request |
| Provider, ProductResolver, Aimux / ModelHost | Unchanged; used only through `ProviderConnectionResolver` and `ModelHost` |
| AuxiliaryModel (S4) | Not used. Shares `invoke_with_recovery`; whichever of S4 and F1 lands first extracts it |
| `sessionMethods` (C1) | Required for the two methods. The host part does not need C1 and can land first |
| Steer (ADR agent-029) | Unchanged for model requests. Compaction-only requests are not steerable |
| Turn consumers (clients, future Goal, SessionTitle) | See a compaction-only turn as an ordinary turn with its own outcome reason (§10) |
| `tree_protocol.rs` `ProductEvent` | Classifies `context/compact` as not client-visible (exhaustive match) |
| HistoryRenderer, snapshots, native storage | Projection unchanged; manual facts are projected and snapshotted like automatic ones. The new event kind and field need a format decision (§10) |
| `--models` profile, mock profile | Keep `ContextModelHost(None)`: no compaction, and `session.context` is not recorded there |
| ManagedProviders plan usage, CLI `/compact` and `/usage` | Not part of F1's acceptance; the CLI calls the two methods |

## 9. Rule check

| Rule | Answer |
| --- | --- |
| Principles §1 (Capability, App Server boundary) | Two Session Capabilities and two supplements in `dim-agent-capabilities`; the App Server only installs the parts in its product profile. |
| §1 one event bus | Manual compaction is a command on the branch Timeline, consumed by the branch Dispatcher. No second bus and no side channel; the method's reply reads the Loop's `StateCell`. |
| §3 dependency direction | New parts depend on core services (`ModelHost`, `ProviderConnectionResolver`, `TreeCommands`, the Loop, ContextPreparation). Nothing existing depends on them except through the existing `ContextModelHost` key, which is always provided where the part is installed. No optional dependency. |
| §3 changing another module (Loop) | Approved change. The need is the Loop's own context management: compaction must be a step of the Loop's progression so the Loop does not advance on a context the summary replaces. It closes within the Loop: automatic compaction already lives in the Loop's preparation step, and manual compaction is served by the same step, either in the next request or in a compaction-only turn. No other module changes shape for it. |
| §4 rule 1 (command events) | The client asks the owner of progression with `context/compact`. The Loop decides when to serve it, and the result facts are written by their owners: Loop (`turn/*`, `context/prepare`) and ContextPreparation (`context/budget`, `context/compaction`). A command that is cleared by an interrupt writes no fact; the caller's reply says so. |
| §4 rule 2 (handoff) | The compaction request is handed to ContextPreparation with `context/prepare`, the existing handoff. Afterwards the Loop uses only `stop/interrupt`. |
| §4 rule 3 (before callback; handlers) | No before callback. Loop handlers register the preparation task and return, as today. |
| §4 rule 4 (no business meaning in core) | Timeline, Dispatcher and runtime facades unchanged. |
| §4 rule 5 (atomic batch) | Each new write is one event; `turn/end` keeps its existing batch. |
| §4 rule 6 (context only from handed-off events) | The summary reaches the model only through the existing projection of the completed fact. Pending inputs are not consumed by a compaction-only step, so they never enter the summarized range (AGENTS rule 10). |
| §4 rule 7 (public events and services) | `TreeCommands`, `Branch::service` on `PublicService`s (`AgentLoop`, `ContextPreparation`), `ProviderConnectionResolver`, `ModelHost`, `sessionMethods`. |
| §4 rule 8 (infrastructure unchanged) | Timeline, Dispatcher, envelope, rutis, loader, Agent assembly and facades are unchanged. The Loop is a capability, not listed infrastructure. A new event kind is plugin content in the existing envelope. No requirement on rutis. |
| §4 rule 9 (only needed facts) | `context/compact` is needed: the Loop's later decisions read it, and it is folded on reload. `context/compaction` is used by projection. Usage writes nothing. |
| ADR 002 D1 (scope) | Host and methods: Session. Command, pending mark and compaction step: branch, in the owner of progression. Cut and facts: branch, in the owner of compaction facts. |
| ADR 002 D2 (no wiring modules) | ContextMethods exposes owners' operations through the sanctioned `sessionMethods`; ProductContextModel is the feature's host. |
| ADR 002 D3 (dependencies) | Hard dependencies on core only. A Provider or Aimux reload propagates to these parts as it does to ModelProvider (accepted cost). |
| ADR 002 D4 (configuration) | No configuration; provisional constants are code (#128). |
| ADR 002 D5 (language-neutral contracts) | §4.1 and §4.3 are the contracts; §4.4 is one binding. |

**Impact (public content added):**

- The `session.context` namespace (`compact`, `usage`): `/compact` and `/usage` need it.
- The command event `context/compact`: manual compaction must enter the Timeline and be consumed
  by the Loop.
- The optional field `context/prepare.compact`: the Loop must tell ContextPreparation to compact
  and whether to stop there.

Internal Rust changes:

- `CompactionInput.cut` and `CompactionInput.trigger`: the producer chooses them.
- `PublicService` on `AgentLoop` and `ContextPreparation`: the Session part reaches them by
  branch.

**Requirements raised to mechanism owners:** none.

## 10. Decisions and open questions

Decided:

| Item | Decision |
| --- | --- |
| Which model compacts | The request's own model; a separate compaction model is deferred |
| Trigger and retention | `0.8 × (window − output)`, K = 2 turns, `bytes / 4`, 1,600 tokens per image: provisional, studied in #128 |
| Manual compaction | A Loop command served in the Loop's progression with real turn and request ids (§5.3) |
| `--models` profile | No capability snapshot, so no compaction: accepted |
| Provider-reported context overflow | Aimux reports it without budget facts; mapping it is a separate Aimux issue, outside F1 |

Open (raised by the Loop change):

1. **Format decision.** `context/compact` is a new event kind, and `context/prepare.compact` is
   a new optional field on a `deny_unknown_fields` type. Both are additive, but an older binary
   cannot read a branch that contains them. Does the native storage format need a version step,
   or is an additive event kind recorded as compatible (as S1 must decide for
   `input/withdraw`)?
2. **Compaction-only turn as a turn.** It is published with `turn/start` and `turn/end`
   (`completed "compacted"`), so clients and future turn consumers (Goal continuation on
   `turn/end`, Hooks turn-end hooks) see it. Acceptable, or should consumers skip turns whose
   outcome reason is `compacted`?
3. **Steer during a forced compaction.** In a `before_model` request, an applied steer input
   still cancels the preparation, including a compaction in progress, as it does for automatic
   compaction today. The next request carries the pending mark and compacts again on the newer
   history. Keep this, or make any preparation that is compacting non-steerable?

## 11. Acceptance (flow tests)

The tests run a product-profile App Server against a mock HTTP provider whose catalog entry has
a small context window (`crates/dim-app-server/tests`), plus Loop flow tests in
`crates/dim-agent-capabilities/tests`.

1. **Automatic.** A conversation grows past the threshold.
   - The next preparation writes `context/budget` and sends one summarizer request: summarizer
     system prompt, one user message, no tools, the Session's model.
   - It commits `context/compaction` `completed` with `trigger: threshold`, and the cursor is
     the legal cut before the last 2 turns.
   - The model request starts with the summary, keeps the last 2 turns verbatim, and drops the
     summarized messages.
2. **Budget recovery.** A request whose estimate reaches the window ends with budget facts. The
   Loop retries once, and the retry compacts with `trigger: provider_output_budget_exhausted`
   (falling back to the latest legal cut when the last 2 turns are all there is).
3. **Manual, idle.** `session.context.compact` produces exactly the sequence of §5.3:
   - `context/compact`, `turn/start`, `context/prepare {compact: "only"}`, `context/budget`,
     `context/compaction`, `turn/end completed "compacted"`.
   - No `step/model-start`, no `model/call` and no model request other than the summarizer.
   - The reply is `completed` with the fact's sequence, and the fact's ids are that turn's.
   - An input submitted during the compaction starts a new turn after `turn/end`, and its
     request starts with the summary.
4. **Manual, during a turn.** The command is written while a tool batch runs. The next request's
   `context/prepare` carries `compact: "before_model"`, its `context/compaction` precedes its
   `model/call`, and the reply arrives after the fact.
5. **Interrupt and reload.**
   - `session.stop` during a compaction-only turn ends it `cancelled`, writes no compaction
     fact, and replies `compaction_cancelled`.
   - Restarting the App Server during one ends the turn `failed` with the reloading reason and
     does not repeat the summarizer request.
6. **Usage.** `session.context.usage` returns the catalog window, output budget, threshold and an
   estimate. The estimate drops after a compaction, and the branch Timeline has no new event
   after the call.
7. **Failure and credentials.**
   - When the provider fails the summarizer, the automatic path commits `failed` and continues
     uncompacted, and manual replies `compaction_failed`.
   - No API key appears in facts, replies or diagnostics.
