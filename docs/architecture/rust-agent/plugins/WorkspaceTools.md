# WorkspaceTools

Owner: [workspace_tools/mod.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/mod.rs),
[files.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/files.rs),
[exec.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/exec.rs).
Provides ToolHost for read, write, edit, glob, grep and exec.

## Dependencies

| Service | Purpose |
| --- | --- |
| `P:HostPlatform` | Home, shell, environment and process supervision. |
| `S:TreeCommands` | Resolve the calling branch per call. |
| `P:FileWriteCoordinator` | Shared write/edit admission across Sessions. |
| `S:SessionConfig` | cwd and collaboration role. |
| `S:SessionIdentity` | Session-specific output directory. |

## Events and lifetime

No direct Dispatcher subscription. Tool results return to ToolExecutor;
background exec transfers ownership to BackgroundExec, which commits ends and
wakes. Per-call lookups obtain FileReads/BackgroundExec on the caller's branch.
Generation cancellation rejects new calls and stops accepted calls; cleanup
waits for tracked calls, including handoff. File mutations stage complete bytes
and retain coordinator admission through I/O/publication. Foreground commands
release their process trees before returning; collaboration members do not
background commands. Output files are not deleted simply by plugin unload.
Permissions/approvals occur upstream: these native tools are not sandboxed.
Unsupported metadata preservation fails unchanged; post-install errors require
re-reading before retry.

## Tests

[workspace_files.rs](../../../../crates/dim-agent-capabilities/tests/workspace_files.rs),
[local_execution.rs](../../../../tests/integration/tests/local_execution.rs),
[local_product.rs](../../../../tests/integration/tests/local_product.rs).

<!-- generated:plugin WorkspaceTools — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `P:FileWriteCoordinator`、`P:HostPlatform`、`S:SessionConfig`、`S:SessionIdentity`、`S:TreeCommands` |
| 提供 | `S:ToolHost` |
| 被注入于 | `ToolRegistry` |
<!-- /generated -->
