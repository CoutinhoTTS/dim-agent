# FileWriteCoordinator

Owner: [workspace_tools/mod.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/mod.rs)
and [files.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/files.rs).
Provides a process-wide per-path FIFO admission domain shared by write/edit
across Sessions.

## Dependencies

No injected services (`Deps = ()`).

## Events and lifetime

No events, tasks or I/O handles in the plugin. Clone the same plugin across
reloads: its shared coordinator preserves the lock domain while old calls
settle. Creating an unrelated coordinator defeats cross-Session serialization.
Waiters can cancel; admitted mutation holds the lock through blocking I/O and
binding publication. Complete bytes are staged before atomic replacement with
revision/identity/metadata checks. This is not compare-and-swap against unrelated
processes or parent-directory replacement. Post-install errors may mean the
target already changed: re-read before retrying.

## Tests

[workspace_files.rs](../../../../crates/dim-agent-capabilities/tests/workspace_files.rs):
`coordinated_sessions_reject_stale_mutations`,
`edits_of_one_file_run_in_fifo_order_and_waiters_can_cancel`,
`atomic_mutations_preserve_native_metadata`.

<!-- generated:plugin FileWriteCoordinator — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 进程 |
| 注入 | — |
| 提供 | `P:FileWriteCoordinator` |
| 被注入于 | `WorkspaceTools` |
<!-- /generated -->
