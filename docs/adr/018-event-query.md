# 018: One bounded event query is the Timeline read surface

Decision status: Proposed.

## Context

Capabilities read Timeline facts through the branch's Timeline service. That service currently
carries one read per consumer shape: whole-chain history, a local range, a page after a cursor,
a context page, the branch segments, one kind across the ancestors, one event by id and by
position, the folded context with its checkpoint, and the policy order, window and facts.

Each method encodes what one consumer wants. The runtime therefore defines what a context page,
a policy window or a recall page is, and other languages see an arbitrary subset of those
methods. Some of the reads are unbounded: the fold that decides a fork rest point, the recall
tool's history read and the branch opening each read every event of the branch.

The event envelope and its data are a frozen compatibility contract;
`crates/dim-agent-types/tests/fixtures/` is the baseline every compatible store must decode.
The storage layout is not part of any contract: tables, columns, indexes, content references and
atomic batches belong to the store.

## Decision

### D1. One bounded query over events

The Timeline exposes one read for capabilities: the scope (this branch, or the branch chain root
first), the position (a sequence range, or a cursor with the sequence it may read through), the
accepted event kinds plus the envelope's context flags, one equality predicate over a data field,
the order, and a required limit. It returns decoded events and the next cursor.

A capability that needs a different selection extends this query; it does not add a method.

### D2. Fixed semantics

A cursor is the last fully read sequence at an atomic batch boundary: a read never splits a batch,
and the limit is a target that the final batch may exceed. Results are ordered by chain position,
then sequence. Events are returned decoded; content references and batch rules never leave the
store.

### D3. The query addresses the event contract, not the storage

Kinds and data-field paths are part of the event wire contract. Table names, column names,
indexes and the storage format are not addressable, and SQL text is not a capability interface.

### D4. Indexes are derived and declared by the core

An index a query needs is created at open from what the core's own reads query. It carries no
data, stays outside the storage format, and a capability never names one.

### D5. Consumer-shaped reads are temporary

A read shaped for one consumer exists only while that consumer moves to the query. The service
keeps three things: append one event, read the published sequence, and this query. Entities that
are not events — branch segments and recovery snapshots — keep typed reads of their own, not the
event query.

## Consequences

- A new capability read adds no service method, and the runtime stops defining context pages,
  policy windows and recall pages.
- Every read has a bound, so an unbounded read has no interface. A consumer that folds a whole
  prefix folds it from bounded pages.
- Rust and other languages see the same query; the generated declarations describe it once.
- A selection the query cannot express is a decision to extend it here, not a method to add.
- While a consumer still uses its old read, both shapes exist and must agree on ordering, batch
  boundaries and cursor meaning.

## Non-goals

- Changing event types, the envelope, the storage format or any fixture.
- Deciding where the context projection lives.
- A general expression language: the first form is equality on one data field.
- Covering branch segments and recovery snapshots.

## Supersession

Follows [ADR 006](006-plugin-contract.md) D1: a rule is stated once, at its layer. Supersedes no
ADR.

## Rule check

| Rule | Application |
| --- | --- |
| §1, ADR 002 D1 | No new part. The query is a Timeline service method the branch's parts already hold. |
| §4 rule 1 | Reads do not change assembly, dependencies or activation. |
| §4 rule 2 | Consumers keep deciding from facts; the query selects facts, it does not decide. |
| §4 rule 3 | No `before` callback and no commit-time I/O. |
| §4 rule 4 | State stays with its owner: a fold remains in the part that owns the state. |
| §4 rule 5 | Unchanged: appends and their batch contract. |
| §4 rule 6 | Unchanged: context enters only through adopted inputs and tool results. |
| §4 rule 7 | Callers keep reading through the Session and branch facades; the query is a public service and speaks only public event data. |
| §4 rule 8 | Requirement raised to the Timeline's owner: one bounded query and the derived-index declaration. |
| §4 rule 9 | No new fact. |
| ADR 002 D2, D3, D5 | No wiring part, no dependency crossing, and the query is language-neutral data. |

## Requirements raised to mechanism owners

The Timeline service owner (runtime) provides the query, its cursor and batch-boundary semantics,
and the derived-index declaration. The storage owners (SQLite and the in-memory store) implement
the same semantics. The plugin contract describes the query as a public service, and the owner of
each consumer-shaped read replaces it with the query.
