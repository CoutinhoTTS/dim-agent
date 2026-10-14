# Management

Status: management-method registration and owner adapters implemented. This
page records the delivered subset of PR #15's Management and session-boundary
designs, including its dependency-direction ADR. It is not a second loader or
configuration authority, nor a claim that every proposed entry is implemented.

## Responsibility and mounting

Methods is the stable RPC-side method port, with no contributor dependencies.
Management is a level-filtered view of that single port. Owners declare
`Arc<Methods>` and their actual business services in `Deps`, register namespaces
and methods during `apply`, and immediately attach `unregister_and_drain` to an
asynchronous `ctx.effect`. Owners must not inject RpcService or inspect the
ConnectionHub. Concrete parameter validation and result serialization belong to
the owner adapter; public Rust DTOs remain available from `protocol`.

The local v3 entry dispatches management contributions. WebSocket does not.
Protocol decodes the envelope and built-in calls, passing other names and raw
parameters to the method port. The longest registered namespace owns a name;
only its declared methods are callable. A namespace cannot capture another
owner's existing method, and built-in namespaces cannot be shadowed. Account,
Codex and Grok own `account`, `account.codex` and `account.grok` independently.
Unknown names and methods on a non-serving entry return `-32601` with
`unknown_method`; malformed envelopes and concrete adapter parameters retain
JSON-RPC errors. Mounting management is host-administrator access.

`management.methods` accepts `{}` and returns a revision and method descriptors
(`name`, `namespace`, `mutating`, `progress`, `notifications`), sorted by name,
including itself. Its catalog is also the management dispatch catalog. Hello
snapshots that catalog for mounted entries; discovery can be called again after
contributions change. Existing Session wire contracts are preserved.

## Registration and accepted work

A registration owns one generation, its accepted task tracker and its unload
cancellation token. There is no shared business job registry. Duplicate names,
reserved namespaces and empty registrations fail atomically.

Dispatch tracks the task under the same catalog lock that removal uses. Cleanup
unregisters only entries still owned by its generation, signals cancellable
calls, and drains accepted tasks. No lock is held across an await. Removal
prevents late admission; old cleanup cannot remove a replacement generation.
`Drain` methods finish even when their caller leaves. `Report` methods report
interruption and signal cancellation on contributor or RPC unload, or client
disconnect, while retaining the operation until it settles. Mutation owners
must not abandon an already accepted write. Configuration, provider and account
operations use `Drain`; MCP and Skills use `Report`. Settings retains its
accepted-write contract.

An adapter unload does not reload Methods, Management, Connection, Rpc or other
adapters. Account, provider, MCP, Skills and configuration management handlers
live with their owners, not in Connection's dispatch table. Session-specific MCP
state, follow and refresh remain Session protocol concerns.

Routing isolation does not remove existing execution dependencies: Rpc still
uses Provider for model preferences, Agents uses Settings and model selection,
and ToolRegistry uses Session MCP contributions. These edges need their own
owner-port migration before claiming business-plugin restart isolation.

## Notifications

`Registration::notifications` supplies a declared generic notification port.
Owners publish names and serialized parameters without knowing connections or
protocol versions. Ports and queued notices are generation fenced. Connection
delivers only to local v3 connections of the current protocol instance, after
hello entered the ordered outbound queue.

ConfigurationManagement owns coalesced loader/persistence invalidations and
emits `configuration.changed` with `{}`. Invalidation acknowledges neither
persistence nor runtime adoption. The feature is advertised only while its
adapter is registered.

## Deliberate limits

Rutis-link export/import and RemoteGateway are not implemented. There is no
new authorization system, loader implementation, framework, dependency or
settings authority in this change.

The port carries method levels, but this entry dispatches contributed management
methods only. Contributed Session authorization, per-call progress,
connection-bound notification channels and shutdown reply handoff are not
implemented here. Existing Session and daemon methods keep their built-in
contracts. Global configuration invalidation is not a substitute for PR #15's
connection-bound `CallChannel`.

## Verification

The public transport flow registers runtime contributors, checks catalog and
hello availability, rejects a duplicate batch without partial registration,
removes and re-adds a contributor through real rutis reload, and proves an
accepted mutation drains after client disconnection. During that drain another
method and the same live Session remain usable. A WebSocket client cannot invoke
the contributed methods. The flow also covers reserved and nested namespaces,
catalog revisions, and cancellation of a `Report` call during unload.
