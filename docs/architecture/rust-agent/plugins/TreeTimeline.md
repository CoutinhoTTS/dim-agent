# TreeTimeline

Owner: [timeline_tree.rs](../../../../crates/dim-agent-capabilities/src/timeline_tree.rs).
Routes branch-local creation facts to the Session tree using the actual parent
identity, and releases pending child reservations when the parent unloads.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:TreeCommands` | Reach the owning Session tree through its command port. |
| `B:TimelineIdentity` | Identify the parent branch. |
| `B:Dispatcher` | Consume creation events. |
| `B:Timeline` | Parent history and creation settlement location. |

## Events and lifetime

Consumes `branch/create`; tree-owned work produces opening/failure facts (see
TimelineTree). The handler registers creation, not a long-running build future.
Cleanup cancels pending reservations for this parent and settles their waiters.
No independent worker or persistent tree is owned here. Reload does not by
itself replay arbitrary old create events; recovery belongs to TimelineTree.
An accepted creation need not survive parent or Session close.

## Tests

[branch_flow.rs](../../../../crates/dim-agent-capabilities/tests/branch_flow.rs):
`parent_close_waits_for_dispatch_before_releasing_branch_reservations`,
`close_releases_an_unconsumed_create_without_waiting_for_dispatch`.

<!-- generated:plugin TreeTimeline — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:Dispatcher`、`B:Timeline`、`B:TimelineIdentity`、`S:TreeCommands` |
| 提供 | — |
| 被注入于 | — |
<!-- /generated -->
