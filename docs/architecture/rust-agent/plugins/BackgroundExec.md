# BackgroundExec

Owner: [workspace_tools/background.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/background.rs).
Owns commands transferred from foreground execution, their terminal records and
bounded completion notices on the calling branch.

## Dependencies

| Service | Purpose |
| --- | --- |
| `B:Dispatcher` | Consume command ends and interrupts in dispatch order. |
| `B:Timeline` | Commit ends/notices and recover unreported ends. |
| `S:SessionLifecycle` | `hold()`: each running task holds the Session, so an idle unload waits for it. |

## Events and lifetime

Produces `exec/ended` and tagged `input/message` wake notices; consumes
`exec/ended` and `stop/interrupt`. A turn stop leaves background commands running;
a branch close cancels them without a wake. Explicit task stop, a reload of this
capability and a Session close cancel and release process trees before
settlement; an idle unload is refused while a task runs. Each task holds its
Session from `start`, before `exec/started`, until its supervisor has appended
`exec/ended`; that end keeps the branch busy until its wake is posted. A start
that gets no hold (the Session is closing) is refused as in a closing branch,
and a refused start stops its command. Tracked tasks are joined on unload.
Replay reposts unreported non-cancelled ends but never restarts a historical
process. The notice is not the full command output; output files carry large
results. An end dispatched after branch-close interruption must not wake it.

Each live task retains its existing bounded output capture until the supervisor's
terminal append attempt settles, including on failure. `output(task_id)` reads
that preview after releasing the task-table guard. Ended and replayed entries
have no capture; a historical path never reconstructs one. WorkspaceTools exposes
this finite read through its existing Session methods contribution.

## Tests

[local_execution.rs](../../../../tests/integration/tests/local_execution.rs):
`background_completion_is_recorded_once_and_wakes_with_a_bounded_notice`,
`a_turn_stop_keeps_background_work_and_its_wake_starts_a_turn`,
`idle_unload_waits_for_tasks_and_a_restart_wakes_for_every_unreported_end`,
`running_tasks_hold_the_session_until_the_last_one_ends`,
`a_reload_or_a_closing_session_refuses_a_start_and_leaves_no_hold`.

<!-- generated:plugin BackgroundExec — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:Dispatcher`、`B:Timeline`、`S:SessionLifecycle` |
| 提供 | `B:BackgroundExec` |
| 被注入于 | — |
<!-- /generated -->
