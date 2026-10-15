# SkillContentInstances

Owner: [skills/mod.rs](../../../../crates/dim-agent-capabilities/src/skills/mod.rs).
The instanced loader group containing one [SkillContent](SkillContent.md) leaf.

## Dependencies

None on the group itself. The leaf declares its Session contribution dependency.

## Events and lifetime

This is a structural template, not an installer or a second lifecycle manager.
Skills creates instances in the Session subtree and supplies immutable private
content. Closing the Session disposes its groups. Template configuration never
carries mutable per-Session definitions. See [Skills](Skills.md) for selection.

## Tests

[skills.rs](../../../../tests/integration/tests/skills.rs) covers independent
instances and selective replacement through public Session flows.

<!-- generated:plugin SkillContentInstances — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | — |
| 提供 | — |
| 被注入于 | — |
<!-- /generated -->
