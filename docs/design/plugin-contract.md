# Plugin contract

Status: Proposed under [ADR 006](../adr/006-plugin-contract.md). Not implemented as a whole.
Each section says what holds today. The theory it builds on, the [time bus](../architecture/rust-agent/time-bus.md)
and the [design rules](../architecture/rust-agent/principles.md), is unchanged and applies now.
[中文](../zh/design/plugin-contract.md)

This document is the one contract every Dim plugin is written against, in any language. It states
the concepts, the conventions and the named services and events, and what the mechanisms guarantee.
Rust and TypeScript are bindings of it ([appendix A](#appendix-a-rust-binding),
[appendix B](#appendix-b-typescript-and-python-binding)). A binding adds no semantics. When a binding
seems to need a rule that is not stated here, change this document first.

## Reading path

| You are | Read |
| --- | --- |
| Writing or reviewing any plugin | [Time bus](../architecture/rust-agent/time-bus.md), then §1–§8 here, then your language's appendix |
| Designing a plugin | Also [ADR 002](../adr/002-plugin-scope.md) (placement, configuration) and §6.5 (the rule check to fill in) |
| Changing a mechanism (Timeline, Dispatcher, assembly, catalog) | Also [principles §4 rule 8](../architecture/rust-agent/principles.md) and §9 |
| Touching an existing part | Its page under [`plugins/`](../architecture/rust-agent/plugins/README.md) or [`design/plugins/`](plugins/README.md) |

The [single-tree](../architecture/rust-agent/single-tree.md) and [lifecycle](../architecture/rust-agent/lifecycle.md)
documents describe how the current Rust implementation realises this contract. They are not
required reading for writing a plugin.

## 1. Concepts

| Term | Meaning |
| --- | --- |
| Instance tree | The process root and the instances of the instanced loader groups `agent`, `session` and `branch` (§2) |
| Part | One rutis plugin mounted as one loader row. A feature, such as a plugin package, usually has several parts in different groups |
| Row | A loader configuration entry: `id`, `name` (the module), `group`, `config`, `inject`, `isolate`, `disabled` (§3) |
| Group | Where a row lives: the process, `session` or `branch`. A row in an instanced group loads once per instance |
| Service | A named object that a part provides to other parts. Its identity is a name and one group (§4) |
| Catalog | The names Dim defines and registers on the loader, published as a generated table (§4.4) |
| Timeline | A branch's ordered, durable log of facts |
| Fact | A committed Timeline event: the `AgentEvent` envelope (`id`, `sequence`, a creation time, optional `tags`, and `type`, `name`, `data`) |
| Domain | An event's `type`. The core reserves its domains; an extension writes only its own (§5.1) |
| Command | A fact or input that asks the owner of some state to change it. The owner decides and writes the result |
| Dispatcher | The branch mechanism that delivers committed facts, in sequence order, to the parts registered with it |
| `before` | A synchronous transform of a draft fact inside the commit lock. Rust, in process only (§5.1) |
| Notice | A rutis event on the root's flat bus: ephemeral, by name, serde payload (§5.3) |
| Generation | One load of a part, from `apply` to unload. A reload starts a new generation |
| Relation | How a part relates to a Timeline: Capability, Submitter or Provider (§6) |
| Owner | The part that writes a piece of state or a domain. Every state has exactly one |

## 2. The instance tree

```text
process (root)                 process parts: storage, catalogs, connections, Agents, Rpc …
└─ agent ⟨instanced⟩          the host's Agent facade; no feature parts
   └─ session ⟨instanced⟩     one Session: its Session parts and Session services
      └─ branch ⟨instanced⟩   one branch: its Timeline, Dispatcher and branch parts
```

| Group | An instance opens | It closes | Mechanism services in the instance |
| --- | --- | --- | --- |
| process | Host start | Host shutdown | — (process parts provide their own) |
| `agent` | Agents creates an Agent | Agent close, host shutdown | Host-internal |
| `session` | An Agent creates or restores a Session | Session close, idle unload | `sessionIdentity`, `sessionLifecycle`, `treeCommands` |
| `branch` | TimelineTree creates a branch | The branch ends, or its Session closes | `timeline`, `dispatcher`, `branchLifecycle`, `timelineIdentity` |

Visibility follows the tree, as in [principles §2](../architecture/rust-agent/principles.md):

| Direction | How |
| --- | --- |
| Up (an enclosing instance, the process) | Inject the service by name |
| Down (Session to branch) | `treeCommands`: input by branch id, branch creation. Never by injecting a branch service |
| Sideways (sibling branches, other Sessions) | Not visible. Go through a common ancestor's business service, such as `sessions` |

The scope roots, the facades and their publication rules are mechanisms. Each Session and branch is
published only after the rows its assembly profile requires are Active
([single-tree](../architecture/rust-agent/single-tree.md#结构)).

*Today:* the tree, the groups and the visibility rules are implemented. `sessionIdentity` and
`sessionLifecycle` exist only as Rust services (requirement R2).

## 3. Parts and rows

A part is exactly one row. The row's group is the part's placement. Nothing else encodes placement:
not a trait, not a factory signature, not an inference from injected names.

| Field | Meaning |
| --- | --- |
| `id` | Stable row identity in the configuration |
| `name` | The module: a built-in name, a path, or a runtime prefix (`py:`, `bun:` …) |
| `group` | The process, `session` or `branch` |
| `config` | Structural configuration, validated by the plugin's schema |
| `inject` | The names the part depends on. A foreign plugin's own `inject` must equal this list or be a subset of it |
| `isolate`, `disabled` | rutis isolation labels; whether the row is off |

Rules:

1. **Place by the owner of the state** ([ADR 002 D1](../adr/002-plugin-scope.md#d1-scope-follows-the-owner-of-the-state-and-its-lifecycle)).
   State shared across Sessions, or outliving them, goes in the process. State shared by a Session's
   branches goes in `session`. A branch's facts, its ordered consumption and turn-bound work go in
   `branch`. Split a feature only where its parts really have different owners, and document the
   parts together in one design.
2. **Required or optional is the host's decision.** The assembly profile lists the rows a Session or
   branch waits for. A row added by configuration is optional: if it is Pending or Failed, only its
   own features are affected.
3. **Structural and business configuration stay apart** ([ADR 002 D4](../adr/002-plugin-scope.md#d4-structural-configuration-and-business-configuration-are-separate)).
   The row holds structural settings. Business settings live in the owner's source, read through
   `settings` when they are used. Changing them reloads nothing.
4. **Build from configuration and identity only.** A factory or module top level does no I/O and
   starts no work. Resources are acquired in `apply` (§7).

*Today:* built-in Rust parts are registered through `AgentBuilder::session_plugin`/`timeline_plugin`
with scope types. The App Server infers a TypeScript row's group from its `inject` (requirement R5).

## 4. Services and the catalog

### 4.1 Identity

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

### 4.2 Shape

| Element | Form |
| --- | --- |
| Methods | A name, `sync` or `async`. Callers in other languages always get a Promise |
| Requests and replies | serde data, described as field tables or JSON shapes |
| Errors | Data, `{code, message}`, with the codes documented by the service. A forwarding part rethrows the error it received, and the wire boundary maps it (R3) |
| Cancellation | A protocol action: a cancel callback or promise in the request, or the consumer dropping the call |
| Callbacks | By reference, as rutis allows. A callback must not be called inside a commit lock |
| Streams | Callbacks or notices; rutis has no stream type |

### 4.3 Crossing languages

A service crosses languages when every method it offers has serde arguments and results. A method
that cannot cross, such as one taking a borrowed value, a channel, a handle or a synchronous callback
inside the commit lock, stays Rust-only. The service is still named and grouped, and the catalog marks
the method or the whole service Rust-only. To make it cross languages, change its shape. There is no
hand-written adapter layer.

### 4.4 The catalog

Each name is declared once. A service that crosses languages declares its name on its Rust type with
`#[rutis::service]`. A Rust-only service, or a name a foreign part provides, is registered in the
host's loader catalog. The catalog shares only those names (`share_registered_only`). `xtask` will
generate the authoritative table from the real assembly (R4). Until then, this table, `crates/dim-app-server/build.rs`
and the [authoring guide](../typescript-plugin-authoring.md#available-services) are the reference.

| Name | Group | Provider | Crosses languages today | Notes |
| --- | --- | --- | --- | --- |
| `sessions` | process | Agents | yes | Start or drive another Session |
| `sessionPermissions` | process | Agents | yes | Generated export |
| `sessionMetadata` | process | SessionStore | yes | SessionTitle is the title's one writer |
| `settings` | process | Settings | yes | Business configuration |
| `auxiliaryModel` | process | App Server | yes | Auxiliary model requests |
| `pluginFiles` | process | host | yes, when the host supplies a root | A process part's own files |
| `workflowFiles` | process | App Server | yes | Generated export |
| `managementMethods:<namespace>` | process | the owner | yes | Protected-local v3 only |
| `collaborationMembers:<kind>` | process | the member provider | designed, not implemented | [ExternalAgent](plugins/ExternalAgent.md) |
| `treeCommands` | `session` | TimelineTree | yes | `input` by branch id, `create`, `list`, `info` |
| `toolContribution:<tool>` | `session` | the tool's part | yes | Consumed by ToolRegistry; today a native and a foreign identity (R6) |
| `sessionMethods:<namespace>` | `session` | the namespace's part | yes | Dispatched by Rpc |
| Prompt sources, tool sources | `session` | their parts | Rust-only | Pre-registered by the host today (R6) |
| `sessionIdentity` | `session` | SessionScope | Rust-only | R2: `{sessionId, restored}` as data |
| `sessionLifecycle` | `session` | SessionScope | Rust-only | R2: `activated`, `closing`, `hold` |
| `sessionEvents:<part>` | `session` | C4 setup | yes | Removed in phase 4; parts listen to `session/{sessionId}` instead |
| `timeline` | `branch` | BranchScope | yes, except `before` | `append` of the part's own domain; history reads |
| `dispatcher` | `branch` | BranchScope | yes, `listen` and reads; `subscribe` is Rust-only | Ordered consumption (§5.2) |
| `branchLifecycle` | `branch` | BranchScope | yes | `activated`, `closing` |
| `timelineIdentity` | `branch` | BranchScope | yes | `{sessionId, branchId}` |

The Rust host ports and registries (ToolRegistry, PromptRegistry, ToolHost, ModelHost,
PrepareModelHost and the others in the [generated services](../architecture/rust-agent/generated/services.md))
get names and groups in phase 2. They stay Rust-only unless a foreign part needs them. So do
`BranchOpening` (opening receipts are Rust futures) and the facades' state reads (`StateCell<S>`,
`PublicService`), which are keyed by Rust type. A foreign part exposes its state through a service.

Two mechanism contracts tie the facades to core parts: a branch is published only with a `LoopState`
provider, and a Session only with a `TimelineTreeControl` provider. Loop and TimelineTree are therefore
core parts, written in Rust. This design does not change that.

## 5. Events

### 5.1 Timeline facts

- **Write through `timeline`.** The producer guarantees the payload against the event's contract;
  the Timeline checks only the envelope ([ADR 001](../adr/001-harness-foundation.md#events-and-ownership-issue-2)).
- **Domains.** The core reserves the domains its typed events use: `input`, `stop`, `approval`,
  `permission`, `exec`, `turn`, `prompt`, `context`, `model`, `step`, `branch`, `collaboration` and
  `session` (`RESERVED_DOMAINS` in `dim-agent-types`). An opaque append in a reserved domain is
  rejected. An extension picks its own domain, such as its plugin id. `type` and `name` are
  non-empty and contain no `/` or `*`.
- **Write only your own domain.** To change state another part owns, write a command or an input
  through that owner's entry ([principles §4 rule 1](../architecture/rust-agent/principles.md)).
  Handing an object to the next owner is an explicit event, and the handover cannot be revoked (rule 2).
- **Batches.** Facts that take effect together are appended in one batch, with the deciding fact
  last. Consumers see the facts one by one, never batch boundaries (rule 5).
- **Only facts that later decisions use** go into the Timeline. What is only for display goes out as
  a Session notice (rule 9).
- **`before`** transforms a draft synchronously inside the commit lock: it never blocks the write and
  never waits for external work. On failure the original draft commits (rule 3). It is Rust and in
  process only, because no cross-process code may run inside a commit critical section
  ([ADR 001](../adr/001-harness-foundation.md#trust-and-cross-language-calls)).
- **Model context** receives only adopted inputs and tool results. A part that needs content in the
  context writes an adopted input with the final content, or returns it in a tool result
  (AGENTS rule 10).

### 5.2 Consuming facts

- Register with the branch's `dispatcher`, naming the keys you consume: an exact `domain/name`,
  `domain/*` or `*`. Delivery follows sequence order. Execution speed, completion order and
  registration order are not semantic order (time bus §1).
- A handler registers its own work and returns. It never waits for a tool, a build, cleanup, a later
  event or activation (rule 3).
- Rebuild on every generation (§7.3). Decisions use only facts the part has consumed, not the
  committed-but-unconsumed tail (time bus §4).

### 5.3 Bus notices

Notices are rutis events on the root's single flat bus. Every listener of a name receives every
notice of that name, wherever the listener is.

- **The name is the contract.** A notice whose meaning belongs to one instance carries that instance's
  identity in its name, through a constructor the owner exports, so callers never concatenate names.
  Alternatively the owner offers a watch or a service method.
- **Payloads are serde.** No handle, channel or lock goes into a notice.
- **Notices are for observation and display.** `emit` does not wait; `parallel` waits and aggregates
  failures. A decision with critical effects reads a fact or the owner's state, never only a notice.
- **Reserved prefixes:** `rutis/*` (rutis's framework events: listen only), `internal/*` (each
  runtime's own), and `dim/*` (Dim's private mechanism channels: no part emits or listens).

Public notices:

| Name | Emitter | Payload | Meaning |
| --- | --- | --- | --- |
| `session/{sessionId}` | SessionBroadcast | A committed event, a Session notice, a fence, or closed | The Session's live projection for clients and observers ([Session broadcast](../architecture/rust-agent/event-driven-flows.md#6-session-广播)) |
| `settings/{source}` | Settings | `{source, scope, revision}` | A business source changed |
| `settings.project_authorized` | App Server Settings | `{root, cwd}` | A project root was authorized |
| `skills.catalog_changed` | SkillsCatalog | catalog version | The Skill catalog advanced |
| `agent/session-restored`, `agent/unloaded-session-deleted` | Agents | ids | Host-internal, consumed by Collaboration |

*Today:* the events still use 0.12.1 instance keys. TypeScript rows receive `session.event` and the
Settings notices through a relay. The [0.16 upgrade](rutis-0.16-upgrade.md) moves the events to the
flat bus, and phase 4 removes the relay.

## 6. Relations and their obligations

A relation is not declared, and no mechanism enforces it ([ADR 006 D6](../adr/006-plugin-contract.md#d6-roles-are-conventions-defined-by-a-parts-relation-to-the-timeline)).
It follows from what the part does, and it shows in what the part injects and provides. A part may
hold several relations; it answers the obligations of each one it holds, plus those of every part.

### 6.1 Telling them apart

| The part | Relation |
| --- | --- |
| Injects `dispatcher` and folds state from a branch's facts | Capability |
| Injects `timeline` and appends facts of its own domain | Capability |
| Writes inputs or commands through an owner's entry (`treeCommands.input`, Session input) and holds no state derived from that branch | Submitter |
| Provides catalog services: tools, methods, prompt sources, host ports | Provider |

Examples:

- The Goal branch part folds facts and decides continuations, so it is a Capability. It is also a
  Submitter, because it writes continuation inputs through `treeCommands`.
- The Goal Session part provides `goal_update` and `session.goal.*` (Provider) and writes change
  inputs by branch id (Submitter). It holds no Goal state.
- Questions provides `ask_user` and `session.question.*` (Provider). Its waiters are its own tool
  calls, so it writes no facts.

### 6.2 Capability

| # | Obligation | Source |
| --- | --- | --- |
| C1 | Consume the branch only through `dispatcher`, in sequence order; base decisions only on consumed facts | Time bus §1, §4 |
| C2 | Rebuild state from the Timeline on every generation (§7.3). State that cannot be rebuilt does not decide anything that outlives the current work | Time bus §3 |
| C3 | Handlers register work and return; network and tool waits run in the part's own tasks | Rule 3 (AGENTS) |
| C4 | Write only facts of the part's own domain; reach other owners through their commands or inputs | Principles §4 rules 1, 7 |
| C5 | Every work the part starts gets a terminal fact on the part's own branch, also when it succeeds and the result is needed | Time bus §2 |
| C6 | Never replay a side effect: a new generation takes over from a sequence position, and settles the previous generation's open work with terminal facts | Time bus §5 |
| C7 | Facts that take effect together go in one batch, with the deciding fact last | Principles §4 rule 5 |
| C8 | Start background work only after `activated`. On `closing`, stop admission, cancel and release first, then write terminal facts, all before the part's services are withdrawn | AGENTS rule 8, §7 |
| C9 | A `before` transform (Rust only) changes only the draft, synchronously, and never blocks or waits | Principles §4 rule 3 |
| C10 | Write only facts that later decisions use | Principles §4 rule 9 |

### 6.3 Submitter

| # | Obligation | Source |
| --- | --- | --- |
| S1 | Write through the owner's entry; the producer guarantees the validity of what it writes | AGENTS rule 4 |
| S2 | Write by branch id and hold no per-branch state; the branch owner decides what the input does | Principles §2, ADR 002 |
| S3 | Do not decide for the owner and do not read its private state; read its results as facts or through its public services | Principles §4 rule 7 |
| S4 | A committed reply proves neither dispatch nor adoption. A command that has no effect writes no event, and the reply says so | Principles §4 rule 1 |

### 6.4 Provider

| # | Obligation | Source |
| --- | --- | --- |
| P1 | The service has a catalog name and group, serde shapes, errors as data and cancellation as protocol (§4) | ADR 002 D5 |
| P2 | A call's facts belong to the caller's owner. For example, ToolExecutor writes a tool call's result; the tool's provider does not | Time bus §2 |
| P3 | The provider's own state belongs to its group's owner; it does not keep branch state in a Session part | ADR 002 D1 |
| P4 | A core consumer never takes a hard dependency on an optional provider. It looks the provider up for each call, or takes in the providers active when it reads | ADR 002 D3 |

### 6.5 Every part

| # | Obligation | Source |
| --- | --- | --- |
| A1 | Placement by the owner of the state; the row's group expresses it (§3) | ADR 002 D1 |
| A2 | No wiring module: no plugin exists only to connect, aggregate or adapt others | ADR 002 D2 |
| A3 | Every dependency is injected by name, including dependencies that only gate load order | ADR 002 D3 |
| A4 | Structural configuration in the row, business configuration in the owner's source | ADR 002 D4 |
| A5 | Acquire resources in `apply` and register their cleanup there immediately | §7 |
| A6 | Interact only through public facts, services and notices; never read another part's internal state | Principles §4 rule 7 |
| A7 | Notices are for observation; anything a decision needs is a fact or an owner's state | §5.3 |
| A8 | A missing mechanism capability is raised as a requirement on its owner, not worked around in the part | Principles §4 rule 8 |

A design document's "Rule check" lists the relations the part holds, then answers each obligation by
number, or says why it does not apply. It also answers A1–A8. The pull request template asks the same
questions.

## 7. Lifecycle of a part

### 7.1 Phases

| Phase | The part may | The mechanism does |
| --- | --- | --- |
| Build | Read its configuration and identity | Constructs the plugin; no I/O |
| `apply` | Acquire resources, provide services, register subscriptions and transforms, register cleanup | Resolves dependencies by name, gates on them, runs `apply` once per generation |
| Activated | Start background work and input consumption | `activated()` resolves when the Session or branch is published; it is `false` if assembly failed |
| Closing | Stop admission, cancel its own work, release resources, write terminal facts | `closing()` resolves when close starts. A Session is closing from the moment its close begins |
| Unloaded | — | Runs cleanup in reverse order, takes the part's listeners off the bus, cancels its running calls and waits up to the rutis deadline |

Dispatcher handlers can receive opening facts before activation. A handler never waits for
activation.

### 7.2 Generations and reload

A reload is a restart from the Timeline (time bus §5). Events dispatched while the part is reloading
do not reach it; the new generation reads them back. The unloading generation settles its own work:
a tool call returns an error result marked as reload settlement, a model request ends `Cancelled`. It
never writes facts for another part. The reason is `capability reloading`. The full table is the
[capability reload protocol](../architecture/rust-agent/single-tree.md#能力重载协议). Detaching a
client is not closing a Session, and a runtime handle is not a durable identity.

### 7.3 The rebuild protocol

Every Capability follows these steps on every generation:

1. Register with `dispatcher` first.
2. Fix the cutoff `S = committed()`.
3. Fold `local(..=S)`, which is this branch's facts without the ancestor prefix. Read `history` only
   when the part's contract says that inherited facts count.
4. Live events with a sequence `≤ S` are skipped; those after `S` are consumed live.
5. Open work found in the folded facts, which has no terminal fact, gets a failure terminal fact
   from its owner and is not executed again.
6. Provide derived services only after the fold.

R1 makes steps 1–4 one Dispatcher registration, for every language. The hand-written form stays
valid.

## 8. Guarantees, conventions and the audit

The guarantees the mechanisms give are listed in [ADR 006 D7](../adr/006-plugin-contract.md#d7-what-the-mechanisms-guarantee-and-what-they-do-not).
Everything in §6 is convention. Three things check conventions:

| Check | What it covers |
| --- | --- |
| `xtask graph` / `check` | Today: crate boundaries, generated assembly graphs, and plugin pages against the code. After R4 it also derives each part's relations from the names it injects and provides, writes the catalog table, checks that each plugin page answers the checks of its relations, and flags Rust code that emits or listens under `dim/*` outside the mechanisms |
| Review and the pull request template | The rule check of §6, by relation |
| Flow tests | Observable behaviour: ordering, rebuild, reload settlement, close |

## 9. Requirements raised to mechanism owners

Each requirement comes from the mechanism's own responsibility and closes inside it
([principles §4 rule 8](../architecture/rust-agent/principles.md)). Issues are opened after this
design is accepted.

### R1. Ordered consumption with rebuild as one Dispatcher registration

- **Blocked:** C1/C2. Every Capability repeats steps 1–4 of §7.3. Examples are
  `crates/dim-agent-capabilities/src/wait.rs:361-380` and `runtime/plugins/goal/branch.ts:120-128`.
  A mistake (a cutoff read before subscribing, `history` instead of `local`) silently loses or
  double-counts events.
- **Existing mechanisms considered:** `subscribe`/`listen` plus `committed`/`local` are correct but
  leave the protocol to every author. An SDK helper per language would replicate the protocol
  (ADR 006 D1).
- **Capability:** `dispatcher.consume(name, keys, handler)`, or an option on `listen`, which
  registers, fixes `S`, delivers `local(..=S)` with `replay: true`, then delivers live events after
  `S`, through one handler and in sequence order. It carries no business meaning.
- **Effect on sequence, batch, interruption, reload:** none on the Timeline. The delivery order equals
  the hand-written protocol's. Reload behaviour is that of §7.2.
- **Owner:** Dim runtime (Dispatcher). Issue to be opened.

### R2. Lifecycle and identity services in every group, for every language

- **Blocked:** C8. A TypeScript Session row has no `activated()` or `closing()`, although a Rust Session
  part has `SessionLifecycle` ([authoring guide](../typescript-plugin-authoring.md#plugin-shape-and-lifetime)).
  The row learns its Session id from the C4 setup rather than from a service.
- **Existing mechanisms considered:** C4's per-row setup carries the id but not the phases.
  `branchLifecycle` already exists for branch rows.
- **Capability:** `sessionLifecycle` (`activated`, `closing`, `hold`) and `sessionIdentity`
  (`{sessionId, restored}`) as catalog services in `session`, with serde shapes. `hold` returns a
  handle released by a call or by the row's unload.
- **Effect:** none on the Timeline. It exposes the existing phases.
- **Owner:** Dim runtime (SessionScope) and App Server (catalog). Issue to be opened.

### R3. Rpc maps service errors of contributed methods to wire errors

- **Blocked:** P1. `runtime/plugins/goal/session.ts:15` maps native error variants (`InvalidInput`,
  `UnknownBranch`, `Closed`, `Reloading`) to JSON-RPC `appError` codes. Every method provider that
  forwards a native error would have to repeat the mapping, and a new native code needs a change in
  every plugin.
- **Existing mechanisms considered:** the `MethodReply` error form `{code, appError, message}` puts the
  wire code into the plugin contract.
- **Capability:** contributed methods return `{code, message}` data, or rethrow the service error they
  received. Rpc maps native and documented codes to the wire `appError`, as it already does for built-in
  methods.
- **Effect:** none on the Timeline. Wire results stay the same.
- **Owner:** App Server (Rpc). Issue to be opened. It changes the TypeScript SDK shape, so it lands in
  phase 4.

### R4. The catalog is generated and the relation audit is automated

- **Blocked:** §4.4, §8. Today the catalog is spread across `build.rs`, the authoring guide and
  design documents, and relations are not audited.
- **Existing mechanisms considered:** `xtask graph` already records injected and provided services
  from the real assembly.
- **Capability:** `xtask graph` writes the catalog table (name, group, provider, crosses languages,
  methods) and each part's relations. `xtask check` verifies that each plugin page answers the checks
  of its relations, and flags `dim/*` use outside the mechanisms.
- **Effect:** none at runtime.
- **Owner:** `xtask`. Issue to be opened.

### R5. Foreign rows declare their group; relays are removed

- **Blocked:** §3, §5.3. The App Server infers a TypeScript row's group from its `inject` and checks
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

- **Blocked:** A3, P1 and [ADR 002 D5](../adr/002-plugin-scope.md#d5-contracts-are-language-neutral)
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

## 10. Migration

Each phase is a complete, reviewable change with its tests and documentation. Observable behaviour
does not change except where a phase says so. Timeline, storage and wire formats never change.

| Phase | Work | Acceptance |
| --- | --- | --- |
| P1 | The [rutis 0.16 upgrade](rutis-0.16-upgrade.md) | That design's flow checks and the full CI sequence |
| P2 | Catalog and Rust binding: register Dim's names and groups; a Rust part declares dependencies by name and gets keys from the catalog and its instance chain (appendix A); both forms work side by side; R4's catalog table | A Rust part that imports no layer type mounts in process, `session` and `branch`; a misplaced row fails at composition with the group it needs |
| P3 | Move every part to a row with name dependencies, in four groups: process and simple parts; core branch parts; core Session parts; App Server host parts. R6: family providers found by name. Delete the `Capability` trait, the scope types, the `In*`/`*Gate` wrappers, the `*Service` markers, `session_plugin`/`timeline_plugin` and the name pre-registration. Regenerate the plugin pages | `xtask check` passes; no layer type remains in code or documentation; existing multi-Session and multi-branch flow tests, ordering, cancellation and facade restrictions unchanged |
| P4 | Cross-language parity: R2, R3 and R5; export Dim's cross-language services with `#[rutis::service]`; foreign runtimes' output reaches the process log instead of being discarded ([ADR 005](../adr/005-observability.md)); one SDK change with the authoring guide; the private managed-provider plugin follows | A TypeScript and a Python row each mount in `session` and `branch` without host inference, consume facts, submit inputs and provide a tool. The Node-backed flow tests pass |
| P5 | R1 | Every Capability rebuild test passes with the one-registration form; the hand-written form still passes |
| P6 | Guidance and generated documents final: the plugin pages, the authoring guide, single-tree and lifecycle describe the implemented contract | `xtask check`; no document requires an API that does not exist |

Proposed issue split, to replace #370–#376 after this design is accepted:

| Existing | Becomes |
| --- | --- |
| #370 (design) | Closed by this design |
| #371 (`Service<T>` and raw mount APIs) | P2, without `Service<T>`: catalog registration and name dependencies |
| #372, #373, #374 (migrations) | P3, the same three groups plus process and simple parts |
| #375 (delete the old API) | The end of P3 |
| #376 (raw rows for other languages) | P4 (R2, R3, R5) |
| #362 (`SkillsCatalog` watch) | Closed: on the flat bus the notice reaches every Session row |
| new | P1 (0.16 upgrade), P5 (R1), R4, R6 |

## 11. Rule check

This design changes the plugin infrastructure's contract, not a business plugin.

| Rule | Answer |
| --- | --- |
| Principles §1 (plugins and Capabilities are bounded) | Capability narrows to a state machine on a branch Timeline (§6). No wiring module is added; the App Server's group inference and relays are removed (R5) |
| §4 rule 1 (commands) | Unchanged; restated as C4 and S1–S4 |
| §4 rule 2 (handover) | Unchanged |
| §4 rule 3 (`before`) | Unchanged; now stated as in process only (§5.1) |
| §4 rule 4 (core has no business meaning) | The catalog, R1 and R3 are generic; no plugin-specific handling is added |
| §4 rule 5 (batches) | Unchanged |
| §4 rule 6 (context) | Unchanged |
| §4 rule 7 (public interaction only) | Restated as A6 and S3 |
| §4 rule 8 (infrastructure unchanged by default) | R1–R6 each come from the mechanism's own responsibility and close inside it; rutis is used as it is |
| §4 rule 9 (only needed facts) | No fact is added |
| ADR 002 D1–D5 | D1–D4 unchanged (A1–A4); D5 completed: one named catalog, registration declared by the plugin (R6), Rust as a binding |

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
        -> BoxFuture<'a, Result<Effect, CordisError>> { /* §7.3, then provide "wait" */ }
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
// A Capability that follows §7.3 by hand (today's form).
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

*Today:* the row's group is inferred, not declared (R5). Session rows have no lifecycle service (R2).
Notices arrive through the `session.event` relay (R5).
