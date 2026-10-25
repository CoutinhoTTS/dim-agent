# InputMethods

Owner: [input_queue.rs](../../../../crates/dim-agent-capabilities/src/input_queue.rs).
Registers the namespace `session.input` with the Session's `sessionMethods`:
`session.input.withdraw` for every branch of its Session, served through that
branch's InputQueue.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:TreeCommands` | Find the branch named by a call and read its InputQueue. |
| `S:SessionMethods` | Register `session.input` from `apply`, until this part unloads. |

## Events and lifetime

Writes no events itself; InputQueue commits `input/withdraw` and its result.
The registration follows the shared Session method admission: an unload
interrupts a call, and a caller's disconnect cancels nothing. While the part
reloads, the withdrawn namespace answers `service_reloading`.

## Tests

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
