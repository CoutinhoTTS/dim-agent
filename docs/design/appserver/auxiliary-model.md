# AuxiliaryModel

Status: Draft (for review). Roadmap item S4 ([#53](https://github.com/arcships/dim/issues/53)).
Overview: [Appendix B.2 Provider row](../overview.md#b2-app-server-process-plugins). Scope:
[ADR 002](../../adr/002-plugin-scope.md) (process, inside the existing Provider plugin).

## 1. Summary

Provider adds one service, `auxiliaryModel`, with one async method
`complete(model, request)`. It makes a single non-streaming model call outside any Session:
it resolves the connection for the given model inside Provider, calls the existing
`ModelHost`, and returns the text. It writes no Timeline, and credentials never leave
Provider.

The caller decides which model to use and keeps that choice in its own business
configuration. Provider holds no per-caller policy: it has no purpose keys, no purpose
storage, and no new management methods.

The S4 done criterion has one consumer: SessionTitle (P4), which needs a call outside its
Session and cannot reach credentials. No new plugin, no new Timeline event, no
infrastructure change.

### 1.1 Why it is this small

The requirement comes from the feature list: "Auxiliary models: configure models for
auxiliary tasks", plus the settings page entry "Auxiliary model and reasoning effort". An
earlier overview revision turned this into a per-purpose policy held by Provider, with one
write path, and routed compaction through this service. Checked against the code, both were
more than the need:

- Configuring an auxiliary model is a business setting of the consumer that runs the task.
  Each consumer keeps its own choice; the CLI settings page shows them together by reading
  each owner. Nothing requires Provider to hold them.
- Compaction already has its own port, `ResolveContextModel` → `ContextModel`
  (`dim-agent-capabilities/src/host.rs`): context window, output budget, token counting and
  `compact`. A text-only `complete` would lose that information. The production compaction
  host (F1) implements that port; it does not use `auxiliaryModel`.

## 2. Structure

```
 Process scope
 +----------------------------------------------------------------+
 |  Provider (existing plugin)                                    |
 |   ProviderService: catalogs, default selection (unchanged)     |
 |   ProductResolver: connection and credentials (unchanged)      |
 |   AuxiliaryModel --uses--> ProviderService, ProductResolver,   |
 |                            ModelHost                           |
 +-------------------------^--------------------^-----------------+
       Aimux: ModelHost ---'                    |
                                                | complete(model, request)
 Session scope                                  |
   SessionTitle (Deps: auxiliaryModel) ---------'
     model = its Settings source `session-title`.model, else the Session's model
```

Dependencies point to the core: Provider gains a hard dependency on Aimux's `ModelHost`
(both core, always together in the product profile). Provider names no consumer.

## 3. Data and state

None. Provider stores nothing new, caches nothing, and tracks no in-flight call beyond its
existing task tracking.

## 4. Interfaces

### 4.1 Language-neutral (ADR 002 D5)

`auxiliaryModel`, process scope, no instance key, provided by Provider.

| Method | Mode | Request | Reply |
| --- | --- | --- | --- |
| `complete` | async | `{model: {providerId, modelId}, request: {system?: string, messages: Message[], maxOutputTokens?: u32}}` | `{text, stopReason: "end_turn" \| "max_tokens" \| "stop_sequence", usage?: {promptTokens, completionTokens, totalTokens}}` |

- `model` is required. The caller resolves it from its own configuration; Provider does not
  fall back to another model.
- `messages` use the existing serializable `Message` type (dim-agent-types). No tools are
  offered; thinking output is dropped; only text blocks form `text`.
- `maxOutputTokens` caps the reply; the default is the model's limit, clamped to the model's
  `max_output_tokens`.
- Non-streaming: the consumer needs the whole text.
- Cancellation: the consumer drops the call (or sends the bridge protocol's cancel request).
  The service has no timeout of its own; the budget is the consumer's.
- Errors are data `{code, message}`: `invalid_request`, `model_unavailable`, `failed`,
  `cancelled` (§6).

### 4.2 Rust binding

In `dim-agent-capabilities::host`, next to `ModelSelectionHost`:

```rust
pub struct AuxiliaryRequest { pub system: Option<String>, pub messages: Vec<Message>,
                              pub max_output_tokens: Option<u32> }
pub struct AuxiliaryReply { pub text: String, pub stop_reason: ModelStopReason,
                            pub usage: Option<Usage> }
pub enum AuxiliaryErrorCode { InvalidRequest, ModelUnavailable, Failed, Cancelled }
pub struct AuxiliaryError { pub code: AuxiliaryErrorCode, pub message: String }
pub trait CompleteAuxiliary: Send + Sync + 'static {
    fn complete(&self, model: ModelId, request: AuxiliaryRequest, cancel: CancellationToken)
        -> Pin<Box<dyn Future<Output = Result<AuxiliaryReply, AuxiliaryError>> + Send + '_>>;
}
pub struct AuxiliaryModelHost(pub Arc<dyn CompleteAuxiliary>);
```

Provider's `apply` provides `AuxiliaryModelHost`; its `Deps` gain `Arc<ModelHost>`.

## 5. Flow

```
 Consumer          AuxiliaryModel (Provider)   ProviderService       ModelHost (Aimux)
    | complete(model, req)  |                       |                       |
    |---------------------->| check request         |                       |
    |                       | configuration(Some(model)) ->| selectable?    |
    |                       |<-- ModelSelection | model_unavailable         |
    |                       | build ModelRequest (system, messages, max_tokens, no tools)
    |                       | ProductResolver.resolve -> connection (credentials stay here)
    |                       | invoke(request, connection, cancel) --------->|
    |                       |<------------ chunks: Text, Usage, MessageStop |
    |<-- {text, stopReason, usage}                                          |
```

If the stream fails because a credential was rejected and the connection's
`RecoverModelConnection` reports recoverable (managed OAuth), Provider recovers once and
retries once. No transient retry or backoff: retry is the consumer's (SessionTitle retries
on a later input). No event is written anywhere.

## 6. Errors

| Code | When | Retry |
| --- | --- | --- |
| `invalid_request` | `messages` empty, `maxOutputTokens = 0` | No (caller defect) |
| `model_unavailable` | The model is not selectable, or its Provider or catalog is missing | After the user fixes the consumer's setting or the catalog |
| `failed` | Connection resolution, transport, HTTP or stream error (after one credential recovery) | Consumer's choice |
| `cancelled` | Call dropped, cancel token fired, or Provider unloading (its stop token) | Consumer's choice |

## 7. Security

- Credentials stay in Provider: `ResolvedModelConnection` is built and used inside
  `complete`, never returned or serialized. Consumers see text, stop reason and usage only.
- Error messages carry no secrets: `failed` uses the existing `HostError` message, which
  carries no credential values; Provider never formats a connection into a message.
- Diagnostics: `complete` records one process `tracing` event, target `auxiliary.complete`, with provider
  id, model id, outcome, duration and token usage, never request or reply content. Consumers
  record outcomes in their own Session diagnostics. Auxiliary calls are not Session facts
  and enter no Timeline.

## 8. Relationship to existing parts

| Part | Relationship |
| --- | --- |
| Default selection (`provider.select`) | Unchanged. `complete` does not consult it |
| SessionTitle (P4) | Calls `complete(model, …)`; `model` is the optional `model` of its Settings source `session-title`, else the model of the latest `context/configured` in the prefix. May declare `auxiliaryModel` in `Deps` |
| Compaction host (F1) | Does not use this service. Implements `ResolveContextModel`; how it picks the model and reads the context window is F1's design |
| Collaboration | No change from S4. A member's model is its profile's model, else the default selection; a separate member default, if ever wanted, is Collaboration's own setting |
| Modality | Keeps its default model per category in its own `modality` Settings source and passes the model explicitly. Non-conversational connection resolution is a separate Provider supplement, not S4 |
| Workflow | `stage.model` > profile model > the Agents default already covers model choice; no stage default in Provider |
| ExternalAgent | Nothing special: the model can be an `external-agent:*` Provider entry; ModelHost routes it like any other request |
| `--models` API-key profile (ConfiguredModels, no Provider plugin) | Does not provide `auxiliaryModel`; SessionTitle then skips titles |

## 9. Rule check

| Rule | Answer |
| --- | --- |
| Principles §3 (dependency direction) | Provider (core) depends only on Aimux (core); consumers depend on Provider; Provider names no consumer. |
| §4 rule 3 (before callback) | Not used. |
| §4 rule 8 (infrastructure unchanged) | No change to Timeline, Dispatcher, event format, rutis or assembly; uses `ModelHost`, `ProductResolver` and the existing catalog. No requirement on rutis. |
| §4 rule 9 (only needed facts in Timeline) | Auxiliary calls write no event. |
| ADR 002 D2 (no wiring modules) | No new plugin; the service lives in the owner of models and credentials. |
| ADR 002 D3 (dependencies) | No core part hard-depends on an optional plugin; SessionTitle (optional) may depend on this service. |
| ADR 002 D4 (configuration) | Provider adds no configuration; each consumer's model choice is its own business configuration. |
| ADR 002 D5 (language-neutral contracts) | §4.1 is the contract; §4.2 is one binding. |

## 10. Open questions

None for S4. Per-purpose reasoning effort (feature list) is a field of the consumer's own
setting if it is ever added, and `complete` would then take it as a request parameter;
not designed now.

## 11. Acceptance (flow tests)

1. `complete` against a mock HTTP provider sends the request to the given model with the
   system text, messages, `max_tokens` and no tools; the reply is text plus usage.
2. A disabled or unknown model: `model_unavailable`, no request sent, no fallback.
3. Deleting the model's Provider: the next call is `model_unavailable`.
4. Dropping the call mid-stream cancels the request; unloading Provider ends an in-flight
   call with `cancelled`.
5. No API key appears in the reply, errors or the trace event; the Session's Timeline has no
   new event after a call.
