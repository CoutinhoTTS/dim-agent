# Wait

Owner: [wait.rs](../../../../crates/dim-agent-capabilities/src/wait.rs).
Contributes the Session wait tool, resolving the caller's branch-local Wait
service to await a matching task notice, user steer, timeout or interruption.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:TreeCommands` | Resolve the calling branch and its Wait service per call. |

## Events and lifetime

No direct event production/subscription; the outcome is a tool result.
WaitBranch records arrivals. Futures use caller cancellation and release the
single-wait slot on exit/drop. Default timeout is 300,000 ms. Generation changes
can make per-call branch lookup fail as reloading; this contribution itself has
no tracker or disposer. It is not a process join primitive: cancelled background
tasks emit no wake and old arrivals are not replayed after WaitBranch reload.

## Tests

[local_execution.rs](../../../../tests/integration/tests/local_execution.rs):
`wait_ends_on_the_matching_completion_not_on_the_input_that_started_the_turn`,
`wait_ends_on_a_steer_or_a_stop_and_refuses_a_second_wait`.

<!-- generated:plugin Wait — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:TreeCommands` |
| 提供 | `S:ToolContribution#wait` |
| 被注入于 | — |
<!-- /generated -->
