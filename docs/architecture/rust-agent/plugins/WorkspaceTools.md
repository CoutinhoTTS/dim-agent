# WorkspaceTools

Owner: [workspace_tools/mod.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/mod.rs),
[files.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/files.rs),
[exec.rs](../../../../crates/dim-agent-capabilities/src/workspace_tools/exec.rs).
Provides ToolHost for read, write, edit, glob, grep and exec, and registers the
leaf namespace `session.exec.output` with `sessionMethods` for the active
command's bounded tail preview. The WorkspaceTools plugin registers this part
and the branch parts FileReads and BackgroundExec, and holds the file-write
coordinator: one per-path FIFO admission domain shared by write and edit across
every Session it serves, which it hands to each Session's part. The plugin keeps
the same coordinator across its generations and theirs, so old calls settle in
the domain new ones enter.

## Dependencies

| Service | Purpose |
| --- | --- |
| `P:HostPlatform` | Home, shell, environment and process supervision. |
| `S:TreeCommands` | Resolve the calling branch per call. |
| `S:SessionConfig` | cwd and collaboration role. |
| `S:SessionIdentity` | Session-specific output directory. |
| `S:SessionMethods` | Register the `session.exec.output` namespace, until this part unloads. |

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

## Shared command construction and output

Exec uses [`HostPlatform::build_process_spec(command, cwd).await`](HostPlatform.md#command-construction).
HostPlatform owns the shell read, argv, environment and runtime PATH construction;
exec uses the returned shell for its own policies. Refusals carry named `code`
and `reason` fields. `cwd` is the resolved working directory, and missing-directory
errors retain the builder's resolved path without rewriting its message.

On macOS/Linux the exec declaration adds HostPlatform's static isolation line
after its environment line, so it stays stable for the Session. Exec keeps the
`Isolation` that preparation returns with its process. An unsuccessful exit, in
the foreground or after the background handoff, appends HostPlatform's failure
note to the result text when the shown output matches; stops, timeouts and
offload failures get none. Codes and structured content are unchanged.

Exec retains its `cwd` argument resolution, POSIX `&` refusal, heavy-search
refusal, timeouts and background ownership. Its existing `exec_shell_unavailable`
tool-result code is preserved, with instructions to fix the file and retry.

The same module exports the existing `Decoder::{write,end}`, `sanitize`,
`truncate_preview` and `EXIT_STDIO_GRACE`, and for ShellInput `capture_preview`,
`tail_bytes`, `signal_name` and `pump`. Keep one incremental decoder per pipe;
finish it at EOF or when the read grace ends. Invalid/truncated bytes become
`<0xNN>`. Sanitization removes control characters except tab/CR/LF, and
U+FFF9–U+FFFB. `capture_preview` keeps the bounded output tail (the last
100 KiB from a line start) that `truncate_preview` cuts to the last 2000 lines
within 50 KiB on UTF-8 boundaries. `pump` reads one pipe through its own
decoder into an `OutputSink` (exec's sink also offloads to the output file) and
keeps reading for the 100 ms grace after the root exits. These are the existing
output rules, exposed without changing their behavior.

## Rule check

The read-only [`session.exec.output`](../../../design/background-task-output.md)
method resolves the requested branch through TreeCommands and reads that branch's
BackgroundExec capture. It accepts only `{sessionId, branchId, taskId}` and returns
`{text}`. C1 owns admission and unload settlement on WorkspaceTools' original
context. The method does not open a client-supplied or historical path, append
events, acquire Session holds or start background work.

- §1 and ADR 002 D1–D3: command construction belongs to HostPlatform; no new
  plugin, wiring layer, state owner or dependency is added.
- §4 rules 1–6 and 9: existing ToolExecutor results and BackgroundExec handover
  are unchanged; no new events, before callbacks or context path are introduced.
- §4 rule 7: callers reuse the public builder and HostPlatform service; no
  consumer reads another plugin's private state.
- ADR 002 D4: exec reads shell configuration for each invocation, not on reload.
- ADR 002 D5: the builder is an existing native execution API; the output read is
  a language-neutral C1 method. Refusals are explicit code/reason data; no new
  runtime binding mechanism is introduced.

## Requirements raised to mechanism owners

None (§4 rule 8). S7 uses the existing host and storage contracts.

## Tests

[workspace_files.rs](../../../../crates/dim-agent-capabilities/tests/workspace_files.rs),
[local_execution.rs](../../../../tests/integration/tests/local_execution.rs),
[local_product.rs](../../../../tests/integration/tests/local_product.rs).

The App Server's [native execution flow](../../../../crates/dim-app-server/tests/transport_flow/shell_input.rs)
reads a live capture through C1, holds its real SQLite terminal commit, checks
bounds and branch/Session ownership, then verifies replay and close behavior.
The existing idle-unload/recovery flow in `local_execution.rs` also checks an
empty live preview and unavailable historical captures.

<!-- generated:plugin WorkspaceTools — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `P:HostPlatform`、`S:SessionConfig`、`S:SessionIdentity`、`S:SessionMethods`、`S:TreeCommands` |
| 提供 | `S:ToolHost` |
| 被注入于 | `ToolRegistry` |
<!-- /generated -->
