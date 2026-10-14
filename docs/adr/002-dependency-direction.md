# 002: Dependency direction and configuration placement

Decision status: Proposed. Applies to the plugin designs in `docs/design/` and to future changes of
existing plugins. It does not change implemented behavior by itself; the review below records
where existing plugins diverge and what changing them requires.

## Context

rutis propagates generations along `Deps`: when a provider reloads, its dependents are evicted and
loaded again, across process, Session and branch scopes; when a provider is missing, its dependents
stay Pending ([single-tree](../architecture/rust-agent/single-tree.md), "upstream reload" and
"upstream missing" rules). A reloaded capability settles its in-flight work with
`capability reloading`, and Loop ends the turn as `Failed` when it sees that settlement. For core
infrastructure this is the intended safety property: a dependent must never keep using a stale
generation (AGENTS rule 8).

The plugin designs, and several existing plugins, let a stable consumer depend on a provider that
changes for unrelated reasons (configuration edits, external connections, a shared Node runtime,
account plugins, remote nodes). Then one such change fails every running turn in every Session,
and a provider that cannot start leaves every Session Pending.

Three facts constrain the options (verified in code):

- `When<D>` is fixed when the plugin is built and is a hard dependency once enabled
  (`crates/dim-agent-runtime/src/capability.rs`). It is not an optional dependency.
- `Contributions<T>` makes every registered name a hard dependency; a registered name without a
  provider fails the Session build ([tool contributions](../architecture/rust-agent/tool-contributions.md)).
- Optional dependencies and settlement on dependency loss are open in issue #6.

The design revision first answered this with an extension registry: a separate registry plugin
discovering configured contributors from loader declarations and mounting one instance per
contributor. That adds a mechanism, makes fail-closed points depend on loader metadata, and keeps
business configuration in loader rows. This ADR replaces it with rules that reuse existing
mechanisms.

## Decisions

### D1. Dependencies point from volatile to stable

A dependency edge points from the party that changes more often to the party that changes less
often. A consumer must not declare a dependency (`Deps`, `When`, `Gate`, `Contributions`) on a
provider that can reload or become unavailable for reasons unrelated to the consumer.

Strong dependencies remain correct for core infrastructure whose generation change must restart
dependents: storage, ServerControl, Aimux's `ModelHost`, scope values (Timeline, Dispatcher,
SessionConfig, TreeCommands), HostPlatform, and stable first-party contributors (`branch`, `wait`,
`skill`, PromptSources, WorkspaceTools' `ToolHost`).

### D2. Contributors register into a stable port owned by the consumer

Where a consumer accepts contributions from volatile plugins, the consumer side provides a
registration port that depends on no contributor. A process-level port is the default:

- The contributor depends on the port, registers in `apply` and unregisters in `ctx.effect`.
- Session and branch consumers depend on the same stable port and read or watch its contents.
- Values a contributor needs about the calling Session (cwd, resource node, interactivity) are
  passed per call; process plugins never read Session state.
- When a contributor unloads, its registrations disappear; its own in-flight calls end with a
  per-call "provider unavailable" outcome, never `capability reloading`, so turns continue.

The port must itself be stable: it is hosted in a plugin with no dependency on any contributor and
no volatile dependency of its own. When the natural owner is volatile (for example Web, Modality or
ExternalAgent owning a backend, adapter or node port), either host the port in a separate stable
plugin or accept that its contributors reload with the owner, and state which.

The existing precedent is ManagedProviderBinding registering into ManagedProviders. The pattern
is the same shape as SkillsCatalog (process) feeding Skills (Session).

No separate registry plugin, loader-declaration discovery or per-contributor mount instance is
introduced. `docs/design/contracts/extension-registry.md` is withdrawn.

### D3. Scope parts and dynamic instances only when scope state is required

A contributor is created as a dynamic instance inside a Session or branch subtree (rutis-loader
`create_instance`/`remove_instance`, declared under the `session` group as the `trace` row is)
only when it must hold that scope's state or write that branch's Timeline, and only when enabled
for that scope. A Session or branch part is kept only when it holds that scope's state or writes
its Timeline (for example CollaborationTimeline, the Questions branch part, TimelineTree, Wait,
Skills); otherwise the feature is a process plugin registering into a port. Static placeholders
installed in every assembly are not used for optional or volatile features. The MCP generation
rows are the existing dynamic-instance precedent.

### D4. Peer notices use events, not dependencies

A process plugin that only needs to hear about another plugin's outcomes (for example Scheduler
run outcomes for Notifications) uses a published event or notice, which forms no `Deps` edge,
instead of `When<Service>`.

### D5. Structural configuration and business configuration are separate

| Kind | Examples | Location | On change |
| --- | --- | --- | --- |
| Structural plugin configuration | enable/disable, runtime and package paths, node binding, listener addresses | loader rows (issue #3 editable layer) | may reload the row; that is a structural change |
| Business configuration | search services, notification channels, hook rules, scheduled jobs, workflow definitions, model purpose policies, MCP server and Skill documents, external agent entries, exec shell | the owning plugin's business source (Settings source or the plugin's own store) | data update plus notification; read at use time; no reload |

This keeps issue #3's decision for plugin intent and confines it to structural configuration.
ADR 001 already treats MCP and Skill documents as business documents with their own authority;
this decision applies the same rule to every new plugin. `x-volatile` loader fields are needed
only for structural settings that should not restart a row.

### D6. Fail-closed requirements come from adopted business configuration

A fail-closed point (tool admission checks, submission admission) denies only when business
configuration the consumer has adopted requires a check that is not registered (for example an
adopted hook rule matching the call). Loader declarations, rows of unknown declarations or the
mere presence of a configured plugin never deny by themselves. The adopted configuration stays
readable while the checker is unavailable.

## Review of existing plugins

| Id | Edge | Effect today | Decision |
| --- | --- | --- | --- |
| A1 | ToolRegistry → McpSession (installed in every Session) → McpService | `restart("Mcp")` or a Settings/Publication reload reloads ToolRegistry, ToolBinding, Loop and ToolExecutor in every Session; all in-flight turns fail, including Sessions without MCP servers | Invert (D2): remove McpSession; Mcp registers its tool sets into the tool port, derived per call from the Session's working directory and the MCP catalog (approval is already rechecked per call) |
| A2 | ToolRegistry → `agent` contribution → Collaboration → Agents | `restart("Agents")` fails every in-flight turn in every Session | Invert: remove Collaboration's Session part; `agent` registers into the tool port; CollaborationTimeline stays a branch capability (it rebuilds from the branch Timeline) and reaches Collaboration per call through a stable port |
| A3 | Rpc → Mcp, Settings, SkillsCatalog, Provider, OAuth, ManagedProviders | any of these reloading reloads Rpc; waiters end and requests return `service_reloading` | Invert: services register methods into Rpc's method port (D2) |
| A4 | Agents → Settings, ModelSelectionHost | latent today; becomes the entry point of A2 once Provider can reload | Invert: read at use time through stable ports; Agents depends only on AgentDirectory, Loader and storage |
| A5 | Provider → `When<ProviderSecretHost>` | a supplier reload reloads Provider, Agents and every ModelProvider | Invert: the supplier registers into a Provider secret port; resolution failure is per request. Agents and ModelProvider resolve model selection through a stable port that Provider and ConfiguredModels register into |
| A6 | HostPlatform exec shell read once in `apply` | changes apply only after a process restart | Business configuration read per spawn (D5) |
| Keep | ModelProvider → Aimux `ModelHost`; ModelProvider → Provider resolver while Provider is stable; Rpc → AgentIndex; storage, ServerControl, scope values; first-party Session contributors; HostPlatform consumers | — | Strong (D1) |
| Precedents | ManagedProviderBinding → ManagedProviders (D2); McpServer generation rows (D3); `trace` instanced group (D3); MCP and Skill documents (D5) | — | Keep |

These are recorded decisions for future changes; implementing them is separate work with its own
tests and does not block the plugin designs.

## Review of the plugin designs

| Area | Problem | Decision |
| --- | --- | --- |
| Extension registry contract | registry plugin, loader-declaration discovery, mount instances, volatile-configuration rule | Withdrawn; replaced by D2, D5, D6 |
| Process tools (Goal, Web, Scheduler, Workflow, Notifications) | ToolRegistry injected contributors through the registry | Process-level tool port owned by ToolRegistry's side (D2) |
| Admission checks (Hooks) and submission admission | denial derived from loader declarations; global blast radius | Ports (D2); denial only for adopted rules that match (D6) |
| Method contribution (Rpc, Management) | mounts and "configured but inactive" from declarations | Rpc method port (D2); Management is a level-filtered view of it; registration state alone decides `unknown_method` vs `service_reloading` |
| ModelHost kinds, Web search backends, Modality adapters | mounts and declaration discovery | Ports in ModelHost, Web and Modality (D2); availability from Provider connection entries, `web.search.service` and purpose policies (D6) |
| Node services | every node a mount | Stable node-services port; the local node registers with strong core dependencies; peer nodes as dynamic instances per configured link (D3) |
| SessionAccess and the broadcast subscription | provided by Agents, yet callers claimed to survive Agents reloads | Hosted in a stable plugin depending only on AgentDirectory and Loader; AgentIndex registers into it and is resolved per call (D2) |
| Workflow `When<Collaboration establish>` | hard dependency contradicts the claimed isolation | Collaboration registers `establish` into the stable Session access port; Workflow resolves it per call |
| Notifications `When<ScheduledRunOutcomes>` | Scheduler reload reloads Notifications | Event notice (D4) |
| Questions | `SessionToolSource` placeholder in every Session | `ask_user` registers into the tool port and is filtered per call by interactivity (D2); the branch part stays a branch capability because it writes `question/*` facts (D3) |
| ExternalAgent | one static bridge row per node | Bridges are structural loader rows bound to a node, present only while an agent is enabled there; entries are business configuration (D5) |
| Business configuration in the loader editable layer | Hooks user rules and `disable_all`, `web.*`, `external_agents.*`, notifications, `goal.maxContinuations`, `sessionTitle.*`, RemoteGateway runtime settings, PluginManager `plugins.configure`, Modality settings | Move to each owner's business source (D5); plugin intent and RemoteGateway `enabled` stay structural |
| Permission cross-Session rules | must not become a reloading dependency of Approval | Read at use time through a stable Settings port |

Scheduler jobs, Workflow definitions, Provider purpose policies, MCP and Skill documents already
follow D5.

## Consequences

- The design documents are revised in the same change set: `overview.md`, the contracts (withdraw
  `extension-registry.md`; revise `process-tool-contribution.md`, `session-access.md`,
  `session-broadcast-subscription.md`, `session-boundary.md`) and the affected plugin documents.
- A contributor's reload still fails that contributor's own in-flight calls; only the blast radius
  shrinks to them.
- Core infrastructure generation changes still restart dependents, by design.
- TypeScript contributors share one Node runtime, so a runtime restart unregisters all of them
  together; their registrations disappear, consumers do not reload.
- `Contributions<T>` names stay fixed at assembly; they are reserved for stable first-party
  contributors.
- Issue #6's optional dependencies become unnecessary for these cases; they may still be useful
  elsewhere.

## Alternatives considered

- **Extension registry with per-contributor mounts and loader-declaration discovery.** Rejected:
  a new mechanism duplicating what consumer-owned ports provide, fail-closed tied to loader
  metadata, and business configuration kept in loader rows.
- **Per-Session dynamic instances for every contributor.** Rejected as the default: correct
  dependency direction, but it adds instances where a process-level port suffices. Kept as the
  exception in D3.
- **Waiting for rutis optional dependencies (issue #6).** Not adopted: `When` is hard today, and an
  optional dependency would still not express fail-closed requirements (D6) or the
  configuration split (D5).
