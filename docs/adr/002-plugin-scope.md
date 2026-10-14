# 002: Plugin scope assignment and configuration placement

Decision status: Proposed. Applies to the plugin designs in `docs/design/` and to future plugins.

## Context

Dim assembles plugins at three scopes: the process, a Session, and a branch
([single-tree](../architecture/rust-agent/single-tree.md)). A plugin's scope decides which services
it can depend on, which events it can observe, which facts it can write and how long it lives.

An earlier revision of the plugin designs moved most new features to the process scope to reduce
the number of capabilities. Those features still had to offer tools to a Session, observe a
Session's events and write a branch's inputs, so the designs added wiring to reach back into
Sessions: an extension registry, registration ports, a Session access host, a process-level
Session event subscription, a process-level tool contribution, admission records and a
management aggregation plugin. Each piece existed only to connect plugins placed at the wrong
scope.

Plugin reloads are rare: they follow structural operations such as installing, upgrading,
enabling or disabling a plugin, or a crash. Changing business settings must not reload anything.

## Decisions

### D1. Scope follows the owner of the state and its lifecycle

| Scope | Holds | Existing examples |
| --- | --- | --- |
| Process | Resources and state shared across Sessions or outliving them: connections, catalogs, stores, external entries, work that spans Sessions | Provider, Mcp, SkillsCatalog, SessionStore, Transport |
| Session | Configuration and capabilities shared by a Session's branches: tools and prompt sources offered to the Session, observation of the Session's events, the Session's own data | ToolRegistry, Skills, WorkspaceTools, TimelineTree |
| Branch | Facts written to the branch Timeline, ordered consumption of branch events, work bound to a turn | Loop, InputQueue, Approval, BackgroundExec |

A feature is split across scopes only where its parts genuinely have different owners, and every
part has exactly one scope. A feature with no state shared across Sessions is not a process plugin.

### D2. No wiring modules

No plugin exists only to connect, aggregate or adapt other plugins (registries, ports, access
hosts, aggregation or adapter plugins). Plugins integrate through existing mechanisms at their
own scope:

| Need | Mechanism |
| --- | --- |
| Offer tools to a Session | Session-scoped `ToolContribution` or `SessionToolSource` |
| Offer instructions | PromptRegistry prompt sources |
| Observe a Session's events | the existing Session broadcast, from a Session-scoped plugin |
| Observe and act on a branch's events in order | the branch Dispatcher, from a branch-scoped plugin |
| Put content into the model context | Session or branch input (adopted inputs), tool results |
| Check a tool call before execution | a branch-scoped admission check contributed to Approval |
| Expose a client method about a Session | a Session-scoped method contribution that Rpc dispatches through the Session |
| Expose a host management method | a process plugin's method in Rpc's method table, marked management level; each entry serves the levels it is configured for |
| Reach other Sessions | Agents (AgentIndex) and the Session public interface, as Collaboration does |
| React to another process plugin's outcomes | an event notice, not a dependency |

### D3. Dependencies follow scope; rare restarts are accepted

A plugin depends on plugins at its own scope or an enclosing scope. Because reloads are rare and
structural, a reload propagating to dependents (and failing in-flight turns) is an accepted cost
of those operations; no mechanism is added to prevent it. Crash isolation of volatile runtimes,
if needed later, belongs to rutis (issue #6), not to application wiring.

### D4. Structural configuration and business configuration are separate

| Kind | Examples | Location | On change |
| --- | --- | --- | --- |
| Structural plugin configuration | enable or disable, runtime and package paths, node binding, listener addresses | loader rows (issue #3 editable layer) | may reload the row |
| Business configuration | search services, notification channels, hook rules, scheduled jobs, workflow definitions, model purpose policies, MCP server and Skill documents, external agent entries, exec shell | the owning plugin's business source (Settings source or the plugin's own store) | data update plus notification; read at use time; no reload |

## Assignment of the new plugins

| Plugin | Scope | Reason |
| --- | --- | --- |
| Questions | Session (`ask_user` tool) + branch (pending questions, `question/*` facts) | the question belongs to a branch's turn |
| Goal | Session (`goal_update` tool) + branch (goal state, continuation) | a goal belongs to a branch and continuation follows its events in order |
| ShellInput | Session | commands run in the Session's workspace and feed its input |
| Attachments | Session | attachments are Session data |
| Web | Session | no state shared across Sessions; it only offers tools |
| SessionTitle | Session | observes the Session's first input; the title is stored by SessionStore |
| Notifications | Session | observes the Session's events and offers `notify` |
| Hooks | Session (rules, Session-start hooks) + branch (pre-tool, post-tool and turn-end hooks; admission check to Approval) | rules apply to a Session; execution happens in a branch |
| Scheduler | process (jobs, triggers) + Session (`schedule` tool) | schedules span Sessions |
| Workflow | process (runs) + Session (`workflow` tool) | a run spans Sessions |
| ExternalAgent | process | external agent connections are shared |
| Modality | process | tasks and provider connections are shared |
| PluginManager | process | installation is per host |
| RemoteGateway | process | an external entry |
| AcpGateway | CLI process | a client |

Management is not a plugin: management methods live in Rpc's method table with a level, and
entries serve levels.

## Withdrawn

The extension registry contract, registration ports and their host plugins, the Session access
host, the process-level Session event subscription, the process-level tool contribution, admission
records (`RequiredCheck`) and the Management plugin are withdrawn.

## Existing plugins

Existing plugins already follow D1. One configuration item does not follow D4: the exec shell is
read once in HostPlatform's `apply`; it should be business configuration read at each spawn.

## Consequences

- The design documents are revised to this assignment: the overview, the contracts (only those
  describing supplements of existing plugins remain) and every plugin document.
- Optional Session and branch plugins are installed only when enabled; they are not placeholders
  in every assembly.
- Rare structural operations may fail in-flight turns; this is documented, not prevented.
