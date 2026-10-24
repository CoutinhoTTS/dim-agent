# How Dim uses rutis

Status: Accepted under [ADR 006](../adr/006-plugin-contract.md) (D2–D4). Not implemented as a whole;
each section says what holds today.
[中文](../zh/design/rutis-usage.md)

rutis owns the plugin machinery. This document states how Dim uses it: how a part is mounted, how it
names and finds services, how a service crosses languages, and what the rutis lifecycle and event bus
give. It holds for every part, in any language, and its appendices are the Rust and TypeScript
bindings.

It does not state Dim's architecture. What an instance means, Timeline facts, ownership, Capability,
publication and closing are in the [plugin contract](plugin-contract.md), on top of the
[time bus](../architecture/rust-agent/time-bus.md) and the [design rules](../architecture/rust-agent/principles.md).
A rule here decides no business question, and a rule there does not depend on a rutis API.

## 1. What Dim takes from rutis

Dim uses these directly, without wrapping them ([ADR 006 D2](../adr/006-plugin-contract.md#d2-rutis-owns-the-plugin-machinery-dim-defines-its-domain-on-top-of-it)):

- plugin lifecycle: apply, effects, unload, cancellation, deadlines;
- dependency gating, Pending and reload propagation;
- services by type and by name, instanced groups and per-instance name resolution;
- loader rows and configuration;
- language runtimes, cross-language calls, the event bus and diagnostics.

Dim adds no plugin trait, author-facing scope type, resolver, manifest, role field, second service
directory, second bus, or per-language relay or adapter.

## 2. Parts are loader rows

A part is one rutis plugin, mounted as exactly one loader row. The row's group is where the part lives.
Nothing else encodes placement: not a trait, not a factory signature, not an inference from injected
names. Which group a part belongs in is Dim's decision ([plugin contract §3](plugin-contract.md#3-placement)).

| Field | Meaning |
| --- | --- |
| `id` | Stable row identity in the configuration |
| `name` | The module: a built-in name, a path, or a runtime prefix (`py:`, `bun:` …) |
| `group` | The process, `session` or `branch` |
| `config` | Structural configuration, validated by the plugin's schema |
| `inject` | The names the part depends on. A foreign plugin's own `inject` must equal this list or be a subset of it |
| `isolate`, `disabled` | rutis isolation labels; whether the row is off |

- A row in an instanced group loads once per instance of that group.
- A factory or module top level reads only its configuration and identity. It does no I/O and starts no
  work; resources are acquired in `apply` (§4).
- The row holds structural configuration only. Business configuration lives in its owner's source
  ([ADR 002 D4](../adr/002-plugin-scope.md#d4-structural-configuration-and-business-configuration-are-separate)).

*Today:* name-bound Rust parts are added with `AgentBuilder::session_part`/`branch_part` or
`register_part` (appendix A), and P3 moves the rest. The others are registered through `AgentBuilder::session_plugin`/`timeline_plugin`
with scope types. Rows of other languages carry their group in the App Server's configuration
(appendix B), and the App Server places each in that group; the loader's own `group` field marks a
row that is a group, so the declared group is not passed on as a row field.

## 3. Services by name

### 3.1 Identity

- A service is a camelCase name in exactly one group. Services that come in sets share a prefix and use `prefix:qualifier`
  names, such as `toolContribution:<tool>`, `sessionMethods:<namespace>` and
  `managementMethods:<namespace>`.
- A name resolves, for each copy of a row, to the nearest enclosing instance of its group. It never
  falls back to another group. A row that names a group it is not inside fails at composition.
- One provider per name per instance. A second provider of the same name in the same instance fails.
- Parts in every language depend on names. A Rust type key is how the Rust binding stores a name,
  not a second identity.
- A consumer of prefixed services finds the providers active in its instance by name, whatever their
  language. The host does not pre-register provider names. A Rust consumer may call a native provider
  through its typed binding, but it finds native and foreign providers the same way (R6).

### 3.2 Shape

| Element | Form |
| --- | --- |
| Methods | A name, `sync` or `async`. Callers in other languages always get a Promise |
| Requests and replies | serde data, described as field tables or JSON shapes |
| Errors | Data, `{code, message}`, with the codes documented by the service. A forwarding part rethrows the error it received, and the wire boundary maps it ([plugin contract](plugin-contract.md#9-requirements-raised-to-mechanism-owners) R3) |
| Cancellation | A protocol action: a cancel callback or promise in the request, or the consumer dropping the call |
| Callbacks | By reference, as rutis allows. A callback must not be called inside a commit lock |
| Streams | Callbacks or events; rutis has no stream type |

### 3.3 Crossing languages

A service crosses languages when every method it offers has serde arguments and results. A method
that cannot cross, such as one taking a borrowed value, a channel, a handle or a synchronous callback
inside the commit lock, stays Rust-only. The service is still named and grouped, and the catalog marks
the method or the whole service Rust-only. To make it cross languages, change its shape. There is no
hand-written adapter layer.

### 3.4 The catalog

Each name is declared once. A service that crosses languages declares its name on its Rust type with
`#[rutis::service]`. A Rust-only service, or a name a foreign part provides, is registered in the
host's loader catalog. The catalog shares only those names (`share_registered_only`). `xtask` will
generate the authoritative table from the real assembly (R4). The names Dim defines are listed in the
[plugin contract §4](plugin-contract.md#4-dims-services).

*Today:* the process services that cross languages are marked `#[rutis::service]`. The services
provided per Session or branch are listed with their group in the App Server's build script
(`Exports::service_in`) instead: rutis 0.16 makes a marked type provided in an instance *claim* its
name there, and every claim runs a loader-wide reconcile, which also rebuilds the copies whose build
was rejected. A required row that failed in a Session's assembly could then be rebuilt during that
Session's creation, and each Session or branch creation would reconcile the whole loader. Both forms
give one name per type, checked against the Rust binding by a test.

### 3.5 How each runtime resolves a name

The rule of §3.1 is the framework's ([ADR 006 D4](../adr/006-plugin-contract.md#d4-a-service-is-a-name-in-one-group-and-the-catalog-is-the-contract)):
the loader resolves each name for each copy, and each runtime carries the result out. Part code
writes names only.

| Runtime | The part writes | What resolves the copy |
| --- | --- | --- |
| Node (TypeScript, Cordis 4 plugins) | `inject: [name]`, `ctx.use(name)` or `ctx.<name>`, `ctx.provide(name, value)` | The loader resolves the name (`key_in`) and isolates it in the Node process under the instance's label (Cordis `isolate`) |
| Python and the other runtimes | `inject`, `use`, `provide` | The same, through the runtime's isolate table |
| Rust | `Keyed<T>`; it provides `T` at the same key | The binding takes the key from the catalog for the name and the copy's instances (appendix A) |

- **Private names.** A part keeps a service to itself, or to a group of parts, with its row's
  `isolate`. The catalog does not share such a name.
- **Rust keys.** A name registered with a Rust type is at that type's key; a name that crosses
  languages is at its dispatcher's key (`host_key`). Both are qualified by the group's instance. A Rust
  part reads a provider of any language at the dispatcher's key. An exported Rust service is at both.
- **Provided names.** rutis publishes a service a row announces without declaring it under its plain
  name for the whole process. Inside a Session or branch, a name meant for that instance is registered
  with its group, so each instance has its own, and a name private to the part is isolated by its row.
  Otherwise other instances see the service, and a second copy that provides it fails with rutis's
  duplicate-provider error.
- **Required and optional.** A required dependency is injected and gates the part. An optional one is
  read when used, without injecting it: Cordis `ctx.get(name)`, rutis `Option<Keyed<T>>`. A dependency
  the configuration chooses is included in the copy's names or not.
- **Cordis.** An unmodified plugin for rutis's Cordis 4 (`@deepseek-ai/cordis`) runs as a Node row and
  gets the same resolution. Cordis 3 plugins are not a target.

## 4. Lifecycle rutis provides

| Phase | The part may | rutis does |
| --- | --- | --- |
| Build | Read its configuration and identity | Constructs the plugin; no I/O |
| `apply` | Acquire resources, provide services, register subscriptions, register cleanup | Resolves dependencies by name, gates on them, runs `apply` once per generation |
| Unload | — | Runs cleanup in reverse order, takes the part's listeners off the bus, cancels its running calls and waits up to the rutis deadline |

A generation is one load of a part, from `apply` to unload. When a dependency is replaced, rutis
unloads and reloads its dependents along their declared dependencies. Dim adds its publication and
closing phases on top ([plugin contract §7](plugin-contract.md#7-lifecycle-of-a-part)).

## 5. The event bus

Since rutis 0.15.1 there is one flat bus per root.

- **Delivery.** Every listener of a name receives every event of that name, wherever it was registered.
  A listener cannot see the emitter's instance. An event reaches only the listeners registered when it
  is emitted; it is not stored or replayed. Events of different names have no common order. `emit`
  does not wait; `parallel` waits and aggregates failures.
- **Order.** The events of one name start their listeners in emit order: an event's listeners start
  once every listener of the previous one has started, and nothing waits for a listener that awaits.
  A listener that needs order does its ordered work before its first await, or consumes facts through
  `dispatcher` ([plugin contract §5.2](plugin-contract.md#52-consuming-facts)).
- **Payloads are serde.** No handle, channel or lock goes into an event.
- **The name is the contract.** An event that belongs to one instance carries that instance's identity
  in its name, built by a constructor its owner exports, so callers never concatenate names.
  Alternatively the owner offers a watch or a service method.
- **Reserved prefixes:** `rutis/*` (rutis's framework events: listen only) and `internal/*` (each
  runtime's own).

How Dim's parts consume bus events and which public bus events Dim defines are in the
[plugin contract §5.3](plugin-contract.md#53-bus-events).

*Today:* Dim is on rutis 0.16.0, and this section holds.

## 6. Requirements raised to mechanism owners

These requirements concern how Dim uses rutis. They follow
[principles §4 rule 8](../architecture/rust-agent/principles.md), and their phases are in the
[plugin contract migration](plugin-contract.md#10-migration). R1–R3 are in the
[plugin contract](plugin-contract.md#9-requirements-raised-to-mechanism-owners).

### R4. The catalog is generated and Capabilities are marked automatically

- **Blocked:** §3.4 here and [plugin contract §8](plugin-contract.md#8-guarantees-conventions-and-the-audit). Today the catalog is spread across `build.rs`, the authoring guide and
  design documents, and nothing marks which parts are Capabilities.
- **Existing mechanisms considered:** `xtask graph` already records injected and provided services
  from the real assembly.
- **Needed:** `xtask graph` writes the catalog table (name, group, provider, crosses languages,
  methods) and marks which parts are Capabilities, from the names they inject (`dispatcher`,
  `timeline`).
- **Effect:** none at runtime.
- **Owner:** `xtask`. Issue to be opened.

### R5. Foreign rows declare their group

- **Blocked:** §2. The App Server infers a TypeScript row's group from its `inject` and checks service
  prefixes per group (`session_typescript.rs:158`, `:185`).
- **Existing mechanisms considered:** rutis rows already have `group`, and the catalog rejects
  misplaced names at composition.
- **Needed:** foreign rows carry their group like native ones. The inference and the per-prefix checks
  are deleted.
- **Effect:** none on the Timeline.
- **Owner:** App Server. Delivered in phase 4 ([#376](https://github.com/arcships/dim/issues/376)),
  with the SDK and the repository's plugins.

### R6. Providers of prefixed services are found by name, with one identity

- **Blocked:** [ADR 002 D5](../adr/002-plugin-scope.md#d5-contracts-are-language-neutral)
  ("Registration: declared by the plugin … not by host-side `AgentBuilder` calls"). The host
  pre-registers provider names (`session_tool`, `session_methods`, `session_prompt_source`,
  `session_tool_source`, `host_tools`). A native tool contribution and a foreign one have different
  keys, so ToolRegistry reads both and detects a retiring foreign generation through a sidecar token
  found by the same string (`tool_registry.rs:486-509`). Prompt and tool sources are closed to
  foreign parts.
- **Existing mechanisms considered:** ToolRegistry already takes in the providers active at each read
  and withdraws retiring ones (ADR 002 D3, #241). rutis reports providers coming and going with
  `rutis/service` events, and a marked Rust service is also served by name in its instance.
- **Needed:** each consumer of prefixed services (ToolRegistry, PromptRegistry, Rpc's method table)
  finds the providers of its prefix active in its own instance by name. Native and foreign providers have one
  identity and one discovery path; their generation ends when their service is withdrawn. Pre-registration
  and the sidecar token go. The discovery path is the same in every language: the consumer listens to
  `rutis/service` events (name, scope, availability, row), keeps those of its prefix whose scope is its
  own instance, and reads each by name.
- **Effect:** none on the Timeline. Tool sampling keeps ToolBinding's existing per-turn rule.
- **Owner:** Dim runtime (ToolRegistry, PromptRegistry) and App Server (Rpc). Issue to be opened.
  It lands in phase 3.

## Appendix A: Rust binding

### A.1 The old forms and their replacements

The replacements exist since P2; the old forms remain until P3 has moved every part.

| Old form | Replacement |
| --- | --- |
| `impl Capability for P { fn keys(&self) }` | No Dim trait. A part is a rutis `Plugin` or `TypedPlugin` |
| `SessionScope`, `BranchScope` passed to factories | `Names::instance(group)` when a factory needs an instance; identity from `sessionIdentity` or `timelineIdentity` |
| `InSession<T>`, `InBranch<T>` | `Keyed<T>`: a dependency on `T`'s name, keyed by the instance of the name's group in this copy's chain (§3.5) |
| `SessionGate<T>`, `BranchGate<T>` | `KeyedGate<T>` on the name |
| `scope.key::<T>()` when providing | `names.key::<T>()`, the same key a consumer of the name gets |
| `Contributions<T>` | The names under one prefix, found in the instance (R6) |
| `When<D>` | `When<D>` with keys from `Names::when`: the factory includes the name or not when it builds the copy. Not rutis `Option<..>`, which reads without gating |
| `SessionService`, `BranchService` markers | The group registered in the catalog (`names::register`) |
| `AgentBuilder::session_plugin`/`timeline_plugin` | `AgentBuilder::session_part`/`branch_part`: rows in the `session` or `branch` group; `register_part` for a process row |
| `AgentBuilder::session_tool`, `session_methods`, `session_prompt_source`, `session_tool_source`, `host_tools` | Nothing: providers are found by name (R6) |
| `build.rs` `Exports::service_in(..)` | `#[rutis::service]` on the type; `Exports` for types whose source cannot change, and for services provided per Session or branch (§3.4) |
| `PublicService` | Unchanged (a facade rule) |

### A.2 The binding

A Rust part is a plain `TypedPlugin`. Its dependencies are `Keyed<T>`, `KeyedGate<T>`,
`Option<Keyed<T>>` (read if present, without gating) or tuples of them (`NamedDeps`). The binding
carries out §3.5 for Rust:

- It names a type as `#[rutis::service]` does: `Timeline` is `timeline`, `dyn LoopState`
  `loopState` (`names::name_of`). One Rust type is one name, so a type provided in two groups
  becomes two types, or one group.
- The key comes from the catalog for the name and the copy's instances. `names.key::<T>()` is the
  form for a name registered with the Rust type `T`, or exported from it: `TypeKey::instance::<T>(id)`
  for the instance of the name's group, `TypeKey::of::<T>()` at the process.
  `names.named::<T>(name)` reads any name at the catalog's key; for a name that crosses languages `T`
  is `dyn HostDispatch`, and a provider of any language is found there. A provider provides at the key
  its consumers get.
- The names are checked where the row is placed when the assembly is composed. A part that names a
  service of a group its row is not inside fails there, naming the group; so does one that names an
  unregistered service.

The catalog is the builder's (`AgentBuilder::catalog`), and the App Server's loader uses the same one.
`names::register::<T>(catalog, group)` declares a Rust name. The runtime registers its own, each crate
the names of the services it provides, and registering a name again in the same group does nothing.
An exported name is served to other languages through its generated dispatcher, and a test checks that
each export's name and group are the ones the binding derives for its type.

```rust
pub struct WaitBranch(DepKey<Wait>);
impl TypedPlugin for WaitBranch {
    // "dispatcher" and "timeline" are branch names; nothing here says so.
    type Deps = (Keyed<Dispatcher>, Keyed<Timeline>);
    fn name(&self) -> &str { "WaitBranch" }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (dispatcher, timeline): Self::Deps)
        -> BoxFuture<'a, Result<Effect, CordisError>> {
        /* rebuild (plugin contract §7.2), then ctx.provide_as(self.0.key().clone(), wait) */
    }
}

// A row in every branch; `names` gives this copy's keys, for its dependencies and for `wait`.
builder.branch_part(|names| names.part(WaitBranch(names.key::<Wait>()?)));
// A process row.
register_part(&mut builtins, builder.catalog(), |names| names.part(Recorder))?;
```

A dependency the configuration enables gets its keys in the factory:
`names.part_with(plugin, (names.key::<Timeline>()?, names.when::<Keyed<FileStorage>>(enabled)?))`.

### A.3 Settled questions

Carried from [#6](https://github.com/arcships/dim/issues/6):

- **A lost dependency.** When the provider of a name goes away, rutis returns the dependent to Pending
  and loads it again when the provider is back. The new generation is a restart from the Timeline
  under the capability reload protocol ([plugin contract §7.2](plugin-contract.md#72-generations-reload-and-rebuild)).
  Nothing is added.
- **Optional and configuration-chosen dependencies.** As §3.5: a dependency the configuration chooses
  gates when chosen (`When<D>`, `Names::when`), and the composition check covers it then; one used only
  if present is `Option<Keyed<T>>`, which does not gate. Its name must still be registered.
- **Startup deadline.** One deadline covers loader startup, reconcile and rows becoming Active. At the
  deadline the creation fails and its rollback cancels the rows; rutis waits for a cancelled `apply` to
  return and never aborts it, so `apply` returns once `ctx.cancelled()` resolves. A test injects a row
  that waits in `apply` until then (`tests/integration/tests/named_parts.rs`). Failing on no progress
  instead of a total deadline is [#387](https://github.com/arcships/dim/issues/387).

## Appendix B: TypeScript and Python binding

Parts use `definePlugin` (in Python `define_plugin`, or a module with `inject`, `provides` and
`apply`) with `inject` and `provides`. The row carries the group: `"session"`, `"branch"`, or none at
the process. The SDK's generated declarations come from the catalog.

```json
{ "id": "goal-branch", "name": "/abs/goal/branch.ts", "group": "branch",
  "inject": ["dispatcher", "timelineIdentity", "treeCommands", "settings", "branchLifecycle"],
  "config": {} }
```

A Python row names its module after the runtime, `py:<module>`; the App Server runs it in its
Python runtime when one is configured, importing the module from the configured project directory.

```json
{ "id": "py-branch", "name": "py:python_branch_part", "group": "branch",
  "inject": ["dispatcher", "timelineIdentity", "treeCommands"], "provides": {}, "config": {} }
```

```python
inject = ["dispatcher", "timelineIdentity", "treeCommands"]
provides = {}

async def apply(ctx, config):
    dispatcher = ctx.use("dispatcher")
    identity = await ctx.use("timelineIdentity").get()
    ...
```

```ts
// A Capability that rebuilds by hand (plugin contract §7.2, today's form).
export default definePlugin({
  name: 'ExampleCounter',
  inject: ['dispatcher'],
  provides: { exampleCounter: { get: 'sync' } },
  async apply(ctx) {
    const dispatcher = ctx.use<Services['dispatcher']>('dispatcher')
    let count = 0, through = 0n
    let ready!: () => void
    const rebuilt = new Promise<void>(resolve => { ready = resolve })
    await dispatcher.listen('example-counter', ['input/message'], async event => {
      await rebuilt
      if (BigInt(event.sequence) > through) count++
    })
    const cutoff = await dispatcher.committed()
    count = (await dispatcher.local(0, cutoff))
      .filter(event => event.type === 'input' && event.name === 'message').length
    through = BigInt(cutoff)
    ctx.provide('exampleCounter', { get: () => ({ count }) })
    ready()
  },
})
```

With R1 the same part registers once and folds every event, replayed or live, in one handler.

The repository's fixtures show both languages: `runtime/fixtures/branch-part.ts` and
`runtime/fixtures/python/python_branch_part.py`.
