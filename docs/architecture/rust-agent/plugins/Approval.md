# Approval

Owner: [approval.rs](../../../../crates/dim-agent-capabilities/src/approval.rs).
Turns permission decisions into call admission or durable approval requests and
rechecks policy before allowing an approved call to start.

## Dependencies

| Service | Purpose |
| --- | --- |
| `B:Permission` | Evaluate current policy and recheck an allowed response. |
| `B:Dispatcher` | Receive approval responses. |
| `B:Timeline` | Commit requests/cancellations and recover abandoned requests. |

## Events and lifetime

Produces `approval/request` and `approval/cancelled`; consumes `approval/respond`.
Caller cancellation settles its waiter; branch close/reload releases remaining
requests. Recovery cancels unmatched historical requests because their execution
did not survive. The service owns request state, not tool execution tasks.
Responses are not unconditional authority: cancelled batches and changed policy
can prevent execution. Approval does not sandbox an admitted tool.

## Tests

[permission_flow.rs](../../../../crates/dim-agent-capabilities/tests/permission_flow.rs):
`an_allowed_approval_is_rechecked_against_the_current_policy`,
`a_response_that_finds_its_batch_stopped_ends_as_an_interrupt`;
[local_product.rs](../../../../tests/integration/tests/local_product.rs).

<!-- generated:plugin Approval — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:Dispatcher`、`B:Permission`、`B:Timeline` |
| 提供 | `B:Approval` |
| 被注入于 | `ToolExecutor` |
<!-- /generated -->
