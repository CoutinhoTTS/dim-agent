# Model failures

What a failed model request reports, and how the retry policy reads it (issue
#1197).

## 1. One closed kind, decided where the provider error is read

The aimux bridge is the only place that holds the provider's structured error,
so it decides `ModelFailureKind`. It uses only the origin, the HTTP status, the
normalized codes and the `AiMuxError` variant. It never matches message text.

| kind | decided from | retried | meaning | action |
| --- | --- | --- | --- | --- |
| `network` | `ApiCall`, origin Transport, no HTTP status | yes | could not reach the provider, or the connection dropped | check the network or proxy |
| `timeout` | `AiMuxError::Timeout`, aimux's own deadline | yes | the provider did not respond in time | check the network or proxy; try again later |
| `rate_limited` | 429, rate-limit codes | yes; `Retry-After` is used while it fits the wait budget | the provider is rate limiting requests | wait and try again |
| `server_error` | 408, 5xx except 501 and 505, server and overload codes, transient stream-event codes | yes | the provider reported a server error | try again later or choose another model |
| `malformed_response` | `InvalidResponseData`, `JsonParse`, an undecodable stream | no | the provider sent a response Dim could not read | if it repeats, check the proxy or gateway |
| `auth_rejected` | 401, 403, invalid-key/auth codes | no (the existing one credential recovery stays) | the provider rejected the credentials | sign in again or check the API key |
| `quota_exhausted` | 402, quota/billing codes | no | the account has no quota left | check the plan or billing |
| `not_found` | 404 | no | the provider did not find the model or endpoint | check the model and the provider's base URL |
| `context_too_long` | 413, context-length codes | no | the conversation is longer than the model accepts | compact the conversation or start a new session |
| `content_filtered` | content-policy codes | no | the provider's content filter blocked the response | rephrase the request |
| `invalid_request` | 400, 409, 422, 501, 505, other 4xx, invalid-request codes, `InvalidArgument`, `InvalidPrompt`, `UnsupportedFunctionality` | no | the provider rejected the request | see the details |
| `output_limit` | the `max_tokens` stop | no | the model stopped at its output limit | increase the output token limit and retry |
| `unknown` | anything else | no | the model request failed | see the details |

Only `AiMuxError::Timeout` is `timeout`. A timeout of the HTTP connection or of
the stream reaches the bridge as a transport `ApiCall` and is `network`.

Codes are compared trimmed and ASCII-lowercased.

- A permanent code wins over a status.
- 409, 501 and 505 stay `invalid_request` whatever code came with them, as they
  are never retried.
- A stream-event failure carries the response's status, not the event's, so only
  its code may decide: a transient code maps to `rate_limited` or
  `server_error`, a permanent code to its kind, anything else to `unknown`.

Every `HostError::Failed` the bridge produces carries facts, including
`AiMuxError::Timeout`, `InvalidResponseData`/`JsonParse`, `InvalidArgument`/
`InvalidPrompt` and `UnsupportedFunctionality`. Where there is no
`ApiCallError`, the facts carry no origin.

The kind is only as good as the provider's structured fields. A provider that
reports, say, quota as a bare 429 with only a message becomes `rate_limited` and
is retried uselessly within the budget; the `reason` line still shows the
provider's own message. A missing code is added to the bridge's code list, never
matched from text.

## 2. The fact

`ModelOutputOutcome::Failed` gains `failure: Option<ModelFailure>`:

```
ModelFailure {
  kind,             // section 1
  status?,          // HTTP status, when a response arrived
  providerCode?,
  requestId?,
  reason?,          // one specific sentence, one line, at most 300 chars
  cause: string[],  // the chain, outermost first; each one line, at most 200 chars, at most 6 frames
  attempts,         // at least 1
  waitedMs,         // total backoff slept
  retryAfterMs?     // the provider asked to wait longer than the budget left, so Dim stopped
}
```

- `failure` is optional. It is absent on events written before it existed, and
  on failures that are not a model provider call's: the agent loop's
  unsettled-call failure on a reload (`agent_loop.rs`) and a context preparation
  failure (`context_preparation.rs`). The field follows `presentation`'s casing
  and `optional_non_null` handling.
- `reason` is the provider's error message for a response or stream-event
  failure, and the innermost frame of the cause chain for a transport failure.
  A JSON parse message embeds the body text it could not read, so its `reason`
  is the parse diagnosis from the error's structure, and is absent when the
  error carries no cause.
- `cause` is the chain from `ApiCallError.message` and `ErrorCause`, outermost
  first, one line per frame.
- Never stored: the request URL, headers, request values, the response body or
  any credential. aimux already drops the URL; the bridge must not copy `url`,
  `request_body_values` or `response_body`, and `reason` and the cause frames
  are text only, with control characters stripped.
- `error`, which old clients read, is the default English line
  `<meaning>: <reason>. <action>`, with `: <reason>` left out when there is
  none. There is no `ApiCall:` prefix. The turn that the failure ends carries
  the same string in `TurnOutcome::Failed.error`, so a client can tell the two
  apart and drop the repeat.
- The `max_tokens` stop keeps its presentation and also writes
  `failure.kind = output_limit`.
- A failure from a host with no provider boundary gets kind `unknown` and the
  message it already carried.

## 3. Who words the line

Clients word their own line from `kind`, through their own i18n. The fact
carries the kind and the data, not a sentence. `presentation` keeps its meaning:
wording that a host or a `ModelFailurePolicy` supplies wins over the client's
kind table. It is an override, not the always-present form.

The terminal fact's `error` is a complete fallback line, so every renderer that
already prints `error` shows the better line without a change of its own.

## 4. Retry

The ModelProvider retries:
- at most 3 attempts;
- with a backoff of 0.5 s × attempt × a jitter in [0.75, 1.0];
- within a 60 s wait budget;
- using `Retry-After` as the delay while it fits the budget left, and stopping
  at once with `retryAfterMs` recorded when it does not;
- only the `network`, `timeout`, `rate_limited` and `server_error` kinds.

It does not retry after irreversible output. An interrupt cancels the sleep. A
credential rejection keeps its one recovery, with no delay.

## 5. Compatibility

The fields are additive. A binary without `failure` cannot decode an event
written with it, because `ModelOutputOutcome` denies unknown fields. That is the
same forward-compatibility limit `presentation` has, so a downgrade across this
change is unsupported, as it already was for `presentation`.
