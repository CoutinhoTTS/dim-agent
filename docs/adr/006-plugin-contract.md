# 006: The plugin contract is concepts and conventions, not types

Decision status: Accepted by the repository owner (2026-10-24). Implementation: not started; the phases are in the
[plugin contract design](../design/plugin-contract.md#10-migration).
[中文](../zh/adr/006-plugin-contract.md)

Relation to other decisions:

- Extends [ADR 002](002-plugin-scope.md). Its D1–D4 stand unchanged. This ADR finishes D5:
  Dim's own runtime services become named contract entries, and Rust becomes one binding of
  the contract instead of its definition.
- Keeps [ADR 001](001-harness-foundation.md#events-and-ownership-issue-2) (envelope, reserved
  domains, descriptive event declarations) and [ADR 003](003-approval-scope.md) (trusted
  in-process plugins are not isolated by declarations).
- Is documented in two designs, kept apart: [How Dim uses rutis](../design/rutis-usage.md) (D2–D4)
  and the [plugin contract](../design/plugin-contract.md) (D5–D8).
- Supersedes the event-scope model of the [rutis 0.15 upgrade design](../design/rutis-0.15-upgrade.md)
  (§4, D2–D4). The [rutis 0.16 upgrade design](../design/rutis-0.16-upgrade.md) replaces it.
- Supersedes the layer-typed authoring API as the target: the `Capability` trait and its `keys`,
  `SessionScope`/`BranchScope`, `InSession`/`InBranch`/`SessionGate`/`BranchGate`,
  `SessionService`/`BranchService`, and `AgentBuilder::session_plugin`/`timeline_plugin`.
  This API remains the implemented Rust binding until the migration replaces it.
- Replaces the unmerged instance-mounting draft (#377) and the decisions of #370.

## Context

Dim's architecture rests on a small theory. The [time bus](../architecture/rust-agent/time-bus.md)
fixes order by sequence, gives each state one owner, routes changes to an owner as commands, makes
handover explicit, limits `before` to transforming a draft, rebuilds state from history and never
replays side effects. ADR 002 D1 adds one placement rule: a part lives where the owner of its state
lives. None of this depends on a programming language.

The plugin API and the guidance around it express this theory as Rust types:

| Concept | Current expression | Extent on `main` (dd87f6af) |
| --- | --- | --- |
| Where a part lives | `session_plugin(\|scope: &SessionScope\| ..)`, `timeline_plugin(\|scope: &BranchScope\| ..)`, `Capability::keys` | 199 `Capability` implementations |
| Which instance a dependency comes from | `InSession<T>`, `InBranch<T>`, `SessionGate<T>`, `BranchGate<T>`, `SessionService`/`BranchService` markers | 221 uses |
| Isolation of event flows | `EventKey::of().instance(id)` | 9 sites, an API rutis 0.13 removed |
| Rules for authors | AGENTS.md rule 1, [principles §1](../architecture/rust-agent/principles.md), [single-tree](../architecture/rust-agent/single-tree.md#实例键与可见性), the plugin page README | 27 English documents name these types |

Four consequences follow.

1. **Non-Rust parts need per-layer bindings that re-derive what the Rust types encode.**
   - The App Server infers a TypeScript row's group from whether its `inject` names a branch
     service (`crates/dim-app-server/src/session_typescript.rs:373`), and hard-codes which service
     prefixes each group may provide (`:400`, `:457-492`).
   - A service's group is declared three times with no cross-check: the `SessionService`/`BranchService`
     markers, `build.rs` `service_in(.., "branch")`, and `BRANCH_SERVICES`.
   - One concept has two identities. A native tool contribution is
     `TypeKey::keyed_dynamic::<ToolContribution>`, a foreign one is `host_key_in("toolContribution:..")`.
     ToolRegistry reads both, and it detects a retiring foreign generation through a sidecar
     cancellation token that it finds by the same string (`tool_registry.rs:486-509`).
   - The host pre-registers provider names (`session_tool`, `session_methods`, `session_prompt_source`,
     `session_tool_source`, `host_tools`), and translates a foreign row's `provides` into those calls.
     ADR 002 D5 says registration is declared by the plugin, not by host-side `AgentBuilder` calls.
   - Session and Settings events reach foreign rows only through a relay on `Process::emit`
     (`:166`, `:251`, `:515`), an API rutis 0.13 removed.
   - TypeScript Session rows have no activation service, although Rust Session parts have one
     ([authoring guide](../typescript-plugin-authoring.md#plugin-shape-and-lifetime)).
2. **Every consumer re-implements the same infrastructure protocols, in every language.** Each
   ordered consumer subscribes, fixes the committed cutoff, folds local history and skips covered
   events: `crates/dim-agent-capabilities/src/wait.rs:361-380`,
   `runtime/plugins/goal/branch.ts:120-128` and the authoring guide's example all do it. A
   TypeScript part translates native error variants into JSON-RPC `appError` codes itself
   (`runtime/plugins/goal/session.ts:15`), which is a boundary conversion done inside a plugin.
3. **Guidance mixes four kinds of statement:** the theory, conventions, the current Rust spelling,
   and delivery status or history. A contributor cannot tell which sentence is a rule and which
   describes today's code. Design "Rule check" sections therefore restate the list instead of
   answering it.
4. **rutis changed underneath, from 0.13 to 0.16:**
   - One flat event bus per root. Every listener of a name receives the event, wherever it was
     registered. A listener cannot see the emitter's instance, and event payloads must be serde.
   - Instance services by name. A catalog name belongs to one instanced group and resolves, for each
     row copy, to the nearest enclosing instance of that group. A row placed outside that group
     fails at composition, whether it is Rust or any other runtime (`ServiceCatalog::register_instance`,
     `register_shared_instance`, `key_in`, `Build::instance`).
   - `#[rutis::service]` serves a Rust service by name in the instance where it is provided.
   - rutis's stated design: "Dependency declarations contain only service names". The model is
     implemented once, in the Rust core, and other languages are leaves with a small SDK. rutis's
     own agent plugin uses plain plugins, domain service traits and named events, with no plugin
     trait of its own.

The 0.15 upgrade design (#361) planned around scoped delivery, which rutis 0.15.1 withdrew. The
#377 draft moved toward the rutis model but kept two things this ADR rejects. It kept a Rust
`Service<T>` identity next to names. It also proposed a loader-enforced Capability role, an
access check that [ADR 003](003-approval-scope.md) says trusted in-process plugins do not have.

## Decisions

### D1. Separate layers, one authority each

| Layer | Content | Authority | Changed by |
| --- | --- | --- | --- |
| Dim architecture: theory | Sequence, ownership, commands, handover, `before`, rebuild, side effects, placement by owner | [time bus](../architecture/rust-agent/time-bus.md), [principles](../architecture/rust-agent/principles.md), [ADR 002](002-plugin-scope.md) | A decision record |
| Dim architecture: plugin contract | What the instance tree means, placement, Dim's services and public bus events, Timeline facts and their consumption, how bus events are consumed, Capability, publication and closing | [plugin contract](../design/plugin-contract.md) | A design change under this ADR |
| Using rutis | Parts as loader rows, services by name and group, the catalog mechanism, crossing languages, the rutis lifecycle and event bus | [How Dim uses rutis](../design/rutis-usage.md) | A design change under this ADR |
| Binding | How one language writes both: Rust traits and macros, TypeScript/Python `definePlugin`, generated declarations | The appendices of How Dim uses rutis, SDK sources, generated files | An implementation change |

The two middle layers are kept apart. D2–D4 decide how Dim uses rutis; D5–D8 decide Dim's
architecture on top of it. A Dim rule is written against what rutis guarantees, never against a rutis
API, and a rule about using rutis decides no business question.

A binding adds no semantics. When a binding needs a rule that no layer states, that layer changes
first. Rust is a binding: it is the most complete one, not the definition. Rules for contributors
(AGENTS.md, principles, templates) are stated at their layer and link to a binding for spelling.

### D2. rutis owns the plugin machinery; Dim defines its domain on top of it

rutis provides the following, and Dim uses it directly:

- plugin lifecycle (apply, effects, unload, cancellation, deadlines);
- dependency gating, Pending and reload propagation;
- services by type and by name, instanced groups and per-instance name resolution;
- loader rows and configuration;
- language runtimes, cross-language calls, the event bus and diagnostics.

Dim defines:

- the instance tree and when its instances open and close (the `agent`, `session` and `branch`
  groups);
- the Timeline mechanism (append, sequence, batch, publication, `before`, Dispatcher);
- the catalog of its named services and public bus events;
- event contracts and reserved domains;
- the part conventions (D6), the assembly profiles and the audit (D9).

Dim adds none of the following: a plugin trait, author-facing scope types, a resolver, a manifest,
a role field, a second service directory, a second bus, or per-language relays and adapters.
If a need seems to require one, check rutis first. A real gap is solved inside Dim's own
mechanisms, following [principles §4 rule 8](../architecture/rust-agent/principles.md); rutis stays
an external dependency.

### D3. A part is a loader row; its group is its placement

Every part is a rutis plugin mounted as a loader row: native or foreign, built in or optional. The
row's group says where it lives: the process (root), `session` or `branch`. The `agent` group holds
the host's facade, not feature parts. Placement follows ADR 002 D1 unchanged.

Nothing else encodes placement: no plugin trait, no factory signature and no inference from
injected names. A Rust factory that needs instance identity reads it from rutis (`Build::instance`,
`Ctx::instance`) or from Dim's identity services.

The host's assembly profile decides which rows a Session or branch waits for before publication
([single-tree](../architecture/rust-agent/single-tree.md#结构)). A plugin does not declare that it
is required. The details are [How Dim uses rutis §2](../design/rutis-usage.md#2-parts-are-loader-rows).

### D4. A service is a name in one group, and the catalog is the contract

- **Identity.** Every service that one part offers another has a camelCase name and exactly one
  group: process, `session` or `branch`. Services that come in sets share a prefix and add a
  qualifier, as in `toolContribution:<tool>`. A name never falls back to another group (rutis rule).
- **One catalog.** Each Dim name is declared once. A service that crosses languages declares its
  name on its Rust type with `#[rutis::service]`; a Rust-only service, or a name a foreign part
  provides, is registered in the host's loader catalog. The catalog shares only registered and
  marked names (`share_registered_only`), so exactly the catalog crosses languages. `xtask`
  publishes it as a generated table from the real assembly: name, group, provider, methods (sync or
  async, serde request and reply shapes), errors, and whether the service crosses languages or is
  Rust-only.
- **Dependencies by name, in every language.** rutis resolves each name for each copy along its
  instance chain and rejects a row that names a group it is not inside. Rust parts use the same
  names: the Rust binding derives keys from the catalog and the copy's instance chain.
  `TypeKey::of::<T>()` remains the key behind a name, not a second identity.
- **Rust-only services stay in the catalog.** A service whose methods cannot cross languages (a
  callback into the commit lock, a handle, a channel) is still named and grouped, and the catalog
  marks it Rust-only. To make it cross languages, give it serde-shaped methods; there is no
  hand-written adapter layer.
- **Errors are data**, `{code, message}` with documented codes (ADR 002 D5). The wire boundary
  (Rpc) maps a service error to a wire error, not the part that forwards it.

- **Prefixed services are found by name.** A consumer of prefixed services (ToolRegistry for
  `toolContribution:*`, Rpc for `sessionMethods:*`, PromptRegistry for prompt sources) finds the
  providers active in its instance by name, whatever their language. The host does not pre-register
  provider names, and there is one identity per provider, not a native one and a foreign one.

The implementation removes: the `Capability` trait and its keys, `SessionScope`/`BranchScope`,
`InSession`/`InBranch`/`SessionGate`/`BranchGate`, `SessionService`/`BranchService`,
`session_plugin`/`timeline_plugin`, the host-side name pre-registration, the native/foreign split of
prefixed providers, the App Server's inference of a row's group, and the per-prefix placement checks
that the catalog's groups already express. `PublicService` stays. It
limits which branch services the Session and branch facades expose to ordinary callers, which is
a facade rule, not a plugin rule.

The details are [How Dim uses rutis §3](../design/rutis-usage.md#3-services-by-name); the names Dim
defines are listed in the [plugin contract §4](../design/plugin-contract.md#4-dims-services).

### D5. Timeline facts and bus events are different things

| | Timeline fact | Bus event |
| --- | --- | --- |
| What it is | A durable, sequenced fact of one branch, in the `AgentEvent` envelope | An ephemeral rutis event on the root's flat bus |
| Written by | The branch's `timeline` service; the producer guarantees the payload | Any part, through rutis `emit`/`parallel` |
| Delivery | The branch's `dispatcher`: in sequence order, every fact to every part registered in the generation | The listeners registered when it is emitted; not stored or replayed; no order across names; `emit` does not wait |
| How a consumer recovers | It rebuilds from the Timeline | As [time bus §3](../architecture/rust-agent/time-bus.md) asks of every state machine: from the Timeline, or by re-reading the owner's state. A missed bus event can delay a decision, not change it |
| Identity | Domain and name; core domains are reserved | A public name in the catalog; serde payload |
| Isolation | Per branch, by construction | By name. A bus event that belongs to one instance carries that instance's identity in its name, built by a constructor the owner exports, or the owner offers a watch or a service method instead |

Instances bound lifecycle and service visibility. They do not bound event delivery. The bus names
that the Dispatcher and Timeline use internally belong to them. Parts consume through `dispatcher`
and register transforms through `timeline.before`, never by those names.

### D6. Capability is a convention: a state machine on a branch Timeline

A Capability is a part that keeps state folded from a branch's facts in sequence order (it consumes
through `dispatcher`), or writes facts of its own domain (through `timeline`). It is defined once, in
[principles §1](../architecture/rust-agent/principles.md). Other parts get no name of their own; they
are parts, and the existing rules apply to them as to every part. Timeline, Dispatcher, TimelineTree,
the scope roots and the facades are mechanisms with their own contracts, not parts in this sense.

This changes the meaning of the word. Before, principles §1 called every Session or branch part a
Capability. Now a Capability is a state machine on a branch Timeline. A Session part that only
provides tools or methods, or only writes inputs, is not a Capability. The capability reload
protocol ([single-tree](../architecture/rust-agent/single-tree.md#能力重载协议)) applies to
Capabilities.

Being a Capability is not declared and not enforced: there is no role field, no trait and no loader
check. Trusted in-process parts are not isolated by declarations (ADR 003, AGENTS rule 11), and on a
flat bus a loader check could cover injection but not listening. Such a check would look like a
boundary without being one. Whether a part is a Capability shows in the names it injects; the audit
(D9) derives it from the real assembly.

### D7. What the mechanisms guarantee, and what they do not

The mechanisms guarantee:

| Concern | Guarantee | Owner |
| --- | --- | --- |
| Visibility | A group's name resolves only inside that group's instances; a misplaced row fails at composition | rutis |
| Exposure | Only catalog names cross languages | rutis catalog, Dim registration |
| Timeline integrity | Envelope check, reserved core domains decoded by their typed contracts, atomic batches, sequence, publication after commit, refusal after close | Timeline (ADR 001) |
| Order | Per-branch delivery in sequence order; the rebuild cutoff semantics | Dispatcher |
| Lifecycle | apply, effects, unload, cancellation and deadlines; publication only after the profile's required rows are Active | rutis, Session and branch assembly |
| `before` | In process, synchronous, inside the commit lock. No foreign runtime registers one ([ADR 001](001-harness-foundation.md#trust-and-cross-language-calls): no cross-process code inside commit critical sections) | Timeline |

Everything else, that is the rules of the time bus, principles §4 and ADR 002, is convention.
Review, the audit (D9) and flow tests check it.

### D8. Protocols every consumer repeats belong to the mechanism's owner

- **Ordered consumption with rebuild becomes one Dispatcher operation.** One registration delivers
  local history through a fixed cutoff, then live events after it, in every language. The
  hand-written form stays valid.
- **The Session and branch scopes each have a lifecycle service, in every language:**
  `sessionLifecycle` (`activated`, `closing`, `hold`) in `session` and `branchLifecycle`
  (`activated`, `closing`) in `branch`. The process scope has none; process parts follow the host's
  start and shutdown through the rutis lifecycle. Today `sessionLifecycle` is Rust-only, and R2
  exposes it.
- **Rpc maps errors to the wire.** Errors of contributed methods are mapped at Rpc. A part rethrows
  the native error data it received.

These are raised as requirements R1–R3 in the contract's [requirements section](../design/plugin-contract.md#9-requirements-raised-to-mechanism-owners),
in the format of principles §4 rule 8; the Dim runtime and the App Server own them. An SDK may add
sugar over the contract, but no semantics (D1).

### D9. Guidance is layered, and the audit is generated

- Each rule is stated in one place. AGENTS.md and principles §1 state rules without Rust spellings,
  which live in the Rust binding. The contract links to rules instead of restating them.
- A design document's "Rule check" says which parts are Capabilities and answers the existing
  list, as before: principles §4 rules 1–9, §1 and ADR 002 D1–D5. The pull request template is
  unchanged.
- Delivery status and history live in ADR delivery tables and design documents, not in rules.
- `xtask` derives from the real assembly which parts are Capabilities (they inject `dispatcher` or
  `timeline`) and writes it into the generated documents, together with the catalog table.

### D10. Order of work

1. The rutis 0.16 upgrade, under [its own design](../design/rutis-0.16-upgrade.md). The relays go
   with it: rows listen to the public bus events directly.
2. The catalog and the Rust name binding.
3. Every part moves to a row with name dependencies, prefixed providers are found by name, and the
   layer types and the host-side pre-registration are deleted. The old types exist only until this
   step has moved every part.
4. Cross-language parity: foreign rows declare their group, the Session and branch scopes get their
   lifecycle services in every language, and Rust services are exported through
   the catalog.
5. The mechanism requirements of D8.
6. Guidance and generated documents follow each step.

Dim is unreleased, so no step keeps a compatibility path: no aliases, shims or transition shapes for
anyone outside the repository. The TypeScript SDK, the repository's plugins, the private
managed-provider plugin and the documents change with the step that needs it. This design changes no
persisted or wire format.

## Consequences

- A service has one identity across languages, so Rust and TypeScript parts in one instance use each
  other's services by name.
- Fewer Dim-specific types. Plugin pages and design documents stop restating Rust spellings.
- The word Capability narrows. Documents that use it for every Session part are revised when they
  are next touched; principles §1 is revised now.
- Being a Capability is not enforced, so a faulty trusted part can still write another owner's domain or listen
  anywhere. Review, the audit and flow tests guard against that, as they do today.
- The private managed-provider plugin follows each SDK change.
- The 33 plugin pages under `docs/architecture/rust-agent/plugins/` move with their parts in step 3.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Keep the layer types and add Rust sugar (`Service<T>`, #370/#371) | Adds a second identity beside names; foreign parts still need host-side placement inference |
| A loader-enforced Capability role (#377) | Looks like isolation without being one (flat bus, trusted plugins); a privileged-service list to maintain |
| Keep per-layer cross-language bindings (C4, C5, process) | Each re-derives placement, lifecycle and delivery, and they already diverge (no Session activation in C4) |
| Put the conventions into SDK helpers | Replicates the framework per language, against rutis's design and D1 |
| Scoped event delivery (the 0.15 design) | Withdrawn by rutis 0.15.1 |
