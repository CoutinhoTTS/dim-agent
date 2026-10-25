# InputQueue

Owner: [input_queue.rs](../../../../crates/dim-agent-capabilities/src/input_queue.rs).
Decides when queued, append and steer inputs become applicable and publishes the
observable input queue.

## Dependencies

| Service | Purpose |
| --- | --- |
| `B:Dispatcher` | Replay the queue, then consume input/turn/interrupt transitions. |
| `B:Timeline` | Commit application decisions. |

## Events and lifetime

Consumes `input/message`, `input/consumed`, `turn/start`, `turn/end`,
`stop/interrupt`; produces `input/applied`. It does not run a model itself.
It registers with `Dispatcher::follow`; the replay writes owed application
decisions, without duplicating already committed ones, before any live event. No background tasks or
external resources. Reload reconstructs state from local history. An idle append
is pending until a driving input; an interrupt is not permission to start a new
turn from previously unapplied input.

## Tests

[host_flows.rs](../../../../crates/dim-agent-capabilities/tests/host_flows.rs):
`steer_and_append_enter_the_current_turn_at_tool_result_while_queue_waits`;
[context_flow.rs](../../../../crates/dim-agent-capabilities/tests/context_flow.rs):
`restoring_pending_inputs_runs_each_queued_input_in_its_own_turn`.

<!-- generated:plugin InputQueue — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:Dispatcher`、`B:Timeline` |
| 提供 | `B:InputQueue`、`B:StateCell<InputSnapshot>` |
| 被注入于 | — |
<!-- /generated -->
