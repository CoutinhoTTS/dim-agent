# Permission

Owner: [permission.rs](../../../../crates/dim-agent-capabilities/src/permission.rs).
Evaluates tool risk and explicit allow/deny policy and exposes versioned
branch-local permission state.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:SessionConfig` | Seed the initial permission policy. |
| `B:Dispatcher` | Apply committed policy changes and settle update waiters. |
| `B:Timeline` | Write changes and replay ancestor/local policies. |

## Events and lifetime

Produces and consumes `permission/changed`. Updates become effective when
dispatched, not merely appended. Replay starts with Session policy, folds the
fixed ancestor prefix and then local changes. No background tasks. Cleanup
closes admission and drops pending waiters, which report Closed. Reload builds
a new state cell from history. Policy is admission logic, not OS isolation;
explicit disallow overrides allow.

## Tests

[permission_flow.rs](../../../../crates/dim-agent-capabilities/tests/permission_flow.rs):
`presets_decide_by_risk_and_denied_calls_do_not_run`,
`tool_lists_override_presets_and_disallowed_wins`,
`updates_are_versioned_per_branch_take_effect_when_dispatched_and_survive_restore`.

<!-- generated:plugin Permission — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:Dispatcher`、`B:Timeline`、`S:SessionConfig` |
| 提供 | `B:Permission`、`B:StateCell<PermissionState>` |
| 被注入于 | `Approval` |
<!-- /generated -->
