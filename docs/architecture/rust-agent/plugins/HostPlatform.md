# HostPlatform

Owner: [host_platform.rs](../../../../crates/dim-agent-capabilities/src/host_platform.rs).
Resolves host home/environment and execution shell and supplies process spawning
and tree-supervision primitives to local tools.

## Dependencies

| Service | Purpose |
| --- | --- |
| `P:FileStorage` | Read configured shell preference; enabled by xtask's shell-file configuration. |

## Events and lifetime

No Timeline events or plugin background tasks. `apply` captures the process
environment, reads the initial shell snapshot and retains the configured
FileStorage source. `exec_shell()` rereads that source on every invocation.
Available resolutions and `EXEC_SHELL_CONFIG_INVALID` errors are reused while
the read result is unchanged; `EXEC_SHELL_UNAVAILABLE` is resolved again on the
next call, so transient detection failures and newly available shells recover
without editing the preference or reloading. One async mutex serializes reads
and resolution across callers and cloned handles. Unchanged available selections
do not repeat Windows shell probes. Unix shell discovery runs on a blocking
worker. Changing an available selection requires a preference edit or
HostPlatform reload. Missing files select automatically;
malformed or unreadable files return `EXEC_SHELL_CONFIG_INVALID`; an unavailable explicit
shell returns `EXEC_SHELL_UNAVAILABLE` without fallback. Resolution uses the
captured service environment, never a caller's environment.

When no shell file is configured the FileStorage dependency is disabled and
`exec_shell()` returns the supplied initial snapshot. `HostPlatform::current`
constructs such a platform, with public facts that hosts/tests may customize;
the preference source and shell snapshot are private. Callers use
`exec_shell()` rather than mutating a snapshot. `persistent_shell().await` reads
the current preference before creating a persistent shell and keeps its existing
supported-shell fallback rules. An already running shell is not restarted when
the exec preference changes. Spawned process lifetime belongs to each
caller/supervisor, not this plugin. An unavailable shell does not prevent publishing HostPlatform; exec
fails explicitly. Environment filtering/tree termination are execution mechanics,
not an OS sandbox or permission policy.

The [F5 isolation spike](../../../design/host-platform-isolation-spike.md) exercises
Seatbelt and Landlock wrappers through these existing process and PTY APIs. It
does not install product isolation; the host-local policy remains open.

## Command construction

`HostPlatform::build_process_spec(command, cwd).await` reads the current shell
itself and returns `(ProcessSpec, ExecShellRuntime)`. The returned runtime is the
same selection used to build the process, so callers can apply shell-specific
policies without another read. `cwd` is an already resolved working directory.
The builder preserves shell flags, forced environment overrides and runtime PATH
prefixes, including Git Bash mapping. It does not start the user's command.

Refusals are `ProcessSpecError { code, reason }` data: malformed/unreadable
preferences (`EXEC_SHELL_CONFIG_INVALID`), unavailable shells
(`EXEC_SHELL_UNAVAILABLE`), a network-share cwd with cmd
(`EXEC_SHELL_CWD_UNSUPPORTED`), a missing cwd (`cwd_not_found`), or failed Git Bash
path mapping (`EXEC_SHELL_PATH_MAPPING_FAILED`).

## Rule check

- §1 and ADR 002 D1–D3: process facts and the preference source remain owned by
  HostPlatform; no new plugin, wiring module or dependency is added.
- §4 rules 1–6 and 9: no commands, handovers, before callbacks, batches or Timeline
  facts are added. Existing command execution and input adoption remain unchanged.
- §4 rule 7: callers use the current-shell methods, command builder and `spawn()`;
  file access stays behind the declared FileStorage service.
- ADR 002 D4: read business configuration at use time, without watchers or reload.
- ADR 002 D5: the native binding returns explicit command/shell and code/reason
  data. Hosts construct fixed platforms with `HostPlatform::current`; there is no
  mutable public shell snapshot or caller-supplied builder shell.

## Requirements raised to mechanism owners

None (§4 rule 8). Existing FileStorage and process primitives cover S7.

## Tests

[host_platform_flow.rs](../../../../crates/dim-agent-capabilities/tests/host_platform_flow.rs):
`shell_resolution_follows_the_preference_and_never_fails_the_service`,
`spawn_isolates_the_environment_and_terminate_releases_the_tree`,
`unchanged_shell_preferences_reuse_detection_and_edits_recover`.
[local_execution.rs](../../../../tests/integration/tests/local_execution.rs):
`exec_and_external_shell_callers_use_live_shell_preferences` exercises the real
plugin, configuration edits, refusals, recovery and the shared builder alongside exec.
[minimal_execution.rs](../../../../tests/integration/tests/minimal_execution.rs):
`new_persistent_shells_and_environment_refresh_use_current_preferences` covers
first spawn after an edit, adopted environment refresh, retention of a running
shell and current selection after shell exit.

<!-- generated:plugin HostPlatform — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 进程 |
| 注入 | `P:FileStorage` |
| 提供 | `P:HostPlatform` |
| 被注入于 | `PromptSources`、`ShellInput`、`SkillsCatalog`、`WorkspaceTools` |
<!-- /generated -->
