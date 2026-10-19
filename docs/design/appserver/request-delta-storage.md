# Request delta storage (native format v17)

Status: Proposed, for team review. Phase 2 of [#193](https://github.com/arcships/dim/issues/193), as agreed in the
issue's latest comment. Phase 1, a bounded content cache shared across the pages of a read, merged as
[#213](https://github.com/arcships/dim/pull/213); phase 3 is a separate runtime proposal (§15). Owner lane:
Agents/storage (yyhhyyyyyy). Reviewers: zerob13, eric8810. Scope: inside the native `TimelineStore` implementation
(`SqliteTimelineStore`, [timeline.rs](../../../crates/dim-app-server/src/storage/timeline.rs) and
[codec.rs](../../../crates/dim-app-server/src/storage/codec.rs)) and the [storage format](native-storage-format.md),
which this design moves to v17. Design only; the implementation follows review.

## 1. Summary

In format v16 a `context/prepare` or `model/call` row stores one content-hash reference per message of its request.
Each request repeats the previous one and adds a few messages, so the k-th model call stores k references. Storage,
write hashing and read decoding all grow with the square of the number of model calls between compactions (§2).

- **Rule.** A v17 request row stores its request as a structural delta against the previous request in the branch's
  logical history: the `base` position, `keep` (the length of the common message prefix), the remaining messages
  inline, and `system` and `tools` either as "same as base" or inline. A request with no earlier request, or one that
  shares no message with it, is a root and stores everything inline (§5).
- **Deterministic.** The bytes are a function of the branch's logical history and the event. A per-branch writer memo
  only spares reading the base back; the writer produces the same bytes with or without it (§6).
- **Reads.** A read decodes requests in order, each one from the request before it. A read that starts mid-history
  fetches the base chain with one recursive primary-key query (§7). A damaged or missing base fails the read; a
  request is never restored empty (§8).
- **Measured.** In the mock, tool and product-window profiles every request outside a compaction extends the previous
  one, and the previous request is always still in memory when the next one is written (§4). At about 10,000 events
  v16 stores 13,778 bytes per event and v17 would store 351, as it does at 1,000 events (plain mock turns; §12).
- **Format.** v17 is an in-place marker bump from v16 in the existing admission transaction, with no table or row
  rewrite. v16 rows stay readable for good, v16 and v17 rows mix in one database, and v17 writers add no content
  rows (§11).

Event types and their frozen fixtures, the Timeline, the Dispatcher, the `TimelineStore` trait, the read API,
capabilities and external protocols do not change. No new event, service, method, wire field, configuration or runtime
requirement.

## 2. Problem and evidence

### 2.1 Restore cost (#193)

Release build, Apple M4, `SqliteTimelineStore` on a disposable database, `defaults::mock_hosts` and
`defaults::install`. History was grown through mock-model turns (a plain turn writes 10 events, a tool turn 18); seven
restores per size. Timed: `Agent::restore_session` plus the first `idle("main")`.

| Events | Plain, median | Tools, median |
| --- | --- | --- |
| ~1,000 | 65 ms | 80 ms |
| ~3,000 | 327 ms | 404 ms |
| ~10,000 | 2.77 s | 3.48 s |

At 10,017 plain events, 2,186 ms of the 2,772 go to 11 full-history reads (about 199 ms each) and 549 ms to
`validate_session`. Each request row stores one reference per message (`encode_refs`), so at 10k events the rows hold
2,000,000 references and 137.5 MB of event rows for 2,001 distinct contents, and decoding resolves every reference
through `ContentCache`.

### 2.2 Phase 1 (#213)

Phase 1 resolves a reference with one `foldhash` lookup and shares one cache, bounded at 64 MiB of bodies, across
the pages of a read and across a `validate_session` pass. The writer and the written bytes are unchanged. Its results
(mean of two interleaved rounds, each the median of seven restores):

| Mix | Events | Restore + first idle | Per read | `validate_session` |
| --- | --- | --- | --- | --- |
| plain | ~1,140 | 71 → 58 ms | 5.0 → 4.3 ms | 9.7 → 5.3 ms |
| plain | ~3,140 | 344 → 245 ms | 25.2 → 19.6 ms | 58.6 → 22.1 ms |
| plain | ~10,140 | 2,866 → 1,779 ms | 208 → 146 ms | 576 → 149 ms |
| tools | ~1,140 | 80 → 71 ms | 5.7 → 5.3 ms | 11.5 → 6.5 ms |
| tools | ~3,130 | 417 → 303 ms | 30.5 → 24.3 ms | 73.3 → 27.8 ms |
| tools | ~10,140 | 3,624 → 2,256 ms | 257 → 184 ms | 724 → 186 ms |

### 2.3 What remains

Fitting Phase 1's plain per-read times to `a·E + b·R`, with `E` events and `R` references (`2T² + 4T` for `T` plain
turns), gives a ≈ 2.9 µs per event and b ≈ 58 ns per reference; the fit reproduces the 1,140-event read (4.5 ms against
4.3 ms). At 10k events the references take 117 of the 146 ms of each read. Only a format change removes them.

Writes pay too. For each request row the v16 writer serializes and SHA-256-hashes every message, the tools and the
system prompt, and runs one `INSERT OR IGNORE` per content. Over the last 200 request rows of a 10k-event plain Session
that is 539 µs per row before any SQL, against 24 µs for the v17 encoding (§6.3). Each model step also reads its
context history twice ([timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#4-读取)), and without
compaction those reads decode every earlier request.

The storage checks done when the store was built used frequent compaction, so request lists stayed short. Many model
calls between compactions fall outside that envelope: the mock and `--models` profiles never compact, and with a large
window a token-based compaction can come hundreds of steps apart.

## 3. Constraints

| Constraint | Source | Here |
| --- | --- | --- |
| Events are written and read as their original types; encoding exists only inside storage | [timeline-storage §2](../../architecture/rust-agent/timeline-storage.md#2-存储编码) | The delta encodes `ModelRequest` inside the store; reads return complete requests |
| The request stays in the events: `model/call` carries the final request handed to the model | [trace.md](../../architecture/rust-agent/trace.md#本地归档) ("`model/call` 中的 request 是交给模型能力的最终请求") | The request is neither derived nor loaded lazily |
| ContextPolicy reads the latest exact request | [context_policy.rs](../../../crates/dim-agent-capabilities/src/context_policy.rs) `ContextPolicy::estimate` compares provider, model, capability snapshot, tools, system, config and metadata with the previous `model/call`, and requires `messages.starts_with` its messages | Decoded requests equal the written ones |
| Event types and the frozen fixtures `crates/dim-agent-types/tests/fixtures/events.json` and `events.ts` with the `*_contract.rs` tests | [principles §6](../../architecture/rust-agent/principles.md#6-验证), AGENTS.md rule 6 | Untouched |
| Timeline (sequence, atomic batches, commit and publish, before), Dispatcher (hot window only), `TimelineStore` trait, read API, capabilities | [timeline-storage §1, §3, §4](../../architecture/rust-agent/timeline-storage.md) | Untouched: the port keeps its signatures and semantics |
| External protocols expand to the original events ("外部协议仍展开为原事件") | [timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#有界读取与恢复扫描) | Untouched |
| An incompatible change needs a new version and a read path; an upgrade only adds structures and reads or rewrites no business row; one-way | [Storage format, 格式版本](native-storage-format.md#格式版本) and the S5a/S5b decisions | v17 by a marker bump (§11) |
| Serialization optimizations measure body copies, encode counts and time, and add no business state | [timeline-storage §4](../../architecture/rust-agent/timeline-storage.md#有界读取与恢复扫描) | Measurements in §4, §6.3, §7.4 and §12; the writer memo holds no business state and decides no byte (§6.2) |
| Idle branch live bytes do not grow with steps | [timeline-storage §7](../../architecture/rust-agent/timeline-storage.md#7-验收) | The memo holds no request; a read holds one decoded request beyond what it returns (§6.2, §7.1) |

## 4. Prefix stability

### 4.1 Why consecutive requests extend each other

The delta stays small only if each request starts with the one before it. Three existing contracts make that hold
between compactions:

- **The projection is append-only.** Events enter model context only through adopted inputs and tool results
  (AGENTS.md rule 10, [principles §4](../../architecture/rust-agent/principles.md#4-通过时间线扩展行为) rule 6), and
  the history renderer places each at the end. A `context/prepare` request is the rendered history at its `through`.
- **Preparation may only append.** ContextPreparation rejects a prepared request that does not start with the rendered
  history ("Context preparation must preserve the timeline history prefix; additional messages may be appended",
  [context_preparation.rs](../../../crates/dim-agent-capabilities/src/context_preparation.rs)). A `model/call` request
  therefore extends its `context/prepare` unless ContextPolicy compacted in between.
- **ContextPolicy already relies on it.** Its provider baseline applies only while `request.messages` starts with the
  messages of the previous `model/call`.

A completed compaction replaces the head of the list (replacement, baseline, retained tail), so the request after it
shares no prefix with its base and is a root (§10). A change of `system` or `tools` (a prompt revision, a tool
adoption) stores that part inline once.

### 4.2 Measured

A throwaway probe (not committed) wraps `SqliteTimelineStore` in a forwarding `TimelineStore` over a disposable
database. At every `append` it compares each request with the previous request on its branch: the common prefix by
`Arc::ptr_eq` and then `==`, `system` and `tools`, and whether the previous request's `Arc` is still alive, which is
what a `Weak` memo would see. It also builds the exact v17 delta JSON. Release build, Apple M4. Profiles:

- **plain** and **tools**: `defaults::mock_hosts` and `defaults::install` with the turns of
  [`restore_cost`](../../../crates/dim-app-server/examples/restore_cost.rs); no compaction.
- **product window**: the product window facts (`ProductContextModel`) and summary strategy
  (`ProductSummaryStrategyPlugin`), `IdentityContext`, `HistoryRenderer::default()`, `MockTools`, and a capability
  snapshot with a 32,000-token window and 2,000 output tokens. The model driver echoes like `EchoModel` and answers the
  summarizer with a short summary that differs each time. Turns alternate plain text and `tool:echo`, each carrying
  1,000 bytes, so the window fills and compacts about every 20 turns.

Every request row is counted:

| Profile | Events | Request rows | Roots (completed compactions) | Rows keeping the base's full length | Kept / total messages | Inline messages per row | Previous request alive at append |
| --- | --- | --- | --- | --- | --- | --- | --- |
| plain | 1,001 / 3,001 / 10,001 | 200 / 600 / 2,000 | 1 (0) | 100% | 99.0% / 99.7% / 99.9% | 1.00 | 100% |
| tools | 1,009 / 3,007 / 10,009 | 224 / 668 / 2,224 | 1 (0) | 100% | 99.1% / 99.7% / 99.9% | 1.00 | 100% |
| product window | 3,007 / 10,017 | 578 / 1,926 | 10 (9) / 33 (32) | 98.4% / 98.3% | 97.4% / 97.4% | 1.45 / 1.48 | 100% |

"Rows keeping the base's full length" counts every request row except the first; in the product window the
remainder is exactly the 9 and 32 rows right after a completed compaction. Per row, a `model/call` inlines no message,
a `context/prepare` inlines two (the model output and the next input or tool result), the first request one, and a
root after a compaction ten in this profile.

- Outside the request right after a compaction, every request extends its base: `keep` equals the base's length.
  Each root is the first request of the Session or the request right after a completed compaction.
- A `model/call` keeps its `context/prepare`'s message `Arc`s, so its prefix is found by pointer. A `context/prepare`
  shares no pointer with the previous `model/call`: the renderer builds new messages each step, so its prefix is found
  by `==`. The value comparison is part of the v17 write cost in §6.3.
- `system` and `tools` were stored inline only in roots.
- At every request append, including the first request of a turn after the Session went idle, the previous request
  on the branch was still alive. The probe observes liveness, not the holder; ContextPreparation and ContextPolicy
  each keep their latest prepared request, and the Timeline keeps recent events in its hot window. A `Weak` memo
  therefore always hits in a running Session.
- No run had a request in a before trace.

### 4.3 Not measured

- Real provider adapters (`ModelProviderPlugin::process`, aimux) and a non-identity preparation host: the profiles
  use `IdentityContext` and an echo driver. A host that appends per-request messages, which the preparation rule
  allows, stores those messages again in each `model/call` row.
- Prompt-source revisions and tool adoption during a Session: each stores `system` or `tools` inline once by
  construction.
- Before callbacks on request events: the product assembly registers none (only tests do, on `model/call`). Drafts
  are covered by the acceptance tests (§18).
- TypeScript plugins and the CLI product profile with a real model.

**Assumption.** Consecutive requests on a branch extend each other between compactions. If a producer rewrote an
early message every step, each row would store its request inline from the first changed message, so storage per row
would again grow with the request: by message bytes, where v16 grows by 67 bytes per reference. Nothing in the product
does that today. It would also defeat provider prompt caching and ContextPolicy's baseline, and ContextPreparation
already refuses it from the preparation host. The acceptance in §18 measures bytes per event so a regression shows.

## 5. Encoding

### 5.1 Base, keep and roots

- **Request event**: a `context/prepare` or `model/call` event; its `data.request` is the request. A draft in a
  before trace is never a base for another record.
- **Logical history** of branch `b`: the ancestor prefixes up to their cuts, then `b`'s own events. This is the
  ancestry `[(branch_id, cut)]` the runtime builds from the branch directory, which the store holds as `parent` and
  `branch_point` in `timeline_branches.info`.
- **Base** of a request event at sequence `s` on `b`: the last request event before it in `b`'s logical history, that
  is the last one on `b` before `s`, else the last one at or before the cut on the nearest ancestor that has one.
- **keep**: the length of the longest common prefix of the two `messages` lists. Each pair is compared with
  `Arc::ptr_eq` first, then with `Message`'s `==`.
- **Root**: there is no base, or `keep` would be 0. A root stores every message, `system` and `tools` inline.
- **`system` and `tools`**: absent → `null`; present and equal to the base's value (pointer, then `==`) → `"base"`;
  otherwise inline.
- **Drafts**: a request in the record's before trace (`WireTrace.original`, each non-null `WireStep.result`) is
  encoded the same way against its own record's event request. That request always exists, since a draft has its
  event's kind.

The writer always takes the immediately previous request and the longest prefix, so the bytes are a function of the
logical history and the event. Equality is the types' own `==`, which is also how the storage acceptance compares
events (§18). One consequence: a JSON object inside a message that is equal in value but serialized with a different
key order counts as kept, and reads back in its base's key order. Consecutive requests render their messages from the
same events, so this does not happen in practice, and no reader depends on key order.

Why a request with `keep` 0 is a root: otherwise a `"base"` system or tool list would follow the chain past the last
compaction, back to the first request of the Session. With the rule, every chain ends at the latest root, which is at
most one compaction back (§7.4).

### 5.2 Persisted JSON types

The v16 `Row`, `WireTrace`, `WireStep` and `References` are unchanged. v17 adds one optional field to `WireEvent` and
to `WireDraft`:

```text
WireEvent    = { event: AgentEvent, refs: References | null, delta?: RequestDelta,
                 trace: WireTrace | null, batch_end: integer }
WireDraft    = { event: EventDraft, refs: References | null, delta?: DraftDelta }
RequestDelta = { base: Position | null, keep: integer, messages: Message[],
                 tools: Inherit<ToolDefinition[]>, system: Inherit<SystemPrompt> }
DraftDelta   = { keep: integer, messages: Message[],
                 tools: Inherit<ToolDefinition[]>, system: Inherit<SystemPrompt> }
Position     = { branch?: string, sequence: integer }
Inherit<T>   = null | "base" | { inline: T }
```

- `delta` is present only on a v17 `context/prepare` or `model/call` record and on its drafts, which then have
  `refs: null`. A v16 request record has `refs` and no `delta`. Every other record keeps `refs: null` and has no
  `delta`, so its bytes are exactly what v16 writes.
- As in v16, the stored `data.request` has `messages: []` and no `tools` or `system`; its other fields stay in place.
- `Position.sequence` is the base's sequence on `Position.branch`. `branch` is omitted when the base is on the
  record's own branch. A position carries no Session id, so a fork's byte copy stays valid (§9).
- `messages` holds the request's messages from index `keep` on, each as the v16 `message` content body
  (`{role, content}`).
- `base: null` (a root) requires `keep: 0` and no `"base"`.
- `{inline: T}` wraps the value because a system prompt may itself be a JSON string.
- A `DraftDelta`'s base is its record's event request, so it has no `base` field.

### 5.3 Example

Three request rows of a Session: a root on `main`, the next prepare on `main`, and the first prepare of a child cut
at `main` 10. Displayed as JSON; the column stores the string.

```json
[
  {"branch_id": "main", "sequence": 4, "kind": "context/prepare", "body": {"encoding": "event", "record": {
    "event": {"id": "e4", "sequence": 4, "create_at": 1004, "type": "context", "name": "prepare", "data": {
      "turn_id": "t1", "request_id": "r1", "through": 3,
      "request": {"providerId": "mock", "modelId": "deterministic", "messages": []}}},
    "refs": null,
    "delta": {"base": null, "keep": 0, "messages": [{"role": "user", "content": "Hi"}],
              "tools": null, "system": {"inline": "You are helpful."}},
    "trace": null, "batch_end": 4}}},
  {"branch_id": "main", "sequence": 9, "kind": "context/prepare", "body": {"encoding": "event", "record": {
    "event": {"id": "e9", "sequence": 9, "create_at": 1009, "type": "context", "name": "prepare", "data": {
      "turn_id": "t2", "request_id": "r2", "through": 8,
      "request": {"providerId": "mock", "modelId": "deterministic", "messages": []}}},
    "refs": null,
    "delta": {"base": {"sequence": 5}, "keep": 1,
              "messages": [{"role": "assistant", "content": "Echo: Hi"}, {"role": "user", "content": "Again"}],
              "tools": null, "system": "base"},
    "trace": null, "batch_end": 9}}},
  {"branch_id": "child", "sequence": 3, "kind": "context/prepare", "body": {"encoding": "event", "record": {
    "event": {"id": "c3", "sequence": 3, "create_at": 1013, "type": "context", "name": "prepare", "data": {
      "turn_id": "t3", "request_id": "r3", "through": 2,
      "request": {"providerId": "mock", "modelId": "deterministic", "messages": []}}},
    "refs": null,
    "delta": {"base": {"branch": "main", "sequence": 10}, "keep": 3,
              "messages": [{"role": "user", "content": "Follow up"}],
              "tools": null, "system": "base"},
    "trace": null, "batch_end": 3}}}
]
```

`main` 5 is the `model/call` of `r1` (`{"base": {"sequence": 4}, "keep": 1, "messages": [], "tools": null, "system":
"base"}`) and `main` 10 the `model/call` of `r2`. The chains:

```text
 main                                                      child (parent main, cut 10)
  4 context/prepare  ROOT  [u1] system inline
  5 model/call       base 4, keep 1
  9 context/prepare  base 5, keep 1, [a1 u2]
 10 model/call       base 9, keep 3  <-----------------------  3 context/prepare  base main 10, keep 3, [u3]
 14 context/prepare  base 10, keep 3, [a2 u4]                  7 model/call       base 3, keep 4
```

## 6. Writing

### 6.1 Finding the base

The writer thread encodes each record inside the write transaction. For a request record at `(b, s)`:

1. **Memo hit.** If the memo (§6.2) has an entry for `b` whose request is still alive, that entry is `b`'s last
   request row, and its request is the base.
2. **Otherwise** the writer finds the position: the last request row on `b` before `s`, scanning the primary key
   backwards over the rows since the previous model call, else the last one at or before the cut on each ancestor
   in turn, using the parents and cuts in `timeline_branches.info`. It then decodes that row with the chain walk of
   §7.2 inside the same transaction, which also sees rows written earlier in that transaction.
3. **No request found**: the record is a root.

Then the writer computes `keep`, `system` and `tools` (§5.1) and serializes only the inline part. The same steps
encode each request draft against the record's event request, which is in memory.

### 6.2 Writer memo

One entry per `(Session, branch)`: the sequence of the last request row the writer encoded on that branch and a
`Weak<ModelRequest>` to its request.

- The memo holds no request: when the runtime drops the request, the entry is dead. Dead entries are removed at the
  end of each write round, so the memo holds one small entry per branch whose last request is still in memory.
- A transaction that fails or rolls back removes the entries of its Sessions, and so do `rollback_session` and a
  delete. A removed entry only costs the next write the lookup of §6.1 step 2.
- The memo decides no byte. On a hit the base is the in-memory request; on a miss it is decoded from the store. The
  comparison is by value and the inline part is always serialized from the record being written, so both produce the
  same bytes (§18 determinism). It is storage-internal acceleration, not business state.

Measured (§4.2): the previous request was alive at every request append in every profile, so a running Session writes
without reading. After a restart, an unload or a fork, the first request write of each branch pays one chain walk
inside the write transaction, once per branch; §7.4 gives its cost.

### 6.3 Work per request row

| | v16 | v17 |
| --- | --- | --- |
| Serialize | Every message, tools and system | Only `messages[keep..]` and a changed `system` or `tools` |
| Hash | SHA-256 of each serialized content | None |
| SQL | One `INSERT OR IGNORE` per content, then the row | The row |
| Compare | None | Prefix with the base: by pointer for `model/call` rows, by `==` for `context/prepare` rows (§4.2) |
| Measured before SQL, median of the last 200 request rows at ~10k events | plain 539 µs, tools 948 µs, product window 495 µs | plain 24 µs, tools 47 µs, product window 39 µs |

v17 writes no `timeline_content` row. Output-row merging and every other record are unchanged.

## 7. Reading

### 7.1 In order

Each read keeps the last request it decoded and that request's position: one per call of `read_segments` or
`read_page`, one per branch in a `validate_session` pass, carried across pages as #213 carries its cache. For each
request row:

- a **root** decodes its inline parts;
- a **delta whose base is the kept position** builds `messages` as the kept request's first `keep` message `Arc`s
  followed by the inline messages, and takes `system` and `tools` from the kept request or inline;
- a **v16 row** decodes through `refs` and #213's bounded content cache;
- a **delta with any other base** walks the chain (§7.2).

The decoded request then becomes the kept one. Drafts decode against their record's request. A read that follows the
logical history (`context`, `history`, full `local` and restore reads) finds every base kept, except the first request
of a segment that the runtime reads in a separate call.

### 7.2 Random access

A base that is not kept costs one recursive query over the primary key, from the base back to the nearest root, v16
row or kept request, followed by decoding the chain oldest first:

```sql
WITH RECURSIVE chain(branch_id, sequence, delta, depth) AS (
  SELECT branch_id, sequence, json_extract(body, '$.record.delta'), 0
    FROM timeline_events WHERE session_id = ?1 AND branch_id = ?2 AND sequence = ?3
  UNION ALL
  SELECT e.branch_id, e.sequence, json_extract(e.body, '$.record.delta'), c.depth + 1
    FROM chain c JOIN timeline_events e
      ON e.session_id = ?1
     AND e.branch_id = coalesce(json_extract(c.delta, '$.base.branch'), c.branch_id)
     AND e.sequence = json_extract(c.delta, '$.base.sequence')
   WHERE c.depth < ?4 AND NOT (c.branch_id = ?5 AND c.sequence = ?6))
SELECT branch_id, sequence, delta FROM chain ORDER BY depth DESC
```

`?4` is the number of rows in the Session, so a damaged chain cannot loop; `?5`, `?6` stop at the kept request. A row
without a delta ends the chain: it is a v16 row, decoded through its references. `EXPLAIN QUERY PLAN` shows a primary-key
search for every step. Each step is validated in Rust (§8).

Random access happens once per read call that does not start at the beginning of the logical history: a page that
starts mid-history, a read by event id, `trace`, a snapshot anchor, a checkpoint suffix read from its `resume`, a
branch's own segment (read apart from its ancestors), a child's first request in a validation pass, and a writer memo
miss. It does not happen per row.

### 7.3 Read path

```text
 read call (read_segments, read_page, a validation pass, read by id, trace)
   kept = none
   for each row, in sequence order:
     v16 request row  --refs--> content rows (#213 cache) ------------------------> request --+
     v17 root         --inline parts--------------------------------------------> request --+
     v17 delta, base = kept position                                                         |
                      --kept.messages[..keep] + inline; system/tools "base" or inline --> request --+
     v17 delta, other base                                                                    |
                      --one recursive PK query: base -> ... -> root | v16 row | kept         |
                        decode oldest first, then this row --------------------------> request --+
                                                                              kept = request <--+
     drafts of the row: their record's request + DraftDelta
```

### 7.4 Chain length, and why there is no bound

A chain runs from a request back to the latest root before it: at most the request rows since the last compaction or
the start of the Session. Each request row adds at least one message in normal operation (a turn's input, the model's
output, a tool result), retries aside, so the chain has about as many rows as the decoded request has messages. A
v16 random access to the same request does one content lookup and SHA-256 check per message, so both are linear in
the request; v17 replaces content lookups with primary-key steps.

Measured on the probe's databases (§12), median of five; the request is the last `model/call` of the Session, and the
v16 read is the store's own read of that one event on `main`:

| Profile | Events | Chain rows back to the root | v16 read of the event | v17 chain query and decode (prototype) |
| --- | --- | --- | --- | --- |
| plain | 1,001 | 200 | 0.48 ms | 0.50 ms |
| plain | 3,001 | 600 | 2.7 ms | 1.7 ms |
| plain | 10,001 | 2,000 | 9.2 ms | 8.8 ms |
| tools | 10,009 | 2,224 | 11.1 ms | 11.3 ms |
| product window | 3,007 | 27 | 0.48 ms | 0.23 ms |
| product window | 10,017 | 41 | 0.69 ms | 0.32 ms |

The same walk serves a writer memo miss (§6.2): about 9 ms inside the write transaction with 2,000 request rows since
the last root, once per branch after a restart, and well under a millisecond when compaction runs.

A depth bound would be a forced root every N requests: the whole request (up to the window) stored again each time,
which brings back storage growth with history to save a cost that already equals v16's for the same request and is paid
once per read call. No bound.

One cost remains quadratic in the number of calls between compactions: every decoded request carries all its message
pointers, as the event type requires (`messages: Vec<Arc<Message>>`). A full-history read of 10k plain events builds
2,000,000 pointers, about 7.7 ms of the prototype's 29.6 ms (§12). v16 builds the same vectors; v17 removes the
lookups and parsing behind them. Phase 3 (§15) removes the repetition of whole reads during a restore.

## 8. Validation and damaged rows

For each v17 request record a read checks, failing with a storage error otherwise:

- a request event has exactly one of `refs` (v16) or `delta` (v17), with `refs: null` when `delta` is present; any
  other event has neither; drafts use their record's encoding;
- with `base: null`: `keep` is 0 and neither `system` nor `tools` is `"base"`;
- with a base: the row exists; it is a single-event row of kind `context/prepare` or `model/call`; it is earlier in
  the logical history, meaning a smaller sequence on the same branch, or a branch on this branch's ancestry at or
  before that ancestor's cut (checked against `timeline_branches.info`);
- `keep` is at most the base's message count; `"base"` for `system` or `tools` requires the base to have one;
- the delta parses as the types above, and the decoded request parses as `ModelRequest`.

A missing or damaged base fails the read explicitly ("missing request base", "invalid request base"); the request is
never restored empty or partial. The bad row keeps its bytes: reads never write. `validate_session` applies the same
checks to every row, so a Session with a broken chain is refused at restore and preserved, as a missing content row is
today. The reach of one damaged row is the same as in v16: every later request that depends on it (in v16, every
request that references a damaged message) until the next root.

## 9. Branches, fork and delete

- **Cross-branch bases.** A child's first request is based on the last request at or before its cut on the nearest
  ancestor that has one, addressed with `branch`. Later requests on the child are based on the child's own rows. A
  parent that keeps writing after the cut does not affect the child: its rows after the cut are not in the child's
  logical history.
- **Fork** ([S5c §6.3](session-deletion-and-fork.md#63-what-a-fork-copies)). `main` rows through the cut are copied
  byte for byte. `main` has no ancestors, so the base of every copied row is an earlier `main` row, which is copied
  too. Copied child directories have head 0 and no events. v16 rows still need their content rows: the fork keeps
  copying the content rows that the copied rows reference, and v17 rows reference none. The fork transaction is
  unchanged.
- **Delete and creation rollback** remove the Session's rows from every table, as today; the writer drops the
  Session's memo entries.

## 10. Compaction

A completed compaction changes the head of the list, so the next request shares no prefix with its base. It is a
root and stores the replacement, the baseline, the retained tail, `system` and `tools` inline once, which is bounded by
the model's window. Following requests extend it. In the product-window profile each completed compaction produced
exactly one root (§4.2).

```text
 seq  event                      messages                         stored
  37  model/call                 [u1 a1 ... u19]                  base 36, keep 37
  40  context/prepare            [u1 a1 ... u19 a19 u20]          base 37, keep 37, [a19 u20]
  41  context/compaction-start
  42  context/compaction         completed: replacement S, retained from u18
  43  model/call                 [S b1 u18 a18 u19 a19 u20]       ROOT: all 7 inline once, at most the window
  50  context/prepare            [S b1 u18 ... u20 a20 u21]       base 43, keep 7, [a20 u21]
```

A checkpoint read starts at the snapshot's `resume`, which lies before the compaction. Its first request row walks
the chain back to the previous root, at most the request rows between two compactions; the rows after the compaction
chain to the new root.

## 11. Format v17

**Decision: version 17, an in-place marker bump from exactly v16. No table, column or index changes, and no business
row is read or rewritten.** Recorded as proposed in [native-storage-format.md](native-storage-format.md#格式版本); the
full format decision moves there with the implementation.

- **Why a new version.** A v16 daemon cannot read a v17 request row: `WireEvent` and `WireDraft` reject unknown fields,
  and without `delta` the request would be empty. The format keeps a version only when old readers interpret the new
  data.
- **Schema.** The v16 tables as they are; `agent_store_format` becomes `CHECK (version = 17)`. `timeline_content`
  stays: it holds v16 rows' content, which v17 still reads. A new v17 Session leaves it empty.
- **Mixed rows.** v16 rows stay as written and readable for good. After the upgrade a Session's next request row is
  v17, with its v16 predecessor as its base (`keep` usually its full length). Rows of both versions mix in one branch
  and in one database, and a fork copies them as they are.

| Store found | Action at startup, in the existing admission transaction (`IMMEDIATE`, before writable PRAGMAs) |
| --- | --- |
| Empty | Create the v17 schema with marker 17 |
| Exact v17 schema, marker 17 | Open |
| Exact v16 schema, marker 16 | Recreate `agent_store_format` with `CHECK (version = 17)` and insert 17. Nothing else; a failure rolls back to the untouched v16 store |
| Exact v15 or v13 schema with its marker | The existing steps, then this one, in the same transaction |
| Anything else | Refuse and preserve, as today |

The upgrade is one-way, as S5a and S5b decided: a v16 daemon refuses an upgraded store with the existing "unsupported
SQLite schema … preserved" error. The daemon writes `Upgraded native store <path> from format v16 to v17; earlier
dimcode versions cannot open it.` to stderr once, after the upgrade commits. In `sqlite.rs` this is one entry,
`(16, &[])`, in `UPGRADES`, and `FORMAT = 17`.

No backfill: converting v16 rows would read and rewrite business rows, which an upgrade must not do, and #213 already
bounds their read cost.

## 12. Estimated effect

Projected storage, from the probe's exact v17 delta JSON for each request row; every other row is unchanged. Bytes are
the `body` text of event rows, plus `kind`, `hash` and `body` of content rows for v16.

| Profile | Events | v16 request row | v17 request row | v16 per event | v17 per event | v16 total | v17 total |
| --- | --- | --- | --- | --- | --- | --- | --- |
| plain | 1,001 | 7,248 B | 560 B | 1,720 B | 352 B | 1.72 MB | 0.35 MB |
| plain | 3,001 | 20,650 B | 548 B | 4,398 B | 351 B | 13.2 MB | 1.05 MB |
| plain | 10,001 | 67,551 B | 545 B | 13,778 B | 351 B | 137.8 MB | 3.51 MB |
| tools | 1,009 | 8,052 B | 605 B | 2,097 B | 400 B | 2.12 MB | 0.40 MB |
| tools | 3,007 | 22,928 B | 595 B | 5,401 B | 399 B | 16.2 MB | 1.20 MB |
| tools | 10,009 | 75,055 B | 592 B | 16,985 B | 399 B | 170.0 MB | 3.99 MB |
| product window | 3,007 | 4,575 B | 2,473 B | 1,613 B | 983 B | 4.85 MB | 2.95 MB |
| product window | 10,017 | 4,707 B | 2,506 B | 1,638 B | 989 B | 16.4 MB | 9.91 MB |

Request rows are averages over all request rows; SQLite pages, keys and indexes are not counted. Without compaction,
v16 bytes per event grow with the history and v17 stays flat. With compaction about every 20 turns and 1 KB messages,
v16 is already bounded and v17 still stores 40% less: a v16 row repeats a 67-byte reference for every message of the
request, and a v17 root repeats the retained tail once per compaction.

Reads. The probe also copies each Session into a second disposable database with every request row rewritten to its
v17 form, and decodes it with a prototype v17 reader (row parse, event parse, delta applied to the kept request) that
returns events equal to the v16 store's read of the same Session. Same machine and run; v16 is the store on `main`,
which includes #213. Its 10k reads (149 ms plain, 180 ms tools) match #213's own measurements (146 and 184 ms).

Full-history read of `main`, median of five:

| Profile | Events | v16 store read | v17 prototype read | Message pointers built |
| --- | --- | --- | --- | --- |
| plain | 1,001 | 3.8 ms | 2.3 ms | 20,000 |
| plain | 3,001 | 20.5 ms | 7.0 ms | 180,000 |
| plain | 10,001 | 149.0 ms | 29.6 ms | 2,000,000 |
| tools | 1,009 | 4.4 ms | 2.5 ms | 25,088 |
| tools | 3,007 | 24.3 ms | 7.9 ms | 223,112 |
| tools | 10,009 | 179.6 ms | 32.8 ms | 2,473,088 |
| product window | 3,007 | 15.8 ms | 9.6 ms | 31,759 |
| product window | 10,017 | 59.8 ms | 37.0 ms | 109,576 |

Other work shared the machine during the runs; v16 and v17 alternate within each repetition, so the comparison is
fair, but absolute times carry that noise.

**Estimate (not measured on an implementation).** Fitting the prototype's plain and tool reads to `a'·E + c·P`, with
`P` message pointers, gives c ≈ 3.5–3.9 ns per pointer and a' ≈ 2.2–2.4 µs per event. The implementation validates
each delta and handles branches and output rows as the store does today, so the estimate takes the store's own
per-event cost from §2.3 (a ≈ 2.9 µs) and adds the pointer term.

| At about 10,140 events | Before #213 | With #213 | v17, estimated |
| --- | --- | --- | --- |
| Full-history read, plain | 208 ms | 146 ms | 30–37 ms |
| Restore and first idle, plain | 2,866 ms | 1,779 ms | 0.38–0.47 s |
| Full-history read, tools | 257 ms | 184 ms | 33–39 ms |
| Restore and first idle, tools | 3,624 ms | 2,256 ms | 0.44–0.51 s |
| Request row encode before SQL, plain | 539 µs | 539 µs | 24 µs |

- A full read is about `2.9 µs × 10,140 + c × P`; plain turns build P ≈ 2.06 M pointers, tool turns about 2.5 M.
  The lower bound is the prototype's measured read at 10,001 and 10,009 events.
- A restore does 11 full reads plus `validate_session`, which costs about one read (§2.1, §2.2), and #213 leaves about
  24 ms (plain) and 46 ms (tools) of other work. Restore is therefore about 12 reads plus that.
- A model step that reads its context history from the store pays the per-read figure.
- The residual quadratic term is the pointer vectors: about 8 ms per read at 10k plain events, and about 70 ms at 30k
  if a Session went that long without compacting. Phase 3 (§15) would pay it once per restore instead of twelve
  times.

Acceptance 7 measures these numbers on the implementation.

## 13. Impact surface

Following [principles §3](../../architecture/rust-agent/principles.md#3-正交): the target is zero changes outside the
store and zero new public content.

| Module or content | Change | Reason |
| --- | --- | --- |
| `dim-app-server` storage (`codec.rs`, `timeline.rs`, `sqlite.rs`) | Encoding, decoding, the writer memo, the chain query, `FORMAT = 17` and one upgrade step | The format is storage's own concern (timeline-storage §2) |
| `TimelineStore` trait, Timeline, Dispatcher, event types, fixtures, capabilities, protocols | None | §3 |
| Public content added | None | The persisted JSON is internal to the store and versioned by its marker; no event, service, method, wire field or configuration |

## 14. Rule check

| Rule | Answer |
| --- | --- |
| [Principles §4](../../architecture/rust-agent/principles.md#4-通过时间线扩展行为) rule 1 (command events for others' state) | Not applicable: no state of another module changes |
| §4 rule 2 (handover) | Not applicable: nothing is handed over |
| §4 rule 3 (before callbacks) | None added or changed. Callbacks still receive and return complete drafts; the store encodes the request drafts of a trace against their own record's request (§5.1) when it writes the record, after the before chain has run |
| §4 rule 4 (no business meaning in core) | The store encodes `ModelRequest` structurally (positions, prefix lengths); it interprets no message and no business rule. Which events carry a request was already the store's knowledge in v16 |
| §4 rule 5 (atomic batches) | Unchanged. A base is always committed in the same or an earlier transaction, since a batch commits whole and the base precedes the row; subscribers see the same events |
| §4 rule 6 (model context) | Unchanged: reads return the same events, so context is the same |
| §4 rule 7 (public events and services) | The store reads no plugin's state; the writer memo is the store's own |
| §4 rule 8 (infrastructure unchanged) | The Timeline, Dispatcher, event envelope and format, assembly and facades are unchanged. The requirement comes from the store itself (its read and write cost), and the change and its verification stay inside the store |
| §4 rule 9 (only decision facts in the Timeline) | No new event or fact; the encoding is not a Timeline fact |
| §1 (Capability and App Server boundary) | Inside the existing SessionStore plugin's `TimelineStore` implementation; no new plugin |
| [ADR 002](../../adr/002-plugin-scope.md) D1 (scope follows state) | Process scope: the store and its writer thread |
| ADR 002 D2 (no wiring modules) | No new module or plugin |
| ADR 002 D3 (dependencies follow scope) | No new dependency |
| ADR 002 D4 (configuration) | None |
| ADR 002 D5 (language-neutral contracts) | No contract changes; §5.2 defines the stored JSON as data |

## 15. Out of scope, and requirements to mechanism owners

None raised. Out of scope, as separate work:

- **Phase 3: restore-scoped shared read.** During a Session restore every capability reads `local(..=S)` for the same
  `S`. A read shared by the restore, decoded once and released when the Session publishes, would remove the remaining
  multiplier without changing a plugin. It is the runtime owner's decision and will be proposed with numbers once v17
  has landed.
- Converting existing v16 rows (§11).
- Event-type changes of any kind (§17).

## 16. Decisions

| # | Decision | Rejected alternative | Why |
| --- | --- | --- | --- |
| D1 | The base is always the immediately previous request in the logical history | The earlier request with the longest prefix; any request the writer still holds | One deterministic rule; a search makes the bytes depend on the writer's state and costs a scan per write |
| D2 | `keep` 0 makes a root | Allow a base with no shared message | Bounds every chain at the latest root, so a `"base"` system or tool list never follows the chain past a compaction (§5.1) |
| D3 | Base addressed as `{branch?, sequence}` inside the Session | An event id; a Session-qualified address | A primary-key step per hop, no index; positions survive the fork's byte copy (§9) |
| D4 | Drafts are encoded against their record's request, with no `base` field | Drafts as their own chain; drafts as v16 references | The record's request is in memory on both sides, and drafts never become bases |
| D5 | `system` and `tools` as `null`, `"base"` or `{inline}` | Their own references to the row holding the value | With D2 the chain already reaches the row holding them; one more address kind is not needed |
| D6 | Writer memo of a position and a `Weak` per branch, dropped on a failed transaction and pruned when dead | A strong memo; no memo | A strong memo would hold the last request of every Session the process ever wrote; without a memo every write reads its base back (§6.2) |
| D7 | No depth bound | A forced root every N requests | §7.4 |
| D8 | No SQL change; the chain query uses `json_extract` on the primary key | A `base` column or an index | An upgrade adds structures only, and the chain needs nothing but the primary key |
| D9 | v16 rows stay as written | Converting them at upgrade or lazily | An upgrade reads or rewrites no business row (§11) |
| D10 | `refs: null` stays on v17 request records | Omit `refs` in v17 | One `WireEvent` shape; every non-request record keeps its v16 bytes |

## 17. Rejected alternatives

- **Content-addressed list nodes** (each message a node pointing to the previous node's hash): still hashes every
  write, and a request whose prefix changes rewrites O(k) nodes.
- **Deriving the request instead of storing it**: `model/call` is the final request handed to the model (trace.md),
  and ContextPolicy reads the exact latest request; a derivation is a second producer of the same fact.
- **Lazy request handles in events**: changes the event type and every consumer, and moves storage errors into
  readers that expect validated data (AGENTS.md data conventions).
- **Kind-filtered reads** (skipping request events for capabilities that ignore them): changes the read API and each
  capability's rebuild, and still leaves the quadratic storage.
- **Capability snapshots** for rebuild: business state for an optimization (timeline-storage §4) in every plugin.
- **A store-wide decoded-event cache**: residency would grow with every loaded or recently read Session, against the
  idle-branch acceptance.
- **Changing `ModelRequest.messages` to a persistent type**: changes the event type and its frozen contract for the
  pointer-copy term only (§7.4).

## 18. Acceptance

The implementation PR passes all of these, on disposable databases only.

Carried from [timeline-storage §7](../../architecture/rust-agent/timeline-storage.md#7-验收):

1. **Storage round trip.** Events read back equal the written ones field by field, covering output rows appended
   across write rounds, atomic batches and before traces, plus: roots, `keep` 0 (compaction), partial and full;
   `system` and `tools` as `null`, `"base"` and inline; request drafts unchanged, with an appended message, with an
   early message changed, and with a changed system prompt; cross-branch bases from a parent and from a grandparent,
   with the parent writing past the cut; mixed rows (a v16 Session continued after the upgrade, its first v17 row based
   on a v16 row); and a fork at a cut of a mixed `main`, whose reads equal the source's prefix.
2. **Long-session residency.** 2,000 steps with an 800 KB context: an idle branch's live bytes do not grow with steps,
   and the peak during preparation does not grow with history before the compaction. After a Session unloads, the
   writer memo holds no live request of it.
3. **Paging and restore.** Long page reads do not decode the whole history first; a page starting mid-history and a
   checkpoint read from a snapshot's `resume` each decode their first request with one chain query; merged rows, batches and cross-page tool calls stay correct. Restore caches
   are bounded (one kept request per read, #213's cache for v16 rows), and a long cold read does not hold the
   connection so as to block writes.
4. **Process interruption and power loss.** Killed before commit, after commit and before publish, and after publish:
   the recovered events read back, sequences are contiguous, and no row has an uncommitted base. Writes after the
   restart (cold memo) produce the same bytes as an uninterrupted run.
5. **Storage failure.** Injected read and commit failures behave as today; after a failed commit the retried and later
   batches encode against committed rows only, with the bytes of a run without the failure.
6. **Corruption.** A missing base, a base that is later, on a branch outside the ancestry, past the cut, of another
   kind or an output row, `keep` past the base's length, `"base"` without a base value, a malformed delta and a cycle
   each fail `read`, `read_page` and `branches` with a storage error; the row keeps its bytes, and no request reads
   back empty. #213's content corruption cases still fail.

New:

7. **Many model calls between compactions.** Plain and tool Sessions grown to about 1k, 3k and 10k events without
   compaction (`restore_cost`). Report restore with first idle, per-read time and bytes per event at each size next to
   #213's numbers. Bytes per event at 10k stay within 10% of 1k, and at 10k plain events the per-read time
   is at most a third of #213's and restore with first idle at most half (1,779 ms on the machine used here), measured
   on the same machine.
8. **Determinism.** The same history, including a child branch, a compaction and request drafts, written once with
   the writer memo and once with it disabled (a `testing` hook), and once with a restart in the middle, gives identical
   `timeline_events` rows byte for byte.
9. **Frozen contracts.** `crates/dim-agent-types/tests/fixtures/` and the `*_contract.rs` tests are unchanged by the
   diff and pass.
10. **Upgrade.** An empty store is created at v17. A disposable v16 store upgrades in place to the exact v17 schema with
    every row unchanged byte for byte and reports it once; v15 and v13 stores reach v17 in one transaction; wrong
    markers, partial upgrades, older and unknown schemas are refused and preserved; the v16 daemon's refusal of a v17
    store is the existing error.

## 19. Changes to other documents

With this design: the [documentation index](../../INDEX.md) links this page, and
[native-storage-format.md](native-storage-format.md#格式版本) names v17 as the proposed next version.

With the implementation: [native-storage-format.md](native-storage-format.md) (current version, marker, the
`timeline_content` row now holding v16 content only, the persisted JSON types, read validation, the v17 decision and
the record example), [timeline-storage](../../architecture/rust-agent/timeline-storage.md) §2 (request content and
reads), §4 (restore row) and §7 (acceptance rows), [ADR 001](../../adr/001-harness-foundation.md) (current format), and
this page's status.

## 20. Open questions for reviewers

1. **Value equality for `keep`.** The writer compares messages with `Message`'s `==`, under which JSON objects with
   the same entries in a different key order are equal, so such a message reads back in its base's key order (§5.1).
   Byte-exact comparison would serialize the kept prefix of every `context/prepare` row, whose messages never share
   pointers with the base (§4.2), which is the write cost v17 removes. Recommendation: `==`. Does any reader depend on
   key order inside message content?
2. **How strict validation is about the base.** §8 accepts any earlier request event in the logical history as a
   base; "always the immediately previous request" (D1) is a writer rule that acceptance 8 tests. Checking D1 on read
   would catch a writer bug at restore, but would refuse a store that decodes correctly, and any later change to the
   writer rule would become a format change. Recommendation: validate what decoding needs, test D1.
3. **Phase 3 owner.** A restore-scoped shared read (§15) belongs to the runtime. Who owns it, and should it wait for
   v17's measured numbers?
4. **One-way upgrade without a backup.** S5a and S5b decided this during development, and v17 follows them. Does that
   still hold for the release that ships v17, or should the upgrade keep a copy of the v16 database?
