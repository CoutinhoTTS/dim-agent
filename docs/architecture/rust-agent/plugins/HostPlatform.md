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
FileStorage source. `exec_shell()` rereads that source and resolves the shell
for each command; edits, invalid configuration and recovery take effect without
reloading the plugin or Session. Missing files select automatically; malformed
or unreadable files return `EXEC_SHELL_CONFIG_INVALID`; an unavailable explicit
shell returns `EXEC_SHELL_UNAVAILABLE` without fallback. Resolution uses the
captured service environment, never a caller's environment.

When no shell file is configured the FileStorage dependency is disabled and
`exec_shell()` returns the supplied initial snapshot. `HostPlatform::current`
constructs such a platform, with public facts that hosts/tests may customize;
the preference source is private. `shell` also remains the initial snapshot
used by persistent-shell selection; an already running shell is not restarted
when the exec preference changes. Spawned process lifetime belongs to each caller/supervisor, not this
plugin. An unavailable shell does not prevent publishing HostPlatform; exec
fails explicitly. Environment filtering/tree termination are execution mechanics,
not an OS sandbox or permission policy.

## Rule check

- §1 and ADR 002 D1–D3: process facts and the preference source remain owned by
  HostPlatform; no new plugin, wiring module or dependency is added.
- §4 rules 1–6 and 9: no commands, handovers, before callbacks, batches or Timeline
  facts are added. Existing command execution and input adoption remain unchanged.
- §4 rule 7: callers use `exec_shell()` and `spawn()`; file access stays behind
  the declared FileStorage service.
- ADR 002 D4: read business configuration at use time, without watchers or reload.
- ADR 002 D5: this existing native host API is unchanged as a language binding;
  the shared command builder returns refusal codes and reasons as data.

## Requirements raised to mechanism owners

None (§4 rule 8). Existing FileStorage and process primitives cover S7.

## Tests

[host_platform_flow.rs](../../../../crates/dim-agent-capabilities/tests/host_platform_flow.rs):
`shell_resolution_follows_the_preference_and_never_fails_the_service`,
`spawn_isolates_the_environment_and_terminate_releases_the_tree`.
[local_execution.rs](../../../../tests/integration/tests/local_execution.rs):
`exec_and_external_shell_callers_use_live_shell_preferences` exercises the real
plugin, configuration edits, refusals, recovery and the shared builder alongside exec.

<!-- generated:plugin HostPlatform — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 进程 |
| 注入 | `P:FileStorage` |
| 提供 | `P:HostPlatform` |
| 被注入于 | `PromptSources`、`SkillsCatalog`、`WorkspaceTools` |
<!-- /generated -->
