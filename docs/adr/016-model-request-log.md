# 016: An opt-in request record for model calls

Decision status: Accepted by the repository owner.

Supersedes the sentence "There is no third channel" in [ADR 005](005-observability.md) D1,
only for a user-enabled diagnostic record of model requests. The Timeline and the process
logs keep their roles.

## Context

ADR 005 removed the trace archive and settled two observation layers: the Timeline for
committed facts and the process logs for runtime behavior. It also removed the only record
of a model request's wire exchange.

That gap matters when a request is slow rather than failed. In
[#701](https://github.com/arcships/dim/issues/701) the Timeline showed a 51.5 s request with
2033 completion tokens and a 99.6% prompt-cache hit, but not whether the time went to the
proxy before response headers, to the provider before the first SSE chunk, or to generation.
The process logs carry no bodies, and they were off for that run.

The Timeline should not take this data: wire bytes are not business facts, and per-chunk
timing is not state the agent loop consumes. A second copy in the Timeline would also grow
the store that every reader loads.

## Decision

### D1. An opt-in third record

A user-scoped settings source may enable recording of model requests. It writes one record
per call to a file outside the Timeline. The design document owns the source, the format,
the path and the failure behavior. It is off by default.

### D2. Diagnostic only

The record is read by people and tools outside Dim. It is never read back into history,
context, prompts, replay or evaluation, and no part of Dim depends on it being present.

### D3. User authority only

Prompts, code and tool output are sensitive, so only the user's local configuration can
enable the record; project content cannot. Credentials are redacted at the writer.

## Consequences

- ADR 005 D1's "There is no third channel" becomes "no third channel by default" for this
  one record. The Timeline and process logs remain the layers for facts and runtime
  behavior, and nothing here reopens the diagnostics channel or the trace archive.
- The file grows with traffic and holds sensitive content; the switch is the boundary that
  keeps it off.
- A later run can attribute wire latency without changing the Timeline's storage format.
