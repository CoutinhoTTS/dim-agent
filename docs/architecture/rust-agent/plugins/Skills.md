# Skills

Owner: [skills/session.rs](../../../../crates/dim-agent-capabilities/src/skills/session.rs).
Maintains a Session view of the catalog, contributes the skill tool and provides
skill instructions/list prompt sources.

## Dependencies

| Service | Purpose |
| --- | --- |
| `P:SkillsCatalog` | Query user/project skills and watch catalog versions. |
| `S:SessionConfig` | Select the Session working directory. |

## Events and lifetime

No direct Timeline events. A tracked catalog watcher replaces the Session view;
SkillsBranch requests list adoption. The tool returns full skill content as a
plain tool result, not a second history-rendering route. Cleanup joins the
watcher; tool handlers reject work after generation cancellation. Reload queries
a fresh view. Unavailable settings isolate skills without disabling unrelated
execution. Listed metadata and loaded body can change between view updates;
loading uses the current view rather than an old model-visible list.

## Tests

[skills.rs](../../../../tests/integration/tests/skills.rs):
`load_returns_the_body_as_a_plain_tool_result`,
`skill_loads_the_latest_view_after_queue_and_steer_interleave`,
`unavailable_settings_isolate_skills_and_refresh_recovers_existing_sessions`.

<!-- generated:plugin Skills — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `P:SkillsCatalog`、`S:SessionConfig` |
| 提供 | `S:PromptSource#dim/skills/instructions`、`S:PromptSource#dim/skills/list`、`S:SessionSkills`、`S:ToolContribution#skill` |
| 被注入于 | `PromptRegistry`、`SkillsBranch`、`ToolRegistry` |
<!-- /generated -->
