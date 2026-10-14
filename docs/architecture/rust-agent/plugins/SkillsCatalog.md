# SkillsCatalog

Owner: [skills/catalog.rs](../../../../crates/dim-agent-capabilities/src/skills/catalog.rs)
and [discovery.rs](../../../../crates/dim-agent-capabilities/src/skills/discovery.rs).
Owns process-wide skill discovery caches and enablement, combining user and
project `.agents/skills` roots into queryable views with issues.

## Dependencies

| Service | Purpose |
| --- | --- |
| `P:HostPlatform` | Resolve the user home root. |
| `P:SkillSettingsHost` | Read/update enablement and watch settings versions. |

## Events and lifetime

No Timeline events; catalog versions are watch notifications. First use scans
roots via blocking work; explicit refresh updates cached discovery. A tracked
settings watcher serializes readbacks so stale responses cannot overwrite newer
settings. Cancellation closes the catalog; cleanup joins its watcher. Reload
creates fresh discovery caches. Read failures remain explicit and queries can
retry unavailable settings; this is not a filesystem watcher or proof that a
settings mutation was durably saved. xtask supplies in-memory settings.

## Tests

[skills.rs](../../../../tests/integration/tests/skills.rs):
`discovery_precedence_and_list_injection`,
`a_late_update_response_does_not_overwrite_a_newer_setting`,
`a_failed_read_back_is_reported_and_recovered_by_the_next_query`.

<!-- generated:plugin SkillsCatalog — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 进程 |
| 注入 | `P:HostPlatform`、`P:SkillSettingsHost` |
| 提供 | `P:SkillsCatalog` |
| 被注入于 | `Skills` |
<!-- /generated -->
