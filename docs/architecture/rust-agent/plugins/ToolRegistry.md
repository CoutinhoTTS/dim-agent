# ToolRegistry

Owner: [tool_registry.rs](../../../../crates/dim-agent-capabilities/src/tool_registry.rs).
Combines configured host tools and named contributions into the Session tool
registry, exposing snapshots/change watches for branch adoption and pinning.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:ToolHost` | Host definitions and execution handlers. |
| `S:SessionConfig` | Select enabled fixed tool names. |
| `S:SessionLifecycle` | Attach optional contributions only after Session publication. |
| `S:Diagnostics` | Report configured optional tools that are unavailable at attachment. |

## Events and lifetime

No Timeline events. After `SessionLifecycle::activated()`, contributions are read
once by registered name from the Session scope, without an injected dependency.
Missing or Pending providers omit only their tools and produce a `tool.attach`
diagnostic. Later providers attach on the next registry reload. Selected duplicate
tools and unknown host tools still fail initialization. Generic source
providers can have tracked change watchers, but this local graph has no
SessionToolSource injections. Cleanup joins watchers; reload rebuilds the
registry from fresh services. ToolBinding owns adoption and request pins.
Configured host tools form an immutable table available during assembly.
Publication attachment builds the complete fixed table in configuration order
and publishes it once per registry generation through `OnceLock`. Fixed lookups
read the host table until attachment completes, then the complete immutable table,
without acquiring a mutex or waiting for attachment. Dynamic source sets retain their
existing synchronization. Publication attachment notifies the
existing ToolBinding watch; Prompt adopts the new tool declarations before they
enter a later request prefix. Already bound requests retain their handlers.
Registry refresh is neither a durable configuration save nor permission
authorization; a new tool is not model-visible until adoption.

## Tests

[flows.rs](../../../../crates/dim-agent-capabilities/tests/flows.rs):
`echo_tools_and_observable_state_follow_the_public_flow`;
[local_product.rs](../../../../tests/integration/tests/local_product.rs)
exercises the configured real-tool registry.

## Rule check

| Rule | Contract |
| --- | --- |
| §4 rule 1 | No state owned by another capability is written. |
| §4 rule 2 | ToolBinding keeps accepted request snapshots; attachment never rebinds them. |
| §4 rule 3 | No before callback is added. |
| §4 rule 4 | Timeline and Dispatcher have no tool-provider-specific behavior. |
| §4 rule 5 | Registry attachment is not an event consumer; existing producers own atomic batches. |
| §4 rule 6 | Declarations reach model context through Prompt adoption; results use ToolExecutor. |
| §4 rule 7 | Contributions use the existing public scoped service; private provider state is not read. |
| §4 rule 8 | No infrastructure change: background work uses SessionLifecycle and existing scoped lookup after publication. |
| §4 rule 9 | Registry availability stays in memory and diagnostics, not new Timeline facts. |
| §1 | ToolRegistry owns attachment, its watches and cleanup; App Server remains assembly only. |
| ADR 002 D1 | The registry and contributions belong to one Session. |
| ADR 002 D2 | No new registry, registration port or adapter plugin is added. |
| ADR 002 D3 | Optional fixed contributors are not hard dependencies; existing required services retain Pending semantics. |
| ADR 002 D4 | Business configuration is not turned into loader reloads. |
| ADR 002 D5 | The named tool contract remains language-neutral; ToolContribution is its native Rust binding. |

## Requirements raised to mechanism owners

None. Apply-time optional dependency declarations remain separate work in issue #6.

<!-- generated:plugin ToolRegistry — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:Diagnostics`、`S:SessionConfig`、`S:SessionLifecycle`、`S:ToolHost` |
| 提供 | `S:ToolRegistry`、`S:ToolSourcesView` |
| 被注入于 | `PrepareModelHost`、`ToolBinding` |
<!-- /generated -->
