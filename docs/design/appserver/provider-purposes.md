# Provider purpose policy and AuxiliaryModel

Status: Draft (for review). Roadmap item S4 ([#53](https://github.com/arcships/dim/issues/53)).
Overview: [Appendix B.2 Provider row](../overview.md#b2-app-server-process-plugins),
[§6.2 constraint 14](../overview.md#62-constraints). Scope:
[ADR 002](../../adr/002-plugin-scope.md) (process, inside the existing Provider plugin).

## 1. Summary

Provider gains two things and nothing else:

1. **Purpose policy.** A user can pin a model to a purpose key such as `session-title` or
   `compaction`. The policy is stored in Provider's own store and has one write path: Provider's
   typed write, exposed to clients as `provider.purposes` / `provider.purpose.set`.
2. **`auxiliaryModel` service.** One async method, `complete(purpose, inherit, request)`, that
   runs a single non-streaming model call outside any Session: it picks the model (policy entry,
   else the caller's `inherit` model), resolves the connection inside Provider, invokes the
   existing `ModelHost`, and returns the text. It writes nothing to any Timeline.

Consumers in S4's "done when": SessionTitle (P4, purpose `session-title`) and the production
compaction host (F1, purpose `compaction`). Collaboration (`collaboration.member`) and Modality
(`modality.<category>`) use the policy later through the same read and write; neither is part of
S4's acceptance. No new plugin, no new Timeline event, no infrastructure change.

## 2. Structure

```
 process scope
 +-------------------------------------------------------------------+
 |  Provider (existing plugin)                                       |
 |   ProviderService ----- provider_selections (SQLite, v13)         |
 |     selection/select     scope = global | workspace:<cwd>         |
 |     purpose/set_purpose  scope = purpose:<key>        (new rows)  |
 |   ProductResolver (connection + credentials, unchanged)           |
 |   AuxiliaryModel  --uses--> ProductResolver, ModelHost            |
 |   managementMethods "provider": + provider.purposes,              |
 |                                   provider.purpose.set            |
 +--------------------------^------------------^---------------------+
        Aimux: ModelHost ---'                  |
                                               | complete(purpose, inherit, request)
 Session scope                                 |
   SessionTitle (Deps: auxiliaryModel) --------+
   F1 compaction host (looks it up per call) --'
   Collaboration (later): ProviderService.purpose("collaboration.member")
```

Dependencies point to the core: Provider gains a hard dependency on Aimux's `ModelHost` (both
core, both always present together in the product profile). Provider names no consumer.

## 3. Data and state

**Purpose key.** Owned and documented by the consumer that uses it (constraint 14); Provider
treats it as opaque and keeps no registry of known keys. Grammar, checked at Provider's boundary
(management method, typed write, `complete`): one to four dot-separated segments, each
`[a-z0-9-]+`, at most 64 bytes in total. The first segment is the consumer's own prefix.

| Key | Owner | Kind of call |
| --- | --- | --- |
| `session-title` | SessionTitle | `auxiliaryModel.complete` |
| `compaction` | F1 compaction host | `auxiliaryModel.complete` |
| `collaboration.member` | Collaboration | Conversational: member Session model |
| `modality.<category>[.<operation>]` | Modality | Non-conversational (Modality's own path) |

**Policy entry.** `{purpose, providerId, modelId, version}`, global only (no consumer needs a
per-workspace purpose). No other parameters: per-purpose reasoning effort is listed in the
feature list but has no design yet (Open question 1).

**Storage.** Rows in the existing `provider_selections` table with `scope = "purpose:<key>"`.
The row shape is exactly a model selection with a CAS version, so no new type, table or format
version is needed; existing scopes are `global` and `workspace:<abs path>` and cannot collide.
Existing behavior already fits: deleting a Provider deletes its selection rows (the entry is gone
and the purpose falls back, below); a model that is temporarily unavailable keeps its row.
The implementation PR documents the new scope value in
[native-storage-format](native-storage-format.md).

**Why Provider's store, not a Settings source.** ADR 002 D4 allows either. A Settings source
would get a second writer once S8b adds the generic `settings.update`, and Settings cannot check
the target against Provider's catalog. The overview and Modality require "one write path"; the
default selection, which is the same kind of data, already lives here with the same CAS write.
No change notification is needed: every consumer reads the policy at use time.

**Runtime state.** None. `complete` keeps no cache and no in-flight registry beyond Provider's
existing task tracker.

## 4. Interfaces

### 4.1 Language-neutral (ADR 002 D5)

`auxiliaryModel`, process scope, no instance key, provided by Provider.

| Method | Mode | Request | Reply |
| --- | --- | --- | --- |
| `complete` | async | `{purpose, inherit: {providerId, modelId}, request: {system?: string, messages: Message[], maxOutputTokens?: u32}}` | `{text, stopReason: "end_turn" \| "max_tokens" \| "stop_sequence", usage?: {promptTokens, completionTokens, totalTokens}}` |

- `messages` uses the existing serializable `Message` type (dim-agent-types). SessionTitle sends
  one user text message; the compaction host sends the history it summarizes plus its
  instruction. No tools are offered; thinking output is dropped; only text blocks form `text`.
- `inherit` is required: the model to use when the purpose has no entry. SessionTitle passes the
  Session's model (its A1 hint, now required rather than optional); the compaction host passes the
  model of the request being compacted.
- `maxOutputTokens` caps the reply (SessionTitle keeps titles cheap); absent means the model's
  maximum. It is clamped to the model's `max_output_tokens`.
- Not streaming: both consumers need the whole text.
- Cancellation: the consumer drops the call (or sends the cancel request of the bridge protocol).
  There is no service timeout; budgets belong to the consumer (SessionTitle: 30 s).
- Errors are data `{code, message}`: `invalid_request`, `model_unavailable`, `failed`,
  `cancelled` (§6).

Client management methods (namespace `provider`, local transport only, like `provider.select`):

| Method | Params | Result |
| --- | --- | --- |
| `provider.purposes` | `{}` | `{purposes: [{purpose, providerId, modelId, version}]}` |
| `provider.purpose.set` | `{purpose, model: {providerId, modelId} \| null, expectedVersion?}` | The entry, or `null` after clearing |

`set` checks the key grammar, then (for a non-null model) that the model is selectable in that
Provider's catalog, the same check as `provider.select`; `expectedVersion` is CAS (absent means
"no entry yet"). Consumers: the CLI `/settings` "auxiliary model" page (feature list) and
Modality's `modality.configure`.

### 4.2 Rust binding

In `dim-agent-capabilities::host`, next to `ModelSelectionHost`, because the compaction port
(`ResolveContextModel`) lives there and a non-Provider profile could implement it too:

```rust
pub struct AuxiliaryRequest { pub system: Option<String>, pub messages: Vec<Message>,
                              pub max_output_tokens: Option<u32> }
pub struct AuxiliaryReply { pub text: String, pub stop_reason: ModelStopReason,
                            pub usage: Option<Usage> }
pub enum AuxiliaryErrorCode { InvalidRequest, ModelUnavailable, Failed, Cancelled }
pub struct AuxiliaryError { pub code: AuxiliaryErrorCode, pub message: String }
pub trait CompleteAuxiliary: Send + Sync + 'static {
    fn complete(&self, purpose: &str, inherit: ModelId, request: AuxiliaryRequest,
                cancel: CancellationToken)
        -> Pin<Box<dyn Future<Output = Result<AuxiliaryReply, AuxiliaryError>> + Send + '_>>;
}
pub struct AuxiliaryModelHost(pub Arc<dyn CompleteAuxiliary>);
```

Provider's `apply` provides `AuxiliaryModelHost`; its `Deps` add `Arc<ModelHost>`.
`ProviderService` gains `purposes()`, `purpose(key) -> Option<PurposeEntry>` and
`set_purpose(key, Option<ModelId>, expected_version)`; the management methods call them.

## 5. Flows

### 5.1 `complete`

```
 Consumer          AuxiliaryModel (Provider)   ProviderService      ModelHost (Aimux)
    | complete(p, inherit, req) |                    |                     |
    |-------------------------->| check key, req     |                     |
    |                           | purpose(p) ------->| row purpose:<p>?    |
    |                           |<-- entry | none ---|                     |
    |                           | target = entry or inherit                |
    |                           | configuration(Some(target)) ->| selectable? config
    |                           |<-- ModelSelection | model_unavailable    |
    |                           | build ModelRequest (system, messages, max_tokens, no tools)
    |                           | ProductResolver.resolve -> connection (credentials stay here)
    |                           | invoke(request, connection, cancel) ---->|
    |                           |<----------- chunks: Text, Usage, MessageStop
    |<-- {text, stopReason, usage}                                         |
```

If the stream fails with a credential rejection that the connection's `RecoverModelConnection`
classifies as recoverable (managed OAuth), Provider recovers once and retries once. There is no
transient retry or backoff: consumers own retries (SessionTitle retries on a later input; the
compaction host follows its own failure rule). No event is written anywhere.

### 5.2 Setting a policy

```
 CLI /settings -> Rpc -> provider.purpose.set {purpose, model, expectedVersion}
   Provider: grammar -> catalog_for(provider).selectable(model) -> CAS upsert/delete row
   reply: entry | null        (no notification; next complete() reads the new row)
```

### 5.3 Model choice, in order

| Step | Condition | Result |
| --- | --- | --- |
| 1 | Entry for the purpose exists | Use it. Not selectable (model disabled, unavailable, catalog missing): `model_unavailable`, **no fallback** |
| 2 | No entry (never set, cleared, or its Provider deleted) | Use `inherit`. Not selectable: `model_unavailable` |

A configured entry never silently falls back: the user chose where that content goes, and
sending it to another Provider would surprise them. Unlike the default selection (which tries
further candidates), `complete` has exactly one candidate after step 1 or 2.

## 6. Errors

| Code | When | Retry |
| --- | --- | --- |
| `invalid_request` | Bad purpose key, empty `messages`, `maxOutputTokens = 0` | No (caller bug) |
| `model_unavailable` | Chosen model not selectable, or its Provider or catalog missing | After the user fixes the policy or catalog |
| `failed` | Connection resolution, transport, HTTP or stream error after the single credential recovery | Consumer's choice |
| `cancelled` | Call dropped, cancel token, or Provider unloading (its stop token) | Consumer's choice |

Management methods map errors as `provider.select` does today (invalid params, unavailable
model, version conflict).

## 7. Security

- Credentials never leave Provider: `ResolvedModelConnection` is built and consumed inside
  `complete`, never returned or serialized. Consumers see only text, stop reason and usage.
- Error messages carry no secrets: `failed` uses the existing `HostError` message, which is
  produced without credential values; Provider never formats a connection into a message.
- Diagnostics: `complete` emits a process `tracing` span `auxiliary.complete` with purpose,
  provider id, model id, outcome, duration and token usage, never request or reply content.
  Consumers record their own outcome in their Session diagnostics (SessionTitle: outcome and
  duration only). Auxiliary calls are not Session facts and do not enter any Timeline.
- The policy is user configuration written only by the local client or a trusted in-process
  owner; paired remote devices are never served `provider.*` methods.

## 8. Relation to existing pieces

| Piece | Relation |
| --- | --- |
| Default selection (`provider.select`) | Unchanged. Not used by `complete`; the caller's `inherit` replaces it |
| SessionTitle (P4) | Calls `complete("session-title", sessionModel, …)`; may declare `auxiliaryModel` in `Deps` (extension → core). Its A.2 shape changes `inherit?` to required |
| Compaction host (F1) | Implements `ResolveContextModel`; `compact` calls `complete("compaction", requestModel, …)`, looked up per call because `ContextModelHost` is a Session value and profiles without Provider have no `auxiliaryModel` |
| Collaboration | Later, its own change: member model = profile model, else `purpose("collaboration.member")` passed as the explicit model, else today's default selection; an unavailable entry fails member creation. Whether "unset" should mean the leader's model is Collaboration's decision |
| Modality (P3) | Stores `modality.*` entries through `set_purpose`; non-conversational connection resolution and catalog entries for non-chat models are a separate Provider supplement, out of S4 |
| Workflow `workflow.stage` | Recommend dropping: `stage.model` > profile model > Agents' default already covers it, no feature asks for a separate stage default, and Workflow would need a policy read by name (#4) |
| ExternalAgent | No special case: an entry may name an `external-agent:*` Provider entry; ModelHost routes it like any request |

## 9. Rule check

| Rule | Answer |
| --- | --- |
| Principles §3 (dependency direction) | Provider (core) depends only on Aimux (core); consumers depend on or look up Provider; Provider names no consumer. |
| §4 rule 3 (before callbacks) | Not used. |
| §4 rule 8 (infrastructure unchanged) | No change to Timeline, Dispatcher, event formats, rutis or assembly; uses `ModelHost`, `ProductResolver`, an existing table and Rpc's `managementMethods`. Nothing raised to rutis. |
| §4 rule 9 (only needed facts in Timeline) | Policy is process state changed by Provider's typed write; auxiliary calls write no event. Compaction's summary is recorded by ContextPreparation's existing fact, not by this service. |
| ADR 002 D2 (no wiring modules) | No new plugin; the service lives in the owner of models and credentials. |
| ADR 002 D3 (dependencies) | Core never hard-depends on an optional plugin; the F1 host looks the service up per call; SessionTitle (optional) may depend on it. |
| ADR 002 D4 (configuration) | Purpose policy is business configuration in Provider's own store; a change is a data update read at use time, no reload. |
| ADR 002 D5 (neutral contracts) | §4.1 is the contract; §4.2 is one binding. |

## 10. Open questions

1. Per-purpose reasoning effort ("Auxiliary model and reasoning effort" in the feature list):
   needs a column or a different row shape, hence a format decision. Deferred until `/settings`
   is designed; S4 stores model only.
2. Does F1 need the chosen model's context window before sending (to trim history for a smaller
   compaction model)? If yes, add a `resolve(purpose, inherit) -> {model, capabilities}` read in
   F1; S4 does not add it speculatively.
3. Should the API-key `--models` profile (ConfiguredModels, no Provider plugin) offer
   `auxiliaryModel`? Today it would not, so titles are skipped and F1 must treat a missing service
   as "compaction unavailable".

## 11. Acceptance (flow tests)

1. `provider.purpose.set` then `provider.purposes` round-trip with CAS; a stale
   `expectedVersion` is a conflict; clearing returns `null`; an invalid key and an unselectable
   model are rejected; the default selection is unaffected.
2. `complete("session-title", inherit, …)` against a mock HTTP provider: with no entry the
   request goes to `inherit`; after `set` it goes to the entry; the request carries the system
   text, messages, `max_tokens` and no tools; the reply is the text with usage.
3. Entry pointing at a disabled model: `model_unavailable`, no request sent, no fallback.
4. Deleting the entry's Provider: the next call uses `inherit`.
5. Dropping the call mid-stream cancels the request; unloading Provider ends in-flight calls with
   `cancelled`.
6. Neither the reply, the errors nor the trace span contain the API key; the Session's Timeline
   has no new events after a call.
7. F1 compaction flow test (in F1) calls `complete("compaction", …)` and records its existing
   compaction fact.
