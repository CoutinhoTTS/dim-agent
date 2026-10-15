# Management methods

Status: process-owner contributions implemented. This page describes the implemented
subset of [ADR 002](../../adr/002-plugin-scope.md) and the
[methods contract](../contracts/session-boundary.md).
Management methods belong to process owners, not a Management plugin. There is
no per-method level; entry admission determines whether they are served.
The earlier stable-port and Management-plugin proposals are withdrawn.

[ADR 002 D5](../../adr/002-plugin-scope.md#d5-contracts-are-language-neutral)
requires language-neutral contracts. This implementation provides the Rust-only subset, not the named
cross-language `managementMethods` contract. Its namespace keys use
`TypeKey::keyed_dynamic::<ManagementMethods>` and still identify a Rust type;
they are not rutis-bridge service names. A non-Rust plugin cannot supply this
contribution merely by declaring `provides` in a loader row.

## Responsibility and assembly

Each process owner provides `ManagementMethods` under its namespace during
`apply`. Its ordinary rutis lifetime owns that service and the calls it accepts.
Rpc does not depend on any owner ([methods contract §3.2](../contracts/session-boundary.md#32-management-method)):
on each call it makes keyed reads of the method name's prefixes, longest first,
and dispatches to the first active provider. No contributor injects Rpc or
Connection; neither contains contributor-specific dispatch. Adding a method
changes its owner's declaration, parsing and implementation. Until rutis can list
the active providers of a service (contract §9.3), assembly passes Rpc the
namespaces of its enabled owner rows for `hello.features`; adding an owner adds
its namespace there.

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

Each provider validates its own declarations: it rejects duplicates, invalid
names, built-in method names, methods outside its namespace and the `hello`,
`daemon`, `agent`, `approval` and `session` roots, so contributed methods cannot
impersonate Session traffic. Two providers of one namespace fail assembly, since
keys are unique per scope. A name belongs to the longest active namespace that
prefixes it: a parent's method inside an active child namespace is neither
dispatched nor listed. The contract assigns it to the longest provided namespace;
until rutis reports registered but inactive providers (§9.3) the two differ only
while a child is inactive, so an owner must not declare methods inside another
owner's namespace. Valid nested namespaces such as `account.codex` remain
independent from the parent `account` namespace.

## Dispatch and lifecycle

Entry admission precedes routing for built-in and contributed methods alike:
a protected profile answers every call except `hello` and `daemon.ping` from a
non-local transport with `local_transport_required`, as before contributions
([methods contract A.3](../contracts/session-boundary.md#a3-entries-and-export)). Local v3 then dispatches management contributions; v1/v2 do not,
and names without an active provider, or that the provider does not declare,
return `unknown_method`. A name outside the declaration grammar (dot-separated
lowercase segments, at most 128 bytes) is unknown without any lookup, so a
caller's name bounds the lookups made under the handler lock. Error codes at the
edges are unchanged: a call before `hello` is `-32600` and one after shutdown has
begun is `-32603`; an unknown method is `-32601` whatever its params, and only
the owner of a declared method rejects its params (`-32602`).
The proposed `ConnectionEntry {name, deviceId}` and paired-device routing remain
RemoteGateway work; its connections must never be served management methods.
`hello.features` lists the methods of the providers active at that `hello`.
A provider that becomes active later appears in the next `hello`. The withdrawn
`management.methods` endpoint is not provided. Existing Session and daemon
methods keep their built-in contracts.

Rpc admits contributed calls under the same handler lock as built-in commands
and tracks reply delivery in its command tracker. The operation belongs to its
provider: the provider admits it under its own gate and tracks it until it
settles. Either unload, the provider's or the calling Rpc load's, signals the
handler. A `Report` call then replies `operation_interrupted`, stating that the
operation may already have committed and the current state must be read before
retrying; a `Drain` call replies with its own result. A provider's unload waits
for every call it accepted, so `Drain` calls hold it (contract §9.2). Rpc's unload
waits for the replies it owes and for each interrupted operation to settle, so the
next load reads its outcome. While Rpc's own load is unloading it sees no
provider, and management calls and `hello` return `service_reloading`. A caller's
disconnect cancels nothing.
This reproduces the behaviour before contributions
([methods contract A.4](../contracts/session-boundary.md#a4-migration-and-compatibility)). Provider and
account methods use `Drain`; MCP and Skills use `Report`. Settings retains its
accepted-write contract.

An owner that is disabled, Pending, Failed or reloading affects only its own
namespace: its methods return `unknown_method` until a generation is active, and
Rpc and open connections are unaffected. This is a compatibility change for a
reloading owner, which returned `service_reloading` while Rpc reloaded with it
(contract A.4); `service_reloading` returns once rutis reports registered but
inactive providers (§9.3). Rpc and Agents depend on no owner, directly or
through each other: `session.create`, `session.model.update` and Collaboration
member creation read Provider and the `ModelSelectionHost` per call and return a
retryable error while it is not active. A transport flow asserts that no
management owner is among Rpc's transitive dependencies. Other execution
dependencies may still reload affected consumers and fail in-flight turns; this
is the accepted cost of rare structural changes. Business configuration edits update the owner's source and
publish invalidation without reloading the owner or Rpc. MCP connections remain
Session-owned. There is no host-local approval
([ADR 003](../../adr/003-approval-scope.md)); a project entry runs only after the
user enables it through `mcp.set_enabled`, which records the choice in MCP's
user-local business document; see ADR 004.

## Notifications

Management methods have no notification channel. Connection-bound notifications
belong to the contract's `callChannel.notify` ([A.2](../contracts/session-boundary.md#a2-call-channel)),
which is not implemented. Settings source watches invalidate business consumers
after successful persistence; they are not loader events or management
notifications. Persistence does not certify runtime adoption or generation
settlement.

## Limits and verification

RemoteGateway, rutis-link export, contributed Session methods, per-call progress,
connection-bound CallChannel and shutdown reply handoff are not implemented in
this subset. There is no new settings authority, loader or authorization system.

Completing D5 requires binding the named `managementMethods[namespace]` service
and its serializable `describe`/`call` contract to provider declarations and
Rpc's per-call lookup. That binding must retain the same owner, entry restrictions and
reload/drain guarantees, without adding an adapter plugin or a second registry.
The revision's Timeline-driven input withdrawal and peer-node offline behavior
belong to the Session event contract and rutis link work; neither is implemented here.

Declaration-driven method discovery and cross-language plugin method declarations
remain deferred: assembly still lists Rust owner namespaces explicitly. Generic
credential references/resolution are future work. D4 keeps business documents in
their source, not editable loader rows; see [ADR 004](../../adr/004-loader-plugin-management.md).

Transport flows cover actual owner methods, hello features, an owner reload
that keeps the same connection, an Rpc reload, local/remote and protocol-version
boundaries, business source CAS, persistence failures and invalidation. Unit
flows cover a provider reload that interrupts `Report` calls while an accepted
`Drain` call holds it, the namespace being unknown between generations, the
calling load's unload, a Pending provider that leaves other namespaces and
`hello` listing intact, reserved roots and nested ownership.
