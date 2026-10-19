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
Rpc does not depend on any owner ([methods contract §3.2](../contracts/session-boundary.md#32-management-method)).
Assembly passes Rpc its profile's namespaces, each with the loader row of the
owner that provides it (`account`, `account.codex`, `account.grok` and `provider`
with a product home; `settings` (SettingsMethods) wherever Settings is assembled; `mcp`, `skills`
and `permission` with local execution). Adding an owner
adds its pair where its row is registered, and Rpc refuses to load when a pair
names no row. On each call Rpc makes a keyed read of the namespace that owns the
name and dispatches to its active provider; it reads that owner row's status
only when there is none (below). Reads go through the process root, so Rpc's own
unload hides no provider. rutis is an external dependency: this uses its keyed
reads and the loader's row status as they are, and raises no requirement to it
([principles §4 rule 8](../../architecture/rust-agent/principles.md)). No
contributor injects Rpc or Connection; neither contains contributor-specific
dispatch. Adding a method changes its owner's declaration, parsing and
implementation.

Account, Codex, Grok, Provider and Mcp provide their methods directly, without
adapter rows. Settings supplies generic BusinessDocument persistence and the declared sources of
[the settings service](settings-service.md), not approvals, a method contribution or a configuration
namespace; SettingsMethods, which depends on it, owns `settings.read`/`settings.update`.
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
keys are unique per scope. A name belongs to the longest configured namespace
that prefixes or equals it, the contract's longest provided namespace: a
parent's method inside a child namespace is neither dispatched nor listed, even
while the child is disabled or not active. Valid nested namespaces such as
`account.codex` remain independent from the parent `account` namespace.

## Dispatch and lifecycle

Entry admission precedes routing for built-in and contributed methods alike:
a protected profile answers every call except `hello` and `daemon.ping` from a
non-local transport with `local_transport_required`, as before contributions
([methods contract A.3](../contracts/session-boundary.md#a3-entries-and-export)). Local v3 then dispatches management contributions; v1/v2 do not.
A name inside a Session namespace recorded with `AgentBuilder::session_methods`
is a Session method instead, dispatched through the Session its `sessionId`
names ([methods contract A.1](../contracts/session-boundary.md#a1-method-contributions));
Rpc fails to load when a recorded Session namespace overlaps a configured
management namespace, so no name has both owners.
A namespace with no enabled provider (no configured namespace owns the name, or
its owner row is disabled), or a name its provider does not declare, returns
`unknown_method`. A namespace whose owner row is not running and has not failed
(Pending, not started yet, or between generations) returns the retryable
`service_reloading`; one whose owner row cannot run again until it is fixed or
restarted (Failed, Unresolved or Disposed) returns the non-retryable
`service_unavailable`. Rpc tells these apart from the loader's row status
(contract §3.2). A Session namespace whose provider is not active in the
named Session answers the same way from the row that provides it, read as
that row's copy in the Session's instance: assembly records the row of each
namespace a TypeScript Session part declares in `RowSchema.provides`, and Rpc
finds the copy whose innermost instance is the Session's (`Session::scope`)
among the loader's entries. A Session namespace with no recorded row (a Rust
Session part, which must be active before its Session is published), or whose
copy left with the Session's instance, returns `service_reloading`, as does a
Session that is reloading. A name outside
the declaration grammar (dot-separated
lowercase segments, at most 128 bytes) is unknown without any lookup, so a
caller's name bounds the lookups made under the handler lock. Error codes at the
edges are unchanged: a call before `hello` is `-32600` and one after shutdown has
begun is `-32603`; an unknown method is `-32601` whatever its params, and only
the owner of a declared method rejects its params (`-32602`), except that Rpc
reads the `sessionId` of a Session method.
The proposed `ConnectionEntry {name, deviceId}` and paired-device routing remain
RemoteGateway work; its connections must never be served management methods.
`hello.features` lists the methods of the providers active at that `hello`.
A provider that becomes active later appears in the next `hello`. The withdrawn
`management.methods` endpoint is not provided. Existing Session and daemon
methods keep their built-in contracts.

How Rpc answers a local v3 call whose name a configured namespace owns, or a
Session method whose namespace has no active provider in its Session, and what
the client does with each reply (contract §3.2, A.1, Appendix C):

```text
provider of the namespace active? --yes--> provider answers
        |                                  (a name it does not declare: unknown_method)
        no
        v
providing row status: owner row        reply                         client
(loader.get) or Session row's copy
------------------------------------   ---------------------------   ----------------------------------
owner row absent, or row Disabled      -32601 unknown_method         stop: nothing serves the name
Session row not recorded, or its       -32000 service_reloading      retry: the part or the Session
  copy left with the Session                                           is coming back
Inactive: not started yet,             -32000 service_reloading      retry: the owner is coming back
  or between generations
Running: Pending, Loading, Unloading   -32000 service_reloading      retry: the owner is coming back
Running: Active (after the lookup)     the provider answers
Running: Failed or Disposed            -32000 service_unavailable    stop: retry after a fix or restart
Unresolved                             -32000 service_unavailable    stop: retry after a fix or restart
Stopped (instance copies only)         -32000 service_unavailable    stop: retry after a fix or restart
a status the loader adds later         -32000 service_unavailable    stop: retry after a fix or restart
```

`loader.get(row)` reads an owner row itself, and the loader reports Stopped
only for a copy inside an instance, so Stopped reaches Rpc only from a Session
row's copy that disposed itself. Two owners
answer `service_reloading` although they will not come back without a fix
([#124](https://github.com/arcships/dim/issues/124)): one that is Pending on a
dependency that failed, and one that disposed itself. The process loader has no
editable layer in which to disable the second one's row, so once the loader
drops its fiber, the row reads Inactive.

Rpc admits contributed calls under the same handler lock as built-in commands
and tracks reply delivery in its command tracker. A request id is the client's
again once its reply is sent, even while an interrupted operation settles. The
operation belongs to its provider: the provider admits it under its own gate and
tracks it until it settles. Either unload, the provider's or the calling Rpc load's, signals the
handler. A `Report` call then replies `operation_interrupted`, stating that the
operation may already have committed and the current state must be read before
retrying; a `Drain` call replies with its own result. A provider's unload waits
for every call it accepted, so `Drain` calls hold it; this is the owner's own
effect within rutis' existing lifecycle. Rpc's unload
waits for the replies it owes and for each interrupted operation to settle, so the
next load reads its outcome. Once Rpc's own load is unloading it admits no
management call or `hello`; they return `service_reloading` and the next load
serves them. A caller's disconnect cancels nothing.
This reproduces the behaviour before contributions
([methods contract A.4](../contracts/session-boundary.md#a4-migration-and-compatibility)). Provider and
account methods use `Drain`; MCP and Skills use `Report`. Settings retains its
accepted-write contract.

An owner that is disabled, Pending, Failed or reloading affects only its own
namespace; Rpc and open connections are unaffected. While its row is Pending,
not started yet, or between generations its methods return `service_reloading`,
as before contributions when Rpc reloaded with it (contract A.4); while it is
Failed, Unresolved or Disposed they return `service_unavailable`; a disabled
owner's methods return
`unknown_method`. Rpc and Agents depend on no owner, directly or
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

A method that declares `progress` sends progress on its call through the
handler's `Progress`; Rpc delivers it as `management.progress {id, method, data}`
to the calling connection only, before the reply, and drops it after a
disconnect ([A.2](../contracts/session-boundary.md#a2-call-channel)). Progress
is best effort: a connection queues at most 256 progress frames and 4 MiB of
them, beside the 1024 frames and 32 MiB of replies and notifications; it
drops a notice that does not fit, so one over 4 MiB never arrives, and stays
open. Management
and Session methods share this path; no management method declares `progress`
today. Connection-bound notifications belong to the contract's
`callChannel.notify`, which is not implemented. Settings source watches invalidate business consumers
after successful persistence; they are not loader events or management
notifications. Persistence does not certify runtime adoption or generation
settlement.

## Limits and verification

RemoteGateway, rutis-link export, connection-bound CallChannel notifications and
shutdown reply handoff are not implemented in this subset. Contributed Session
methods (`SessionMethods`) share its declarations, handler, errors and call
admission, which live in dim-agent-capabilities `methods`
([methods contract Appendix F](../contracts/session-boundary.md#appendix-f-rust-binding)). There is no new settings authority, loader or authorization system.

Completing D5 requires binding the named `managementMethods[namespace]` service
and its serializable `describe`/`call` contract to provider declarations and
Rpc's per-call lookup. That binding must retain the same owner, entry restrictions and
reload/drain guarantees, without adding an adapter plugin or a second registry.
The revision's Timeline-driven input withdrawal and peer-node offline behavior
belong to the Session event contract and rutis link work; neither is implemented here.

Declaration-driven method discovery and cross-language plugin method declarations
remain deferred: assembly lists the Rust owner namespaces and their rows
explicitly. There is no generic credential service; each owner handles its
own secrets. D4 keeps business documents in
their source, not editable loader rows; see [ADR 004](../../adr/004-loader-plugin-management.md).

Transport flows cover actual owner methods, hello features, an owner reload
that keeps the same connection, an Rpc reload, local/remote and protocol-version
boundaries, business source CAS, persistence failures and invalidation; every
flow assembles the real owner rows, so a pair naming no row fails it. Unit flows
on a real loader cover a provider reload that interrupts `Report` calls while an
accepted `Drain` call holds it and answers `service_reloading` between
generations, the calling load's unload, Pending owners answering
`service_reloading` and Failed and Unresolved owners answering
`service_unavailable` while other namespaces and the `hello` listing stay intact,
disabled and unconfigured owners answering `unknown_method`, reserved roots, and
nested ownership with an enabled or a disabled child.
Session method flows (`transport_flow/session_methods.rs`) contribute a
namespace from a test plugin and cover dispatch through the Session, the
routing errors, a provider reload, Session close and restore, `session.methods`,
per-call progress, and progress dropped for a caller that does not read while
its reply still arrives. TypeScript Session part flows
(`transport_flow/session_typescript.rs`) cover a Pending part's namespace
answering `service_reloading`, and Failed and Unresolved parts' answering
`service_unavailable` on repeated calls.
