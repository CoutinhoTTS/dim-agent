# Bounded latest-turn history window

Status: Draft, design for [#712](https://github.com/arcships/dim/issues/712); no
implementation yet. Scope: the protocol v3 `session.history` read and the
persistence reads behind it. It adds no event, service, plugin or process state.
The process-caller read stays as it is; see [Session access A.1 and A.5](../contracts/session-access.md#a1-the-sessions-service)
and [sessions service §6.7](sessions-service.md#67-history).

## 1. Behavior today

### 1.1 The read

`session.history {sessionId, branchId?, after, limit, through?}` decodes into
`protocol::History` ([protocol.rs:504](../../../crates/dim-app-server/src/protocol.rs#L504))
and is served by `Connection::history`
([connection/tree.rs:75](../../../crates/dim-app-server/src/connection/tree.rs#L75)):

1. It pins `through`: the request's value, or the branch's published head. A
   value beyond the branch's dispatched position fails `invalid`; the call then
   waits until the branch dispatches up to `through`.
2. It reads `Session::local_page(branch, after, limit, through)`
   ([agent.rs:530](../../../crates/dim-agent-runtime/src/agent.rs#L530)), which
   rejects a `through` beyond the published head, then calls the store's
   `read_page` ([timeline_store.rs:156](../../../crates/dim-agent-runtime/src/timeline_store.rs#L156),
   SQLite [storage/timeline.rs:2383](../../../crates/dim-app-server/src/storage/timeline.rs#L2383)).
   `after` must end a batch. `limit` is required and must be positive; the v3
   method sets no upper bound today, and clients send 256 or 512. A page takes
   `limit` events and then runs to the end of that batch, so it can exceed
   `limit`; it never passes `through`.
3. It replies `{events, through, nextAfter, done}`
   ([tree_protocol.rs:142](../../../crates/dim-app-server/src/tree_protocol.rs#L142)).
   `events` are the branch's own events, ascending by sequence, filtered to the
   kinds `ProductEvent` shows for the connection's protocol version
   ([tree_protocol.rs:32](../../../crates/dim-app-server/src/tree_protocol.rs#L32));
   the cursors use the raw sequences, so a page can be sparse and still be
   complete.

The method is protocol v3 only
([connection.rs:408](../../../crates/dim-app-server/src/connection.rs#L408)).
Only the `sessions` service caps its own pages at 256 for the bridge
([sessions service D11](sessions-service.md#3-decisions)).

Older events are reached only by a forward page with `after` set to a batch
boundary. The CLI and the web client read a branch from its start to its head,
`ceil(head / limit)` calls, when they open it
([CLI session.rs:442](../../../crates/dim-cli/src/session.rs#L442),
[web chat.ts:290](../../../web/src/stores/chat.ts#L290)); a client that wants the
newest content has to find a batch boundary near the head and walk back from
there, which is what the Desktop reader does (§1.3).

### 1.2 The turn boundary and its input

Every turn begins with one `turn/start` that Loop appends on its own
([agent_loop.rs:181](../../../crates/dim-agent-capabilities/src/agent_loop.rs#L181),
`append` is a one-event batch). So a turn start is a batch start, and the
sequence before it ends the previous batch. A running turn has no `turn/end`;
its start is still the newest `turn/start` of the branch.

The input that drives the turn is written before it: `input/message` carries the
user's content and is appended when the client submits
([tree.rs:19](../../../crates/dim-app-server/src/connection/tree.rs#L19)); Loop
then writes `input/consumed` for each input its first request takes
([agent_loop.rs:415](../../../crates/dim-agent-capabilities/src/agent_loop.rs#L415)),
before that request's `step/model-start`. A client renders the user row at the
`input/message` event ([CLI projection.rs:110](../../../crates/dim-cli/src/projection.rs#L110),
[web projection.ts:60](../../../web/src/projection.ts#L60)), so a window that
starts at `turn/start` alone would show a turn without its prompt.

Nothing indexes the kind. The store can page by sequence and by event id, and it
has a partial index for `step/tool-use-start`
([sqlite.rs:90](../../../crates/dim-app-server/src/storage/sqlite.rs#L90)), but a
reader that wants the newest `turn/start` can only read events until it meets
one.

### 1.3 What a client that opens a thread pays

The cost measured in [#712](https://github.com/arcships/dim/issues/712) with the
Desktop reader against a native store: a thread open reads the newest page, then
walks back until it meets a `turn/start` or the previous turn's end.

| main head | history pages | App Server history time | open, cold | open, warm | client fold |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1,956 | 1 | 67 ms | 410 ms | 82 ms | 4 ms |
| 3,122 | 6 | 258 ms | 1,193 ms | 335 ms | 8 ms |
| 6,862 | 13 | 420 ms | 1,021 ms | 478 ms | 13 ms |

The newest 512-event pages of the 3,122- and 6,862-event Sessions held 490 and
499 `step/thinking` events and no `turn/start`, so the walk back is bounded only
by the turn's own size. The client's fold after the pages arrive takes 1 to
13 ms: the cost is the reads, not the client.

## 2. Problem

The turn boundary is a stored, server-side fact, but the protocol has no way to
name it. A client cannot ask for "the newest turn" or "the turn that contains
the head", so a long turn makes opening a thread an unbounded backward read, and
the reply that reaches the client is a sequence of pages whose count grows with
the turn. A fixed tail window does not solve it: a tail can stop in the middle
of a turn, lose the `turn/start`, or both, and it does not know which inputs the
turn consumed.

## 3. Decisions

| # | Decision | Rejected alternative | Why |
| --- | --- | --- | --- |
| D1 | Extend `session.history` with two optional request fields (`latestTurn`, `before`) and two optional reply fields (`turnStart`, `nextBefore`) | A new method, `session.history.latest` | The watermark, the batch rules, the v3 event filter and the error set are the same read. A second method would duplicate them, need its own feature entry and split clients between two paging loops. Older clients ignore the new reply fields, and a request without the new fields is today's read (D9) |
| D2 | Window boundaries are sequence bounds, not opaque cursors | An opaque cursor or turn id | Clients already hold sequences, pin a watermark and key events by sequence. A turn id would need a second lookup and would not fix the paging; a sequence is the same currency as `after`, `through` and `nextAfter` |
| D3 | The window is `(windowStart - 1, through]`: the newest turn at or below `through` together with the `input/message` events that turn consumed (§4.2); with no `turn/start` the window starts at the branch start | Start at `turn/start` only | The prompting input precedes the turn start, and clients render it as their user row (§1.2). A window from `turn/start` gives a turn without its prompt. A fixed number of events before the turn start is arbitrary: a queued input can be much older |
| D4 | A `latestTurn` page is the newest part of the window, up to `HISTORY_TURN_LIMIT` (4,096) events; when the window is larger, the reply is its newest 4,096 events and `nextBefore` names the cursor for the rest | Always return the whole turn, or impose no cap and let the client's `limit` decide | A turn has no size bound. The v3 method has no upper bound today, so this mode carries one; otherwise a client could ask for "the latest turn" of an arbitrarily long turn and materialize it in one reply. 4,096 events makes the measured 1,956- and 3,122-event turns one reply and the 6,862-event turn one reply plus one backfill page |
| D5 | `before` adds backward paging: the page ends at the last batch end at or below `before - 1`, and starts at a batch start, so `after = from - 1` reads the same page forward | Read the newest page as `after = head - N`, as [#276](https://github.com/arcships/dim/issues/276) suggested | `after` must end a batch; `head - N` need not, and when it does not the read fails with "cursor is inside an atomic batch", so the client has to search for a boundary. A backward page has to walk batches from their ends |
| D6 | The turn start and the prompting input are store lookups through one partial index, not a scan and not state rebuilt during restore | Fold the newest turn start and the consumed inputs during restore validation; keep them in process memory | [#601](https://github.com/arcships/dim/issues/601) makes a re-acquired Session skip validation, so a value rebuilt there would be missing exactly when the read is cheap. Process memory would be dropped on unload. The lookups are single statements, independent of the turn's size |
| D7 | A whole-history read keeps using forward paging; the window is opt-in | Replace `after`-paging with backward paging | Clients that show scrollback (CLI, web, export) read from the start and need no window; replacing the direction would change every one of them and the export's fixed-prefix rule |
| D8 | `sessions.history` (the process service) does not gain the fields | Mirror the fields in both reads | It has no consumer that opens a thread; the fields are a client read. Adding them would advertise a contract nothing uses, and the service can adopt the read when a consumer needs it |
| D9 | All new request and reply fields are optional; a request without them keeps today's meaning, limits and errors | A protocol version bump | The wire stays v3; replies only gain fields, and request decoding is strict only for fields the client sends. A client learns the feature from `hello.features` (`session.history.latestTurn`), so it never sends the fields to an older daemon that would reject them as unknown |

## 4. Contract

### 4.1 Request

| Field | Meaning |
| --- | --- |
| `sessionId`, `branchId?` | Unchanged |
| `through?` | The fixed upper watermark, unchanged: default the published head, must not exceed the dispatched position, and the call waits until the branch dispatches up to it. It must end a batch, as today |
| `after?` | Unchanged forward cursor: exclusive, must end a batch. Absent in a window read; default 0 otherwise |
| `before?` | Backward cursor: exclusive upper bound. The page ends at the last batch end at or below `before - 1` and never above `through`. Must be at least 1 and at most `through`. Mutually exclusive with `after` |
| `latestTurn?` | `true` selects the turn window (§4.2). Mutually exclusive with `after`. `false` behaves as absent |
| `limit?` | Events per page before the page runs to its batch boundary. Without `latestTurn`: positive, no upper bound (unchanged), default 256 when absent. With `latestTurn`: 1 to 4096, default 4096 |

A `latestTurn` read is newest-first. A read with `before` and without `latestTurn`
is newest-first over the branch. A read with neither is today's forward read.

### 4.2 Window

```
sequence   1     I            T                F              H
           |.....|............|================|==============|
           br.   windowStart  turn/start        page start     through
           start               = turnStart                    (pinned)

latestTurn, whole window (H - I + 1 <= limit):
           events = [I..H]   turnStart = T   done = true   nextBefore = I

latestTurn, larger window:
           events = [F..H]   turnStart = T   done = false  nextBefore = F
           then history {latestTurn:true, before:F, through:H} returns [I..F-1]
```

The window starts at the smallest of:

- `T`, the sequence of the newest `turn/start` of the branch at or below
  `through`;
- the `input/message` sequence of each input the turn's first request consumed,
  read from the `input/consumed` events between `T` and that request's
  `step/model-start`, both at or below `through`;
- the newest `input/message` at or below `T`, which keeps the prompting input in
  the window when the first request is not committed at `through` yet.

With no `turn/start` at or below `through` the window is the whole branch,
starting at sequence 1, and `turnStart` is absent. Every candidate is at or
below `through`, so the window is stable while `through` is pinned.

### 4.3 Reply

| Field | Meaning |
| --- | --- |
| `events` | Unchanged form and ascending order: the branch's own events, filtered as today. Every page starts and ends on a batch boundary |
| `through` | As today: the watermark this read is pinned to |
| `nextAfter` | Forward: as today, the last returned sequence (raw, before filtering), or `after` when the page is empty; the client passes it back as `after`. Backward: the page's top, that is its last event's sequence, or the top boundary when the page is empty |
| `done` | The end of the read in its direction: forward, `nextAfter == through` (unchanged); backward, the page reached the window start |
| `nextBefore?` | Backward reads: the sequence of the first returned event. Pass it back as `before` to continue, also after `done`, to read below the window. Absent when the page is empty |
| `turnStart?` | `latestTurn` reads only: the sequence of the newest `turn/start` at or below `through`. Absent when the branch has none and when the request is not a window read |

### 4.4 Rules

- `limit` counts raw events, as it does today; the visible page can be smaller
  because `ProductEvent` filters kinds.
- A page is batch-complete at both ends. Forward: it takes `limit` events and
  runs to the end of that batch. Backward: it takes up to `limit` events
  downward from the top and runs to the start of the batch that holds the oldest
  taken event. A page can therefore exceed `limit` by its boundary batch, as
  forward pages already can.
- A truncated window is a suffix: `done` is false and `nextBefore` is behind
  `turnStart` or inside the turn. The client shows what it has and decides
  whether to load the rest. A window can begin inside a turn, inside a step,
  inside a text or thinking block, or after a tool call started; blocks and
  calls that opened before the window are rendered from the loaded events, and
  their first loaded event identifies them until the earlier page arrives. An
  open block's identity is still read from `session.model.progress`, which
  carries `textPartId` / `thinkingPartId`
  ([clients](../clients.md#timeline-block-identity)).
- Folding the pages of a window in sequence order returns exactly the forward
  read of the same range: same events, same order, no overlap and no gap. A
  client that wants the current-turn state equal to today's fold loads until
  `done`.
- `latestTurn` with `limit` at or above the window size is one reply and folds
  like today's read of that turn.

### 4.5 Validation and errors

| Condition | Result |
| --- | --- |
| `latestTurn` with `after` | `invalid`: the window chooses its own lower bound |
| `before` with `after` | `invalid`: one page has one direction |
| `before > through`, or `before == 0` | `invalid` |
| `limit` not positive, or above 4096 in a `latestTurn` read | `invalid`, as today |
| `after > through`, `after` inside a batch | `invalid`, as today |
| `through` beyond the dispatched position | `invalid`, as today; the call waits for dispatch up to `through` |
| `through` inside a batch | As today: the store rejects the page as not ending on a batch boundary |

## 5. Storage

### 5.1 Format decision

The native store moves from v27 to **v28**: one partial index, no column and
no row change.

```sql
CREATE INDEX timeline_history_anchors
  ON timeline_events(session_id, branch_id, kind, sequence)
  WHERE kind IN ('turn/start', 'input/message');
```

The upgrade creates it in the same startup transaction as the existing v27
step, and the marker becomes 28. A v27 daemon refuses a v28 store as it refuses
any newer format. The index holds one row per turn and one per user input,
unlike the per-event indexes that dominate write amplification
([#588](https://github.com/arcships/dim/issues/588)); it is a partial index on
`kind`, like `timeline_tool_use_starts`
([sqlite.rs:90](../../../crates/dim-app-server/src/storage/sqlite.rs#L90)). The
[storage format](native-storage-format.md) owner records the v28 step when the
implementation lands.

### 5.2 Queries

- Turn start:
  `SELECT max(sequence) FROM timeline_events WHERE session_id=?1 AND branch_id=?2 AND kind='turn/start' AND sequence<=?3`.
- Newest input before the turn, the window's floor:
  `SELECT max(sequence) FROM timeline_events WHERE session_id=?1 AND branch_id=?2 AND kind='input/message' AND sequence<=?3`.
- First request: forward from `T` on the primary key until the first
  `step/model-start` at or below `through`, collecting the `input/consumed` ids
  before it. A few events, not a scan of the turn.
- Input messages: each `input_id` is an event id, so its sequence comes from
  `timeline_events_by_id`, which already exists
  ([sqlite.rs:86](../../../crates/dim-app-server/src/storage/sqlite.rs#L86)).
  `SessionStore::read_events` performs the same lookup today
  ([storage/timeline.rs:1020](../../../crates/dim-app-server/src/storage/timeline.rs#L1020)).
- Backward page: rows are read from the primary key
  `(session_id, branch_id, sequence)` downward from the top, whole batches, so
  the read does not use `timeline_events_by_end`, which v25 removed
  ([#596](https://github.com/arcships/dim/issues/596)). The row model already
  allows a physical row to hold several events (`last_sequence`); the page
  respects row boundaries and returns events in ascending order.

### 5.3 Where the reads live

Both reads are persistence reads of the Session's own branch, next to the store
reads the App Server already has for Collaboration and summaries: `read_page`
([storage/timeline.rs:999](../../../crates/dim-app-server/src/storage/timeline.rs#L999)),
`read_events`
([storage/timeline.rs:1020](../../../crates/dim-app-server/src/storage/timeline.rs#L1020))
and `read_tool_use_starts`
([storage/timeline.rs:1056](../../../crates/dim-app-server/src/storage/timeline.rs#L1056)).
The window is not a Timeline mechanism: it names a turn fact, a message lookup
and a sequence range, and it is reached from `Connection::history` through
`AgentIndex`, as `read_tool_use_starts` is
([agents.rs:670](../../../crates/dim-app-server/src/agents.rs#L670)). A host
without a native store (tests, `ServerConfig.database_path: None`) answers the
same contract from the loaded branch through the Session facade.

The runtime, the Session facade and `sessions.history` are untouched;
`session.attach`, `session.event`, `session.state` and the live delivery path
are untouched.

## 6. Client flow

```
Desktop, opening main (H = the sequence from session.attach)
  session.attach {sessionId}                                  -> SessionInfo, H
  session.history {sessionId, branchId:"main", latestTurn:true,
                   through:H}                                 -> turn window
       the window with its inputs, or its newest 4,096 events + nextBefore
  session.event ...                                            live events after H
  scroll up inside the window:
  session.history {..., latestTurn:true, before:nextBefore,
                   through:H}                                 -> earlier pages
  scroll into older history (after done):
  session.history {..., before:nextBefore, through:H}           -> newest page below
  ...                                                          until done at 1
```

The attach reply carries the pinned `through` for `main`
(`SessionInfo.sequence`, [protocol.rs:906](../../../crates/dim-app-server/src/protocol.rs#L906)).
Events after the pin arrive as `session.event`; the client applies them after the
window pages, as it already does after forward pages.

## 7. Compatibility

- A request without `latestTurn` and without `before` is today's read, including
  the `after` semantics and the unlimited `limit` (an absent `limit` now takes
  256 instead of failing; no client relies on that error).
- New reply fields are additive. The Rust client's `History` type does not deny
  unknown fields ([wire.rs:216](../../../crates/dim-client/src/wire.rs#L216));
  the web reader takes named fields
  ([read.ts:131](../../../web/src/rpc/read.ts#L131)). Neither needs a change to
  keep working.
- Protocol v1 and v2 never reach the method.
- `hello.features` gains `session.history.latestTurn` next to `session.history`
  ([transport.rs:706](../../../crates/dim-app-server/src/transport.rs#L706)).
  A client that sees it sends the new fields; one that does not keeps the
  forward read. This is the only way to be safe against an older daemon, whose
  strict request decoding rejects unknown fields.
- Existing `after`-paging clients (CLI, web, export, member reads) are not
  changed by this design.

## 8. Other changes and interactions

| Change | Interaction |
| --- | --- |
| [#601](https://github.com/arcships/dim/issues/601) skips full revalidation when the stored head is unchanged | Independent and complementary. The window's anchors come from indexes, so a skipped validation does not remove them; [#601](https://github.com/arcships/dim/issues/601) removes the restore scan before the read, and this design removes the client scan after it |
| [#713](https://github.com/arcships/dim/issues/713) profiles the cold first `session.attach` | Independent. The read stays after attach and acquires the Session like every Session method; if attach already restored it, the window is served from the loaded branch. If #713 later bounds the first attach, the window contract does not change |
| [#588](https://github.com/arcships/dim/issues/588) / [#596](https://github.com/arcships/dim/issues/596) read-path audit | The backward page reads the primary key (§5.2) and does not depend on the removed `by_end` index. The two lookups add partial indexes; they do not change how history pages are read |
| [Restore-scoped shared read](restore-shared-read.md) | The window reads after publication, when the opening's copy is released; it is an ordinary committed-history read |
| [Idle unload](idle-unload.md) | Unchanged: `session.history` acquires the Session, and a read during an unload waits for the restore and then serves the window |
| [sessions service](sessions-service.md#67-history) | Unchanged (D8) |
| `session.catalog`, `session.summaries`, `session.event` | Unchanged; none reads the indexes |

## 9. Rule check

| Rule | Answer |
| --- | --- |
| [Principles §4](../../architecture/rust-agent/principles.md#4-通过时间线扩展行为) rule 1 (command events for others' state) | Not applicable: the read changes no state and writes no event |
| §4 rule 2 (handover) | Not applicable: nothing is handed over |
| §4 rule 3 (`before` callbacks) | Not applicable: no producer or draft changes |
| §4 rule 4 (no business meaning in core) | No core mechanism learns the window. The App Server's own persistence read names the turn and input facts it serves; the runtime is untouched |
| §4 rule 5 (atomic batches) | Kept: every page starts and ends on a batch boundary in both directions, and a cursor inside a batch is rejected |
| §4 rule 6 (model context) | Unchanged: the read feeds clients, not the model; no adoption or insertion path is added |
| §4 rule 7 (public events and services) | The window returns published facts from the Session's store; it reads no part's internal state and decides nothing for another module |
| §4 rule 8 (infrastructure) | Unchanged: no Timeline, Dispatcher, event envelope, assembly, lifecycle or facade change. The read is the App Server's own persistence read over the store's existing tables and two indexes |
| §4 rule 9 (only decision facts in the Timeline) | No new fact and no UI state in the Timeline; the indexes are derived from existing facts |
| §1 (Capability and App Server boundary) | No Capability, plugin or loader row. The App Server decodes the request and computes the window |
| §3 (orthogonality) | No other module's design changes; `sessions.history`, attach and live delivery are untouched |
| [ADR 002](../../adr/002-plugin-scope.md) D1 (scope follows state) | No state: a read of the Session's store |
| ADR 002 D2 (no wiring modules) | No new module or plugin |
| ADR 002 D3 (dependencies follow scope) | No new dependency |
| ADR 002 D4 (configuration) | None |
| ADR 002 D5 (language-neutral contracts) | The contract is the v3 wire, not a Rust spelling; any language's client reads it |

## 10. Requirements raised to mechanism owners

None. The design was checked against [§4 rule 8](../../architecture/rust-agent/principles.md#4-通过时间线扩展行为):
the App Server already reads the Session's store for its own views
(`read_page`, `read_events`, `read_tool_use_starts`), and the window needs only
reads over the store's existing tables plus one index. A Timeline facade read
(`Session::local_page_before`) was considered and rejected: it would add a
protocol view's needs to the runtime's public read contract, while the window
also needs the App Server's event-id and kind lookups, and `sessions.history`
does not need the mode (D8).

## 11. Acceptance

The implementation PR carries these; this design phase has no code.

1. **One request for the newest turn.** A turn larger than one client page
   (the measured 512-event pages contain no `turn/start`) is served by one
   `session.history {latestTurn: true, through}` reply: the whole window when it
   holds at most 4,096 events, otherwise its newest 4,096 events with
   `nextBefore` and `turnStart`.
2. **The prompt is in the window.** The reply contains the `input/message` of
   the input that started the turn, and of every input the first request
   consumed, even when those events precede `turn/start`. A queued input older
   than the newest message is included.
3. **Same fold.** For the whole-window replies of the 1,956- and 3,122-event
   Sessions, the returned sequences equal the forward paging of the same range,
   so the fold is today's. For the 6,862-event Session, the window and one
   `before` page cover the turn with no gap or overlap, and the union folds to
   the same state.
4. **Running turn.** With the turn still running, the window returns its input
   and `turn/start` through the pinned head; `done` is true when the window
   fits, and live `session.event` notifications after the pin still arrive.
5. **Older history.** After `done`, `before` pages reach the branch start and
   `done` becomes true; the existing forward read from `after: 0` returns the
   same events as before.
6. **No turn.** A branch with only appended inputs has no `turn/start`: the
   window is the branch's newest page and `turnStart` is absent.
7. **Compatibility.** A request without the new fields behaves as today, and a
   v3 client that does not read the new fields still folds pages.
   `hello.features` lists `session.history.latestTurn`.
8. **Format.** A v27 store upgrades in place to v28 with the index and no row
   change; a v28 daemon opens it; reopening does not upgrade again.
9. **Measurement.** The implementation reports request count and latency for
   the 1.9k / 3.1k / 6.9k-event heads with the Desktop reader or an equivalent
   harness, and the encoded size of the largest reply, so the 4,096-event
   constant can be reviewed.

## 12. Rejected alternatives

- **Return the whole turn regardless of size.** One reply must stay bounded; an
  unbounded turn would produce an unbounded reply (D4).
- **A window that starts at `turn/start`.** It loses the prompting input and
  every queued input the turn consumed, so a client cannot render the turn's
  user rows (D3).
- **A fixed number of events before `turn/start`.** A queued input's message can
  be arbitrarily older than the turn start; the window would sometimes include
  the wrong events and sometimes miss the input.
- **Expose only the turn start, e.g. `latestTurnStart` on attach.** The client
  would still page the turn from its start to the head, so the request count
  would grow with the turn, which is the measured problem.
- **A new method.** D1.
- **Opaque cursors.** D2.
- **Fix it in the client.** The boundary is server-side state; a client cache
  would duplicate it and still need the first read.
- **Stop materializing thinking deltas as separate events** (the issue's third
  option). That is a write-path and live-delivery change with its own
  durability rules ([#588](https://github.com/arcships/dim/issues/588)); it
  would reduce the turn's size but not bound it.

## 13. Changes to other documents

| Document | Change |
| --- | --- |
| [Documentation index](../../INDEX.md) | Add this design |
| [Native storage format](native-storage-format.md) | The v28 step and the index, when the implementation lands |
| [Clients](../clients.md) | None now; a client document that describes opening a thread can link this window instead of forward paging |
| [sessions service](sessions-service.md), [session access](../contracts/session-access.md) | None (D8) |

## 14. Open questions

1. Is 4,096 events the right ceiling, or should the bound be bytes? A count
   matches the existing page unit, but a single event's size is not bounded by
   the protocol. The measurement in §11 is meant to settle it.
2. Should the first page of a turn be its newest part, as designed, or its
   oldest part so a client can fold forward as live events arrive? Newest-first
   matches "open a thread and see the newest content"; oldest-first would show
   the turn's beginning and hide the running output.
3. Should `sessions.history` adopt the fields once a process caller needs a
   tail? Nothing needs it today (D8).
4. Should the forward mode gain the same 4,096-event cap? Today it accepts any
   positive `limit`, and a change would alter existing callers; this design
   leaves it and bounds only the new mode.
5. When the pinned head is the `turn/start` itself and the first request is not
   committed at `through`, the window starts at the newest input message
   (§4.2). A later read with a new pin can start earlier. Is that acceptable?
   The design keeps every candidate at or below the pin so a pinned read is
   stable.
