# Client Session catalog

Issue: [#556](https://github.com/arcships/dim/issues/556), whose required outcome this page serves: a bounded
`/resume` first page and catalog metadata without decoding full effective-context replacements.
[#557](https://github.com/arcships/dim/issues/557) adds the classification filters this contract reserves a place for.
Overview: [Appendix B.2](../overview.md#b2-app-server-process-plugins), Rpc and SessionStore rows. Scope:
[ADR 002](../../adr/002-plugin-scope.md) process scope, in the existing Agents and Rpc built-ins, with one new Rpc
built-in. Consumers: clients through `session.catalog` and `session.summaries`, first the CLI's `/resume` picker.
Rules: [design rules](../../architecture/rust-agent/principles.md), [ADR 002](../../adr/002-plugin-scope.md).

## 1. Summary

A client lists Sessions with `session.summaries`, and that reply is built from history: for every Session the App
Server reads up to 256 events for a preview and then the last event whole for a time, one Session after another, and
replies one array with no page boundary. An imported Session's last event is a complete `context/compaction`, so a
listing decodes the whole catalog's history, and a client sorts and converts all of it before it can draw a row.

The catalog read becomes bounded work. SessionStore keeps a projection of what a listing needs — per Session its last
activity time, its first user input as the preview, and its workspace — written in the transaction that commits each
`main` batch. A new built-in, `session.catalog`, pages that projection: 50 rows by default, an opaque keyset cursor,
and `query` and `cwd` applied in the query before the page boundary. `session.summaries` keeps its contract and reads
the same projection, so neither method reads history.

- **Projection.** SessionStore's, written with its batches; this page owns only how the catalog is read (§4).
- **Method.** `session.catalog {limit?, cursor?, query?, cwd?} -> {sessions, nextCursor?}` (§3.1).
- **Lease-free.** A listing loads no Session, restores none, and starts no model, tool or MCP server (§3.4).
- **Clients.** The picker pages, searches on the server and drops a pending read (§6).

## 2. Scope and non-goals

This page owns the reply contract of `session.catalog`, the catalog read of `session.summaries`, the order, cursor
and page-boundary semantics, and the place where #557's filters apply.

It does not own:

- the projection's table, index, write transaction, delete and fork maintenance, and the upgrade's backfill — the
  [native storage format](native-storage-format.md#556-格式决定) decides those;
- the classification fields, their defaults and their presentation — [#557](https://github.com/arcships/dim/issues/557);
- the picker's interaction and the project/all switch — the client that adds them (§6);
- `session.list`, `session.attach`, and how a client sorts or presents rows.

## 3. Own contract

### 3.1 `session.catalog`

`session.catalog {limit?, cursor?, query?, cwd?} -> {sessions: SessionSummary[], nextCursor?: string}`

| Item | Rule |
| --- | --- |
| Kind | Rpc built-in. Agents answers it from the catalog projection, joined with the `agent_sessions` rows and the title metadata, as it answers the directory read; Rpc validates the envelope |
| Protocol | v3. v1 and v2 reply `unknown_method` ("Method requires protocol v3"), as they do for `session.summaries` |
| Discovery | `hello.features` lists it under the conditions that list `session.summaries`, so a client that does not see it keeps using that method |
| `limit` | 1 to 200, default 50. A larger one is rejected, not clamped, as `history`'s `limit` is |
| `cursor` | Opaque. It encodes the last returned row's `(updatedAt, sessionId)` and pages by keyset; a client echoes it back unchanged, and it is not a Session id |
| Order | `updatedAt` descending, then `sessionId` ascending |
| `query` | Case-insensitive substring over the title and the preview, applied in the catalog query before the page boundary |
| `cwd` | Exact match on the workspace, applied before the page boundary |
| Reply | `SessionSummary` entries, the shape `session.summaries` returns: the fields of a `session.list` record with `preview` and `updatedAt`. `nextCursor` is absent on the last page |

An unreadable or foreign `cursor` is rejected rather than treated as the first page, so a client cannot silently read
the catalog from the start again.

### 3.2 Order, cursor and paging

The order is the projection's activity time, newest first, with the Session id breaking a tie, so a page is a
contiguous run of one total order and the same store always pages the same way. The cursor is the last row's key, not
a position: a Session that becomes active between two pages moves to the first page, and the later pages neither
repeat nor skip any other Session.

### 3.3 `session.summaries`

`session.summaries` keeps its parameters, its reply type and its per-client protocol v3 rule, and answers from the
same projection instead of reading history. It is how a client that does not find `session.catalog` lists, and how
clients on a daemon older than this change list; both stay working because the method's shape does not change.

### 3.4 Lease-free

A catalog read never loads, restores or starts a Session: the projection answers the stored fields, so a listing costs
no model, tool or MCP startup. `running` and `attachedClients` come from the process's in-memory state, as
`session.list` reads them: an unloaded Session reads as not running, and neither field is stored, so reading them
takes no lease and loads nothing.

### 3.5 Catalog filters (#557)

Hierarchy and origin filters are conditions of the same catalog query and apply before the page boundary, so a match
on a later page is never hidden by what the page already fetched. [#557](https://github.com/arcships/dim/issues/557)
defines their fields and defaults. This contract reserves where they apply and requires that a row's position, the
cursor and any count stay consistent across refreshes with them in place.

## 4. Events and state

This part writes no event and owns no state. The projection is derived state: every column follows from events the
Timeline already holds, and the upgrade rebuilds it once for the whole store from them
([native storage format](native-storage-format.md#556-格式决定)). Nothing here enters the Timeline: the catalog is
display data for a picker, which [design rules §4 rule 9](../../architecture/rust-agent/principles.md) keeps out of it.

## 5. Dependencies

| Dependency | What this part requires |
| --- | --- |
| SessionStore catalog projection | One row per Session with the last activity time, the preview and the workspace, present from the same transaction as the events it describes, and absent for a Session that does not exist |
| [Title metadata](session-title-metadata.md) | The stored title of a Session, joined for display and for `query`, never read from a Session |
| `hello.features` (App Server) | The method list a client discovers the catalog through, as it discovers `session.summaries` |
| Agents' directory | The protocol-version and agent filtering the catalog applies, as `session.list` applies it |
| Agents' index and the connection hub | The live `running` and `attachedClients` of the listed Sessions, read from the process's state without a lease, as `session.list` reads them |

No Session facade call, no lease and no other plugin service is used.

## 6. Clients

The picker behind `/resume` is the first consumer, and this page states what it requires of a client; the client's
own design owns its interaction.

- **First page.** The first page is the catalog with no `cwd` filter, because `/resume` lists every workspace today.
- **Pages.** The picker asks for the next page as the selection nears the end of the loaded rows, and keeps the
  server's order across pages.
- **Search.** Its filter goes to the server as a debounced `query`, instead of narrowing only the rows it holds.
- **Cancellation.** Closing or refreshing the panel drops its pending read, and a command entered afterwards runs at
  once.
- **Fallback.** A daemon that does not advertise the method is listed through `session.summaries`.

The project/all switch is a separate client decision: `cwd` exists in this contract for it and applies before the
page boundary, and the picker's first page does not use it. The gap is recorded in
[Clients](../clients.md#32-session-management).

## 7. Decisions

| # | Decision | Rejected alternative | Why |
| --- | --- | --- | --- |
| C1 | The cursor is opaque and pages by keyset on `(updatedAt, sessionId)` | Offset paging with `offset` or `skip` | An insert shifts every later row, so an offset page repeats or skips Sessions the client already showed |
| C2 | A new built-in, `session.catalog` | Adding `limit` and `cursor` to `session.summaries` in place | Its reply is an array; changing it breaks every protocol v3 client of that method, and an array has no page boundary to keep |
| C3 | The preview and the activity time are read from the projection | Computing them per request from history | That is the cost this design removes: it reads every Session's history, and an imported Session's last event is a complete compaction replacement |
| C4 | `query` and `cwd` are applied in the catalog query | Filtering the rows the client already loaded | It hides matches on pages the client has not fetched |
| C5 | `session.summaries` keeps its contract and reads the same projection | Leaving it on the history read; removing it | It is how a client lists without the new method, and once the projection answers both, keeping its contract costs nothing |
| C6 | Neither method takes a Session lease | One lease per listed Session | A lease restores and pins every listed Session; the projection, and the process's live state for `running` and `attachedClients`, answer without one |

## 8. Changes to other documents

- [Native storage format](native-storage-format.md#556-格式决定): the catalog table, its index, the same-transaction
  write, delete and fork maintenance, and the upgrade's backfill and its one-time cost.
- [Documentation index](../../INDEX.md) lists this page.
- [Clients](../clients.md#32-session-management) §5 names `session.catalog` where the live Session list row names
  `session.summaries`, and the CLI documents `/resume`'s paging, server search and cancellation. Both describe
  behaviour that exists only once the client ships them, so the client's implementation makes those two edits.

## 9. Rule check

| Rule | Answer |
| --- | --- |
| §4 rule 1 (a command changes another owner's state) | Nothing is written. The catalog reads rows SessionStore owns, through Agents |
| §4 rule 2 (handover) | None: a read hands nothing to another module |
| §4 rule 3 (before callbacks) | None |
| §4 rule 4 (no business meaning in core) | No core change; the projection is SessionStore's business row |
| §4 rule 5 (batch) | The row is written in SessionStore's existing batch transaction; the catalog adds no batch and no event |
| §4 rule 6 (model context) | Unchanged: no event, so no context path |
| §4 rule 7 (public events and services) | Agents answers from SessionStore's stored rows, as `session.list` does; Rpc reaches neither the store nor another plugin's state directly |
| §4 rule 8 (infrastructure unchanged) | The Timeline, Dispatcher, event envelope and format, assembly and the Session facade are unchanged. The store gains a derived table and no event, by the storage page's own format decision |
| §4 rule 9 (only facts later decisions use enter the Timeline) | The catalog is picker display data, so it stays out of the Timeline; no event is added |
| ADR 002 D2 (no wiring module) | No new plugin: Rpc's built-in and Agents' existing directory answer it |
| ADR 002 D5 (language-neutral contracts) | §3 is the contract as data; the Rust spelling is the implementation's |

## 10. Acceptance (flow tests)

- A first page of 50 by default, an explicit `limit` within 1 to 200, and a last page without `nextCursor`.
- Paging by the returned cursor returns every remaining Session exactly once in the stated order, including when a
  Session is updated between two pages.
- `query` matches title and preview without regard to case and finds a Session that is only on a later page; `cwd`
  matches the workspace exactly; both apply before the page boundary.
- A Session whose `main` ends in a full `context/compaction` is listed with its stored preview without decoding that
  event.
- A listing taken while the Session is idle unloaded replies the same stored fields as while it is loaded, reads it
  as not running, and starts no model, tool or MCP server.
- `session.summaries` keeps its reply shape and contents for protocol v3.
