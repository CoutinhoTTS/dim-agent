# SkillContributions

Owner: [skills/content.rs](../../../../crates/dim-agent-capabilities/src/skills/content.rs).
Provides the private Session collection of ready Skill content services.

## Dependencies

None. It is ready before Skills and SkillContent, avoiding an apply-time dependency cycle.

## Events and lifetime

No Timeline events, persistence, discovery or tasks. Leaves register and withdraw
their contributions; Skills reads the collection to assemble the current view.
The collection cannot create, restart or dispose plugins and is not another
service container or lifecycle manager. See [Skills](Skills.md).

## Tests

[skills.rs](../../../../tests/integration/tests/skills.rs) covers isolated Session
contributions and their replacement when a catalog change retires content.

<!-- generated:plugin SkillContributions — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | — |
| 提供 | `S:SkillContributions` |
| 被注入于 | `SkillContent`、`Skills` |
<!-- /generated -->
