# ToolRegistry

Owner: [tool_registry.rs](../../../../crates/dim-agent-capabilities/src/tool_registry.rs).
Combines configured host tools and named contributions into the Session tool
registry, exposing snapshots and rutis change events for branch adoption and pinning.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:ToolHost` | Host definitions and execution handlers. |
| `S:SessionConfig` | Select enabled fixed tool names. |
| `S:SessionIdentity` | Supply the owning Session id in a foreign tool's neutral call. |
| `S:Diagnostics` | Report configured optional tools that are unavailable at attachment. |

## Events and lifetime

No Timeline events. The first ToolBinding reconciliation during branch
initialization reads the complete fixed table synchronously, even without an
initial model configuration. Contributions are read once by registered name from
the Session scope, after required Session services are Active and without an
injected dependency on the optional providers.
Missing or Pending providers omit only their tools and produce a `tool.attach`
diagnostic. Later providers attach on the next registry reload. Selected duplicate
tools and unknown host tools still fail initialization. Generic source
providers emit `tools.provider_changed` on the Session instance's
rutis channel after updating their state. The registry subscribes before its
initial read and rereads the sets of its registered providers. Notifications
carry no data because every refresh reads all those providers.
After replacing or withdrawing sets it emits `tools.changed` on that Session's
channel from its own context, independent of the source event's emitter lifetime.
Rejected notification emissions are logged. ToolBinding owns the adoption task.
This local graph has no
SessionToolSource injections. Listeners end with their owning contexts; reload rebuilds the
registry from fresh services. ToolBinding owns adoption and request pins.
The standard library's `LazyLock` builds the complete fixed table in configuration
order once per registry generation. It coordinates initialization; subsequent
fixed lookups read the immutable table without a per-read mutex. A provider that
becomes Active after this first read, including before Session publication, waits
for the next registry or Session reload. Dynamic source validation reserves
configured fixed names and every registered contributor name without triggering
table initialization. Dynamic source sets retain their existing synchronization
and change notifications. Already bound requests retain their handlers.
Registry refresh is neither a durable configuration save nor permission
authorization; a new tool is not model-visible until adoption.

Named foreign tools use `toolContribution:<name>` through the existing bridge's
instance key. The same first read decodes their synchronous `declaration()` as
`ToolDefinition`, checks identity/risk, and retains that generation's handler.
The handler also retains the original row context's existing cancellation token,
published under the same tool service name and Session instance as a typed native
key. Row cancellation rejects later executions through that handler and ends its
in-flight native wait; the old table never selects a replacement generation.
TypeScript effects still own settlement of the JavaScript work.
Malformed declarations are omitted with an attach diagnostic. Execution maps the
native call/context and Session configuration to the neutral A.5 request, then
decodes `ToolResult` once. Each call's cancellation Promise uses a child of the
batch token, so finishing its lease cannot cancel another call in that batch.
Row cancellation ends the affected call's lease without cancelling the batch token.
The model-visible error says the provider generation is unloading, the tool stays
unavailable until the Session reloads, and retrying in that Session will not help.
The host decides how Sessions unload and restore; App Server recovery is described
in [the process binding](../../../design/process-typescript.md#51-p6p7-access-through-declared-feature-dependencies).

Initial attachment and adoption satisfy
[Session boundary A.5](../../../design/contracts/session-boundary.md#a5-tool-contribution)
through ToolBinding initialization and the existing Prompt / BranchOpening path,
before Session publication and input admission. A branch without initial model
configuration uses the same fixed table through PrepareModelHost / `bind_current`.

## Tests

[flows.rs](../../../../crates/dim-agent-capabilities/tests/flows.rs):
`echo_tools_and_observable_state_follow_the_public_flow`;
[local_product.rs](../../../../tests/integration/tests/local_product.rs)
exercises the configured real-tool registry.
[tool_binding.rs](../../../../tests/integration/tests/tool_binding.rs) covers
immediate first requests, Pending providers, restoration, provider replacement
and request snapshots. A queued source notification whose emitter closes still
reaches branch adoption, while another Session keeps its own source version.
The real Node/Rpc flows in
[session_typescript.rs](../../../../crates/dim-app-server/tests/transport_flow/session_typescript.rs)
cover the foreign first-read boundary, restore, Pending/disabled parts, malformed declarations
and results, and cancellation without cancelling sibling tools in the same batch.

## Rule check

| Rule | Contract |
| --- | --- |
| §4 rule 1 | No state owned by another capability is written. |
| §4 rule 2 | ToolBinding keeps accepted request snapshots; attachment never rebinds them. |
| §4 rule 3 | No before callback is added. |
| §4 rule 4 | Timeline and Dispatcher have no tool-provider-specific behavior. |
| §4 rule 5 | Memory notifications add no Timeline batches; existing producers retain atomic commit ownership. |
| §4 rule 6 | Declarations reach model context through Prompt adoption; results use ToolExecutor. |
| §4 rule 7 | Contributions use the existing public scoped service; private provider state is not read. |
| §4 rule 8 | No infrastructure change: first-read collection uses existing scoped lookup, ToolBinding initialization and BranchOpening. |
| §4 rule 9 | Registry availability stays in memory and diagnostics, not new Timeline facts. |
| §1 | ToolRegistry owns attachment and rutis subscriptions; App Server remains assembly only. |
| ADR 002 D1 | The registry and contributions belong to one Session. |
| ADR 002 D2 | No new registry, registration port or adapter plugin is added. |
| ADR 002 D3 | Optional fixed contributors are not hard dependencies; existing required services retain Pending semantics. |
| ADR 002 D4 | Business configuration is not turned into loader reloads. |
| ADR 002 D5 | The named tool contract remains language-neutral; ToolContribution is its native Rust binding. |

## Requirements raised to mechanism owners

None. Declared optional dependencies were rejected by the mechanism owner in
[the roadmap decision](https://github.com/arcships/dim/issues/114#issuecomment-6190712696).
Initial fixed-table collection is owned by ToolRegistry; branch initialization
and adoption use existing ToolBinding and Prompt contracts.

<!-- generated:plugin ToolRegistry — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:Diagnostics`、`S:SessionConfig`、`S:SessionIdentity`、`S:ToolHost` |
| 提供 | `S:ToolRegistry`、`S:ToolSourcesView` |
| 被注入于 | `PrepareModelHost`、`ToolBinding` |
<!-- /generated -->
