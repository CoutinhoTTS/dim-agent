# ToolBinding

Owner: [tool_binding.rs](../../../../crates/dim-agent-capabilities/src/tool_binding.rs).
Adopts a Session tool set into branch model configuration and pins handlers to
requests so later registry changes cannot silently replace executing tools.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:ToolRegistry` | Current tools and registry-change watch. |
| `B:Prompt` | Adopt tool configuration and await initial prompt completion. |
| `B:Timeline` | Reconcile committed configuration/request pins. |
| `B:BranchLifecycle` | Gate background refresh on activation/closing. |
| `B:Dispatcher` | Handle branch opening. |
| `B:BranchOpening` | Register initial tool-adoption completion receipt. |

## Events and lifetime

Consumes `branch/start` and `branch/resume`. Configuration facts are published
through Prompt rather than a separate tool-event route. Opening work and a
registry watcher are tracked and joined on cleanup. Reload reconciles the
already adopted configuration. Initialization reads the registry even without
initial model configuration, fixing optional contributors before the first input.
With a model configuration, opening reconciles its tools through Prompt before
the existing BranchOpening receipt completes. Failed adoption retains pins and the prefix;
a later change retries. Pins are runtime handles, not persistent executable
identities; missing tools cannot be recovered by replaying their old definitions.

## Tests

[tool_binding.rs](../../../../tests/integration/tests/tool_binding.rs):
`a_call_runs_the_version_its_request_declared_and_old_versions_are_released`,
`a_restored_branch_adopts_the_new_versions_before_its_first_request`,
`fixed_contributors_are_sampled_before_the_first_request_without_initial_configuration`,
`tool_updates_wait_for_branch_activation_and_do_not_run_after_failed_assembly`.

<!-- generated:plugin ToolBinding — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:BranchLifecycle`、`B:BranchOpening`、`B:Dispatcher`、`B:Prompt`、`B:Timeline`、`S:ToolRegistry` |
| 提供 | `B:ToolBinding` |
| 被注入于 | `Loop`、`ToolExecutor` |
<!-- /generated -->
