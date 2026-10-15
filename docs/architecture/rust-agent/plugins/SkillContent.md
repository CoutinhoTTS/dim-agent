# SkillContent

Owner: [skills/content.rs](../../../../crates/dim-agent-capabilities/src/skills/content.rs).
An immutable enabled Skill definition contributed by a Session-owned loader instance.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:Contributions` | Register ready content and withdraw it on disposal. |

## Events and lifetime

No Timeline events or external execution. Private instance values contain the
definition and scoped service key; loader options never contain the Skill body.
The leaf provides its definition and registers that same service in the Session
collection. Its effect withdraws the contribution. Skills coordinates selection;
rutis owns the instance lifecycle. See [Skills](Skills.md) for update and failure rules.

## Tests

[skills.rs](../../../../tests/integration/tests/skills.rs) covers loading actual
content, selective replacement, disabled content and independent Session closure.

<!-- generated:plugin SkillContent — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:Contributions` |
| 提供 | `S:Contribution#user:graph` |
| 被注入于 | — |
<!-- /generated -->
