# Management methods

Status: process-owner contributions implemented. This page describes the shipped
subset of [PR #15](https://github.com/arcships/dim/pull/15)'s revised plugin-scope
ADR and method contract (revision `384b3d4`). Management is a method level, not a
plugin. The earlier stable-port and Management-plugin proposals are withdrawn.

The subsequent [language-neutral contract revision](https://github.com/arcships/dim/commit/165174a94b16af0f0fb89b67d43b3b104a5deb00)
adds ADR D5. This implementation provides the Rust-only subset, not the named
cross-language `managementMethods` contract. Its namespace keys use
`TypeKey::keyed_dynamic::<ManagementMethods>` and still identify a Rust type;
they are not rutis-bridge service names. A non-Rust plugin cannot supply this
contribution merely by declaring `provides` in a loader row.

## Responsibility and assembly

Each process owner provides `ManagementMethods` under its namespace during
`apply`. Its ordinary rutis lifetime owns that service. Rpc declares
`Contributions<ManagementMethods>` in `Deps`, with names supplied by process
assembly, and builds an immutable method table for that load. No contributor
injects Rpc or Connection; neither entry contains contributor-specific dispatch.
Adding a method changes its owner's declaration, parsing and implementation.
Adding an owner also registers its namespace in assembly, as Session tools do.

Account, Codex, Grok, Provider and Mcp provide their methods directly, without
adapter rows. Settings provides `configuration.*` when it has the native
UserConfiguration source. App Server's SkillsCatalog mounts the reusable catalog
and publishes both catalog and management interfaces in one plugin, with the
catalog's original `Deps` and effects; the capabilities crate does not depend on
App Server's wire protocol. Public request and response DTOs remain in `protocol`.

`Contributions<T>` collects named services at the process scope (`None`) or a
Session/branch instance (`Some(instance)`). A listed but missing provider gates
the collector. Namespace and method validation rejects reserved built-ins,
duplicates, invalid names and nested ownership that would capture another
contributor's declared method. Valid nested namespaces such as `account.codex`
remain independent from the parent `account` namespace.

## Dispatch and lifecycle

Local v3 dispatches management contributions; v1/v2 and WebSocket do not.
Unknown names and methods on non-serving entries return `unknown_method`.
`hello.features` snapshots the collected methods and declared notifications
from the same Rpc load as the protocol instance. The withdrawn
`management.methods` endpoint is not provided. Existing Session and daemon
methods keep their built-in contracts.

Rpc admits contributed calls under the same handler lock as built-in commands.
Both operations and reply delivery belong to Rpc's existing command tracker.
On unload it removes admission, signals cancellation and drains accepted work
before dependencies are released. `Drain` methods finish after disconnect or
reload. `Report` methods signal cancellation and report interruption while their
accepted operations remain tracked until settled. Configuration, provider and
account methods use `Drain`; MCP and Skills use `Report`. Settings retains its
accepted-write contract.

An owner reload propagates to Rpc through normal rutis dependencies. Other
execution dependencies may also reload affected consumers and fail in-flight
turns; this is the accepted cost of rare structural changes, not hidden by
registration ports. Business configuration edits update the owner's source and
publish invalidation without reloading the owner or Rpc. MCP connections remain
Session-owned and retain the host-local execution approval boundary.

## Notifications

Owners declare notification names and publish serialized parameters through
their contribution. Rpc follows those channels for its current load, forwarding
only to local v3 connections of the current protocol instance after hello.
Pointer identity fences delivery from an old Rpc load. Settings owns the
coalesced loader/persistence invalidation task and publishes
`configuration.changed`; it acknowledges neither persistence nor adoption.

## Limits and verification

RemoteGateway, rutis-link export, contributed Session methods, per-call progress,
connection-bound CallChannel and shutdown reply handoff are not implemented in
this subset. There is no new settings authority, loader or authorization system.

Completing D5 requires binding the named `managementMethods[namespace]` service
and its serializable `describe`/`call` contract to provider declarations and Rpc
collection. That binding must retain the same owner, entry restrictions and
reload/drain guarantees, without adding an adapter plugin or a second registry.
The revision's input admission checks and peer-node offline behavior belong to
the Session input boundary and rutis link work; neither is implemented here.

Transport flows cover actual owner methods, hello features, owner/Rpc reloads,
local/remote and protocol-version boundaries, configuration CAS, persistence
failures and invalidation. A rutis contribution flow verifies reload propagation,
Report cancellation, disconnected Drain completion and replacement admission.
Namespace validation covers reserved names and nested collisions in either
collection order. Existing tool/prompt contribution flows protect instance
scoping while the shared collector also supports process services.
