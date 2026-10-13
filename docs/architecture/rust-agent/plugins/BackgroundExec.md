# BackgroundExec

Owner: [workspace_tools/background.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/background.rs).
Owns commands transferred from foreground execution, their terminal records and
bounded completion notices on the calling branch.

## Dependencies

| Service | Purpose |
| --- | --- |
| `B:Dispatcher` | Consume command ends and interrupts in dispatch order. |
| `B:Timeline` | Commit ends/notices and recover unreported ends. |

## Events and lifetime

Produces `exec/ended` and tagged `input/message` wake notices; consumes
`exec/ended` and `stop/interrupt`. A turn stop leaves background commands running;
a branch close cancels them without a wake. Explicit task stop and unload cancel
and release process trees before settlement. Tracked tasks are joined on unload.
Replay reposts unreported non-cancelled ends but never restarts a historical
process. The notice is not the full command output; output files carry large
results. An end dispatched after branch-close interruption must not wake it.

## Tests

[local_execution.rs](../../../../tests/integration/tests/local_execution.rs):
`background_completion_is_recorded_once_and_wakes_with_a_bounded_notice`,
`a_turn_stop_keeps_background_work_and_its_wake_starts_a_turn`,
`idle_unload_stops_tasks_and_a_restart_wakes_for_every_unreported_end`.

<!-- generated:plugin BackgroundExec — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:Dispatcher`、`B:Timeline` |
| 提供 | `B:BackgroundExec` |
| 被注入于 | — |
<!-- /generated -->
