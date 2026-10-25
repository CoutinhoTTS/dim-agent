# WaitBranch

Owner: [wait.rs](../../../../crates/dim-agent-capabilities/src/wait.rs).
Provides branch-local wait state recording eligible arrivals during an open turn.

## Dependencies

| Service | Purpose |
| --- | --- |
| `B:Dispatcher` | Replay whether a turn is open, then consume arrivals and turn boundaries. |

## Events and lifetime

Consumes `input/message`, `turn/start`, `turn/end`; produces no events directly.
Tagged background notices and steer inputs can end a wait; the input opening a
turn does not. Turn end clears pending arrivals. It registers with
`Dispatcher::follow`. Reload rebuilds only open-turn status, not old arrival
notifications. Wait futures are caller-owned and use caller cancellation; the
plugin does not spawn tasks. Only one wait can run per branch.

## Tests

[local_execution.rs](../../../../tests/integration/tests/local_execution.rs):
`wait_ends_on_the_matching_completion_not_on_the_input_that_started_the_turn`,
`wait_ends_on_a_steer_or_a_stop_and_refuses_a_second_wait`.

<!-- generated:plugin WaitBranch — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:Dispatcher` |
| 提供 | `B:Wait` |
| 被注入于 | — |
<!-- /generated -->
