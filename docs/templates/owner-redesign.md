# Owner redesign

Use this shape when an existing owner's public surface is redesigned under
[design rules §7](../architecture/rust-agent/principles.md#7-所有者与操作). It has two parts. The design
part is durable and goes into the owner's design document, `docs/design/plugins/<Owner>.md`,
created from the [design template](design.md) when it does not exist. The delivery part is
temporary and goes into the owner's issue.

Write conclusions first. Name things directly instead of referring back. Do not restate rules or
describe how the document was produced. Link evidence next to the claim it supports.

## Design part

### Use and scope: responsibility split

- The owner's responsibilities, in the owner's own vocabulary.
- The responsibilities that stay with callers as business logic.
- One row per caller requirement:

| Requirement | Can callers compose it from existing operations without breaking the owner's invariants? | Handling |
| --- | --- | --- |

An operation or parameter exists only where the answer is no.

### Caller example

The caller code after the change for each main intent. It shows the owner operations it calls and
the errors it handles, and nothing of the owner's internal mechanisms.

### Own contract

Each operation: the intent it serves, request and reply data, cancellation, and error kinds named by
what the caller can do next. For each operation that holds a resource or waits, a flow covering the
normal path, the caller cancelling or unloading, and the owner closing or reloading, with the party
that settles each resource.

Ids, values, read-only views and callbacks cross owners. Where an object still has to be handed out,
say why none of those works, and link the design shared by the plugins involved that fixes who
creates, holds, hands over and cleans up the object, and what happens when any side closes or reloads.

### Exceptions with reasons

One line each.

### Rule check and requirements raised to mechanism owners

As in the [design template](design.md).

### Binding appendix

The `#[rutis::service]` block on the owner type, and the request, reply and error types.

## Delivery part

Written in the owner's issue; it is history once the change lands.

- **Deletion list**: public methods, types, wrappers and catalog entries that must not exist after the
  change.
- **Caller migration**: one row per call site, `caller | current call | target operation`.
- **Implementation task**: rewrite the owner's public surface; do not add an interface beside it.
  Read the responsibility split, write the callers' target code as flow tests, make the old public
  methods `pub(crate)` and use the compile errors as the migration list, implement the operations,
  then migrate every caller. A plugin area's new operations, every caller and the removal of the old
  surface are one change, with no period where old and new coexist.
- **Acceptance**: the flow tests pass; no cross-crate reference to a deleted name remains; the
  catalog has no `internal(reason)` entry for the owner; the diff removes the old surface.
