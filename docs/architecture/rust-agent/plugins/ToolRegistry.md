# ToolRegistry

Owner: [tool_registry.rs](../../../../crates/dim-agent-capabilities/src/tool_registry.rs).
Combines configured host tools and named contributions into the Session tool
registry, exposing snapshots/change watches for branch adoption and pinning.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:ToolHost` | Host definitions and execution handlers. |
| `S:ToolContribution#branch` | Branch-creation tool. |
| `S:ToolContribution#skill` | Skill-loading tool. |
| `S:ToolContribution#wait` | Wait tool. |
| `S:SessionConfig` | Select enabled fixed tool names. |

## Events and lifetime

No Timeline events. Contributions are resolved by declared names; selected
duplicate tools and missing host tools fail initialization. Generic source
providers can have tracked change watchers, but this local graph has no
SessionToolSource injections. Cleanup joins watchers; reload rebuilds the
registry from fresh services. ToolBinding owns adoption and request pins.
Registry refresh is neither a durable configuration save nor permission
authorization; a new tool is not model-visible until adoption.

## Tests

[flows.rs](../../../../crates/dim-agent-capabilities/tests/flows.rs):
`echo_tools_and_observable_state_follow_the_public_flow`;
[local_product.rs](../../../../tests/integration/tests/local_product.rs)
exercises the configured real-tool registry.

<!-- generated:plugin ToolRegistry — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:SessionConfig`、`S:ToolContribution#branch`、`S:ToolContribution#skill`、`S:ToolContribution#wait`、`S:ToolHost` |
| 提供 | `S:ToolRegistry`、`S:ToolSourcesView` |
| 被注入于 | `PrepareModelHost`、`ToolBinding` |
<!-- /generated -->
