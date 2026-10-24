# ToolBinding

Owner: [tool_binding.rs](../../../../crates/dim-agent-capabilities/src/tool_binding.rs).
Adopts a Session tool set into branch model configuration and pins handlers to
requests so later registry changes cannot silently replace executing tools.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:ToolRegistry` | Current tools and their source-state watch (`sources_state()`). |
| `B:Prompt` | Adopt tool configuration and await initial prompt completion. |
| `B:Timeline` | Reconcile committed configuration/request pins. |
| `B:BranchLifecycle` | Gate background refresh on activation/closing. |
| `B:Dispatcher` | Handle branch opening. |
| `B:BranchOpening` | Register initial tool-adoption completion receipt. |

## Events and lifetime

Consumes `branch/start` and `branch/resume`. Configuration facts are published
through Prompt rather than a separate tool-event route. Opening work and a
adoption task are tracked and joined on cleanup. It takes the registry's
source-state watch before its initial registry read. A forwarding task only wakes
its own adoption task; an update before activation remains pending until the
existing BranchLifecycle opens. The forwarding task ends with its context or the
registry. Reload reconciles the
already adopted configuration. Initialization reads the registry even without
initial model configuration, fixing optional contributors before the first input;
this read and `bind_current` attach changed foreign contributors, and `bind`
attaches them first through `attach_pending()`. The adoption task reads with
`ToolRegistry::current()`, which never calls into a foreign provider, so a
foreign generation that a request attached is adopted for the next request.
With a model configuration, opening reconciles its tools through Prompt before
the existing BranchOpening receipt completes. Failed adoption retains pins and the prefix;
a later change retries. Pins are runtime handles, not persistent executable
identities; missing tools cannot be recovered by replaying their old definitions.

## Rule check

| Rule | Application |
| --- | --- |
| §4 rule 1 | Prompt still owns configuration facts; notifications only request a reread |
| §4 rule 2 | Accepted request snapshots retain their selected handlers |
| §4 rule 3 | No before callback |
| §4 rule 4 | No core or plugin-specific assembly change |
| §4 rule 5 | Prompt retains its existing atomic configuration commit |
| §4 rule 6 | Tool declarations reach the model only through adopted configuration |
| §4 rule 7 | Uses the registry's public snapshot and source-state watch |
| §4 rule 8 | The registry's existing watch, BranchLifecycle and adoption task suffice |
| §4 rule 9 | Change notifications stay outside the Timeline |
| §1, ADR 002 D1 | Adoption and its pending wake belong to this branch capability |
| ADR 002 D2 | No wiring plugin, new bus or shared notification channel |
| ADR 002 D3 | Existing declared dependencies remain unchanged; contributed tools are registry sources (#241), and a configuration without a tool part adopts none of their changes |
| ADR 002 D4 | No configuration or reload policy change |
| ADR 002 D5 | Source changes are read from one Session's registry watch; no wire protocol change |

## Requirements raised to mechanism owners

None. The forwarding task wakes the capability's existing task through a private
`Notify`; it does not hand that primitive to another plugin. The permit retains
updates before branch activation and coalesces subsequent rereads. No additional
task, mutex, lifecycle phase or protocol field is needed.

## Tests

[tool_binding.rs](../../../../tests/integration/tests/tool_binding.rs):
`a_call_runs_the_version_its_request_declared_and_old_versions_are_released`,
`a_restored_branch_adopts_the_new_versions_before_its_first_request`,
`contributors_are_current_for_the_first_request_without_initial_configuration`,
`tool_updates_wait_for_branch_activation_and_do_not_run_after_failed_assembly`,
`a_replaced_foreign_provider_is_read_only_by_a_request`.

<!-- generated:plugin ToolBinding — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:BranchLifecycle`、`B:BranchOpening`、`B:Dispatcher`、`B:Prompt`、`B:Timeline`、`S:ToolRegistry` |
| 提供 | `B:ToolBinding` |
| 被注入于 | `Loop`、`ToolExecutor` |
<!-- /generated -->
