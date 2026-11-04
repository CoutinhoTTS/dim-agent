# 021: A read follows the object that owns the state

Decision status: Proposed.

## Context

[ADR 019](019-event-query.md) makes one bounded query the read surface of a Timeline, and [ADR 020](020-timeline-read-convergence.md) removes the consumer-shaped reads and replay. Both describe a line that is loaded: a `Timeline` exists, it owns append, the publication state and the query, and `Branch` and `Session` forward the query.

They do not say what reads when no `Timeline` exists. Three cases do read then, and each reaches the store around the service:

- `session.history` for a Session the process does not hold (`dim-app-server/src/agents.rs`, `history_page`) falls to `SessionStore::read_page_without_request_bodies`, which calls `SqliteTimelineStore` directly.
- An ended line's page falls to the same page read, or to `TimelineTree`'s own `read_page` when the host keeps no store.
- `TimelineTree` reads `branch/ended`, `branch/end` and `branch/create` through `TimelineStore::read` and `read_page` while restoring lines.

The result is that the page rules — ascending order, the published bound, an end that falls on a batch boundary, and the request-body decode policy — exist twice: once in the `Timeline` query path, which adds `batch_end` in the caller, and once in the App Server's SQL. Nothing keeps the two equal, and a client that reads a page can see a different shape from the one a part folds.

The store port is also reachable from any part that declares it, so a part can read facts around the service it is supposed to use.

The trigger for this decision is an interface that opens in order to render messages and a timeline: `session.attach` loads the Session and builds the branch's `Timeline`, and the history read follows it. The read and the object that owns the state have the same lifetime.

## Decision

### D1. A loaded line is read only through its Timeline

The Timeline of a loaded line is the read surface for that line: the query, the checkpoint selection, the segments and the batch-end check. `Branch` and `Session` keep forwarding the query. No caller reads a loaded line's facts through the store port.

### D2. An ended line is read through the object that owns its end

A line that ended still belongs to a loaded Session. `TimelineTree` owns its ended head, its end record and its place in the tree, so `TimelineTree` serves its history. The client protocol reaches it through the tree's own service, not through a page read of the store by the App Server.

### D3. No live object, no client read

`session.history` is a client read of a Session the client attached to. It requires the Session to be loaded, and it does not fall back to a store read when it is not: the caller attaches, which loads the Session, and then reads. A read whose subject has no live object is not a client read and is not served by the client protocol.

### D4. A read with no live object is its own process-level API

A one-shot read of a Session this process does not hold — `dim export`, a scheduler reading a title — is a process-level API of the App Server. It states that it reads only committed stored facts, returns them in the page shape the query defines, and is not reachable from the client protocol. It does not claim to be a Timeline read.

### D5. The store port is reachable from the Timeline and from TimelineTree

`TimelineStore` is a runtime port. `Timeline` and `TimelineTree` may declare it; `Session`, `Branch` and other parts may not. `SessionStore` keeps Session files, titles, grants, fork and delete, and no longer serves event reads for the client protocol.

### D6. No read is served by a copy that outlives its readers

A shared decoded copy of a branch's history exists only for readers alive at the same time as the branch's opening. The opening copy that kept `1..=head` in memory until publication was built for replay; replay is gone and no regular reader fills it, so it goes ([#1063](https://github.com/arcships/dim/issues/1063)). A read that needs facts reads them through the object of D1, in pages it drops when it is done.

## Consequences

- `agents.rs::history_page` loses the branch that reads `SessionStore` for a Session the process does not hold; a client read of an unloaded Session is answered by loading it.
- `idle-unload.md` acceptance 1 and 2, which require `session.summaries` to list 50 unloaded Sessions and serve their history without restoring, describe the process-level API of D4 and must say so.
- A read holds one page at a time: neither the process-level read nor a part's fold materializes a whole branch ([#1065](https://github.com/arcships/dim/issues/1065)).
- The page rules have one owner per path: the query path for a loaded line and the process-level page read for facts with no live object. A divergence between them is a defect of the process-level read, tested against the query's page shape.
- `TimelineTree` keeps the store port for the reads of lines it owns while they are not loaded; four of those reads select events and become the port's query, which changes cost, not ownership ([#1064](https://github.com/arcships/dim/issues/1064)).
- The App Server no longer calls methods of `SqliteTimelineStore` that the port does not declare. Where the port lacks a capability the App Server needs, the port gains it.

## Non-goals

This decision does not change the query's selection, the publication bound, the checkpoint format, the batch-end check or the rebuild handover. It does not add a stateful object for a line that is not loaded.

## Supersession

None. This ADR refines [019](019-event-query.md) D1 and [020](020-timeline-read-convergence.md) D6 for the case where no `Timeline` exists.
