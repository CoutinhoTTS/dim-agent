# InputQueue

The InputQueue plugin registers InputMethods in every Session and InputQueue in every branch.

## InputMethods

Owner: [input_queue.rs](../../../../crates/dim-agent-capabilities/src/input_queue.rs).
Registers the namespace `session.input` with the Session's `sessionMethods`:
`session.input.withdraw` for every branch of its Session, served through that
branch's InputQueue.

### Dependencies

| Service | Purpose |
| --- | --- |
| `S:TreeCommands` | Find the branch named by a call and read its InputQueue. |
| `S:SessionMethods` | Register `session.input` from `apply`, until this part unloads. |

### Events and lifetime

Writes no events itself; InputQueue commits `input/withdraw` and its result.
The registration follows the shared Session method admission: an unload
interrupts a call, and a caller's disconnect cancels nothing. While the part
reloads, the withdrawn namespace answers `service_reloading`.

### Tests

[session_methods.rs](../../../../crates/dim-app-server/tests/transport_flow/session_methods.rs)
covers `session.input.withdraw` through Rpc.

<!-- generated:plugin InputMethods — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:SessionMethods`、`S:TreeCommands` |
| 提供 | — |
| 被注入于 | — |
<!-- /generated -->

## InputQueue

Owner: [input_queue.rs](../../../../crates/dim-agent-capabilities/src/input_queue.rs).
Decides when queued, append and steer inputs become applicable and publishes the
observable input queue.

### Dependencies

| Service | Purpose |
| --- | --- |
| `B:Dispatcher` | Consume input/turn/interrupt transitions. |
| `B:Timeline` | Commit application decisions and replay the queue. |

### Events and lifetime

Consumes `input/message`, `input/consumed`, `turn/start`, `turn/end`,
`stop/interrupt`; produces `input/applied`. It does not run a model itself.
Subscription and replay share a sequence gate; restore writes owed application
decisions without duplicating already committed ones. No background tasks or
external resources. Reload reconstructs state from local history. An idle append
is pending until a driving input; an interrupt is not permission to start a new
turn from previously unapplied input.

### Tests

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
