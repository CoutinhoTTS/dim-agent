# Process-level tool contribution

Status: Draft. Overall design: [overview §3 method A](../overview.md#3-integration-methods), [§7.1 ToolRegistry supplement](../overview.md#71-branch-and-session-capabilities). Phase: P0. Language: Rust (ToolRegistry side); a contributor may be implemented in any language that provides the service contract.

ToolRegistry defines a generic extension point through which a process plugin offers model tools to every Session. ToolRegistry injects contributors directly in `Deps`, filters their tools per Session by generic attributes the contributor declares, binds Session values to each handler, and treats the result as an existing dynamic tool source (`ToolSourceSet`). Adoption, pinning, execution, admission and reload therefore reuse the implemented contract of [tool contributions §7](../../architecture/rust-agent/tool-contributions.md). ToolRegistry knows no specific tool or contributor; tool semantics stay inside each contributor.

Not covered: Session-scoped contributions that must resolve the calling branch's service per call (precedent [Wait](../../architecture/rust-agent/plugins/Wait.md)); they keep using `ToolContribution`.

## 1. User flows

1. A contributing process plugin is enabled. Every Session assembled afterwards injects it; ToolRegistry keeps the tools whose declared requirements match the Session, and the Session's branches adopt them into `context/configured` before their next request.
2. The model calls one of these tools. ToolExecutor resolves the pinned handler, Approval admits the call by its declared risk category, the contributor's handler runs with the Session values and the per-call context, and the result returns as `step/tool-result`.
3. The contributor changes its tool set (configuration change, credential removed). It publishes a new set version; branches adopt it at the next opportunity; calls already bound keep the old handlers.
4. The user interrupts the turn while a handler runs. The handler observes cancellation, releases its resources and returns; ToolExecutor writes the result and Loop ends the turn as cancelled.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Contributor service type, key and registration name | Tool business logic (each contributor's document) |
| Generic per-Session requirement attributes and their matching | Approval/Permission decisions (Permission owns the category list) |
| Session values and per-call context passed to handlers | Session-scoped contributions (`ToolContribution`, `SessionToolSource`) |
| Version, pinning, cancellation and reload rules for contributed sets | Work a contributor continues after its call returns (its own document) |

## 3. Scope and dependencies

**Contracts defined here** (owner: ToolRegistry): `ProcessToolSource`, `ProcessToolProvider`, `ProcessToolSet`, `ProcessTool`, `ToolRequirements`, `ProcessToolHandler`, `ProcessToolSession`. **Contracts depended on**: the existing tool source contract (`ToolSourceSet`, `ToolContribution`, `ToolHandler`, `ToolExecutionContext`), ToolBinding's adoption and pinning, ToolExecutor's settlement, the `ToolDefinition.risk` category contract read by Permission. Contributors depend on this contract, never on ToolRegistry internals; ToolRegistry depends on the contract, never on a contributor. Any plugin that provides `ProcessToolSource` under a registered name can replace another.

| Part | Scope | Injects | Provides | Status |
| --- | --- | --- | --- | --- |
| Contributor | Process | its own dependencies | `ProcessToolSource` under its registered name | Proposed per contributor |
| `ToolRegistryPlugin` | Session | existing `S:ToolHost`, `S:SessionConfig`, `Contributions<ToolContribution>`, `Contributions<SessionToolSource>`; added `S:SessionIdentity`, `When<ProcessContributions<ProcessToolSource>>` | `S:ToolRegistry`, `S:ToolSourcesView` (unchanged) | Supplement |
| ToolBinding, ToolExecutor, Approval, Permission | Branch | unchanged | unchanged | Existing |

Assembly: `AgentBuilder::process_tool_source(name)` (proposed, next to the existing `session_tool_source`) records the name in `ToolNames` (new field `process_sources`). The ToolRegistry factory builds `When<ProcessContributions<ProcessToolSource>>` as `Some` with those names when the list is non-empty, `None` otherwise; the choice is fixed when the Session's plugin is built ([AGENTS rule 1](../../../AGENTS.md)). `ProcessContributions<T>` (proposed in `dim-agent-runtime`, mirroring `Contributions<T>` in `capability.rs`) injects process keys `TypeKey::keyed_dynamic::<T>(name)` without an instance. The host registers a name only while the contributing plugin row is configured, so a disabled contributor adds no dependency to new Sessions and its tools are simply absent (A1). A registered name must not equal a `session_tool_source` name or a fixed tool name; `AgentBuilder` rejects the duplicate.

Lifecycle follows the dependency: while a contributor is Pending or reloading, every injecting ToolRegistry (and, through the existing chain, ToolBinding, Loop and ToolExecutor of each branch) is Pending or reloads per the [capability reload protocol](../../architecture/rust-agent/single-tree.md). Contributors therefore express ordinary changes (configuration, credentials, external outages) as set versions or revocation, never as a plugin reload (A8).

## 4. Events

None. Contributed set changes are Session-internal registry state; branches record what they adopted in the existing `context/configured` (`tools`, `tool_sources`). Calls and results use the existing `step/tool-call`, `step/tool-start`, `step/tool-completed`, `step/tool-result` and `approval/*`. Nothing about a contributed tool is a fact that cannot be derived from these.

## 5. State

| State | Owner | Rebuild |
| --- | --- | --- |
| Per-Session adapted sets (filtered, handlers bound to `ProcessToolSession`) | ToolRegistry | Recomputed from the contributor's current `sets()` on every refresh and `apply` |
| Current and replaced sets, held versions, `ToolSourcesState` | ToolRegistry (existing `Sources`) | Not persisted |
| Pins and request snapshots | ToolBinding (existing) | Pins from the adopted `context/configured` in the prefix; snapshots are runtime only |
| Contributor business state | Contributor, process storage | Contributor's document |

## 6. Interfaces

```rust
/// Process service; key `TypeKey::keyed_dynamic::<ProcessToolSource>(name)`, no instance.
pub struct ProcessToolSource(pub Arc<dyn ProcessToolProvider>);

pub trait ProcessToolProvider: Send + Sync + 'static {
    /// Every complete set currently offered, one per key; same rules as `ToolProvider::sets`.
    fn sets(&self) -> Vec<Arc<ProcessToolSet>>;
    /// Signals that `sets()` changed; carries no data and may coalesce.
    fn changed(&self) -> tokio::sync::watch::Receiver<()>;
}

pub struct ProcessToolSet {
    pub key: String,      // "<registered name>" or "<registered name>:<instance>"
    pub version: String,  // never reused for this key, across generations and restarts
    pub tools: Vec<ProcessTool>,
}
pub struct ProcessTool {
    pub definition: ToolDefinition,          // `risk` required
    pub requires: ToolRequirements,
    pub handler: Arc<dyn ProcessToolHandler>,
}
/// Generic attributes matched against the Session configuration (A2).
pub struct ToolRequirements {
    pub interactive: bool,                              // offered only to interactive Sessions
    pub roles: Option<BTreeSet<CollaborationRole>>,     // None = any role
}
pub trait ProcessToolHandler: Send + Sync + 'static {
    fn execute(&self, session: &ProcessToolSession, call: ToolCall,
        context: ToolExecutionContext, cancel: CancellationToken) -> HostFuture<'_, ToolResult>;
}
/// Session values ToolRegistry passes; the contributor never reads Session state.
pub struct ProcessToolSession {
    pub session_id: String,                 // SessionIdentity.session_id
    pub cwd: String,                        // SessionConfig.cwd
    pub collaboration_role: CollaborationRole,
    pub interactive: bool,                  // proposed SessionConfig field (overview §7.2 Agents)
    pub resource_node: Option<String>,      // proposed SessionConfig field (#5); None = control node
}
```

Per-Session adaptation by ToolRegistry, per contributed set and refresh: keep each tool whose `requires` matches the Session (`interactive` requires `ProcessToolSession.interactive`; `roles` contains the Session's role), wrap its handler as a `ToolHandler` that calls `ProcessToolHandler::execute` with the Session's `ProcessToolSession`, and produce a `ToolSourceSet { key, version, tools }` with the contributor's key and version. The Session configuration is fixed after creation, so the filtered result of one version never changes within a Session. A set whose tools are all filtered out is treated as revoked for that Session. ToolRegistry never matches tool names.

| Item | Rule |
| --- | --- |
| Version | Never reused for a key; include a per-generation random id and a counter (A4) |
| Definitions | Parsed at the contributor's external boundary; `definition.name` matches `[A-Za-z0-9_-]{1,64}`; `source` is `ToolSource::Extension` (A5) |
| Risk | `definition.risk` is required; ToolRegistry rejects a set containing a tool without it, through the existing rejection path (A3) |
| Names | Unique within the set and not equal to fixed tools or other keys' current tools (existing validation) |

Per-call context, all from existing values:

| Value | Source |
| --- | --- |
| Session id, cwd, role, interactive, resource node | `ProcessToolSession` |
| Branch id, turn id, request id | `ToolExecutionContext { branch_id, turn_id, request_id }` from ToolExecutor |
| Call id, tool name, input | `ToolCall { id, name, input }` |
| Cancellation | The `CancellationToken` ToolExecutor passes (interrupt, branch close, reload) |

A handler that needs Session facts reads them through the [session-access](session-access.md) contract with these ids, and writes model-visible follow-up only through its input. Host resources on the Session's resource node are not reachable through control-node process services; a tool needing them is outside this contract.

Risk categories are opaque strings owned by Permission (`Risk::of` in [permission.rs](../../../crates/dim-agent-capabilities/src/permission.rs)): existing `read_filesystem`, `write_filesystem`, `execute_process`, `network`, `session`; new categories are registered with Permission (overview §7.1), e.g. `outbound_message`, `persistent_configuration` (A6). Until Permission recognizes a string, it is unclassified (denied under read-only, asked under workspace-write, allowed under FullAccess).

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Inject and watch | ToolRegistry `apply` resolves each `ProcessToolSource`, subscribes to `changed()` before the first `sets()` read (existing order), and joins watcher tasks in its `ctx.effect` disposer |
| Refresh | Existing `ToolRegistry::refresh` over the adapted sets: validate the whole set, replace atomically or reject (`ToolSourcesState.rejected`), notify `changed()`. No Timeline write |
| Adoption | ToolBinding's task (after `BranchLifecycle::activated()`) sees `changed()` and calls `Prompt::adopt_tools`; Prompt's serial queue publishes `context/configured` through the configuration publisher's `Timeline::append_batch_registered` (the registration callback fails the batch if the adoption was superseded). The adopted sequence becomes a pin |
| Request pinning | Loop calls `ToolBinding::bind(request_id, through, tools, tool_sources)` from the configuration in the prefix; the snapshot holds the `Arc<ToolSourceSet>` of the declared version, so a later change cannot replace a handler of that request |
| Execution | ToolExecutor consumes `step/tool-call` on the Dispatcher, registers the batch task and returns; per call it appends `step/tool-start` (`Timeline::append`), asks Approval (which may write `approval/request` and wait for `approval/respond`), calls the handler, appends `step/tool-completed`, and after all calls appends one `step/tool-result` |
| Settlement | Owned by ToolExecutor: cancellation before execution, reload settlement, panic. The contributor writes nothing to the Timeline for the call |
| State and recheck | No new `StateCell`; existing `StateCell<ToolSourcesState>` (`S:ToolSourcesView`). The contributor makes no Timeline decision; ToolBinding compares current versions with the last adopted configuration it consumed |
| Reload and restart | Version change is not a reload. Contributor reload: rutis reloads every injecting ToolRegistry and its downstream; in-flight batches settle as reload; branches whose prefix versions are unavailable re-adopt in `apply`. Process restart: restored branches re-adopt at `branch/resume` before their first request; committed calls without results are settled by ToolExecutor's rebuild, never re-executed |

Handlers retained by pins or snapshots after their contributor generation unloaded return an explicit stop error (existing rule, [tool contributions §4](../../architecture/rust-agent/tool-contributions.md)).

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| V2 adopted after a request's `step/model-start`, before `bind` | adoption of V2 > model-start | Request binds V1 from its prefix; V1 released after the next successful bind at or after V2 | ToolBinding |
| V2 published while a V1 call executes | — | Call completes on V1; next request declares V2 | ToolBinding snapshot |
| Set revoked mid-call | — | Call completes; next request lacks the tool; a later call by name gets `Unknown tool` | ToolBinding |
| Turn `stop/interrupt` before the batch is dispatched | interrupt < `step/tool-call` consumption | Not executed, `Tool cancelled before execution` | ToolExecutor |
| Turn `stop/interrupt` while the handler runs | interrupt > `step/tool-start` | Token cancelled; handler releases and returns; a truly completed result is kept; turn ends cancelled | ToolExecutor, Loop |
| Interrupt while Approval is pending | interrupt > `approval/request` | `approval/cancelled`; handler never called | Approval |
| Branch-scoped interrupt or branch close | branch interrupt | As turn interrupt; work the contributor continues after returning is not cancelled by this contract (A7) | ToolExecutor; contributor |
| Batch not yet settled | handler returned, others running | Result held until the batch's single `step/tool-result`; its side effects already happened | ToolExecutor |
| Handler writes Session input during the call | `input/message` < `step/tool-result` | Consumed per mode after the batch result (Steer/Append) or after the turn (Queue) | InputQueue, Loop |
| Compaction after the result | `context/compaction` > `step/tool-result` | Result survives only through the summary; declarations stay in configuration | ContextPreparation |
| Child branch creation | cut before `step/model-start` | Child inherits the adopted configuration; its ToolBinding pins or re-adopts; never inherits snapshots | ToolBinding (child) |
| Contributor reload mid-call | — | Batch settled as reload; turn `Failed` | ToolExecutor, Loop |
| Process restart mid-call | `step/tool-call` without result | Settled as not surviving recovery; not re-executed | ToolExecutor |
| Session unload and restore | `branch/resume` | Re-inject; new versions adopted before the first request | ToolRegistry, ToolBinding |
| Contributor Pending | — | Injecting ToolRegistries Pending; requests and RPC get `service_reloading` | rutis lifecycle |
| Handler returns after its token was cancelled | — | Exactly one result per call | ToolExecutor |
| Client detach | — | No effect on calls | — |

## 9. Context mapping

| What the model sees | Form and position |
| --- | --- |
| Tool name, description, input schema | Declarations in the adopted `context/configured.tools`, effective for requests whose prefix contains that configuration; fixed tools first, then source tools by key |
| Call outcome | `ToolResult.content` at the tool result position of its batch; `structured_content` is not model text |
| Follow-up after the call | Only input written through session-access, placed where Loop consumes it |

Declarations are configuration, not history, so compaction keeps the latest adopted set; results survive compaction only through the summary. Child branches inherit the configuration adopted in their prefix and the results before the cut. Non-interactive Sessions never see tools declaring `interactive: true`. The model never sees requirement attributes, risk categories, set keys or versions, the per-call context, diagnostics, or other plugins' events.

## 10. Errors and recovery

| Case | Result |
| --- | --- |
| Name conflict, invalid name, missing `risk`, reused version | Whole set rejected; previous set kept; `ToolSourcesState.rejected`; diagnostics |
| Registered name without a provider | ToolRegistry Pending; Session creation fails at the startup deadline like any missing dependency |
| Handler error | Error `ToolResult` for that call only |
| Handler ignores cancellation | Turn completion waits for it (stop waits for real settlement); reported in diagnostics |
| Adoption failure | Pins and prefix kept; retried on the next change (existing) |

## 11. Security

Contributed tools are admitted per call by Approval and Permission through their declared risk; ToolExecutor calls Approval before every handler, so a contributor cannot bypass admission. Contributors are trusted in-process code; `Deps` is not a sandbox (AGENTS rule 11). Credentials stay inside the contributor as references; they never appear in definitions, results or diagnostics. A contributor exposing project-sourced executable content checks host-local approval on every call, not at refresh time (A9); replay never restores it.

## 12. Acceptance

Flow tests in `tests/integration` with a test contributor (controllable versions, recorded calls, observable `Drop`):

1. A registered contributor's tools appear in declarations of a new Session; a call reaches its handler with the Session id, branch id, call id and request ids.
2. A tool with `interactive: true` is absent in a non-interactive Session; a `roles` restriction is honored.
3. Version change during a pinned request and during a running call; revocation.
4. Interrupt before dispatch, during the handler and during approval; one result per call.
5. Handler-written Steer input is consumed after the batch result.
6. Missing `risk` rejects the set and keeps the previous one; an unknown risk string is unclassified.
7. Contributor reload settles in-flight batches; restart settles committed calls without re-execution; restored branches re-adopt before the first request.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Whether a Session injects a contributor is fixed by assembly registration when the Session is built; disabling the contributor affects only later Sessions | Overview §7.1 ("enabled by configuration"); timing is judgment | Runtime enable/disable needs a stable always-present provider |
| A2 | Per-Session filtering attributes are `interactive` and collaboration `roles` | Overview §5.1 (non-interactive Sessions omit interactive tools); role is judgment | Attribute set changes; matching stays generic |
| A3 | Every contributed tool must declare `risk`; otherwise the set is rejected | Overview §3 constraint 7; rejection policy is judgment | Undeclared tools fall back to unclassified handling |
| A4 | Versions combine a per-generation random id and a counter | Judgment from the "never reused" rule | Any scheme never reused across generations and restarts |
| A5 | Contributed tools use `ToolSource::Extension` | Judgment | Display only |
| A6 | Example category strings `outbound_message`, `persistent_configuration` | Overview §5.8, §5.11, §7.1 names; strings are judgment | Renamed in Permission and contributors together |
| A7 | Interrupting the calling branch never cancels work a contributor continues after its call returned | Overview §5.9, generalized by judgment | Each contributor defines interrupt propagation itself |
| A8 | Ordinary contributor changes are versions, never plugin reloads | Judgment from the reload protocol | Such changes would fail running turns in all Sessions |
| A9 | Approval of project-sourced executable tool content is rechecked per call | Overview §9 Trust; timing follows the MCP precedent (judgment) | Revocation would not stop calls through retained bindings |

## 14. Open questions

| Question | Description |
| --- | --- |
| Session opt-out | Whether `SessionConfig` or the Agent profile may exclude a whole contributor, beyond requirement attributes |
| Disabling at runtime | A contributor disabled through plugin management leaves existing Sessions Pending until they unload, unless the loader keeps a stable empty provider |
| Reload blast radius | A contributor reload fails running turns in every Session through the ToolRegistry chain |
| Overview §7.1 Permission | `network` already exists in `permission.rs`; only the remaining categories are new |
