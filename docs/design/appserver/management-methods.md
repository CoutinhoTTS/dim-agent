# Management methods

Status: process-owner contributions implemented. This page describes the implemented
subset of [PR #15](https://github.com/arcships/dim/pull/15)'s revised plugin-scope
ADR and method contract (latest inspected branch `213506f`, including D4/D5).
Management methods belong to process owners, not a Management plugin. There is
no per-method level; entry admission determines whether this table is served.
The earlier stable-port and Management-plugin proposals are withdrawn.

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
adapter rows. Settings supplies generic BusinessDocument persistence, not
approvals, a method contribution or a configuration namespace.
MCP owns its configuration types in `mcp/config.rs`; the Skills owner supplies
the existing `SkillSettingsHost`. App Server's SkillsCatalog mounts the reusable catalog
and publishes both catalog and management interfaces in one plugin, with the
host's `HostPlatform`/`SettingsService` dependencies and the catalog's original effects;
the Skill settings adapter stays private to that owner. The capabilities crate does not depend on
App Server's wire protocol. Management request and response DTOs belong to their
business owners; `protocol` retains the envelope and built-in Session contracts.
MCP management and built-in Session methods share the MCP owner's error mapping.

`Contributions<T>` collects named services at the process scope (`None`) or a
Session/branch instance (`Some(instance)`). A listed but missing provider gates
the collector. Namespace and method validation rejects reserved built-ins,
duplicates, invalid names and nested ownership that would capture another
contributor's declared method. Valid nested namespaces such as `account.codex`
remain independent from the parent `account` namespace.

## Dispatch and lifecycle

Entry admission precedes routing for built-in and contributed methods alike:
a protected profile answers every call except `hello` and `daemon.ping` from a
non-local transport with `local_transport_required`, as before contributions
(PR #15 A.3). Local v3 then dispatches management contributions; v1/v2 do not,
and unknown names or methods return `unknown_method`.
The proposed `ConnectionEntry {name, deviceId}` and paired-device routing remain
RemoteGateway work; its connections must never be served management methods.
`hello.features` snapshots the collected methods and declared notifications
from the same Rpc load as the protocol instance. The withdrawn
`management.methods` endpoint is not provided. Existing Session and daemon
methods keep their built-in contracts.

Rpc admits contributed calls under the same handler lock as built-in commands.
Both operations and reply delivery belong to Rpc's existing command tracker.
On unload it removes admission, signals cancellation and drains accepted work
before dependencies are released. Every accepted call observes that unload
signal and stays tracked until it settles; a caller's disconnect cancels
nothing. On unload a `Report` method replies `operation_interrupted`, stating
that the operation may already have committed and the current state must be
read before retrying, while Rpc waits for a `Drain` method's own result. This
reproduces the behaviour before contributions (PR #15 A.4). Provider and
account methods use `Drain`; MCP and Skills use `Report`. Settings retains its
accepted-write contract.

An owner reload propagates to Rpc through normal rutis dependencies. Other
execution dependencies may also reload affected consumers and fail in-flight
turns; this is the accepted cost of rare structural changes, not hidden by
registration ports. Business configuration edits update the owner's source and
publish invalidation without reloading the owner or Rpc. MCP connections remain
Session-owned. MCP keeps its project execution approval until this branch merges
PR #16's removal under PR #15's approval-scope ADR; see ADR 004.

## Notifications

Owners declare notification names and publish serialized parameters through
their contribution. Rpc follows those channels for its current load, forwarding
only to local v3 connections of the current protocol instance after hello.
Pointer identity fences delivery from an old Rpc load. Settings source watches
invalidate business consumers after successful persistence; they are not loader
events or a `configuration.changed` management notification. Persistence does not
certify runtime adoption or generation settlement.

## Limits and verification

RemoteGateway, rutis-link export, contributed Session methods, per-call progress,
connection-bound CallChannel and shutdown reply handoff are not implemented in
this subset. There is no new settings authority, loader or authorization system.

Completing D5 requires binding the named `managementMethods[namespace]` service
and its serializable `describe`/`call` contract to provider declarations and Rpc
collection. That binding must retain the same owner, entry restrictions and
reload/drain guarantees, without adding an adapter plugin or a second registry.
The revision's Timeline-driven input withdrawal and peer-node offline behavior
belong to the Session event contract and rutis link work; neither is implemented here.

Declaration-driven method discovery and cross-language plugin method declarations
remain deferred: assembly still lists Rust owner namespaces explicitly. Generic
credential references/resolution are future work. D4 keeps business documents in
their source, not editable loader rows; see [ADR 004](../../adr/004-loader-plugin-management.md).

Transport flows cover actual owner methods, hello features, owner/Rpc reloads,
local/remote and protocol-version boundaries, business source CAS, persistence
failures and invalidation. A rutis contribution flow verifies reload propagation,
Report interruption, Drain completion after the unload signal and replacement
admission.
Namespace validation covers reserved names and nested collisions in either
collection order. Existing tool/prompt contribution flows protect instance
scoping while the shared collector also supports process services.
