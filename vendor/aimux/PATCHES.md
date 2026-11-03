# Agent integration patches

Upstream base: `v0.6.0`, `fa29a30ad25e362acc349bea6ce4b025bdd8be5a`.
One patch remains from the earlier integration, the smallest change that keeps a behaviour the Agent depends on. It has an upstream issue asking for the same contract.

| File | Change | Why it survives | Pinning test |
| --- | --- | --- | --- |
| aimux-core/src/parse_tool_call.rs | Export `parse_json_input`. | `arcships/aimux#463`: the Agent drives the stream and executes tool calls itself, so it parses raw tool input through this function. | `dim-agent-aimux/src/stream.rs` calls it, so the crate does not compile without the export; `src/stream/tests.rs`: `errors_release_transport_and_do_not_expose_request_or_tool_input` pins its `InvalidToolInput` contract. |
| aimux-core/src/error.rs | Round a fractional `retry-after-ms` hint up to the next millisecond instead of truncating it. | `arcships/aimux#458`: a truncated hint starts a retry before the instant the server named. | `dim-agent-aimux/tests/model_flow/native.rs`: `native_failures_expose_only_normalized_model_facts` answers with `retry-after-ms: 25.1` and expects 26 ms. |
| aimux-core/src/recording.rs | Classify a key as sensitive when its name, with separators removed, ends in `key`, `auth` or `token`, or contains `secret`, in addition to the exact names the tag already lists. | `arcships/aimux#459`: configured connection headers such as `x-portkey-virtual-key` and `helicone-auth` would otherwise be recorded in clear text. | `dim-agent-aimux/tests/model_flow.rs`: `recorded_connection_headers_redact_sensitive_names`. |

The workspace member list is reduced to the four crates the Agent uses; production source is retained verbatim except for the change above.

The Agent turns on `CallOptions.strict_stream` for every call, so the stream contract itself is the tag's. The checks that remain in the adapter are the product rules: an unsupported warning fails the call, provider-executed tools and provider-generated files, sources and tool results are refused, and a content-filtered or errored finish is a failed turn.

The vendored tree carries production source only. The three regression test files the integration used to add were removed with the upgrade; their assertions are covered by the existing model-flow suites.

The patch does not add timeouts, retries, repair, or new model configuration fields, and nothing here enables Core orchestration or host-side tool-call repair in the Agent.
