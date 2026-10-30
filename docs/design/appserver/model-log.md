# Model request log

The user's switch for recording the wire exchange of every model request
([#701](https://github.com/arcships/dim/issues/701)). The recording format is
aimux's RFC-0023 JSONL (`vendor/aimux/aimux-core/src/recording.rs`).

## Scope and owners

| Owner | Responsibility |
| --- | --- |
| ModelLog, process | Register the `model-log` Settings source, start and stop the recorder, follow its changes |
| Aimux, process | Bind a request to the recorder that is on when the request is built, and close the recording when its stream ends |
| Settings service | Validate and persist `model-log.json`, user scope only |

The switch is user scope only: recordings hold prompts, code and tool output, so
project content must not turn them on.

## Source

`model-log.json`:

```json
{
  "enabled": true
}
```

`enabled` defaults to false. When it is true every model request appends one
line to `<home>/model-log/recordings.jsonl`; when it is false nothing is
recorded. The recorder is replaced on `settings.changed` and stopped when the
plugin unloads.

With file-backed settings the document is `<home>/model-log.json`; with native
storage it is the same document in the settings table, written through
`settings.update` with `source: "model-log"` and `scope: "user"`.

A line holds one call: the input prompt and options, the provider identity, one
entry per HTTP exchange (request headers and body, response status, headers and
body, chunk count, `latency_ms`, `ttfb_ms`) and the outcome (`success`, `error`,
`incomplete`, `cancelled`, usage). Aimux redacts credentials and authorization
headers.

## Reading it

- `latency_ms` is the time from sending the request to receiving the response
  headers.
- `ttfb_ms` is set to the same value; it is not the arrival of the first stream
  chunk.
- `response.body` is the stream's SSE frames as received.

## Failure

When the switch is on and the recorder cannot start (for example an unwritable
home), the plugin fails at startup. A later settings change that cannot be
applied is logged as a warning and leaves the previous state.

## Observation layers

[ADR 005](../adr/005-observability.md) D1 keeps two observation layers and says
there is no third. This source adds an opt-in diagnostic record that the
Timeline cannot carry: wire bytes and per-exchange timings. ADR 005 needs to
accept or reject it before anything depends on this switch.

## Verification

- `settings_flow::model_log_switch_starts_the_recorder`: enabling the source
  starts the recorder and creates the file.
- `model_flow::recorded_requests_write_their_wire_exchange`: a turn records its
  request, exchange, provider and outcome.
