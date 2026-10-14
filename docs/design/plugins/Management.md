# Management

Status: Draft. Overall design: [overview §5.16](../overview.md#516-management), [common constraint 8](../overview.md#3-integration-methods), [§3.1 Multiple nodes](../overview.md#31-multiple-nodes), [§7.2 Rpc change](../overview.md#72-app-server-process-plugins). Phase: P0. Language: Rust.

Management is the management plane of the App Server process. It collects management methods that process plugins contribute as named contributions, serves them as one dispatchable service, and is mounted on entries chosen by assembly: the local RPC entry by default, never the RemoteGateway entry by default, and other nodes only when the service is exported over a rutis link. Whether management is possible depends only on mounting and export; there are no identity tiers and no per-method authorization.

## 1. User flows

1. The CLI connects over the local socket with protocol v3. `hello.features` lists the management methods currently mounted, as today. The CLI calls `mcp.save`; Rpc does not know the method, finds Management mounted on the local entry, and Management routes the call to the `mcp` contribution, which validates and executes it.
2. A client calls `management.methods` and receives the methods available on its entry with a revision; after a contributor reload it calls again and sees the same set with a new revision.
3. The Mcp plugin is restarted. While its contribution is not active, `mcp.*` calls return `service_reloading`; Session methods and every other management namespace keep working.
4. A user installs a plugin package that contributes `schedule.*`. Management discovers the new contribution from the loader and mounts it; `management.methods` now lists it.
5. A phone connected through RemoteGateway calls `provider.save`: the remote entry does not mount Management, so the call fails with `unknown_method`.
6. Another node is given host administration: the operator exports Management over the rutis link to that node; the peer calls `ManagementService::call` through the link with the same methods and results as the local entry.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| The management contribution contract and its mounting per contributor | Business logic, parameter formats and errors of each method (the contributor) |
| Method naming and namespace ownership rules | Session methods (Rpc) |
| Dispatch, admission accounting, interruption mapping, `management.methods` | Authentication of entries or links (Transport, RemoteGateway, rutis link) |
| Protocol compatibility of migrated methods for v1–v3 clients | Choosing which entries mount it (assembly and link configuration) |
| Migration plan of `account.*`, `provider.*`, `skills.*`, `mcp.*` out of Rpc | Notifications and push channels (none are added) |

## 3. Scope and dependencies

**Contracts defined here** (owner: Management, the consumer of contributions): `ManagementContribution`, `ManagementProvider`, `MethodDecl`, `Interruption`, `ManagementCall`, `CallOrigin`, `ManagementError`, `ManagementService`, and the method naming rules of §6. **Contracts depended on**: the rutis-loader `Loader` (entries, row declarations, instanced group instances, `LoaderChanged`), the Rpc mount point (Rpc change, §6.4), rutis service export over links ([rutis nodes](https://github.com/arcships/rutis/blob/main/docs/guide/nodes.md)). Contributors depend only on this contract and never on Management internals; Management never knows a contributor or a method's meaning. Any plugin providing `ManagementService` with these rules can replace Management.

| Part | Scope | Injects | Provides | Status |
| --- | --- | --- | --- | --- |
| `ManagementPlugin` | Process | `Arc<Loader>` | `ManagementService` | Proposed |
| `ManagementMount` (instanced group `management-mount`, one instance per contribution name) | Process | `Arc<ManagementService>`, the keyed `ManagementContribution` of its name | — (registers into the service) | Proposed |
| Contributors | Process | their own dependencies | `ManagementContribution` under key `TypeKey::keyed_dynamic::<ManagementContribution>(name)` | Proposed per plugin |
| `RpcPlugin` | Process | existing minus management services; added `When<Arc<ManagementService>>` | `RpcService` | Change |

**Why per-contributor mount instances instead of one aggregated `Contributions` injection** (A1). ToolRegistry injects all named contributions in its own `Deps`. Doing the same here would make Management (and through it Rpc) Pending or reloading whenever any contributor is: a broken Mcp row would also disable `plugin.*`, the methods needed to repair it, and every contributor restart would reload Rpc. Each `ManagementMount` instance instead injects exactly one named contribution and the service, so a contributor's lifecycle affects only its own methods, and Management itself depends on no contributor. Contributors still follow the ToolRegistry pattern: they provide a named contribution and know nothing about who consumes it.

**Discovery.** Management reads loader entries and their declarations (`Resolved::declares`: what a row waits for and provides) and creates one `management-mount` instance for every name `n` such that an enabled row declares it provides `ManagementContribution` keyed `n`; it removes the instance when no enabled row declares it. It re-evaluates on `LoaderChanged`. Built-in plugins and packaged plugins (installed by PluginManager) are discovered the same way; neither writes Management configuration (A2). Mount instances are created after `ManagementService` is provided; the host's ready latch additionally waits until the mounts discovered from the initial entries are Active or failed, so the first `hello` sees the complete set (A3).

## 4. Events

None. Management writes and consumes no Timeline event. Management calls are host configuration and command operations; any Session-visible consequence (for example a newly approved MCP server being discovered by an active Session) is produced by the owning plugin through its own facts. Call records go to the diagnostics channel.

## 5. State

| State | Owner | Rebuild |
| --- | --- | --- |
| Registry: name → registered contribution, its `MethodDecl`s and mount generation | `ManagementService` | Rebuilt by the mount instances' `apply` on every load; not persisted |
| Configured namespaces (rows declaring a contribution, enabled) | `ManagementService` | Recomputed from `Loader::entries` and declarations |
| Revision of the method set | `ManagementService` | `"<generation uuid>:<counter>"`, new per Management generation, never reused (A4) |
| In-flight call accounting per mount generation | `ManagementMount` | Runtime only; drained or reported on unload |
| Business state behind each method | Contributor | Contributor's document |

## 6. Interfaces

### 6.1 Contribution contract

```rust
/// Process service; key TypeKey::keyed_dynamic::<ManagementContribution>(name), no instance.
pub struct ManagementContribution(pub Arc<dyn ManagementProvider>);

pub trait ManagementProvider: Send + Sync + 'static {
    /// Complete method list of this generation; fixed for the generation (A5).
    fn methods(&self) -> Vec<MethodDecl>;
    /// Validates `params` at this boundary and executes. `cancel` fires only when
    /// the caller is gone AND the method is `Interruption::Report`; the provider
    /// may ignore it and keep its admitted work (it owns that work).
    fn call(&self, call: ManagementCall, cancel: CancellationToken)
        -> HostFuture<'_, Result<serde_json::Value, ManagementError>>;
}
pub struct MethodDecl {
    pub name: String,              // full wire name, e.g. "mcp.save"
    pub mutating: bool,            // changes host configuration or runs a command
    pub interruption: Interruption,
}
pub enum Interruption {
    /// On Rpc or mount unload the caller receives `operation_interrupted`;
    /// the operation may already have committed (today's mcp/skills behavior).
    Report,
    /// Unload waits for the call to finish (today's account/provider behavior).
    Drain,
}
pub struct ManagementCall {
    pub method: String,
    pub params: serde_json::Value, // raw JSON-RPC params, `{}` when absent
    pub origin: CallOrigin,        // diagnostics only, never authorization
}
pub enum CallOrigin { Local { connection_id: String }, Entry { entry: String, connection_id: String },
    Link { peer: String } }
pub struct ManagementError {
    pub code: i32,                     // -32602, -32601 or -32000
    pub app_error: Cow<'static, str>,  // [a-z][a-z0-9_]*, e.g. "mcp_settings_conflict"
    pub message: String,               // public text; never secrets or private configuration
}
```

A contributor implemented in another language provides the same service through a rutis runtime row (precedent: the managed provider in the [App Server contract](../../../crates/dim-app-server/README.md#optional-managed-provider)): `methods()` and `call({method, params, origin})` are `async` and return the envelope `{ok: true, value} | {ok: false, error: {code, appError, message}}`. Rutis 0.10 does not carry cancellation to Node, so `Report` callers are released while the Node call continues (A6).

### 6.2 Method naming

| Rule | Content |
| --- | --- |
| Namespace | The contribution's registered name, `[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*`; chosen and owned by the contributor (overview common constraint 10) |
| Method name | `<namespace>.<segment>(.<segment>)*`, segments `[a-z][a-z0-9_]*`, at most 128 bytes |
| Ownership | A name belongs to the longest registered namespace that prefixes it. Sub-namespaces are allowed (`account` and `account.codex`); the shorter owner must not declare names inside a registered longer one |
| Reserved | `management` (Management's own methods); every name Rpc serves and its namespaces (`hello`, `daemon`, `session`, `approval`, `agent`) |
| Rejection | A contribution whose list violates a rule, or collides with another active contribution, is rejected as a whole: its mount instance reports the error in diagnostics and is not registered; the earlier active owner keeps its names (A7) |
| Stability | Wire names are stable across releases and implementation languages ([overview §11.4](../overview.md#114-moving-from-ts-to-rust)) |

### 6.3 Management's own methods

| Method | Params | Result | Notes |
| --- | --- | --- | --- |
| `management.methods` | `{}` | `{revision: string, methods: [{name: string, namespace: string, mutating: bool}]}` | The methods registered and active now, sorted by name; available wherever Management is mounted |

No push notification is added; a client re-reads `management.methods` after `unknown_method` or `service_reloading` (A8).

### 6.4 Rpc change and mounting

| Item | Content |
| --- | --- |
| Rpc table | Rpc decodes the envelope and serves only its own methods (§6.5). For any other method on a v3 connection whose entry mounts Management, it admits the call as an ordinary command (request id, per-connection limit, command tracker) and calls `ManagementService::dispatch` |
| Entry mount flag | Each connection records its entry at accept time: the local transport entry has `mounts_management = true` when Management is configured; the bare WebSocket entry and RemoteGateway's entry have `false` unless their assembly says otherwise ([RemoteGateway](RemoteGateway.md)). The flag is assembly data, never derived from identity |
| `hello.features` (v3) | Today's protocol and Session features, plus, on mounting entries, `management.methods` and the names of `ManagementService::methods()` at hello time |
| Not mounted or v1/v2 | `-32601`, `appError: "unknown_method"`, "Management method is unavailable", exactly as v1/v2 and unavailable methods behave today |
| Namespace configured but inactive | `-32000`, `service_reloading` (retryable) |
| Unknown name | `-32601`, `unknown_method` |
| Link export | `ManagementService` is exported through the rutis node configuration of a link; its `dispatch` takes `CallOrigin::Link`. Export configuration is host-local link configuration, not a management method of the exported plane (A9) |

```rust
impl ManagementService {
    pub fn methods(&self) -> MethodSet;                         // revision + active decls
    pub async fn dispatch(&self, call: ManagementCall, caller_gone: CancellationToken)
        -> Result<serde_json::Value, ManagementError>;
    pub fn register(&self, ctx: &Ctx, name: &str, provider: Arc<dyn ManagementProvider>)
        -> Result<(), ManagementError>;                         // used only by ManagementMount
}
```

### 6.5 Classification and migration of existing methods

Source of the current lists: `MANAGEMENT_METHODS`, `MCP_METHODS`, `LOCAL_EXECUTION_METHODS` and `Call::is_management` in [protocol.rs](../../../crates/dim-app-server/src/protocol.rs); gating in `validate_call` in [connection.rs](../../../crates/dim-app-server/src/connection.rs); the Rpc row of [single-tree assembly](../../architecture/rust-agent/single-tree.md).

| Methods | Destination | Contributor (namespace) |
| --- | --- | --- |
| `account.providers/status/recover/logout`, `account.login.begin`, `account.login.browser.begin/complete`, `account.login.status/cancel` (9) | Management | `ManagedProvidersPlugin` (`account`) |
| `account.codex.*` (7) | Management | `OpenAICodexOAuthPlugin` (`account.codex`) |
| `account.grok.*` (6) | Management | `GrokOAuthPlugin` (`account.grok`) |
| `provider.*` (11) | Management | `ProviderPlugin` (`provider`) |
| `skills.catalog`, `skills.set_enabled`, `skills.refresh` | Management | `SkillsCatalogPlugin` (`skills`) |
| `mcp.catalog/set_enabled/save/remove/disconnect/approve/revoke` | Management | `McpPlugin` (`mcp`) |
| `session.create/list/summaries/attach/detach/submit/input_lookup/history/stop/idle/close/branch_create`, `approval.respond`, `session.permission.get/update`, `session.exec.stop`, `session.skills`, `session.model.update`, `session.prompt.refresh`, `session.mcp`, `session.mcp.refresh`, `agent.create/list/close` | Rpc (Session methods) | — |
| `hello`, `daemon.ping` | Rpc (protocol) | — |
| `daemon.shutdown` | Rpc, available only on entries that mount Management (A10) | — |

Interruption declarations reproduce today's behavior: `mcp.*` and `skills.*` are `Report` (today they race `rpc.unloading` and return `operation_interrupted`); `account.*`, `provider.*` are `Drain`. Mutating flags follow each method's contract. After migration Rpc no longer injects `ProductServices` or the management part of `ExecutionServices`; it keeps only what Session methods use. A Provider or Mcp restart therefore no longer reloads Rpc.

Compatibility for existing clients:

| Client | Before | After |
| --- | --- | --- |
| v1, v2 | Management methods → `-32601 unknown_method` | Same |
| v3 local, product profile | `hello.features` contains the 33 product names; calls succeed | Same names (from contributions active at hello) plus `management.methods`; same params, results, codes and `appError` values |
| v3 local, execution | `LOCAL_EXECUTION_METHODS` + `MCP_METHODS` in features | Same names; `session.*` entries now come from Rpc's own feature list |
| v3 over WebSocket, protected profile | Everything except `hello`/`ping` → `local_transport_required` | Same for the bare WebSocket entry |
| v3, MCP absent | `mcp.*` → `-32601 unknown_method` ("MCP integration is unavailable") | `-32601 unknown_method` with Management's message (A11) |

No protocol version is added. Persistence formats are unchanged; the move is internal routing.

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Writes | None. Management and mount instances never append to any Timeline; contributors that change Session-visible state do so through their owners' public ports |
| Dispatcher | Not subscribed. Management has no `StateCell` and no branch or Session instance |
| Registration | `ManagementMount::apply` resolves its two `Deps`, reads `methods()`, validates names (§6.2) and calls `register`; the registration's removal is its `ctx.effect` disposer, which first applies the interruption rule to its admitted calls (§8) and then unregisters. The registry bumps its revision on every change |
| Dispatch | `dispatch` looks up the owner by longest namespace, takes an admission slot from the owner's mount generation (refused once that generation started unloading) and awaits `ManagementProvider::call`. Rpc tracks the request as a command, so Rpc unload drains or reports it per `Interruption` |
| Recheck | Management decides nothing from Session facts; contributors recheck their own authority per call (for example Mcp's approval recheck) |
| Contributor reload | rutis reloads the dependent mount instance only: methods of that namespace return `service_reloading` until the new generation registers; nothing else reloads |
| Management reload | Every mount instance reloads (it depends on the service) and Rpc reloads (`When<ManagementService>`), as today's Rpc reload table describes; contributors are unaffected |
| Process restart | Nothing persisted; discovery and mounts rebuild from loader entries before the ready latch opens |

## 8. Timing and conflicts

Management has no Timeline facts; rows decide by admission order on the service and by each contributor's own persistence revision checks.

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Turn `stop/interrupt`, branch interrupt or branch close during a management call | — | No effect; management calls are not branch work | — |
| Concurrent Steer/Queue/Append, unsettled tool batch, compaction, child branch cut | — | Not applicable: Management reads and writes no Timeline | — |
| Two clients mutate the same configuration concurrently | storage revision order | Second gets the contributor's conflict (e.g. `mcp_settings_conflict`) | Contributor |
| Contributor reload while its `Report` call runs | admitted < unload | Caller gets `operation_interrupted`; the operation may have committed; the contributor's disposer settles it | Mount, contributor |
| Contributor reload while its `Drain` call runs | admitted < unload | Unload waits for the call; then reloads | Mount |
| Call arrives during contributor reload | unload < call | `service_reloading`; not executed | Management |
| Rpc reload with calls in flight | admitted < unload | `Report` → `operation_interrupted`; `Drain` drained; new calls `service_reloading` | Rpc |
| Management reload | — | Mounts and Rpc reload; window calls `service_reloading` | rutis lifecycle |
| Duplicate or late call (client retries after a lost reply) | two admissions | Executed twice; idempotence is the contributor's contract; callers read state before retrying a `mutating` method | Contributor, client |
| Client detaches or disconnects mid-call | — | Call continues; reply discarded; `Report` cancel token fires, contributor may keep its work (detach is not cancel) | Contributor |
| Namespace collision on install | second registration | Rejected; first owner keeps names | Management |
| Method set changes between `hello` and a call | — | Call uses the current set; features are a hint | Management |
| Contributor depends on a resource node that is offline | contributor Pending | Its namespace returns `service_reloading` | rutis lifecycle |
| Link to an exporting peer drops mid-call | — | Peer sees a link error; the call on this node continues (detach semantics) | rutis link, contributor |
| Process restart mid-call | — | Reply lost; outcome per contributor's persistence; not replayed | Contributor |
| Session unload/restore | — | No effect | — |

## 9. Context mapping

No model-visible effect. Management adds no tool, input, prompt source or Timeline event, and management calls are never projected into model context. Management methods are not model tools; a plugin that also offers the model an operation (for example `schedule`) does so through a process-level tool contribution with its own risk category ([process tool contribution](../contracts/process-tool-contribution.md)). Diagnostics of Management never reach the model.

## 10. Errors and recovery

| Case | Result |
| --- | --- |
| Invalid params | Contributor returns `-32602 invalid_params` ("Invalid request parameters"), as Rpc does today |
| Method not served on this entry, or v1/v2 | `-32601 unknown_method` |
| Namespace configured but its mount not Active | `-32000 service_reloading` |
| Contributor generation unloading with a `Report` call | `-32000 operation_interrupted`, "the operation may already have committed. Read the current state before retrying" |
| Contribution rejected (names) | Not registered; diagnostics name the row and conflicting names |
| Contributor panics in `call` | `-32000 management_failed`; mount stays registered; diagnostics only (A12) |
| Malformed cross-language envelope | `-32000 management_failed`; reply text not forwarded |

## 11. Security

Mounting is authority: an entry that mounts Management, and a peer to which it is exported, has host administrator rights, including changing approvals and configuration and running commands without Approval (overview §3.1). Management performs no authentication and no per-method authorization; `CallOrigin` is recorded in diagnostics only. Defaults: local protected transport mounts; the bare WebSocket entry and RemoteGateway do not; nothing is exported over links unless explicitly configured. Contributors are trusted in-process or trusted runtime code (AGENTS rule 11). Credentials never appear in params echoed to diagnostics, in `management.methods`, or in error messages; contributors keep today's redaction rules. Host-local approvals (`mcp.approve`, `hooks.approve`, `plugin.approve`) remain management methods and are therefore unavailable on non-mounting entries.

## 12. Acceptance

Flow tests in the App Server suite, through the wire protocol only:

1. v3 local product and execution profiles: `hello.features` equals today's sets plus `management.methods`; a recorded transcript of every migrated method (params, results, error code and `appError`) matches the pre-migration transcript.
2. v1 and v2 connections receive `unknown_method` for every management name; the bare WebSocket entry of a protected profile receives `local_transport_required`.
3. `management.methods` lists active methods; restarting Mcp yields `service_reloading` for `mcp.*` while `provider.*`, `skills.*` and Session methods succeed; afterwards the revision changed.
4. A test contributor with a `Report` method blocked mid-call is reloaded: caller gets `operation_interrupted`; a `Drain` method delays the reload until it returns.
5. A second contribution claiming `mcp.save` is rejected; a sub-namespace `account.codex` coexists with `account`.
6. An entry configured without the mount returns `unknown_method` for all management names and `daemon.shutdown`.
7. With Management exported over a test rutis link, the peer calls `mcp.catalog` successfully; without export the service is not reachable.
8. Disconnecting during a mutating call does not roll it back; reconnecting and reading state shows the committed result.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | One mount instance per contribution rather than aggregated `Contributions` injection | Judgment (lockout and reload blast radius); overview §5.16 says "like ToolRegistry" | One Pending contributor disables the whole management plane and contributor reloads reload Rpc |
| A2 | Contributors are discovered from loader row declarations; no contributor list is configured | Judgment; [single-tree](../../architecture/rust-agent/single-tree.md) "dynamic sources through a declared management service" | The host or PluginManager maintains Management's list explicitly |
| A3 | The ready latch waits for initial mounts | Judgment (preserve today's `hello.features`) | Early clients may miss features and must call `management.methods` |
| A4 | Revision format `"<generation uuid>:<counter>"` | Judgment | Any never-reused scheme |
| A5 | A contribution's method list is fixed per generation | Judgment | Dynamic lists need a change signal like `ProcessToolProvider::changed` |
| A6 | Node contributors are not cancelled; callers are released | Rutis 0.10 limitation in the App Server README | — |
| A7 | Collisions reject the newcomer, first owner keeps names | Judgment | Ownership could flap with load order |
| A8 | No management change notification | Judgment; overview adds none | A `management.changed` notification would be added |
| A9 | Export is configured host-locally, not through a management method of the exported plane | Overview §3.1 "exported only when explicitly configured" | Exported peers could widen their own exports |
| A10 | `daemon.shutdown` stays in Rpc but only on mounting entries | Judgment; it needs the requesting connection's reply writer ([App Server shutdown](../../../crates/dim-app-server/README.md#shutdown-completion)) | Moved into a `daemon` contribution with a connection handoff port |
| A11 | Error messages may change; codes and `appError` do not | Judgment | Contributors reproduce exact message texts |
| A12 | `management_failed` as the generic failure code | Judgment | Another code name |

## 14. Open questions

| Question | Description |
| --- | --- |
| Rpc "only Session methods" (overview §7.2) | `hello`, `daemon.ping`, `daemon.shutdown` and `agent.*` stay in Rpc; this document treats `agent.*` as Session-plane directory methods and gates `daemon.shutdown` by the mount flag. Maintainers confirm |
| `session.shell.*` naming (overview §5.3) | ShellInput's management methods use the `session.` prefix, which this contract reserves for Rpc; suggest `shell.run`/`shell.stop` |
| Session methods that change authority | `session.permission.update` is a Session method and therefore reachable from every entry that serves Session methods, including RemoteGateway; see RemoteGateway open questions |
| Loader declaration read API | Reading row declarations through the `Loader` handle needs confirming against rutis-loader 0.10; otherwise rows carry an explicit meta field |
