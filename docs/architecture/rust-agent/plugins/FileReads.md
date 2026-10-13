# FileReads

Owner: [workspace_tools/file_reads.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/file_reads.rs).
Records canonical-path bindings for files read on this branch: raw-byte revision,
encoding, BOM and line endings. Existing-file write/edit admission requires a
binding established by that branch.

## Dependencies

No injected services (`Deps = ()`). WorkspaceTools resolves this branch service
per file-tool call, rather than injecting it into the Session plugin.

## Events and lifetime

No events, tasks or external resources. A mutex-protected map is created in each
`apply`. Reload loses its bindings; it does not reconstruct them from history.
A fresh read is therefore needed before mutation. The record is not a filesystem
lock, permission grant or cross-process compare-and-swap.

## Tests

[workspace_files.rs](../../../../crates/dim-agent-capabilities/tests/workspace_files.rs):
`writes_require_a_fresh_read_on_the_same_branch`,
`a_failed_offload_does_not_bind_the_file`,
`edits_and_writes_preserve_bom_and_line_endings`.

<!-- generated:plugin FileReads — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | — |
| 提供 | `B:FileReads` |
| 被注入于 | — |
<!-- /generated -->
