# InputMethods

Owner: [input_queue.rs](../../../../crates/dim-agent-capabilities/src/input_queue.rs).
Provides the Session's `session.input.withdraw` method through `sessionMethods`.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:TreeCommands` | Resolve the requested branch and its InputQueue for each call. |

## Events and lifetime

Validates method parameters at the request boundary, then passes the withdrawal
and optional Steer input to the branch's InputQueue. InputQueue owns the command,
the atomic replacement batch and the decision; InputMethods returns its answer
without changing queue state itself. An already adopted or unknown input returns
`notHeld`. The method uses the existing SessionMethods admission and unload
ownership, declares interruption reporting, and owns no background tasks or
persistent state.

## Tests

[host_flows.rs](../../../../crates/dim-agent-capabilities/tests/host_flows.rs):
`a_held_input_is_withdrawn_once_and_an_applied_one_is_not`.

<!-- generated:plugin InputMethods — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:TreeCommands` |
| 提供 | `S:SessionMethods#session.input` |
| 被注入于 | — |
<!-- /generated -->
