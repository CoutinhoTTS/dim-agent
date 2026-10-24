# Upgrading to rutis 0.16.0

Status: design, not implemented. Phase P1 of the [plugin contract](plugin-contract.md#10-migration),
under [ADR 006](../adr/006-plugin-contract.md). It replaces the
[rutis 0.15 upgrade design](rutis-0.15-upgrade.md), whose scoped-delivery model rutis 0.15.1 withdrew.
[中文](../zh/design/rutis-0.16-upgrade.md)

The upgrade moves `rutis`, `rutis-loader`, `rutis-bridge` and the bundled Node packages from 0.12.1
to 0.16.0. It does not change behaviour that plugins or clients can observe. The plugin contract's
later phases make the contract changes.

## 1. Scope, and why it is one step

| Item | Now | Target |
| --- | --- | --- |
| `Cargo.toml` workspace dependencies | `rutis`, `rutis-loader` (`node`), `rutis-bridge` 0.12.1 | 0.16.0; `rutis-macros` arrives through `rutis` |
| `crates/dim-app-server` build dependency | `rutis-bridge` 0.12.1 with `exports` | 0.16.0 with `exports` |
| `runtime/package.json` | `@arcships/rutis-runtime`, `@arcships/rutis` 0.12.1 | 0.16.0 |
| `runtime/plugins/{web,workflow,session-title}/package.json` | `@arcships/rutis` 0.12.1 | 0.16.0 |
| Authoring pin | [authoring guide](../typescript-plugin-authoring.md): "main uses 0.12.1" | 0.16.0 |

Since 0.13, every end declares `events-v2` in its handshake and refuses a peer that does not, before
anything loads. The Rust host, `@arcships/rutis-runtime`, every plugin package that bundles the SDK
and their lock files therefore move together. Upgrading only the Rust side leaves every TypeScript
row unable to connect.

Not in scope: `rutis-host`, Go, Swift and Kotlin runtimes, dylib plugins, node links and Cordis
mounts. Dim does not use them.

## 2. What changed between 0.12.1 and 0.16.0

| Version | What matters to Dim |
| --- | --- |
| 0.13 | Unified events: listeners are found by name, payloads are serde, one listener table, five modes; instance-keyed event APIs removed; `Process::emit`, `.emits` and `Mount.emits` removed; `events-v2` handshake. Plugin API 3; API 2 plugins still load |
| 0.15 (no 0.14) | `#[rutis::service]`, `rutis-macros`, `Loader::waiting_for`; the local runtime greeting timeout is 30 s; a one-field error's `code` is its type name |
| 0.15.1 | **One flat bus:** every listener of a name receives the event, wherever it is; scope only describes a listener for filters |
| 0.16 | The 0.15.1 scope shims are removed; `Ctx::is_shutting_down`; fixes for `#[cfg]` methods and alias generics in exports |

Plugin traits (`Plugin`, `TypedPlugin`, `Deps`), effects, `TypeKey` with instances, instanced
loader groups, `Build`, `ServiceCatalog` and `Exports` keep their 0.12.1 contracts.

### 2.1 Rust kernel APIs Dim calls

| 0.12.1 | 0.16.0 |
| --- | --- |
| `Listener::call(&self, ctx, event)` | `call(&self, ctx, event, call: Call<'_, E::Value>)`; the listener gets the ctx that registered it |
| `PatternListener::call(&self, ctx, key, event)` | One more last parameter, `Call<'_, E::Value>` |
| `SyncEvent` marker | Removed; `on_sync` accepts any `Event` |
| `on_waterfall`, `on_waterfall_sync`; `Next`, `SyncNext` | `on`, `on_sync`; the listener calls `call.next()` |
| `waterfall(..) -> Result<V>`, terminal `-> Result<V>` | `-> Result<Option<V>>` for both |
| `serial(ctx, key, &E)` | `serial(ctx, key, Arc<E>)` |
| `on_pattern(ctx, pattern, ..)` | `on_pattern(ctx, &pattern, ..)` |
| `EventKey::of().instance(id)` and every `*_instance`/`*_keyed` method | Removed. Isolation is by name (§4) |
| `EventKey::name() -> Option<&str>` | `-> &str` |
| `DispatchMode` | `rutis::Mode` |
| `EventBacklog.key`, `DispatchAttempt.key` | `name` |
| `CordisError::ReentrantEvent` | Removed (`SyncWaitCycle` reports a real wait cycle) |
| `parallel` failures | Always `CordisError::ListenersFailed { name, errors }`, even for one failure |

`TypeKey::instance`, `with_instance` and `keyed_dynamic` are unchanged; only event keys lose
instance identity.

### 2.2 Bridge and loader APIs Dim calls

| 0.12.1 | 0.16.0 |
| --- | --- |
| `LocalRuntime::emits([..])`, `RuntimePlugin::emits` | Removed: runtime listeners register themselves on the host bus |
| `Process::emit(name, args)`, which waited for all listeners | Removed: dispatch on `ctx.events()` with `EventKey::<rutis_bridge::events::Args>::dynamic(name)`; `parallel` is the waiting form |
| `Mount.emits`, `Mount.events` | Removed. `Mount` gains `ctx` and `runtime`. Dim never builds a `Mount` itself |
| `Process::load_row(key, entry, config, isolate, inject)` | One more last parameter, `scope: &[rutis::Scope]`: `&[]` at the root, `&ctx.scope()` in an instance. It describes listeners only |

### 2.3 Behaviour changes that touch Dim's correctness

- **The flat bus** is the change Dim must design around (§4).
- `emit` still returns at once. A listener's failure goes to the error sink, never to the emitter.
- Unloading takes a row's listeners off the bus, cancels running calls through `ctx.cancelled()` and
  waits for them up to `RUTIS_SHUTDOWN_TIMEOUT` (10 s).
- Events emitted from Rust reach TypeScript, Python and Go listeners with no declaration, so the serde
  form of a public notice is visible to every runtime.

## 3. Dim's current usage

Counted in `crates`, `tests` and `xtask` at dd87f6af: 10 `Event` implementations, 23 listener
implementations, 46 `EventKey` uses, 16 `emit` and 2 `parallel` sites, 2 `waterfall_sync` sites,
2 `on_pattern` sites, 2 `Process::emit` sites and 1 `.emits(..)` call.

| Event | Name | Payload | serde today |
| --- | --- | --- | --- |
| `SessionBroadcastEvent` (`dim-agent-capabilities/src/session_broadcast.rs`) | `session.broadcast`, dispatched under `session/{id}` | `Arc<AgentEvent>`, `Arc<SessionNotice>`, `Fence`, `Closed` | No derive; `SessionNotice` is `Serialize` only |
| `Committed` (`dim-agent-runtime/src/core/dispatcher.rs:15`) | `timeline.committed` | `Arc<AgentEvent>` | No derive |
| `BeforeCommit` (`dim-agent-runtime/src/core/timeline.rs:27`) | `timeline.before` | `EventDraft` and local trace steps | No; the steps are not serde |
| `SettingsChanged` (`dim-agent-capabilities/src/settings.rs:24`) | dispatched under `settings/{source}` | `source`, scope, revision | Yes |
| `CatalogChanged` (`dim-agent-capabilities/src/skills/catalog.rs:32`) | `skills.catalog_changed` | `u64` version | No derive |
| `ToolsChanged`, `ToolProviderChanged` (`tool_registry.rs:29`, `:36`) | `tools.changed`, `tools.provider_changed` | unit | No derive |
| `ProjectAuthorized` (`dim-app-server/src/settings.rs:78`) | `settings.project_authorized` | `root`, `cwd` | Yes |
| `SessionRestored` (`dim-app-server/src/agents.rs:84`) | `agent/session-restored` | `agent_id` and a **`Session` handle** | No; a handle is not serde |
| `UnloadedSessionDeleted` (`agents.rs:96`) | `agent/unloaded-session-deleted` | two ids | No derive |

## 4. The semantic change: one flat bus

0.12.1 isolated an instance-keyed event to its instance's subtree. On 0.16 every listener of a name
receives every event of that name. A flow that relied on the instance key would now deliver across
branches or Sessions.

| Flow | Emitter | Listeners | Isolation on 0.12.1 | On the flat bus | Action |
| --- | --- | --- | --- | --- | --- |
| `timeline.committed` | Dispatcher (branch) | the branch's subscribers | branch instance key (`dispatcher.rs:113`, `:216`) | **Every branch's subscribers would receive every branch's facts** | U2 |
| `timeline.before` | Timeline (branch) | the branch's transforms | branch instance key (`timeline.rs:870`) | **Every branch's transforms would run on every commit** | U2 |
| `tools.changed` | ToolRegistry (Session) | ToolBinding (branch) | Session instance key (`tool_registry.rs:351`, `tool_binding.rs:375`) | Every Session's bindings would wake on any Session's change | U3 |
| `tools.provider_changed` | McpSession (Session) | ToolRegistry (Session) | Session instance key (`mcp/session.rs:156`, `tool_registry.rs:895`) | Every Session's registry would refresh on any Session's change | U3 |
| `agent/session-restored`, `agent/unloaded-session-deleted` | Agents (process) | Collaboration (process) | root instance key (`agents.rs:1528-1529`) | Unchanged: there is one root | U4 |
| `session/{id}` | SessionBroadcast (branch) | connections, the TypeScript relay, tests | the dynamic name | Unchanged | — |
| `settings/{source}`, `settings.project_authorized` | Settings (process) | SkillsCatalog, MCP, the relay, tests | process-wide | Unchanged | — |
| `skills.catalog_changed` | SkillsCatalog (process) | Skills (Session rows) | process-wide | Unchanged: it is meant for every Session | — |
| `rutis::FiberStatusChanged`, `rutis::ServiceChanged` | rutis | ToolRegistry, ProjectedServices, MCP availability | process-wide | Unchanged: listeners already filter by key or re-read | — |
| `session.event` and Settings to TypeScript rows | the App Server relay, `Process::emit` | rows, each filtering by its bound Session id | runtime-wide | `Process::emit` is gone | U5 |

The first two rows are correctness requirements. The two tool rows would still be correct, because
every listener re-reads its own registry, but they wake every Session for one Session's change.

## 5. Decisions

### U1. One upgrade unit for host, runtime and plugins

Bump the three crates, `runtime/package.json`, the three plugin packages and their lock files
together. `runtime/plugin-sdk.test.mjs` packs `npm/`, copies `runtime/node_modules/@arcships/rutis`
and type-checks the authoring guide, so the authoring types and the SDK version move together.

### U2. Branch mechanism channels get private names

The Dispatcher and the Timeline build their bus names from their branch instance:
`dim/timeline/{instance}/committed` and `dim/timeline/{instance}/before`. The constructors are private
to `dim-agent-runtime`. Parts keep using `Dispatcher::subscribe`/`listen` and `Timeline::before`, and
never these names ([ADR 006 D5](../adr/006-plugin-contract.md#d5-timeline-facts-and-bus-notices-are-different-things)).
The `instance` fields of `Dispatcher` and `Timeline` stay, now to build the names. `InstanceId` is
runtime routing only; it is never persisted or shown.

### U3. Per-Session tool notices

- `tools.changed` is removed. ToolBinding reads ToolRegistry's existing source-state watch
  (`sources_state()`, `tool_registry.rs:236`) on the handle it already declares, with the contract of
  [0.15 design D3.1](rutis-0.15-upgrade.md#d31-toolregistry-source-state-exists): first read at open,
  one refresh per replacement, rejections visible, no wake when nothing changed, and the task ends at
  close.
- `tools.provider_changed` gets a per-Session private name, `dim/tools/{instance}/provider-changed`.
  `tool_registry` exports its constructor, and McpSession uses it. A watch on each provider is not
  used, because ToolRegistry does not depend on optional providers ([ADR 002 D3](../adr/002-plugin-scope.md#d3-dependencies-follow-scope-and-point-from-extensions-to-the-core)).

### U4. Root notices drop the instance

`agent/session-restored` and `agent/unloaded-session-deleted` use `EventKey::of()`; one process has one
root.

### U5. The relays dispatch on the bus

Both `Process::emit` sites (`session_typescript.rs:166`, `:251`) become a `parallel` dispatch of
`rutis_bridge::events::Args` under the same names and arguments, from the relay's own process context.
`.emits(..)` (`:515`) is removed. On the flat bus that dispatch reaches every row, exactly as
`Process::emit` did, and each row still filters by its bound Session id. `parallel` keeps the
per-Session queue's ordering and the failure report. The 0.15 design's agent-context dispatch (D4) is
not needed. Contract phase 4 replaces the relays with direct subscription (R5).

### U6. Every payload is serde, and local state stays out of it

- Derive `Serialize`/`Deserialize` on `SessionBroadcastEvent`, `Committed`, `CatalogChanged`,
  `ToolProviderChanged` and `UnloadedSessionDeleted`, and add `Deserialize` to `SessionNotice`
  (`Arc` needs serde's `rc`, already enabled).
- `BeforeCommit` keeps its trace steps local with `#[serde(skip)]`. The event is dispatched only by
  its own branch's `waterfall_sync`, in process.
- `SessionRestored` carries `agent_id` and `session_id`. Collaboration resolves the `Session` through
  `AgentIndex`, where the handle already lives.
- The serde form of `session/{id}` becomes visible to every runtime (§2.3). Rows keep using the relay
  until phase 4, which documents that form as the public payload.

### U7. Listener migration is mechanical, with one cancellation change

Add the `Call`/`SyncCall` parameter; replace `on_waterfall_sync` with `on_sync` and `SyncNext` with
`SyncCall`; return `Option` from the transform chain and its terminal; pass `Arc<E>` to `serial` and
`&EventPattern` to `on_pattern`; adapt `EventKey::name()`; and let `AgentError::aggregate` flatten
`ListenersFailed`. The Dispatcher's `SubscriptionListener` selects the handler against the
registering ctx's cancellation, so branch close keeps its latency now that unload waits for running
listeners:

```rust
tokio::select! {
    _ = ctx.cancelled() => Ok(None),
    result = handler(event) => { /* as today */ }
}
```

### U8. Exports and the catalog stay as they are

The `build.rs` `Exports` list keeps working in 0.16. `#[rutis::service]` and `share_registered_only`
belong to the catalog phases of the plugin contract (P2, P4), not to this upgrade.

### U9. No public renames

Public names keep their spelling. `skills.catalog_changed` stays, because the flat bus delivers it to
every Session row. The `SkillsCatalog::changes()` watch that the 0.15 design required
([#362](https://github.com/arcships/dim/issues/362)) is not needed.

## 6. Implementation plan

0.13 has no compatibility path, so steps 1–4 land in one pull request. Each step is its own commit.

| Step | Work | Gate |
| --- | --- | --- |
| 1 | Pins and lock files (U1); the authoring pin | `cargo metadata` resolves; the lock files name 0.16.0 |
| 2 | Kernel events: U2, U3, U4, U6, U7 | The flow checks of §7 for the Timeline, tools, Skills, Settings and the broadcast |
| 3 | Bridge and runtime: U5, `.emits` removed, `load_row` scope argument | Node-backed flow tests: a real row receives `session.event` and the Settings notices |
| 4 | TypeScript packages: drop `emits: [..]` in tests, wait with `parallel` instead of `emit`, read failures from `reported` | Plugin tests in `runtime/plugins/*`; `runtime/plugin-sdk.test.mjs` |
| 5 | Tooling and documentation (§8) | `xtask check`; `node --test scripts/*.test.mjs` |

## 7. Verification

The full CI sequence at the workspace root, in CI order:

1. `./dev check`
2. `cargo clippy --workspace --all-targets --locked -- -D warnings`
3. `cargo test --workspace --locked`
4. `cargo run --locked -p xtask -- check` and `graph`. Assembly and crate dependencies do not change,
   so the snapshots should not move; a diff means the upgrade changed something it should not
5. `node --test scripts/*.test.mjs`
6. The Node bindings as CI runs them, including `runtime/plugin-sdk.test.mjs`, the plugin tests, the
   `tsc` checks and `git diff --exit-code -- npm/branch-services.d.ts`
7. The Node-backed Rust flows (`DIM_TEST_NODE`, `DIM_TEST_JS_RUNTIME`, `DIM_TEST_RUTIS_SDK`) and
   `cargo test -p dimcode --test cli_flow -- --ignored`

Flow checks the suite must cover; extend existing tests only where the contract moved:

- **Branch isolation (new, U2):** two branches of one Session and branches of two Sessions. A
  subscriber receives only its own branch's facts, and a `before` transform runs only on its own
  branch's commits.
- **Tool sources (U3):** a source change refreshes ToolBinding from `sources_state()` with the D3.1
  checks; an MCP provider change in Session A refreshes A's registry and not B's.
- **Relays (U5):** a Settings change and a project authorization reach a TypeScript row; a
  `session.event` reaches the rows of its Session in order.
- **Skills:** a catalog advance wakes the Session row after publication (existing test).
- **Restore (U6):** a restored Session is observed by Collaboration through ids.
- **Close (U7):** branch close while a listener runs keeps bounded latency and leaves no task.
- **Broadcast:** connection notifications are unchanged.

## 8. Documentation to update with the upgrade

- [Authoring guide](../typescript-plugin-authoring.md): the pin, and leaf event semantics (`emit` does
  not wait; `parallel` does; no `emits` declaration).
- [Single-tree](../architecture/rust-agent/single-tree.md) §事件 and §rutis, and
  [lifecycle](../architecture/rust-agent/lifecycle.md): instance-keyed events become names; the
  version baseline.
- [Session](session-typescript.md), [branch](branch-typescript.md) and
  [process](process-typescript.md) TypeScript designs; [management methods](appserver/management-methods.md),
  [sessions service](appserver/sessions-service.md), [P6/P7 bindings](p6-p7-bindings.md),
  [ExternalAgent](plugins/ExternalAgent.md), the [roadmap](roadmap.md) and the
  [release notes](../release/README.md): 0.12.1 references.
- The `docs/zh/` counterparts where they exist.

## 9. Risks

| Risk | Handling |
| --- | --- |
| A flow that relied on an instance key now delivers across instances, with no error | §4 lists every `.instance(..)` event key; the branch isolation check is a merge gate; grep for `EventKey` with an instance before merging |
| Host and runtime versions drift (the handshake refuses) | Step 1 bumps every pin in one commit; the staged release archive records `toolchain.rutis` from `runtime/package.json` |
| Unload waits for listener calls | U7's cancellation in the Dispatcher listener; the close check |
| The private managed-provider plugin (rutis 0.10, API 2) behaves differently under a 0.16 runtime | API 2 still loads; re-verify before release and rebuild with a 0.16 SDK if needed |
| `ListenersFailed` is no longer flattened into `AgentError::Multiple` | U7 extends `AgentError::aggregate` |

Rollback: revert the upgrade change and the pins. No persisted format moves; Timeline, storage and
wire fixtures are untouched.

## 10. Requirements raised to mechanism owners

None. Every decision uses rutis 0.16 as it is and Dim's existing services. The 0.15 design's R1
(#362) is withdrawn (U9).

## 11. Rule check

The upgrade adds no part and no event.

| Rule | Answer |
| --- | --- |
| Principles §4 rules 1–3, 5–7, 9 | Unchanged: no command, handover, `before` contract, batch, context path or fact changes |
| §4 rule 4 (core has no business meaning) | U2 and U3 are generic mechanism names; no plugin-specific handling |
| §4 rule 8 (infrastructure unchanged by default) | The change comes from the dependency upgrade and closes inside the runtime, the capabilities crate and the App Server's relay; no requirement on rutis |
| AGENTS rule 6 (frozen fixtures) | `crates/dim-agent-types/tests/fixtures/` is not touched |
| ADR 002 D1–D5 | No placement, wiring, dependency or configuration change; D5 unchanged until the contract phases |
| ADR 006 D5 | U2 and U3 follow it: per-instance flows are private names built by their owners |

## 12. Ablation

- One event goes (`tools.changed`); no event is added. U2 and U3 replace instance keys with names
  and add no state.
- U5 adds no service or scope override: two dispatch sites change, in one file. No "emit by name"
  helper is introduced for two sites.
- No compatibility shim: 0.13 has no old path, so a shim would only hide a broken flow.
