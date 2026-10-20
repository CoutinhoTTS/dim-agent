# TypeScript process rows and management methods

Status: Proposed for [#4](https://github.com/arcships/dim/issues/4). This document
defines an App Server binding for review; it is not implemented. The existing
[C4 Session](session-typescript.md), [C5 branch](branch-typescript.md) and
[Rust management](appserver/management-methods.md) bindings remain the delivered APIs.

## 1. Scope and existing mechanisms

The App Server already assembles process plugins with rutis-loader, runs Node
rows, exports process services by name and dispatches native `ManagementMethods`
contributions. Its public TypeScript input currently installs only Session and
branch rows. A foreign `managementMethods:<namespace>` declaration is not bound
to a native management contribution.

Complete those two host bindings using the existing runtime, loader and method
owners. Plugin business state and work stay in its package. There is no new
Capability, adapter row, registry, manifest, event, RPC endpoint or rutis protocol.
Package acquisition, editable installation and CLI installation remain
[PluginManager](plugins/PluginManager.md) / [#3](https://github.com/arcships/dim/issues/3)
work. The embedding caller supplies the rows explicitly, as it does for C4.

Source anchors: [`serve` / `Processes`](../../crates/dim-app-server/src/lib.rs),
[`Parts` / same-row binding](../../crates/dim-app-server/src/session_typescript.rs),
[`ManagementMethods` / `Owners`](../../crates/dim-app-server/src/methods.rs) and
the [shared method owner](../../crates/dim-agent-capabilities/src/methods.rs).

## 2. Host input and process placement

Extend the existing public `SessionTypeScriptConfig` with
`process_rows: Vec<serde_json::Value>`. Keep `ServerConfig.session_typescript`, its
runtime arguments and its existing `rows` member. An empty `process_rows` keeps
the current Session/branch installation. The name of the configuration type is
retained to avoid an unrelated rename. Rust struct-literal callers must add
`process_rows: Vec::new()`; update all repository examples and transport fixtures
together. This is a Rust source change, not a persisted or wire format change.

Both arrays use ordinary loader rows and the existing `RowSchema.provides`
snapshot. For example, a row in `process_rows` can declare:

```json
{
  "id": "example-process",
  "name": "/absolute/example/process.ts",
  "provides": {
    "managementMethods:example": { "describe": "async", "call": "async" }
  }
}
```

The placement rules are:

| Input | Placement and lifetime |
| --- | --- |
| `process_rows` | Ordinary top-level rows in the existing process assembly; one instance per host, independent of Session creation and closure. |
| `rows` | Existing C4/C5 classification and instanced Session/branch placement, unchanged. |

Use the same public Node runtime, resolver and declaration catalog for both
arrays. Keep its existing runtime name and Settings event forwarding. Do not
start another runtime for each package or reuse the private managed-provider
slot. A process row uses its original non-instanced factory, not a fabricated
`SessionScope`. The original row remains the owner of its foreign resources and
any native method contribution.

This embedding input is trusted host configuration. It does not let a project
enable its own package or grant permissions, and introduces no host-local approval.

Validate the combined external configuration before assembly:

- Row ids are unique across both arrays and do not collide with host rows or
  generated module names. Rows of one module share the same declaration snapshot.
- Register process service names in the existing `ServiceCatalog` as shared,
  without an instance key. Reject duplicate process identities and collisions
  with native exports. Scoped identities retain their existing instance keys.
- A process row cannot inject Session/branch-only services or provide
  `toolContribution:*`, `sessionMethods:*` or `sessionEvents:*`. Session rows do
  not acquire process placement by declaring a management family.
- `managementMethods:<namespace>` has a nonempty, valid namespace and exactly
  the asynchronous `describe` / `call` modes. Other process-provided services
  retain the ordinary bridge method declarations; no host business parser is
  introduced for them.
- Reuse C4's comparison of the online module declaration with its structural
  `provides` snapshot, including its narrowly identified offline placeholder.
  A mismatch fails resolution of that row; names do not change at runtime.

Read `settings` source declarations from both arrays at the existing external
boundary, enforce one owner per source and strip that host-only metadata before
passing rows to the loader. A package declares a shared source on its process
row, rather than also declaring it on every Session part. Business updates
continue through Settings and its events; they do not reconcile loader rows.

### 2.1 Startup and availability

Keep the existing single initial process reconcile and startup deadline. The
configured process rows join the same initial assembly and the current process
readiness wait, including Pending rows. Do not skip Pending, add an optional
layer, detach reconciliation, or create another readiness phase or background
installation owner. Runtime declaration discovery still precedes host readiness.

This selects the existing required-process startup semantics for explicitly
configured rows: a Pending, failing or hung configured process row can prevent
the host from starting. Omitting the package installs no row and adds no wait.
The C4 guarantee that optional Session rows do not gate Session publication does
not promise optional process startup isolation.

Architecture review must confirm that startup boundary against ADR 002 D3 before
implementation. If optional process startup isolation is required, this binding
does not supply it; do not silently change the waiting rule or claim P6/P7 Ready.
After successful startup, Rpc continues to isolate an inactive management
provider to its namespace, using the existing lookup and row-status rules.

## 3. Management declaration and dispatch

Derive `(namespace, original row id)` from each process row's
`managementMethods:<namespace>` entry and append it to the existing `Owners`
input. There is no separate namespace configuration. Retain disabled owners in
the known management namespace inventory, matching native longest-namespace
routing: a disabled child namespace is unknown and is never served by its parent.

Apply existing namespace grammar, reserved roots, built-in exclusions, duplicate
ownership and management/Session overlap validation at assembly. Do not accept
a namespace that would fail `ManagementMethods::provide`. Rpc continues to
validate that every configured namespace names a loader row. It neither injects
nor retains a foreign provider; each request looks up the native contribution.

The language-neutral payloads are the implemented subset of
[session-boundary A.1](contracts/session-boundary.md#a1-method-contributions):

| Method | Input | Output |
| --- | --- | --- |
| `describe()` | No arguments. | `MethodDecl[]`: `name`, `mutating`, `interruption` (`report` / `drain`), `progress`. |
| `call(call, cancelled, progress)` | `{callId, method, params}`, a by-reference cancellation `Promise<void>`, and a callback receiving one JSON value. | `{kind:"value", value?}` or `{code, appError, message}`. Omitted value means JSON `null`. |

`callId` is the binding's opaque correlation id. Preserve the client's params
without adding a Session id or inventing origin fields. Notifications, handoff,
paired-device management, a generic `callChannel` service and new discovery
endpoints are outside this subset. Unsupported declarations or replies fail
explicitly, as in C4; they are never accepted and then ignored.

The native binding lives in the original process row's apply:

1. Subscribe to the existing service-projection notification before applying
   the resolved runtime row. Register its cleanup immediately with `ctx.effect`.
2. Await this row's declared foreign projections, using uninstanced
   `host_key("managementMethods:<namespace>")`. Read only its own services;
   external dependencies remain declared. Row cancellation ends a pending bind.
3. Call `describe`, decode once at the bridge boundary and provide native
   `ManagementMethods` with its existing declaration validation and
   `MethodProvider`. Publish it before this row becomes Active.
4. Reuse C1's foreign method decoding and invocation in the shared method owner;
   do not copy a second DTO/parser/handler implementation into Rpc. Session and
   management contributions keep their distinct native keys and scope rules.

Use the current protected-local protocol-v3 entry admission, `Owners` routing,
`hello.features`, row-status errors and progress/reply delivery. No call may
switch to a new provider generation after admission. Declared application
errors preserve their code and `appError`; transport or malformed-reply errors
use a redacted binding failure rather than exposing runtime stacks.

## 4. Work ownership and cleanup

Register native call admission/drain cleanup after foreign row cleanup, as C4
does, so accepted calls settle before the foreign row is released. Provider or
admitting Rpc unload resolves the cancellation Promise for both interruption
modes. `report` replies `operation_interrupted` while its original work settles;
`drain` returns the original work's result. Both remain tracked until settlement.
End each Promise lease on ordinary completion as well. A client disconnect does
not cancel admitted work. Uncooperative JavaScript may delay unloading; no forced
success, timeout, rollback or replay is inferred.

Business background tasks and their effects belong to the process package.
Closing a Session does not dispose that package. Process shutdown follows the
existing host shutdown and loader disposal sequence; the binding adds no second
lifecycle manager or generic process quiescence service. This does not claim to
settle P6/P7 timer, report or native-writer lifetime acceptance.

## 5. Impact and verification

| Owner | Necessary implementation scope |
| --- | --- |
| App Server assembly / TypeScript binding | Add `process_rows`, validate combined declarations, use original process factories, assemble their rows, collect Settings sources and derive management owner pairs. |
| Shared method owner and native management owner | Reuse the existing foreign decoding/call boundary; provide `ManagementMethods` inside the original row, with the existing native admission and cleanup. |
| SDK types and author guide | Describe the supported process contribution using the existing method payload definitions; retain existing Session exports for callers. |
| Examples and transport fixtures | Update all configuration literals and add a real Node process-owner fixture through normal protected-local Rpc. |

Implementation acceptance uses public flows, not private helper assertions:

1. One configured process owner serves two Sessions, survives either closing,
   and exposes a declared management method through local v3 `hello` and Rpc.
   Existing C4 Session/branch rows still use the same runtime correctly.
2. Empty `process_rows` preserves current C4/C5 behavior. A disabled row is not
   callable; malformed/colliding declarations are rejected; a real missing
   dependency reaches the existing startup deadline and cleanup. No Pending
   row is silently skipped.
3. After startup, reload the owner and observe existing management status and
   declaration behavior. Rpc, connections and Sessions stay loaded. Existing
   namespace precedence, local-transport restrictions and v1/v2 exclusions hold.
4. Exercise progress, a provider application error and an invalid reply through
   Rpc; verify admitted `report` and `drain` calls settle on original-owner and
   Rpc reload before the foreign cleanup finishes, including client disconnect.

Use disposable homes. Run `xtask check` / `graph` when assembly implementation
changes, the affected strict TypeScript and native Node flows, and the repository
merge checks for that implementation. This proposal changes documentation only.

This is not full P6/P7 readiness: their per-call `scheduleJobs` / `workflowRuns`
access from [#35](https://github.com/arcships/dim/issues/35#issuecomment-6188408082),
P6 live-policy/reuse flow and P7 definition-source/native-writer contracts remain
separate. A process service declaration does not authorize undeclared `ctx.use`,
an optional lookup API or a hard dependency from every Session to its owner.
P7 A17 already uses the first actual `sessions.launch` reply's policy.

## 6. Rule check

| Rule | Application |
| --- | --- |
| §4.1 Commands/results | No Timeline mutation is added. Process business methods retain their owner's typed writes; branch effects use existing Sessions/input contracts. |
| §4.2 Handover | Existing method admission and branch-input handovers remain unchanged; a lost reply does not undo work. |
| §4.3 Before/entry checks | No before callback. Host checks structural declarations; method owners validate business params. |
| §4.4 Core has no business meaning | Assembly and Rpc read service identities and namespaces, without Scheduler/Workflow cases. |
| §4.5 Batches | No event or batch change. |
| §4.6 Model context | Management bindings introduce no context path; inputs and tool results retain their existing owners. |
| §4.7 Public access | Only declared process dependencies and the original row's own projections are read. No private plugin state is accessed. |
| §4.8 Infrastructure | The host's existing cross-language assembly/management responsibility is completed under #4; §7 records its boundary. Startup waits are retained (§2.1). |
| §4.9 Durable facts | Assembly metadata and process call bookkeeping do not enter Timeline. |
| §1 Scope/App Server | Process parts are loader rows, not Capabilities; App Server owns assembly/wire binding, packages own all feature work. |
| ADR 002 D1 | Process rows outlive Sessions; existing Session/branch rows retain their own lifetimes. |
| ADR 002 D2 | Bind in the original row; no adapter or management plugin. |
| ADR 002 D3 | Rpc has no dependency on optional providers and resolves per call. The explicitly configured required-process startup boundary is a review point in §2.1, not a new optional-startup guarantee. |
| ADR 002 D4 | Structural declarations stay in rows; Settings documents remain business data and do not reload rows. |
| ADR 002 D5 | Named `managementMethods:<namespace>` implements the existing neutral payloads; Rust `ManagementMethods` is its native binding. |

## 7. Requirements raised to mechanism owners

| Item | Content |
| --- | --- |
| Blocked contract | ADR 002 D5 names process `managementMethods`, but App Server's public TS input only installs Session/branch rows and its native management key cannot be provided by a bridge declaration alone. |
| Existing mechanisms considered | rutis already supports process rows. Reuse the public C4 Node runtime, `RuntimeResolver`, `ServiceCatalog`, same-row projection pattern, native `Owners` and `MethodProvider`. Session factories have the wrong lifetime; the private account slot has the wrong ownership; events cannot install an absent process row or contribute a management namespace. |
| Generic binding | Explicit process rows in the existing embedding input, and same-row projection of the existing named management contract. No new core service, facade, rutis requirement or loader API. |
| Ordering and lifecycle impact | No Timeline sequence, batch or event change. Preserve process startup wait/deadline, local entry admission and provider-generation report/drain settlement; initial startup limitation is explicit in §2.1. |
| Implementation owner / issue | zerob13, App Server assembly and method binding under #4. Architecture review by the existing assembly/management maintainers; this does not assign new work to S2 or rutis. |
