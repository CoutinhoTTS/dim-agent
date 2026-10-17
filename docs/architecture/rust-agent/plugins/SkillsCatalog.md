# SkillsCatalog

Owner: [skills/catalog.rs](../../../../crates/dim-agent-capabilities/src/skills/catalog.rs)
and [discovery.rs](../../../../crates/dim-agent-capabilities/src/skills/discovery.rs).
Owns process-wide skill discovery caches and enablement, combining user and
project `.agents/skills` roots into queryable views with issues.

## Dependencies

| Service | Purpose |
| --- | --- |
| `P:HostPlatform` | Resolve the user home root. |
| `P:SkillSettingsHost` | Read and update enablement. |

## Events and lifetime

No Timeline events. Each time its version advances (a rescan, or a changed
disabled set) the catalog emits `skills.catalog_changed {version}` on
`catalog_changes()` (`skills/catalog`) from its own context, under its state
lock, so announcements follow version order; listeners query the catalog for
the view.

Before its first settings read, the catalog listens with `ctx.events().on` to
`settings.changed` on `settings_changes(SOURCE)`, its Settings source `skills`.
The listener only wakes a tracked reader, which rereads the settings one read
at a time so a stale response cannot overwrite newer settings. A host whose
storage changes outside the catalog announces those writes there; the catalog
reads back its own writes, so `MemorySkillSettings` (xtask, tests) announces
nothing.

First use scans roots via blocking work; explicit refresh updates cached
discovery. Cancellation closes the catalog; cleanup joins its reader. Reload
creates fresh discovery caches. Read failures remain explicit and queries can
retry unavailable settings; this is not a filesystem watcher or proof that a
settings mutation was durably saved.

## Tests

[skills.rs](../../../../tests/integration/tests/skills.rs):
`discovery_precedence_and_list_injection`,
`enablement_changes_the_list_and_loading`,
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
