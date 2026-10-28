# Recorded conversation inputs

This supplements [Session input](contracts/session-access.md#5-context-mapping).
It adopts existing conversation content through ordinary inputs. It does not
reconstruct the execution that produced it. Migration is a consumer; the input
and context owners know no legacy schema.

## Input

An `input/message` may carry a `messages` block:

```json
{"type":"messages","source":"Imported from a legacy conversation.","messages":[
  {"id":"old-1","createdAt":"2026-01-01T00:00:00Z","message":{"role":"user","content":"Hello"},"inContext":true}
]}
```

`source` is final model-visible text describing the origin. Each record has a
nonempty identity unique within the block, an optional source timestamp, the
existing `Message` type and an explicit `inContext` flag. Source timestamps never
become Timeline timestamps. Readers show every record in supplied order;
`inContext: false` retains readable history without adopting it. Summaries are
supplied messages identified by source text, not new compression facts. Producers
choose records and their effective subset at the external boundary.

A block is a complete context boundary. Adopted tool calls have unique nonempty
identities and matching result parts in order; no later user or assistant message
may intervene before their results. User messages cannot carry tool or thinking
parts. Invalid adopted content is rejected before writing. Historical content
excluded from context may retain incomplete calls as evidence.

An optional nonempty `context_error` preserves a conversation whose effective
context cannot be represented. Every record then has `inContext: false`.
Adopting the input fails with the recorded reason instead of silently substituting
history. Clients can still read and export the records.

## Adoption and history

Use `append` for history that must not start a turn. InputQueue applies it and Loop
consumes it with the next normal request. At consumption the context projection
places `source` as one user message for a block with effective records, followed by them with their
original roles and parts. It executes no tools, creates no approvals, and claims
no past model request occurred. The block is not a keyed persistent rule.

For an unpublished Session, the [conversation receiver](appserver/conversation-import.md)
commits new admission and consumption together as initialization. Thus the
published Session already has adopted context, including before its first fork.

Each projected message's identity derives from its consumed event, block position
and record identity. The consumed event remains its source for compaction and fork
boundaries. Chunks end at complete tool groups. Retained suffixes, full replay,
recovery checkpoints and forks use existing event boundaries. No history-specific
renderer or alternate context-input path exists.

CLI history shows a source marker and one row per record, retaining source times
and history-only status. Markdown export retains origin, role, record identity,
time, tool call/result identities and content; JSON retains the typed input.
The shape is available to protocol v3 Sessions only; v1 serialization explicitly
rejects it. Frozen fixtures remain unchanged.

```text
[Imported from legacy conversation]
User       [source time]  original text
Assistant  [source time]  tool call and arguments
Tool       [source time]  result
User       [native time]  new request
```

## Persistence and verification

The block is an incompatible committed input format. Native storage v21 admits it;
older daemons reject the v21 database at startup. Existing event bytes are not
rewritten. The default context projection interpreter becomes v4 so an older
checkpoint cannot conceal different input semantics. Tests cover append without
execution, roles and pairing on the next request, excluded history, unsupported
context, restore, fork and compaction boundaries.
