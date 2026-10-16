# InputMethods

Owner: [input_queue.rs](../../../../crates/dim-agent-capabilities/src/input_queue.rs).
Provides `sessionMethods["session.input"]`: `session.input.withdraw` for every
branch of its Session, served through that branch's InputQueue.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:TreeCommands` | Find the branch named by a call and read its InputQueue. |

## Events and lifetime

Writes no events itself; InputQueue commits `input/withdraw` and its result.
The method provider follows the shared Session method admission: an unload
interrupts a call, and a caller's disconnect cancels nothing.

## Tests

[session_methods.rs](../../../../crates/dim-app-server/tests/transport_flow/session_methods.rs)
covers `session.input.withdraw` through Rpc.

<!-- generated:plugin InputMethods — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:TreeCommands` |
| 提供 | `S:SessionMethods#session.input` |
| 被注入于 | — |
<!-- /generated -->
