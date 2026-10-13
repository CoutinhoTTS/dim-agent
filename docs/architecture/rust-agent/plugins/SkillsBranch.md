# SkillsBranch

Owner: [skills/session.rs](../../../../crates/dim-agent-capabilities/src/skills/session.rs).
Exposes the Session skill view from a branch and requests adoption of a changed
skill list through `ConfigureControl`; initial adoption belongs to Prompt.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:SessionSkills` | Current skill view and change watch. |
| `B:BranchLifecycle` | Wait for activation; stop on closing. |
| `B:dyn ConfigureControl` | Adopt the changed list as prompt configuration. |

## Events and lifetime

No direct Timeline subscription or production. A tracked watcher starts after
activation, coalesces Session view changes and requests `dim/skills/list`
configuration. Generation cancellation stops the watcher; cleanup joins it.
Reload creates a new watcher, not a filesystem watcher or durable catalog.
Failed adoption is logged; a later view change can retry.

## Tests

[skills.rs](../../../../tests/integration/tests/skills.rs), especially
`a_view_change_inside_a_turn_reaches_the_next_request` and
`enablement_changes_the_list_and_loading`.

<!-- generated:plugin SkillsBranch — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:BranchLifecycle`、`B:dyn ConfigureControl`、`S:SessionSkills` |
| 提供 | `B:SkillsBranch` |
| 被注入于 | — |
<!-- /generated -->
