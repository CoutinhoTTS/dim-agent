# PromptRegistry

Owner: [prompt_registry.rs](../../../../crates/dim-agent-capabilities/src/prompt_registry.rs).
Collects named sources into the Session PromptHost and initial configuration
that adopts all registered sources plus the Session-selected model.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:PromptSource#dim/base` | Base instructions. |
| `S:PromptSource#dim/environment/context` | Runtime environment context. |
| `S:PromptSource#dim/environment/date` | Current date content. |
| `S:PromptSource#dim/instructions/project` | Project AGENTS.md instructions. |
| `S:PromptSource#dim/instructions/user` | User AGENTS.md instructions. |
| `S:PromptSource#dim/session` | Session-specific prompt content. |
| `S:PromptSource#dim/skills/instructions` | Skill-use instructions. |
| `S:PromptSource#dim/skills/list` | Current skill list. |
| `S:SessionConfig` | Initial model selection. |

## Events and lifetime

No events, worker tasks or I/O ownership. Contributions are resolved into a
fresh map each generation. Loads forward target and caller cancellation to the
selected source; unknown names fail explicitly. Initial targets use main's
branch scope, inherited by derived branches. Registry construction is not
adoption and does not independently refresh branch context. Source names are
assembled by the builder; this is not open-ended discovery of arbitrary services.

## Tests

[skills.rs](../../../../tests/integration/tests/skills.rs) verifies list
injection; [prompt_sources.rs](../../../../tests/integration/tests/prompt_sources.rs)
checks `system_is_static_and_instructions_and_environment_arrive_as_inputs`.

<!-- generated:plugin PromptRegistry — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:PromptSource#dim/base`、`S:PromptSource#dim/environment/context`、`S:PromptSource#dim/environment/date`、`S:PromptSource#dim/instructions/project`、`S:PromptSource#dim/instructions/user`、`S:PromptSource#dim/session`、`S:PromptSource#dim/skills/instructions`、`S:PromptSource#dim/skills/list`、`S:SessionConfig` |
| 提供 | `S:InitialPromptConfiguration`、`S:PromptHost` |
| 被注入于 | `Prompt`、`PromptSourcesBranch` |
<!-- /generated -->
