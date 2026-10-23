# ToolRegistry

Owner: [tool_registry.rs](../../../../crates/dim-agent-capabilities/src/tool_registry.rs).
Combines configured host tools and named contributions into the Session tool
registry, exposing snapshots and rutis change events for branch adoption and pinning.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:ToolHost` | Host definitions and execution handlers. |
| `S:SessionConfig` | Select enabled fixed tool names. |
| `S:SessionIdentity` | Supply the owning Session id in a foreign tool's neutral call and the `tool.attach` span. |
| `S:SessionLifecycle` | `activated()` starts the background refresh task once the Session is published; `closing()` ends it. |

## Events and lifetime

No Timeline events. The fixed table holds the configured host tools in
configuration order. Each configured contributed tool is a registry-owned
versioned set keyed `#contributed:<tool>` (#241), read by registered name from the
Session scope without an injected dependency on the optional provider. Registered
source names must match `[A-Za-z0-9_-]{1,64}`, so their keys never meet the
registry's own `#contributed` keys; another name fails the Session. The
registry may not read undeclared services while it applies, so contributors are
first read by ToolBinding's first snapshot during branch initialization, even
without an initial model configuration. Once the Session is published
(`SessionLifecycle::activated`), the registry's own task rereads them whenever a provider enters or leaves Active (`FiberStatusChanged`)
or a service binding changes (`ServiceChanged`); the listeners only wake that
task. A foreign generation's `declaration()` is a synchronous call into the
provider's runtime, which can wait forever on a runtime that is stopping or
restarting. Only reads that serve a request or a branch's first request make
it: `snapshot()` (ToolBinding's initialization and `bind_current`),
`definitions()` (PrepareModelHost) and `attach_pending()`, which `bind` calls
first. They read changed foreign declarations before taking any registry lock
and commit a result only if that generation is still the contributor. The
registry's task, the `tools.provider_changed` listener and ToolBinding's
background adoption (`current()`) never call into a foreign provider: they
withdraw a replaced foreign set, record the new generation as absent and
attach native contributions. So the next request offers a new foreign
generation; a branch whose configuration has a tool part attaches it at its
next request and adopts it for the request after. The task ends when the Session starts closing (`SessionLifecycle::closing`),
so a provider retiring during the close withdraws nothing and emits no
`tools.changed`; unloading the registry aborts the task and waits for a refresh
already running. A contributor's set version changes only when its provider
object changes. Each version is a new UUID, as MCP's are, so it is never reused,
also across registry instances and processes: a version persisted in a branch's
`tool_sources` names one generation, and a restored branch with a tool part
adopts the restored generation under a new version before its first request. A
missing provider, or a foreign generation that is unloading, withdraws the set. Missing or Pending providers produce a `tool.attach`
span in the process log and are offered from the request after they become active, without a
Session reload. Selected duplicate tools and unknown host tools still fail
initialization. Generic source providers emit `tools.provider_changed` on the
Session instance's rutis channel after updating their state. The registry
subscribes before its initial read and rereads the sets of every provider,
contributed ones first. Notifications carry no data because every refresh reads
all providers.
After replacing or withdrawing sets it emits `tools.changed` on that Session's
channel from its own context, independent of the source event's emitter lifetime.
Rejected notification emissions are logged. ToolBinding owns the adoption task.
This local graph has no
SessionToolSource injections. Listeners end with their owning contexts; reload rebuilds the
registry from fresh services. ToolBinding owns adoption and request pins.
Dynamic source validation reserves configured names and every registered
contributor name; only a contributed tool's own set may offer that name.
Already bound requests retain their handlers. Registry refresh is neither a
durable configuration save nor permission authorization; a new tool is not
model-visible until adoption, or until the next `bind_current` request of a
branch whose configuration has no tool part.

Named foreign tools use `toolContribution:<name>` through the existing bridge's
instance key. The read that attaches a generation decodes its synchronous
`declaration()` as `ToolDefinition`, checks identity/risk, and retains that
generation's handler in that generation's set.
The handler also retains the original row context's existing cancellation token,
published under the same tool service name and Session instance as a typed native
key. Row cancellation rejects later executions through that handler and ends its
in-flight native wait; an old set never selects a replacement generation.
TypeScript effects still own settlement of the JavaScript work.
Malformed declarations are omitted with a `tool.attach` span and warning. Execution maps the
native call/context and Session configuration to the neutral A.5 request, then
decodes `ToolResult` once. Each call's cancellation Promise uses a child of the
batch token, so finishing its lease cannot cancel another call in that batch.
Row cancellation ends the affected call's lease without cancelling the batch token.
Before foreign invocation, the model-visible error says the provider generation
is unloading, the call was not made, and the tool is offered again once its
provider is active. If row cancellation ends an issued call's native wait, the
error instead warns that the call may already have taken effect and requires
checking its state before repeating it with the current provider.
The host decides how Sessions unload and restore; App Server recovery is described
in [the process binding](../../../design/process-typescript.md#51-p6p7-access-through-declared-feature-dependencies).

Initial attachment and adoption satisfy
[Session boundary A.5](../../../design/contracts/session-boundary.md#a5-tool-contribution)
through ToolBinding initialization and the existing Prompt / BranchOpening path,
before Session publication and input admission. A branch whose configuration
has no tool part declares the registry's current host and contributed tools
through PrepareModelHost / `bind_current` on each request; a contributed change
needs no adoption there.

## Tests

[flows.rs](../../../../crates/dim-agent-capabilities/tests/flows.rs):
`echo_tools_and_observable_state_follow_the_public_flow`;
[local_product.rs](../../../../tests/integration/tests/local_product.rs)
exercises the configured real-tool registry.
[tool_binding.rs](../../../../tests/integration/tests/tool_binding.rs) covers
immediate first requests, Pending providers, restoration, provider replacement
and request snapshots. A live Session follows a provider's retirement and next
generation without a request, nothing changes once the Session is closing, a
foreign provider replaced in place is read only by a request (no background
reader calls its declaration) and is adopted for the request after, and a
persisted contributed version resolves to nothing after a restore. A queued source notification whose emitter closes still
reaches branch adoption, while another Session keeps its own source version.
The real Node/Rpc flows in
[session_typescript.rs](../../../../crates/dim-app-server/tests/transport_flow/session_typescript.rs)
cover foreign attachment, restore, Pending/disabled parts, a part that becomes active later, malformed declarations
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
| §4 rule 8 | No infrastructure change: collection uses existing scoped lookup, rutis lifecycle events, `SessionLifecycle`, ToolBinding initialization and BranchOpening. |
| §4 rule 9 | Registry availability stays in memory and process logs, not new Timeline facts. |
| §1 | ToolRegistry owns attachment and rutis subscriptions; App Server remains assembly only. |
| ADR 002 D1 | The registry and contributions belong to one Session. |
| ADR 002 D2 | No new registry, registration port or adapter plugin is added. |
| ADR 002 D3 | Optional contributors are not hard dependencies and are followed as they activate and retire; existing required services retain Pending semantics. |
| ADR 002 D4 | Business configuration is not turned into loader reloads. |
| ADR 002 D5 | The named tool contract remains language-neutral; ToolContribution is its native Rust binding. |

## Requirements raised to mechanism owners

None. Declared optional dependencies were rejected by the mechanism owner in
[the roadmap decision](https://github.com/arcships/dim/issues/114#issuecomment-6190712696).
Following contributors is owned by ToolRegistry with existing rutis lifecycle
events; branch initialization and adoption use existing ToolBinding and Prompt
contracts.

<!-- generated:plugin ToolRegistry — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:SessionConfig`、`S:SessionIdentity`、`S:SessionLifecycle`、`S:ToolHost` |
| 提供 | `S:ToolRegistry`、`S:ToolSourcesView` |
| 被注入于 | `PrepareModelHost`、`ToolBinding` |
<!-- /generated -->
