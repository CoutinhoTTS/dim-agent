# ContextCompaction

Status: Draft (for review). Roadmap item F1 ([#72](https://github.com/arcships/dim/issues/72)).
Overview: [Appendix B.1 ContextPreparation row](../overview.md#b1-session-and-branch-capabilities).
Scope: [ADR 002](../../adr/002-plugin-scope.md) (two Session parts and a supplement to the
existing branch capability ContextPreparation). Nothing in this document is implemented.

## 1. Summary

Automatic compaction already exists: on every `context/prepare`, ContextPreparation's
ContextPolicy estimates the input, writes `context/budget`, and, above the threshold or on a
budget recovery, calls `ContextModel::compact` and commits `context/compaction`
([context_policy.rs](../../../crates/dim-agent-capabilities/src/context_policy.rs)). It never
runs in production because the App Server binds `ContextModelHost(None)`
([main.rs](../../../crates/dim-app-server/src/main.rs)), so ContextPreparation has no policy
and Loop has no budget recovery.

F1 adds three things and nothing else:

1. **A production host.** A Session part, `ContextModel`, provides
   `ContextModelHost(Some(..))`. It implements the existing port
   `ResolveContextModel` → `ContextModel`
   ([host.rs](../../../crates/dim-agent-capabilities/src/host.rs)) from two existing process
   services: `ProviderConnectionResolver` (connection and credentials) and `ModelHost` (Aimux,
   execution). The model that compacts is the request's own model; its context window and
   output budget come from the request's adopted capability snapshot; token counting is a
   local estimate.
2. **Manual compaction.** `session.context.compact` runs the same compaction path on an idle
   branch and commits the same `context/compaction` fact.
3. **Usage query.** `session.context.usage` returns the budget the policy would compute now:
   context window, output budget, threshold and estimated input. It writes nothing.

Both methods are exposed through `sessionMethods` (namespace `session.context`) by a second
Session part, `ContextMethods`, which reaches the branch's ContextPreparation per call.

No new event, no new configuration, no new process plugin, no change to Provider, Aimux,
Timeline, Dispatcher, the event format or rutis.

### 1.1 Why it is this small

- **Not AuxiliaryModel.** S4's `auxiliaryModel.complete` returns text only. Compaction needs
  the context window, the output budget and token counts of the model whose context is being
  compacted, which that call does not carry ([auxiliary-model §1.1](https://github.com/arcships/dim/blob/roadmap/s4-provider-purpose-policy-and-auxiliarymod/docs/design/appserver/auxiliary-model.md)).
- **Not inside Provider.** Everything the host needs is already reachable through the generic
  ports in `dim-agent-capabilities::host`: the capability snapshot travels on the
  `ModelRequest`, credentials are resolved by `ProviderConnectionResolver` (Provider's
  `ProductResolver` in the product profile) exactly as ModelProvider does for every turn, and
  execution is `ModelHost`. Putting summarization and token estimation in Provider would give
  the owner of catalogs and connections context-policy logic it has no other use for.
- **No compaction-model setting.** The Session's adopted model is used. A separate model is a
  later business setting of ContextPreparation (§10), not part of the done criterion.
- **No new fact.** Manual compaction commits the existing `context/compaction` fact; the
  existing projection, checkpoint selection and snapshots apply to it unchanged.

## 2. Structure

```
 Process scope (existing, unchanged)
 +-------------------------------------------------------------------+
 |  Aimux: ModelHost                                                 |
 |  Provider: ProviderConnectionResolver (ProductResolver)           |
 +-----------------^-------------------------------^-----------------+
                   | inject (hard)                 | inject (hard)
 Session scope     |                               |
 +-----------------+-------------------------------+-----------------+
 |  ContextModel (new part)                                          |
 |    provides S:ContextModelHost(Some(ProductContext))              |
 |                                                                   |
 |  ContextMethods (new part)                                        |
 |    provides sessionMethods["session.context"]: compact, usage     |
 |    per call: TreeCommands.get(branchId).service::<ContextPreparation>()
 +---------------------------------^---------------------------------+
        Rpc (C1) dispatches -------'          | S:ContextModelHost
 Branch scope                                 v (existing injection)
 +-------------------------------------------------------------------+
 |  ContextPreparation (existing, supplement)                        |
 |    automatic path: ContextPolicy.prepare (unchanged)              |
 |    new: compact(), usage(); turn mark folded from turn/start,end  |
 |    writes context/budget (auto only), context/compaction          |
 |  Loop (existing): budget recovery active because the host is Some |
 +-------------------------------------------------------------------+
```

The two Session parts are separate because their dependencies differ: branches inject
`ContextModelHost`, so its provider must not depend on `TreeCommands`, which the branch tree
provides. Both live in `dim-agent-capabilities` next to ContextPreparation. All dependencies
point from the new parts to existing core services; no existing part names them.

## 3. Data and state

| Item | Owner | Kind |
| --- | --- | --- |
| `context/compaction` with `reason: manual` | ContextPreparation | Existing fact, existing format |
| Turn mark: the turn id between a consumed `turn/start` and its `turn/end` | ContextPreparation | Private memory state, folded from consumed events, rebuilt on load (it already folds these events for its interrupt mark) |
| Running manual compaction (cancellation token) | ContextPreparation | I/O state of its own task; nothing to settle on reload because no event opens it |

The hosts hold no state. `ProductContext` holds `Arc<ModelHost>` and
`Arc<ProviderConnectionResolver>`; the `ContextModel` it returns for one request holds that
request. A resolved connection lives only inside one `compact` call.

## 4. Interfaces

### 4.1 Language-neutral (ADR 002 D5)

`sessionMethods`, Session scope, key `session.context`, provided by ContextMethods
([session-boundary A.1](../contracts/session-boundary.md#a1-method-contributions)). Both
methods have `interruption: report`, `progress: false`, `notifications: []`, `reply: direct`.

| Method | Mutating | Request | Reply |
| --- | --- | --- | --- |
| `session.context.compact` | yes | `{sessionId, branchId}` | `{status: "completed", sequence, estimatedInputTokens, compactedInputTokens}` or `{status: "skipped", reason: "nothing_to_compact"}` |
| `session.context.usage` | no | `{sessionId, branchId}` | `{through, contextWindow, outputTokens, thresholdTokens, estimatedInputTokens, baselineSource: "provider_baseline" \| "full_estimate"}` |

- `sequence` is the committed `context/compaction` event. `estimatedInputTokens` is the
  policy's estimate before compaction; `compactedInputTokens` is `count_tokens` of the
  compacted request the policy validated.
- `through` is the committed head the usage was computed at. The numbers are those a
  preparation at that head would write into `context/budget`, plus `thresholdTokens`.
- Cancellation: per session-boundary A.1 (unload interrupts, a caller's disconnect cancels
  nothing). A manual compaction is also cancelled by the branch's own facts (§5.3).
- Errors are `MethodError` data (§6).

The `ContextModel` port is an existing runtime host port inside the Rust runtime, not a plugin
contract; it stays a Rust trait like the other typed host ports (ADR 002 D5: exposing them by
name is #4).

### 4.2 Rust binding

```rust
// dim-agent-capabilities/src/context_model.rs (new)
pub struct ContextModelPlugin { scope: SessionScope }
impl TypedPlugin for ContextModelPlugin {
    type Deps = (Arc<ModelHost>, Arc<ProviderConnectionResolver>);   // process scope
    // apply: ctx.provide_as(scope.key::<ContextModelHost>(),
    //          Arc::new(ContextModelHost(Some(Arc::new(ProductContext { host, resolver })))))
}
struct ProductContext { host: Arc<ModelHost>, resolver: Arc<ProviderConnectionResolver> }
impl ResolveContextModel for ProductContext { /* §5.1 */ }

// dim-agent-capabilities/src/context_methods.rs (new)
pub struct ContextMethodsPlugin { scope: SessionScope }
impl TypedPlugin for ContextMethodsPlugin {
    type Deps = InSession<TreeCommands>;
    // apply: SessionMethods::provide(ctx, &scope, "session.context", decls, handler)
}

// dim-agent-capabilities/src/host.rs (change to an existing port, same crate)
pub struct CompactionInput { /* existing fields */ pub trigger: CompactionTrigger }

// dim-agent-capabilities/src/context_preparation.rs (supplement)
impl ContextPreparation {
    pub async fn compact(&self, cancel: CancellationToken) -> Result<CompactOutcome, ContextOpError>;
    pub async fn usage(&self, cancel: CancellationToken) -> Result<ContextUsage, ContextOpError>;
}
impl PublicService for ContextPreparation {}   // lib.rs, like Approval and Permission

// dim-app-server/src/main.rs, product profile only (no --models file):
builder
    .session_plugin(ContextModelPlugin::new)
    .session_plugin(ContextMethodsPlugin::new)
    .session_methods("session.context");
// replaces .session_value(Arc::new(ContextModelHost(None))) there; the --models and
// mock profiles keep ContextModelHost(None)
```

`SessionMethods`, `MethodDecl` and `Handler` are C1's types in `dim-agent-capabilities::methods`.
`CompactionInput.trigger` is set by ContextPolicy (`Threshold`, `ProviderOutputBudgetExhausted`
when `provider_budget` is present, `Manual`), so the host fills the diagnostics' `reason` and
`trigger` from its input instead of ContextPreparation rewriting the host's result.

## 5. Flows

### 5.1 Production host

`ProductContext::resolve(request)`:

1. `request.capability_snapshot` must be present and valid for the request's model, else
   `InvalidRequest` "Model capabilities are not adopted; select the model again" (the same
   condition and text as `ProductResolver`, which refuses such a request anyway).
2. Returns a `ContextModel` bound to the request:

| Method | Value |
| --- | --- |
| `context_window` | `snapshot.capabilities.contextWindow` (catalog value, with the catalog's existing overrides) |
| `output_tokens` | `request.config.maxTokens` if set, else `snapshot.capabilities.maxOutputTokens`, capped by the latter |
| `threshold_tokens` | `0.8 × (context_window − output_tokens)` (value: §10) |
| `count_tokens` | Local estimate: UTF-8 bytes of the serialized system prompt, messages and tool declarations divided by 4, with each image part counted as a fixed 1,600 tokens instead of its encoded bytes (values: §10). The policy uses it only for deltas over the provider's reported `prompt_tokens`, for the first request, for `validate`, and to check that a compaction reduces input |
| `compact` | Below |

`compact(input, cancel)`:

```
 ContextPolicy         ContextModel (host)        ProviderConnectionResolver   ModelHost (Aimux)
   | compact(input) ------>|                                 |                       |
   |                       | cursor = last valid cut of input.history (same rule as  |
   |                       |   ContextPolicy's cut check); none -> Skipped            |
   |                       | chunks = prior summary + entries[..=cursor] as text,    |
   |                       |   each call within window - output - prompt             |
   |                       | for each chunk:                                         |
   |                       |   request = {same provider/model/snapshot/config,       |
   |                       |     maxTokens = output_tokens, system = summarizer      |
   |                       |     prompt, one user message, no tools}                 |
   |                       |   resolve(request) ----------->|                       |
   |                       |   <-- ResolvedModelConnection (request-local)           |
   |                       |   invoke_with_recovery(...) ------------------------->|
   |                       |   <-------------------------------- text, MessageStop |
   |                       |   running summary = text                                |
   |<-- Completed {cursor, summary, diagnostics} | Skipped | Err(HostError)          |
```

- **Model.** The request's provider, model, capability snapshot and config. Thinking output is
  dropped; only text forms the summary. A tool stop or an empty text is a failure.
- **What is summarized.** Everything up to the last cut that ContextPolicy's cut check accepts
  (no tool result retained for a removed call, the next entry is not tool context), including
  the previous summary from `input.checkpoint`. Keyed baseline blocks are not summarized: the
  policy replays them from the cut's authoritative baseline as today.
- **Transcript.** Each entry is rendered as role-labelled text; tool calls as name and input,
  tool results as their text, images as `[image]`. An entry that alone exceeds a call's input
  budget is truncated in the middle with a marker and counted in
  `compactedToolPayloadCount`.
- **Chunking.** When the transcript does not fit one call, chunks are folded in order: each call
  gets the running summary and the next chunk. `chunkCount` and `modelCallCount` record it. This
  also covers budget recovery, where the history can exceed the window.
- **Summarizer prompt.** A fixed text owned by this module: keep the user's goals,
  constraints and latest request, decisions, files and commands touched, the current state and
  pending next steps; output only the summary.
- **Failure policy.** `invoke_with_recovery` (`dim-agent-capabilities::model_provider`), the
  function ModelProvider uses: transient backoff within the shared budget, one credential
  recovery, settlement of a terminal rejection, `forbids_retry`. S4 extracts it; if F1 lands
  first, F1 performs the same extraction.
- **Diagnostics.** The required `CompactionDiagnosticsRecord` fields are filled from the run
  (`reason`/`trigger` from `input.trigger`, `coverage: full`, counts, estimates, `durationMs`);
  optional fields the host does not compute are omitted.

ContextPolicy then does what it does today: validates the cut and that the compacted request
is smaller and below the window, commits `context/compaction`, and saves the optional snapshot.

### 5.2 Automatic compaction

Unchanged code path; it starts working because ContextPreparation now has a policy. With the
host bound, Loop's budget recovery is active too: when `validate` finds the estimate at or
above the window, the request ends with budget facts and Loop retries once with
`recovery`, which forces compaction (`trigger: provider_output_budget_exhausted`).

### 5.3 Manual compaction

```
 Client     Rpc (C1)     ContextMethods      ContextPreparation (branch)        Timeline
   | session.context.compact {sessionId, branchId}
   |---------->|---------->| TreeCommands.get(branchId)
   |           |           |   .service::<ContextPreparation>().compact(cancel)
   |           |           |------------------>| state lock: closed? turn open?
   |           |           |                   |   manual compaction running?
   |           |           |                   |   -> context_busy
   |           |           |                   | register task (tracked), mark running
   |           |           |                   | head = committed; context(head); render
   |           |           |                   | request = latest context/configured
   |           |           |                   |   settings + system + projected messages
   |           |           |                   |   (none -> context_not_configured)
   |           |           |                   | policy: resolve, estimate,
   |           |           |                   |   compact(trigger: Manual), validate
   |           |           |                   | Completed -> append context/compaction --->|
   |           |           |                   |   {turnId = requestId = op id,
   |           |           |                   |    through = head}; save snapshot
   |<---------------------------------------- {status, sequence, ...}
```

- The request is built from the fixed context the way Loop builds one: the latest
  `context/configured` settings (provider, model, snapshot, tools, config, metadata) and system,
  with the projected history (`context_messages`). No tools are bound because no model request
  with tools is made.
- ContextPolicy's compaction step (call `compact`, validate, append, snapshot) is shared by
  the automatic and manual paths; manual skips `context/budget` (it is not a preparation) and
  commits only a `Completed` outcome. A `Skipped` outcome is the `skipped` reply; a failure is
  the `compaction_failed` reply. Neither writes a fact: no later decision reads them (rule 9),
  and a command that has no effect writes no result (rule 1).
- The fact's `turnId` and `requestId` carry one fresh operation id (a UUID). No `turn/start`
  matches it: ContextPolicy's retry filter matches only its own turn, and the projection takes
  the latest completed compaction regardless of ids. Trace groups the event as its own
  operation.
- Cancellation, from the branch's own consumed facts: a `stop/interrupt` (as for a running
  preparation) or a `turn/start` consumed while it runs cancels it with `compaction_cancelled`.
  A compaction the host already returned is committed anyway, as in the automatic path. Unload
  of ContextPreparation or the method provider interrupts it (`operation_interrupted`);
  ContextPreparation's disposer waits for the task like its preparation tasks.

### 5.4 Usage query

`usage(cancel)` reads the committed head, builds the request as in §5.3, resolves the
`ContextModel` and runs ContextPolicy's existing estimate over the context's events (provider
baseline plus delta when the last `model/usage` matches the request prefix, else a full
estimate). It may run during a turn. It takes no lock, starts no task and writes nothing.

## 6. Errors

`MethodError` codes (`code -32000` unless noted):

| Code | Method | When | Retry |
| --- | --- | --- | --- |
| `-32602` | both | `branchId` missing or not a string | No |
| `session_invalid_input` | both | Unknown branch (existing mapping of `AgentError::UnknownBranch`) | No |
| `session_closed` | both | Session or branch closing | No |
| `context_not_configured` | both | No `context/configured` in the branch's history | After a model is adopted |
| `context_model_failed` | both | `resolve` failed (no capability snapshot) | After selecting the model again |
| `context_busy` | compact | A turn is open (a preparation runs only inside a turn) or another manual compaction runs | When idle |
| `compaction_failed` | compact | Host error after the shared failure policy, empty summary, invalid cut, or the result does not reduce input below the window; message is the `HostError` text | User's choice |
| `compaction_cancelled` | compact | `stop/interrupt` or `turn/start` consumed while running | User's choice |
| `operation_interrupted` | compact | ContextPreparation or ContextMethods unloads (session-boundary A.1, `report`) | User's choice |

Automatic compaction errors are unchanged: a host error becomes a `Failed` outcome in the fact
and the request continues uncompacted; in recovery it fails the request.

## 7. Security

- **Credentials stay request-local.** The host obtains `ResolvedModelConnection` from
  `ProviderConnectionResolver` inside one `compact` call and passes it to `ModelHost`, exactly
  as ModelProvider does for turns. It never stores, returns, serializes or logs it. Provider is
  unchanged and remains the only place credentials are read.
- **No new data egress.** The summarizer sends the Session's own history to the same provider
  and model that receive it on every turn.
- **The summary is untrusted model output.** It enters context only through the existing
  `summary_message`, which marks it as data and escapes it.
- **No secrets in results.** Facts, replies and diagnostics carry counts, ids and `HostError`
  messages, which carry no credential values; no request or summary content goes to tracing.
- **Authorization.** Rpc authorizes `sessionId` for Session methods (C1). Every connection served
  Session methods may compact, as it may submit input.

## 8. Relationship to existing parts

| Part | Relationship |
| --- | --- |
| ContextPolicy | Automatic logic unchanged; its compaction step is shared with the manual path; it sets `CompactionInput.trigger` |
| ContextPreparation | Supplement: `compact`, `usage`, the turn mark, `PublicService` marker. Preparation, steer, interrupt and reload handling unchanged |
| Loop | Unchanged code; budget recovery becomes active in the product profile because `ContextModelHost` is `Some` |
| ModelProvider | Unchanged code; `validate` now enforces the window estimate in the product profile (a request whose estimate reaches the window ends with budget facts before the network call, and Loop recovers) |
| Provider, ProductResolver | Unchanged; used only through `ProviderConnectionResolver` |
| Aimux / ModelHost | Unchanged |
| AuxiliaryModel (S4) | Not used. Shares `invoke_with_recovery`; whichever of S4 and F1 lands first extracts it |
| `sessionMethods` (C1) | Required for the two methods. The host part does not need C1 and can land first |
| HistoryRenderer, snapshots, native storage | Unchanged; the manual fact is projected and snapshotted like an automatic one |
| Trace | A manual compaction shows as its own operation id |
| `--models` profile (ConfiguredModels) | Keeps `ContextModelHost(None)`: its requests carry no capability snapshot, so there is no context window to compact against (§10) |
| Mock profile, tests | Keep `ContextModelHost(None)` or their own test hosts |
| ManagedProviders plan usage | Not part of F1; `/usage` may show it next to this reply when that supplement exists |
| CLI `/compact`, `/usage` | Call the two methods; rendering is CLI work |

## 9. Rule check

| Rule | Answer |
| --- | --- |
| Principles §1 (Capability, App Server boundary) | Two Session Capabilities in `dim-agent-capabilities`; the App Server only installs them in its product profile. |
| §1 one event bus | Uses the branch Timeline and Dispatcher only; no second bus, no new subscription mechanism. |
| §3 dependency direction | New parts depend on core services (`ModelHost`, `ProviderConnectionResolver`, `TreeCommands`, ContextPreparation); nothing existing depends on the new parts except through the existing `ContextModelHost` key, which is always provided in the profiles that install the part. No optional dependency. |
| §4 rule 1 (command events) | A client asks the owner through the owner's method, and the owner decides and writes its own fact, as session-boundary prescribes ("a method that changes branch state calls its owner, which writes"). No effect, no fact. |
| §4 rule 2 (handoff) | Not affected; no object changes owner. |
| §4 rule 3 (before callback) | Not used. |
| §4 rule 4 (no business meaning in core) | Timeline, Dispatcher and runtime facade unchanged. |
| §4 rule 5 (atomic batch) | The manual fact is a single event. |
| §4 rule 6 (context only from handed-off events) | The summary reaches the model only through the existing projection of the completed compaction fact; no new context path (AGENTS rule 10). |
| §4 rule 7 (public events and services) | `TreeCommands`, `Branch::service` on a `PublicService`, `ProviderConnectionResolver`, `ModelHost`, `sessionMethods`. |
| §4 rule 8 (infrastructure unchanged) | No change to Timeline, Dispatcher, event envelope or format, rutis, loader, Agent assembly or Session and branch facades. No requirement on rutis. |
| §4 rule 9 (only needed facts) | Only completed manual compactions are written, because later projections use them; usage writes nothing. |
| ADR 002 D1 (scope) | Host and methods: Session (shared by branches, no cross-Session state); operations and turn mark: branch, in the owner of compaction facts. |
| ADR 002 D2 (no wiring modules) | ContextMethods exposes the owner's operations through the sanctioned `sessionMethods`; ContextModel is the feature's host. Neither connects other plugins. |
| ADR 002 D3 (dependencies) | Hard dependencies on core only; a Provider or Aimux reload propagates to these parts like it does to ModelProvider today (accepted cost). |
| ADR 002 D4 (configuration) | No configuration added; the constants are code. |
| ADR 002 D5 (language-neutral contracts) | §4.1 is the contract of the methods; §4.2 is one binding. `ContextModelHost` is an existing typed runtime service, unchanged. |

**Impact (public content added):** the `session.context` namespace with two methods
(`compact`, `usage`), needed for `/compact` and `/usage`; no existing contract carries them.
Internal Rust changes: `CompactionInput.trigger` (an existing port of the same crate, so the
producer states the trigger) and `PublicService` on ContextPreparation (so the Session part
reaches it by branch).

**Requirements raised to mechanism owners:** none.

## 10. Open questions

1. **Constants.** Threshold `0.8 × (window − output)`, the estimate `bytes / 4` and 1,600
   tokens per image are proposals. The estimate undercounts some scripts and tokenizers; the
   provider baseline corrects it after the first request. Are these values acceptable, or
   should a provider tokenizer be used later?
2. **Retained suffix.** The proposal summarizes everything up to the last valid cut, including
   the latest user request (as the existing example host does). Keep the most recent user
   input verbatim instead, at the cost of a retained-budget rule?
3. **Manual fact ids.** A fresh UUID in `turnId` and `requestId` keeps the format unchanged. Is
   that acceptable, or should manual compaction run inside a turn, which would need Loop to
   start a turn without input?
4. **`--models` profile.** It has no capability snapshot and therefore no compaction. Accept, or
   give ConfiguredModels a context window (a configuration change outside F1)?
5. **Separate compaction model.** Deferred. If wanted, it is ContextPreparation's own business
   setting, resolved to a capability snapshot through the existing `ModelSelectionHost`.
6. **Provider-reported overflow.** Aimux reports a provider's context-length error without
   budget facts, so only the local estimate triggers recovery. Mapping provider overflow errors
   to budget facts is an Aimux question outside F1.

## 11. Acceptance (flow tests)

Product-profile App Server against a mock HTTP provider whose catalog entry has a small context
window (`crates/dim-app-server/tests`):

1. **Automatic.** A conversation grows past the threshold; the next preparation writes
   `context/budget`, sends one summarizer request (summarizer system prompt, one user message,
   no tools, the Session's model), commits `context/compaction` `completed` with
   `reason: auto, trigger: threshold`, and the turn's model request starts with the summary
   and no longer contains the summarized messages.
2. **Budget recovery.** A request whose estimate reaches the window ends with budget facts,
   Loop retries once, and the retry compacts with `trigger: provider_output_budget_exhausted`.
3. **Manual.** `session.context.compact` on an idle branch replies `completed` with the
   committed sequence; the fact has `reason: manual`; the next turn's request starts with the
   summary. While a turn runs the reply is `context_busy`; `session.stop` during a manual
   compaction replies `compaction_cancelled` and writes no fact.
4. **Usage.** `session.context.usage` returns the catalog window, output budget, threshold and
   an estimate; after a compaction the estimate drops; the branch Timeline has no new event
   after the call.
5. **Failure.** The provider fails the summarizer request: the automatic path commits a
   `failed` outcome and the turn continues uncompacted; manual replies `compaction_failed`
   with no fact.
6. **Credentials.** A managed account whose recovered credential is rejected again is settled
   once; no API key appears in facts, replies or diagnostics.
