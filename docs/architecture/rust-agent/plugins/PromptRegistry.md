# PromptRegistry

Owner: [prompt_registry.rs](../../../../crates/dim-agent-capabilities/src/prompt_registry.rs).
Keeps the Session's prompt sources, which parts register with it by name, and
serves them as the Session PromptHost and as the initial configuration that
adopts every registered source plus the Session-selected model.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:SessionConfig` | Initial model selection. |

## Events and lifetime

No events, worker tasks or I/O ownership. A part registers a source with
`register(ctx, name, source)` from its own `apply`; the registration is
withdrawn when that part unloads, and a name registered already fails
([rutis usage R6](../../../design/rutis-usage.md#r6-contributions-register-with-their-owner)).
Sources stay Rust-only until a part of another language needs one. Loads forward
target and caller cancellation to the registered source; a name nothing
registers now fails explicitly. The initial configuration is read when a branch
opens: it targets every source registered then, in name order, with main's
branch scope, inherited by derived branches. A part depends on PromptRegistry to
register with it, so a reloaded registry reloads the parts registered with it.
Registry construction is not adoption and does not independently refresh
branch context.

## Tests

[skills.rs](../../../../tests/integration/tests/skills.rs) verifies list
injection; [prompt_sources.rs](../../../../tests/integration/tests/prompt_sources.rs)
checks `system_is_static_and_instructions_and_environment_arrive_as_inputs`.

<!-- generated:plugin PromptRegistry — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `S:SessionConfig` |
| 提供 | `S:InitialPromptConfiguration`、`S:PromptHost`、`S:PromptRegistry` |
| 被注入于 | `Prompt`、`PromptSources`、`PromptSourcesBranch`、`Skills` |
<!-- /generated -->
