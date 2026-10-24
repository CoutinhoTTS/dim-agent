# How Dim uses rutis

Status: Proposed under [ADR 006](../adr/006-plugin-contract.md) (D2–D4). Not implemented as a whole;
each section says what holds today.
[中文](../zh/design/rutis-usage.md)

rutis owns the plugin machinery. This document states how Dim uses it: how a part is mounted, how it
names and finds services, how a service crosses languages, and what the rutis lifecycle and event bus
give. It holds for every part, in any language, and its appendices are the Rust and TypeScript
bindings.

It does not state Dim's architecture. What an instance means, Timeline facts, ownership, the relations,
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

*Today:* built-in Rust parts are registered through `AgentBuilder::session_plugin`/`timeline_plugin`
with scope types. The App Server infers a TypeScript row's group from its `inject` (R5).

## 3. Services by name

### 3.1 Identity

- A service is a camelCase name in exactly one group. A family of services uses `family:qualifier`
  names, such as `toolContribution:<tool>`, `sessionMethods:<namespace>` and
  `managementMethods:<namespace>`.
- A name resolves, for each copy of a row, to the nearest enclosing instance of its group. It never
  falls back to another group. A row that names a group it is not inside fails at composition.
- One provider per name per instance. A second provider of the same name in the same instance fails.
- Parts in every language depend on names. A Rust type key is how the Rust binding stores a name,
  not a second identity.
- A consumer of a family finds the providers active in its instance by name, whatever their
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
| Streams | Callbacks or notices; rutis has no stream type |

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
- **Payloads are serde.** No handle, channel or lock goes into an event.
- **The name is the contract.** An event that belongs to one instance carries that instance's identity
  in its name, built by a constructor its owner exports, so callers never concatenate names.
  Alternatively the owner offers a watch or a service method.
- **Reserved prefixes:** `rutis/*` (rutis's framework events: listen only) and `internal/*` (each
  runtime's own).

How Dim's parts consume notices, Dim's own reserved prefix and Dim's public notices are in the
[plugin contract §5.3](plugin-contract.md#53-bus-notices).

*Today:* Dim is on rutis 0.12.1 with instance-keyed events; the [0.16 upgrade](rutis-0.16-upgrade.md)
moves it to the flat bus.

## 6. Requirements raised to mechanism owners

These requirements concern how Dim uses rutis. They follow
[principles §4 rule 8](../architecture/rust-agent/principles.md), and their phases are in the
[plugin contract migration](plugin-contract.md#10-migration). R1–R3 are in the
[plugin contract](plugin-contract.md#9-requirements-raised-to-mechanism-owners).

### R4. The catalog is generated and the relation audit is automated

- **Blocked:** §3.4 here and [plugin contract §8](plugin-contract.md#8-guarantees-conventions-and-the-audit). Today the catalog is spread across `build.rs`, the authoring guide and
  design documents, and relations are not audited.
- **Existing mechanisms considered:** `xtask graph` already records injected and provided services
  from the real assembly.
- **Capability:** `xtask graph` writes the catalog table (name, group, provider, crosses languages,
  methods) and each part's relations. `xtask check` verifies that each plugin page's Rule check names
  its relations, and flags `dim/*` use outside the mechanisms.
- **Effect:** none at runtime.
- **Owner:** `xtask`. Issue to be opened.

### R5. Foreign rows declare their group; relays are removed

- **Blocked:** §2 and §5. The App Server infers a TypeScript row's group from its `inject` and checks
  service families per group (`session_typescript.rs:373`, `:400`). It relays notices through
  `Process::emit` (removed in rutis 0.13).
- **Existing mechanisms considered:** rutis rows already have `group`, and the catalog rejects
  misplaced names at composition. On the 0.16 flat bus a row receives `session/{sessionId}` and
  `settings/{source}` directly.
- **Capability:** foreign rows carry their group like native ones. The inference and the family
  checks are deleted. Rows listen to public notices by name.
- **Effect:** none on the Timeline. Notice delivery to rows equals today's broadcast-and-filter.
- **Owner:** App Server. Issue to be opened. It changes the TypeScript SDK shape, in phase 4.

### R6. Family providers are found by name, with one identity

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
- **Capability:** each family consumer (ToolRegistry, PromptRegistry, Rpc's method table) finds the
  providers of its family active in its own instance by name. Native and foreign providers have one
  identity and one discovery path; their generation ends when their service is withdrawn. Pre-registration
  and the sidecar token go.
- **Effect:** none on the Timeline. Tool sampling keeps ToolBinding's existing per-turn rule.
- **Owner:** Dim runtime (ToolRegistry, PromptRegistry) and App Server (Rpc). Issue to be opened.
  It lands in phase 3.

## Appendix A: Rust binding

### A.1 Today and target

| Today | Target |
| --- | --- |
| `impl Capability for P { fn keys(&self) }` | No Dim trait. A part is a rutis `Plugin` or `TypedPlugin` |
| `SessionScope`, `BranchScope` passed to factories | The copy's instance chain from rutis (`Build::instance`, `Ctx::instance`); identity from `sessionIdentity` or `timelineIdentity` |
| `InSession<T>`, `InBranch<T>` | A dependency on the name; the key comes from the catalog for this copy |
| `SessionGate<T>`, `BranchGate<T>` | A gate on the name (rutis `KeyedGate` with the catalog key) |
| `Contributions<T>` | The names of a family, found in the instance (R6) |
| `When<D>` | A dependency the row's configuration enables: the factory includes the name or not when it builds the copy. Not rutis `Option<..>`, which reads without gating |
| `SessionService`, `BranchService` markers | The group registered in the catalog |
| `AgentBuilder::session_plugin`/`timeline_plugin` | Rows in the `session` or `branch` group of the assembly profile |
| `AgentBuilder::session_tool`, `session_methods`, `session_prompt_source`, `session_tool_source`, `host_tools` | Nothing: providers are found by name (R6) |
| `build.rs` `Exports::service_in(..)` | `#[rutis::service]` on the type; `Exports` only for types whose source cannot change |
| `EventKey::of().instance(id)` | Removed by rutis 0.13; see §5.3 and the 0.16 upgrade |
| `PublicService` | Unchanged (a facade rule) |

### A.2 Sketch

The exact API is decided in P2. The sketch shows the intent only: names come from the catalog, and
nothing in the part says which group it is in.

```rust
// Target sketch, not an API.
pub struct WaitBranch;
impl TypedPlugin for WaitBranch {
    // Keys resolved by the catalog for this copy: "dispatcher" and "timeline" are branch names.
    type Deps = (Keyed<Dispatcher>, Keyed<Timeline>);
    fn name(&self) -> &str { "WaitBranch" }
    fn apply<'a>(&'a self, ctx: &'a Ctx, (dispatcher, timeline): Self::Deps)
        -> BoxFuture<'a, Result<Effect, CordisError>> { /* rebuild (plugin contract §7.2), then provide "wait" */ }
}

// Assembly: one row in the branch group; the factory reads keys from the catalog.
builtins.register_with("WaitBranch", |build| catalog.typed::<WaitBranch>(build));
```

## Appendix B: TypeScript and Python binding

Parts use `definePlugin` (`define_plugin` in Python) with `inject` and `provides`. The row carries
the group. The SDK's generated declarations come from the catalog.

```json
{ "id": "goal-branch", "name": "/abs/goal/branch.ts", "group": "branch",
  "inject": ["dispatcher", "timelineIdentity", "treeCommands", "settings", "branchLifecycle"],
  "config": {} }
```

```ts
// A Capability that rebuilds by hand (plugin contract §7.2, today's form).
export default definePlugin({
  name: 'ExampleCounter',
  inject: ['dispatcher'],
  provides: { exampleCounter: { get: 'sync' } },
  async apply(ctx) {
    const dispatcher = ctx.use<BranchServices['dispatcher']>('dispatcher')
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

*Today:* the row's group is inferred, not declared (R5). Session rows have no lifecycle service (plugin contract R2).
Notices arrive through the `session.event` relay (R5).
