# PromptSources

Owner: [prompt_sources.rs](../../../../crates/dim-agent-capabilities/src/prompt_sources.rs).
Provides named base, Session, environment/date and user/project instruction
sources for PromptRegistry; source loading is separate from branch adoption.

## Dependencies

| Service | Purpose |
| --- | --- |
| `P:HostPlatform` | Home, environment and shell/platform context. |
| `S:SessionConfig` | Working directory and Session context. |

## Events and lifetime

No Timeline subscriptions/production or background tasks in this Session plugin.
Each source shares a generation cancellation token and checks it on load.
Reload publishes new source handles; PromptSourcesBranch checks mutable sources.
Instructions come from the public AGENTS.md search logic, not private agent
documentation. Sources must be loaded then adopted by Prompt before reaching
model context. Polling does not guarantee immediate visibility of file changes.

## Tests

[prompt_sources.rs](../../../../tests/integration/tests/prompt_sources.rs):
`system_is_static_and_instructions_and_environment_arrive_as_inputs`,
`user_instructions_appear_once_and_absent_files_produce_no_entry`,
`a_restored_session_receives_the_changed_environment_once`.

<!-- generated:plugin PromptSources — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `P:HostPlatform`、`S:SessionConfig` |
| 提供 | `S:PromptSource#dim/base`、`S:PromptSource#dim/environment/context`、`S:PromptSource#dim/environment/date`、`S:PromptSource#dim/instructions/project`、`S:PromptSource#dim/instructions/user`、`S:PromptSource#dim/session` |
| 被注入于 | `PromptRegistry` |
<!-- /generated -->
