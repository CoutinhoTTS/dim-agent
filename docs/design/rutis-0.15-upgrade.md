# Upgrading to rutis 0.15.0

Status: design, not implemented. This document is the upgrade contract for the
`rutis`, `rutis-loader`, `rutis-bridge` and bundled Node runtime packages, from
0.12.1 to 0.15.0. It replaces no other document; where a design describes rutis
behaviour that changes here, that design is updated in the same change.
[中文](../../zh/design/rutis-0.15-upgrade.md).

## 1. Scope and why it is one step

| Item | Now | Target |
| --- | --- | --- |
| `Cargo.toml` workspace dependencies | `rutis`, `rutis-loader`, `rutis-bridge` 0.12.1 | 0.15.0 |
| `runtime/package.json` | `@arcships/rutis-runtime` 0.12.1 | 0.15.0 |
| `runtime/plugins/*/package.json` | `@arcships/rutis` 0.12.1 | 0.15.0 |
| Pinned authoring declaration | `docs/typescript-plugin-authoring.md` "main uses 0.12.1" | 0.15.0 |

There is no 0.14.0 release: 0.14's work (Swift and Kotlin SDKs, Go runtime
`restart`) shipped inside 0.15.0. The relevant intermediate release is 0.13.0,
which replaced events wholesale.

The bump cannot be split across hosts and runtimes. Every 0.13 end declares
`events-v2` in its handshake and refuses a peer that does not, before anything
loads:

```text
incompatible session: the far end (<name> <version>) does not declare events-v2
(the event protocol of rutis 0.13): upgrade it to rutis 0.13 or later
```

So the Rust host (this workspace), `@arcships/rutis-runtime` in `runtime/`, every
plugin package that bundles the SDK, and every linked node move together. A
Rust-only bump leaves every TypeScript row unable to connect.

Not in scope: `rutis-host` (dim does not run it), Go/Swift/Kotlin runtimes,
dylib plugins, node links, Cordis mounts (`cordis` feature is not enabled in
this workspace).

## 2. What changed

### 2.1 0.13: unified events

- One logical bus per host. Listeners are found by **event name**, not by Rust
  type: `EventKey::<E>::of()` is `E::NAME`, and two types with the same name
  share listeners, converted through serde. Dynamic names (`EventKey::dynamic`,
  `named`) are unchanged.
- `Event` requires `Serialize + DeserializeOwned`; so does `Event::Value`.
  A value that is not serde (a handle, a channel, a `Mutex`) cannot be in an
  event.
- One listener table; the emitter chooses the mode. Five modes exist everywhere:
  `emit`, `parallel`, `serial`, `waterfall`, `bail` (Rust also `waterfall_sync`).
  `emit` returns at once, starts listeners in registration order, and reports a
  failure to the error sink instead of returning it. `parallel` waits and reports
  `CordisError::ListenersFailed` (aggregate, registration order). `Some(false)`
  is "no result". A synchronous mode meeting an async listener fails with
  `AsyncListener`.
- **Scopes** replace instance-keyed channels: the emitter's scope is its loader
  instance chain, and a listener receives only events emitted in its own scope or
  below it (§4).
- A listener is called with the `Ctx` that registered it, not the emitter's.
- Unloading takes a row's listeners off the bus, cancels the running ones
  (`ctx.cancelled()`), and waits for them for at most `RUTIS_SHUTDOWN_TIMEOUT`.
- `rutis/*` names are reserved framework events (`rutis/service`, `rutis/row`,
  `rutis/runtime`): plugins may listen, never emit.
- Plugin API 3 (adds `parallel`, `serial`, `waterfall`, `bail`, `filter`).
  Runtimes still load API 2 plugins.

### 2.2 0.13: Rust kernel API that dim calls today

| 0.12.1 | 0.15.0 |
| --- | --- |
| `Listener::call(&self, ctx, event)` | `fn call(&self, ctx, event, call: Call<'_, E::Value>)` |
| `PatternListener::call(&self, ctx, key, event)` | one more last parameter `Call<'_, E::Value>` |
| `SyncEvent` (marker trait) | removed; `on_sync` accepts any `Event` |
| `on_waterfall`, `on_waterfall_sync` | `on` / `on_sync`; the listener calls `call.next()` |
| `Next`, `SyncNext` | `Call::next()`, `SyncCall::next()` |
| `waterfall(..) -> Result<V>`, terminal `-> Result<V>` | `-> Result<Option<V>>`, terminal `-> Result<Option<V>>` |
| `serial(ctx, key, &E)` | `serial(ctx, key, Arc<E>)` |
| `on_pattern(ctx, pattern, ..)`, `EventPattern` by value | `on_pattern(ctx, &pattern, ..)` |
| `EventKey::of().instance(id)`, `EventKey::instance`, `*_instance`, `*_keyed` | removed; scopes (`Ctx::scope`, `EventOptions`) |
| `EventKey::name() -> Option<&str>` | `-> &str` |
| `EventPattern::prefixes()`, `any_prefix` | `as_str()`, one prefix per registration |
| `WaterfallListener`, `SyncWaterfallListener`, `PatternWaterfallListener`, `SyncPatternWaterfallListener` | implement `Listener` / `SyncListener` / `PatternListener` / `SyncPatternListener` and call `next()` |
| `ListenerKind::{AsyncWaterfall, SyncWaterfall}` | removed |
| `EventSubscription { key, prefixes, selected, invoked }` | `{ name, prefix, remote, global, calls, .. }` |
| `DispatchMode` | `rutis::Mode` |
| `EventBacklog.key`, `DispatchAttempt.key` | `name` |
| `CordisError::ReentrantEvent` | removed (`SyncWaitCycle` for a real wait cycle) |

`TypeKey` keeps `instance`, `with_instance`, `keyed_dynamic`; only event keys lose
instance identity.

### 2.3 0.13: `rutis-bridge` / `rutis-loader` that dim calls today

| 0.12.1 | 0.15.0 |
| --- | --- |
| `LocalRuntime::emits([..])`, `RuntimePlugin::emits` | removed: runtime listeners register themselves |
| `Process::emit(name, args)` (waited for all listeners) | removed: dispatch on `ctx.events()`; `parallel` is the waiting equivalent, `EventKey::<rutis_bridge::events::Args>::dynamic(name)` |
| `Mount.emits`, `Mount.events` (`EventSink`) | removed. `Mount` gains `ctx: Option<rutis::Ctx>` (bus and scope; without it the runtime's events stay in its process) and `runtime: Option<String>` |
| `Process::load_row(key, entry, config, isolate, inject)` | one more last parameter `scope: &[rutis::Scope]` (`&[]` at the root, `&ctx.scope()` in an instance) |
| node `EventsPlugin`, `NodeEvent`, `Features { outbound, inbound }` | `Features { export, import }`; not used by dim |

dim never builds `Mount` itself: `NodeRuntime` forwards `rutis::Plugin::apply`
with the session ctx (`crates/dim-app-server/src/lib.rs:1044`), so 0.15 sets
`Mount::ctx` to the session ctx automatically. Rows loaded later need the
explicit `scope` argument.

### 2.4 0.15: what is new or fixed for a Rust embedder

Relevant:

- Local runtimes have 30 s to greet instead of 10 (slow start no longer fails).
- `session::Dispatch` and `peer::Handler` gained defaulted methods.
- Error `code` of one-field `build.rs` export errors is now the type name.
- `Tap::answered`, `events.expected.json` keys: only relevant to runtime and
  testing-tool authors; dim uses neither.
- `#[rutis::service]`, `rutis::CallError`, `TypeKey::type_name`,
  `Loader::waiting_for`, `Provider::Marked`, `rutis-host types`: an alternative
  to the `build.rs` `Exports` list. dim's `Exports` list keeps working, so this
  is an option, not a requirement (§5 D7).

Not relevant: Swift/Kotlin app hosting, `runtimes.swift`, Go `restart`, dylib
marking, `rutis-host check`.

### 2.5 Behaviour changes that touch dim's correctness

- **Scope filtering** is the one change dim must design around (§4).
- `emit` still does not wait (0.12 behaved the same), so dim's `emit` sites keep
  their meaning. The returned error is only a refusal; a listener's failure is
  reported where it runs (`rutis: listener of <name> in row <row> failed: …`),
  never to the emitter, so dim's `tracing::warn!(%error, "… was not announced")`
  lines keep covering refusals only.
- `parallel` failures arrive as `CordisError::ListenersFailed { name, errors }`,
  which `AgentError::aggregate` (`crates/dim-agent-runtime/src/error.rs:37`)
  does not flatten (it flattens `Aggregate`). dim has two `parallel` sites; the
  flattening case is added so diagnostics stay one line per cause.
- Unloading now waits for running listener calls. dim handlers await storage and
  services, so a slow handler can extend branch close by up to
  `RUTIS_SHUTDOWN_TIMEOUT` (10 s) before it is cancelled. The dispatcher's
  listener (`SubscriptionListener`) should observe the registering ctx's
  cancellation so close keeps its current latency (§5 D6).

## 3. dim's current usage

Counts from this workspace (`crates`, `tests`, `xtask`): 10 `Event`
implementations, 23 `Listener` / `PatternListener` implementations across 16
files, 46 `EventKey` uses, 14 `emit` and 2 `parallel` call sites, 1
`waterfall_sync`, 1 `on_waterfall_sync`, 2 `on_pattern`, 2 `Process::emit` sites
and 1 `.emits(..)` builder call.

### 3.1 Event types

| Event | Name | Payload | serde today |
| --- | --- | --- | --- |
| `SessionBroadcastEvent` (`crates/dim-agent-capabilities/src/session_broadcast.rs:13`) | `session.broadcast`, plus dynamic `session/{id}` | `Arc<AgentEvent>`, `Arc<SessionNotice>`, `Fence`, `Closed` | no derive on the enum; `SessionNotice` is `Serialize` only |
| `Committed` (`crates/dim-agent-runtime/src/core/dispatcher.rs:17`) | `timeline.committed` | `Arc<AgentEvent>` | no derive |
| `BeforeCommit` (`crates/dim-agent-runtime/src/core/timeline.rs:25`) | `timeline.before` | `EventDraft` + `Mutex<Vec<TraceStep>>` | no derive; `steps` is not serde |
| `SettingsChanged` (`crates/dim-agent-capabilities/src/settings.rs:19`) | dynamic `settings/{source}` | `source`, scope, revision | yes |
| `CatalogChanged` (`crates/dim-agent-capabilities/src/skills/catalog.rs:29`) | `skills.catalog_changed` | `u64` version | no derive |
| `ToolsChanged`, `ToolProviderChanged` (`tool_registry.rs:28`, `:35`) | `tools.changed`, `tools.provider_changed` | unit | no derive |
| `ProjectAuthorized` (`crates/dim-app-server/src/settings.rs:74`) | `settings.project_authorized` | `root`, `cwd` | yes |
| `SessionRestored` (`crates/dim-app-server/src/agents.rs:80`) | `agent/session-restored` | `agent_id`, **`Session` handle** | no derive; handle is not serde |
| `UnloadedSessionDeleted` (`agents.rs:93`) | `agent/unloaded-session-deleted` | two ids | no derive |

`SessionRestored` cannot satisfy the 0.13 bounds: `Session` is a runtime handle.
`BeforeCommit` carries the local trace steps, which must stay local.

### 3.2 Where dim removes instance keys

| Site | Current | After |
| --- | --- | --- |
| `timeline.rs:870` `before_key` | `EventKey::of().instance(self.0.instance)` | `EventKey::of()` |
| `timeline.rs:205` dispatch | same key | `waterfall_sync(..)`, terminal returns `Ok(Some(draft))` |
| `dispatcher.rs:113`, `:216` | `.instance(self.instance)` | `EventKey::of()` |
| `tool_registry.rs:351`, `:895` | `.instance(scope.instance)` | `EventKey::of()` |
| `tool_binding.rs:375` | `.instance(self.scope.session.instance)` | `EventKey::of()` (replaced, §5 D3) |
| `agents.rs:1466-1467` | `.instance(self.root.instance())` | `EventKey::of()` |
| `mcp/session.rs:156` | `.instance(self.scope.instance)` | `EventKey::of()` |

All of these keys are equal to, or below, the listeners' scopes today, except the
flows listed in §4.

### 3.3 Bridge and runtime sites

- `session_typescript.rs:511-517`: `LocalRuntime::node(..).named(..).emits([..])`
  — `.emits(..)` is removed.
- `session_typescript.rs:166` and `:251`: `process.emit(name, vec![value])` —
  removed; both wait for all listeners today (ordered, per-session queue).
- `session_typescript.rs:77`: `key.name().and_then(..)` — `name()` returns `&str`.
- `session_typescript.rs:112`, `:242`: `on_pattern(ctx, pattern, ..)` passes the
  pattern by value.
- `session_typescript.rs:212-222`: the Settings/project relay and
  `forward_session_events` register at the session scope.

### 3.4 TypeScript packages and tests

- `runtime/plugins/session-title/session-title.test.mjs:48` passes
  `{ emits: ['session.event'] }` — removed in 0.13, with an explicit message.
- `loaded.emit(..)` is awaited in `session-title`, `goal` and `web` tests to make
  listeners run; from 0.13 `emit` returns at once. Waiting becomes
  `loaded.parallel(..)`; listener failures are read from `t.reported`
  (`loaded.reported()` in the shared-bus form).
- `docs/typescript-plugin-authoring.md` pins 0.12 and describes leaf events as
  0.11-era.

## 4. The semantic change: events travel upward only

0.13's single rule: **a listener receives events emitted in its own scope and in
every scope below it.** Not above, and never a sibling's.

```text
process root                 scope []                     hears: everything
└── agent instance           [agent]
    └── session instance     [agent, session]
        ├── session rows     [agent, session]             hears: session and below
        └── branch instance  [agent, session, branch]
            └── branch rows  [agent, session, branch]    hears: branch only
```

dim used instance-keyed events as a two-way channel inside one instance subtree
(0.12 delivery did not care about direction). Under 0.13 the same key gives
unidirectional delivery, so the flows have to be re-expressed.

| Flow | Emitter (scope) | Listeners (scope) | 0.13 |
| --- | --- | --- | --- |
| `session/{id}` broadcast | branch | app server connections (root), session TS relay (session), tests (root) | delivered (upward) |
| `timeline.committed` | branch | branch subscribers | delivered (equal) |
| `timeline.before` | branch | branch middleware | delivered (equal) |
| `tools.provider_changed` | session | ToolRegistry, MCP session (session) | delivered (equal) |
| `agent/session-restored`, `agent/unloaded-session-deleted` | root | Collaboration (root) | delivered (equal) |
| `rutis/service`, `rutis/row` | the provider's row (session, branch) | ToolRegistry, `ProjectedServices` (session) | delivered for session and branch rows; a root-scope provider is not heard |
| `tools.changed` | session | `ToolBinding` (branch) | **not delivered** (downward) |
| `skills.catalog_changed` | root | Skills session (session) | **not delivered** (downward) |
| `settings.changed` | root | Skills catalog (root), session TS relay (session), tests (root) | root delivered; **session relay not** |
| `settings.project_authorized` | root | MCP config (root), session TS relay (session) | root delivered; **session relay not** |

Consequences if the bump lands without this section: TypeScript parts stop
receiving host events at all (`Process::emit` is gone, D4) and, once re-plumbed,
still miss `settings.changed` / `settings.project_authorized`; branch
`ToolBinding` stops refreshing after tool-source changes; the Skills session row
stops waking on catalog changes. All are silent (no error, no timeout), which is
why this is a design decision rather than a compile fix.

## 5. Decisions

### D1 — One upgrade unit for host, runtime and plugins

Bump the three crates, `runtime/package.json`, the three plugin packages and
their lock files together. Keep `runtime/plugin-sdk.test.mjs` green: it packs
`npm/`, copies `runtime/node_modules/@arcships/rutis` and type-checks the
authoring guide, so the authoring types and the SDK version must move together.

### D2 — Scopes replace event instance keys

Event keys lose instance identity; isolation comes from the instance chain the
loader marks (`Ctx::mark_scope` for every created group instance). No dim code
calls `mark_scope`; `Loader::create_instance` already produces the instances the
assembly uses (`crates/dim-agent-runtime/src/assembly.rs:543`, `:549`). Remove
`EventKey::of().instance(..)` everywhere (§3.2) and delete the `instance` fields
that only fed keys (`Timeline`, `Dispatcher`) once nothing else reads them.

### D3 — Downward notification is not an event

Three shapes, applied per flow:

1. **A dependent reads its owner's state.** A branch consumer of a session-owned
   service uses that service's state instead of an event:
   `ToolBinding` (branch) reads `ToolRegistry`'s existing source-state watch; the
   Skills session row reads a version watch on `SkillsCatalog`. Both already hold
   the service handle as a dependency, so no new service, transport or event is
   introduced. `tools.changed` and `skills.catalog_changed` stop being dispatched
   for these consumers; keep the events only if another (upward) consumer needs
   them — none today, so both are removed and their names disappear from
   diagnostics.
2. **A process-global fact consumed inside a session.** Settings and project
   authorization are process-wide facts (their events are process-wide by
   construction). The one session-scoped consumer, the TypeScript relay
   (`forward_settings_events`), registers its listeners on `ctx.root_ctx()` — the
   root of its own tree, whose listeners hear every event — and owns their
   disposal:

   ```rust
   let root = ctx.root_ctx();
   let disposers = vec![
       root.events().on_pattern(
           &root,
           &rutis::EventPattern::<SettingsChanged>::prefix("settings/"),
           Enqueue(send.clone()),
       )?,
       root.events().on(&root, &project_authorizations(), Enqueue(send))?,
   ];
   ctx.effect(move || {
       Effect::AsyncDisposer(Box::new(move || {
           Box::pin(async move {
               for disposer in disposers {
                   if let Err(error) = disposer.dispose().await {
                       tracing::warn!(%error, "process-scope listener disposal failed");
                   }
               }
               Ok(())
           })
       }))
   });
   ```

   Disposal must be explicit: dropping a rutis `Disposer` does not unregister
   anything in 0.15 (`Disposer::dispose` is the only path).

   The listener is owned by the root fiber, hence the explicit effect: the
   registering context is only the root, and the relay must still stop at
   session close. The relay keeps forwarding into the session's own scope (D4).
3. **Everything else stays an event.** No new relay plugin, no scope override.

Rejected alternatives: dispatching with an explicit `Dispatch.scope` (public, but
it bypasses the one scope rule and makes delivery unreviewable); making the
app-server push settings into each session through a new service method (a second
path for the same fact); registering every listener at the root (loses
per-subtree isolation that dim relies on for `timeline.*` and `tools.*`).

### D4 — Host-to-row events are dispatched in the row's scope

`Process::emit` is gone. The relay dispatches on the logical bus with its own
ctx — the session ctx, because `NodeRuntime` applies the runtime with the session
context (`lib.rs:1044`), so 0.15 sets `Mount::ctx` to it:

```rust
let key = EventKey::<Args>::dynamic(name);
ctx.events()
    .parallel(&ctx, &key, Arc::new(Args(args)))
    .await?;
```

`parallel` keeps today's ordering guarantee (the relay's per-session queue awaits
each delivery) and its failure report. Consequence to record in
[session-typescript.md](session-typescript.md): a host event reaches rows loaded
in the session scope and below. A branch row that needs one is dispatched from
the branch ctx (per branch fan-out); no branch row listens to host events today.

### D5 — Every event payload is serde, and local state stays out of it

- Derive `Serialize`/`Deserialize` on `SessionBroadcastEvent`, `Committed`,
  `CatalogChanged`, `ToolsChanged`, `ToolProviderChanged`,
  `UnloadedSessionDeleted`; add `Deserialize` to `SessionNotice` (`Arc` needs
  serde's `rc`, already enabled in the workspace).
- `BeforeCommit` keeps its trace steps local: `#[serde(skip)] steps:
  Mutex<Vec<TraceStep>>` (default on deserialize). A trace never crosses
  processes by design; the event is only dispatched by the branch's own
  `waterfall_sync`, in process.
- `SessionRestored` carries `agent_id` and `session_id`; the listener resolves
  the `Session` through `AgentIndex`, where the handle already lives
  (`crates/dim-app-server/src/collaboration.rs:522`). A handle in an event is a
  contract violation on 0.13.

### D6 — Listener migration is mechanical, with one cancellation change

Add the `Call` / `SyncCall` parameter, replace `on_waterfall_sync` with `on_sync`
and `SyncNext` with `SyncCall` (`timeline.rs:846-870`), return `Option` from the
timeline middleware and terminal, pass `Arc<E>` to `serial`
(`dispatcher.rs:216`), pass `&EventPattern` (`session_typescript.rs:112`, `:242`),
and adapt `EventKey::name()`.

In `SubscriptionListener::call`, select the handler against the registering ctx's
cancellation, so branch close keeps its latency once unload waits for listeners:

```rust
tokio::select! {
    _ = ctx.cancelled() => Ok(None),
    result = handler(event) => { .. }
}
```

### D7 — Keep the `build.rs` export list

`Exports::new().service(..).service_in(..).declared(..).typescript(..)` keeps
working in 0.15, and `include_exports!` is unchanged. `#[rutis::service]` would
remove the `build.rs` list but adds a marker and a second way to declare the same
contract; adopt it only in a separate change, after the upgrade is green.

### D8 — No event renames

Names are now global, so they are the contract. dim's names are already unique
and outside `rutis/*` and `internal/*`; keep them and document that new names
must not collide with framework names. `tools.changed` and
`skills.catalog_changed` disappear with D3.1 rather than being renamed.

## 6. Implementation plan

Each phase is a complete, reviewable change with its own verification; P1 is one
commit because 0.13 has no compatibility path and a partial topology change is
silently wrong.

| Phase | Work | Gate |
| --- | --- | --- |
| P0 | Bump `Cargo.toml` (3 crates), `cargo update -p rutis -p rutis-loader -p rutis-bridge`; bump `runtime/package.json`, the three plugin `package.json`s and lock files (`npm install` in each); update the authoring pin | `cargo metadata` resolves; locks name 0.15.0 |
| P1 | Kernel event migration: serde derives and payload changes (D5), listener signatures (D6), `on_sync`/`SyncCall`/`Option` (timeline), `serial(Arc)`, `on_pattern(&)`, drop instance keys (D2), topology (D3.1 watches, D3.2 root-scope relay) | `cargo test --workspace --locked` for the affected flows: timeline batch, tool binding, skills, settings, session broadcast |
| P2 | Bridge/runtime migration: remove `.emits(..)`; replace both `Process::emit` sites with scoped `parallel` (D4); confirm rows join the bus and branch rows receive per-scope events in a test that loads a real Node row | `cargo test --workspace --locked`; the Node-backed profile tests |
| P3 | TypeScript: drop `emits: [..]`, replace awaited `emit` with `parallel`, assert failures through `t.reported`; update `docs/typescript-plugin-authoring.md` (version, event semantics) | plugin tests in `runtime/plugins/*`; `runtime/plugin-sdk.test.mjs` |
| P4 | Tooling and docs: `cargo run --locked -p xtask -- graph` and `check`; update 0.12.1 references (§8); note the scope rule in the architecture docs | `xtask check`; `node --test scripts/*.test.mjs` |

## 7. Verification

Full sequence at the workspace root (CI order):

1. `./dev check`
2. `cargo clippy --workspace --all-targets --locked -- -D warnings`
3. `cargo test --workspace --locked`
4. `cargo run --locked -p xtask -- check` and `graph` (assembly and crate
   dependencies are unchanged, so the snapshots should not move; a diff means the
   bump changed something it should not)
5. `node --test scripts/*.test.mjs`
6. Node bindings, as CI runs them: `npm ci --prefix runtime` and the three plugin
   packages, `npm install --no-save --install-links` for the linked plugins,
   `node --test runtime/plugin-sdk.test.mjs`, the four plugin tests
   (`runtime/plugins/{goal,web,session-title,workflow}/*.test.mjs`) and the three
   `tsc -p runtime/plugins/*/tsconfig.json` type checks;
   `git diff --exit-code -- npm/branch-services.d.ts` confirms the generated
   declarations (and therefore the `Exports` generator) did not move.
7. Node-backed Rust flows: the `node_` tests of `dim-app-server` with
   `DIM_TEST_NODE`, `DIM_TEST_JS_RUNTIME` and `DIM_TEST_RUTIS_SDK` set, plus
   `cargo test -p dimcode --test cli_flow -- --ignored`.

Flow checks that the suite must cover for this change (existing tests, extended
only where the contract moved):

- settings change → TypeScript row receives `settings.changed` (relay at root
  scope, dispatch in session scope)
- project authorization → TypeScript row receives `settings.project_authorized`
- tool source change → branch `ToolBinding` refreshes (watch, not event)
- catalog version change → Skills session row refreshes (watch, not event)
- branch close while a listener runs: close latency bounded, no orphan task
- session broadcast → connection notifications unchanged (upward delivery)

The managed provider plugin (private, packaged against rutis 0.10 API 2, checked
at `crates/dim-app-server/src/lib.rs:542`) runs inside the upgraded Node runtime.
API 2 plugins are still loadable by a 0.13+ runtime, but its `emit` timing and
its bundled SDK must be re-verified against the 0.15 host before release; the
`rutisVersion` guard stays as it is unless the plugin is rebuilt.

## 8. Documentation to update

- `docs/typescript-plugin-authoring.md`: pinned version, leaf event semantics
  (`emit` does not wait, use `parallel`; `emits` declaration removed).
- `docs/design/session-typescript.md`, `branch-typescript.md`,
  `process-typescript.md`: 0.12.1 references and the host-to-row delivery scope.
- `docs/design/appserver/management-methods.md`, `sessions-service.md`,
  `p6-p7-bindings.md`: 0.12.1 references.
- `docs/design/appserver/settings-service.md`: delivery path (root-scope relay,
  session-scope dispatch).
- `docs/design/plugins/Tui.md`, `docs/design/roadmap.md`: version references.
- `docs/architecture/rust-agent/lifecycle.md`, `principles.md`,
  `single-tree.md`: instance keys → scopes for events.
- `docs/INDEX.md`: this document.
- `docs/zh/` counterparts where they exist.

## 9. Risks

| Risk | Handling |
| --- | --- |
| Silent loss of a downward flow not in §4 | P1 gate runs the five affected flow tests; grep for every `emit` site and classify its listeners before merging |
| Runtime and host versions drift (refused handshake) | P0 bumps every pin in one commit; the staged release archive records `toolchain.rutis` from `runtime/package.json` (`scripts/stage-runtime.mjs:165`), so a forgotten bump is visible in the release manifest |
| Unload waits for listener calls | D6 cancellation in the dispatcher listener; branch close tests assert latency |
| `SessionRestored` payload change | Internal to the app server; the restore flow test covers it |
| Root-scope registrations leak if a session panics | Disposer owned by `ctx.effect`; a watcher assertion in the relay test |
| `ListenersFailed` no longer flattened into `AgentError::Multiple` | Extend `AgentError::aggregate` to traverse it; no user-visible format change |
| Private plugin (0.10 API 2) behaves differently under a 0.15 runtime | Re-verify before release; rebuild with a 0.15 SDK if needed |

Rollback: revert the upgrade change and the four pins; no persisted format moves
(Timeline, storage and wire fixtures are untouched by this upgrade).

## 10. Open questions for maintainers

1. D3.2: is `ctx.root_ctx()` registration accepted as the way a session-scoped
   part hears process-global events, or should the app server push settings into
   sessions through a service method instead?
2. D3.1: removing `tools.changed` and `skills.catalog_changed` removes two names
   from diagnostics and any future cross-language use; confirm that the watchers
   are the intended contract.
3. Keep the `rutisVersion == "0.10.0"` plugin package guard, or make it a minimum
   version once the private plugin is rebuilt?
4. Adopt `#[rutis::service]` (D7) as a follow-up, or keep the generated
   `Exports` list as the only declaration?

## 11. Design-rule check

The upgrade adds no capability and no plugin; the checks below state what each
rule means for a change that moves the event mechanism.

| Rule | Answer |
| --- | --- |
| §4.1 Capability and `Deps` | No new Capability. Because `ToolBinding` and the Skills session row already declare `ToolRegistry` / `SkillsCatalog` in `Deps`, D3.1 needs no new dependency declaration. |
| §4.2 Events for critical effects | Unchanged for what stays on the bus. Two events are removed (D3.1) because their only consumers were downward; no new event is introduced, and no coordination moves into a direct call on a handle a Capability does not own. |
| §4.4 State ownership | D3.1 keeps the state with its owner (the registry's source state, the catalog's version) and has the dependent read it; no state is copied, cached or rebuilt from an event. |
| §4.6 Frozen fixtures | `crates/dim-agent-types/tests/fixtures/` is not touched: event kinds, kinds' keys and stored formats do not move. |
| §4.7 Reuse existing mechanisms | `Dispatcher`, `StateCell`-style state, and the existing `watch` channels; no new port or host trait. |
| ADR 002 D1–D5 | D1/D2: no wiring module, no direct cross-plugin calls; the root-scope registration in D3.2 is a listener registration on an existing bus. D5: only named services carry state; no new binding. |
| Requirements raised to mechanism owners | None. `ctx.root_ctx()` and scoped dispatch already exist in 0.15; if maintainers reject D3.2 (§10 Q1), the requirement becomes a settings subscription API and is raised separately. |

## 12. Ablation review

- Scopes remove code rather than add it: `before_key`, `Committed`'s instance
  key, `Dispatcher::instance`, and the instance half of `Timeline::branch` exist
  only to isolate events; after D2 they are deleted, not replaced.
- D3.1 adds no abstraction: both consumers use a watch their owner already has
  (ToolRegistry) or one derived from a counter it already keeps (SkillsCatalog).
- D3.2 adds no service, plugin or event; it registers the existing relay at the
  root scope with the disposal the plugin already needs.
- No wrapper for "emit by name" is proposed: D4 has two call sites, both in one
  file; a helper would be invented coupling. If a third appears, extract then.
- No rename, no new event name, no compatibility shim: 0.13 has no old path, so a
  shim would only hide a broken flow.
