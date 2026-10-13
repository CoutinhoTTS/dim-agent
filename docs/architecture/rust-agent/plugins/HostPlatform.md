# HostPlatform

Owner: [host_platform.rs](../../../../crates/dim-agent-capabilities/src/host_platform.rs).
Resolves host home/environment and execution shell and supplies process spawning
and tree-supervision primitives to local tools.

## Dependencies

| Service | Purpose |
| --- | --- |
| `P:FileStorage` | Read configured shell preference; enabled by xtask's shell-file configuration. |

## Events and lifetime

No Timeline events or plugin background tasks. `apply` reads preference, captures
environment and probes shells before publishing a platform value. Reload
resolves afresh. When no shell file is configured the FileStorage dependency is
disabled. Spawned process lifetime belongs to each caller/supervisor, not this
plugin. An unavailable shell does not prevent publishing HostPlatform; exec
fails explicitly. Environment filtering/tree termination are execution mechanics,
not an OS sandbox or permission policy.

## Tests

[host_platform_flow.rs](../../../../crates/dim-agent-capabilities/tests/host_platform_flow.rs):
`shell_resolution_follows_the_preference_and_never_fails_the_service`,
`spawn_isolates_the_environment_and_terminate_releases_the_tree`.

<!-- generated:plugin HostPlatform — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 进程 |
| 注入 | `P:FileStorage` |
| 提供 | `P:HostPlatform` |
| 被注入于 | `PromptSources`、`SkillsCatalog`、`WorkspaceTools` |
<!-- /generated -->
