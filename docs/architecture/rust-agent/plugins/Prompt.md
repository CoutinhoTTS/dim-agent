# Prompt

Owner: [prompt.rs](../../../../crates/dim-agent-capabilities/src/prompt.rs) and
[prompt_state.rs](../../../../crates/dim-agent-capabilities/src/prompt_state.rs).
Loads and adopts prompt/model/tool configuration in timeline order, exposes
ConfigureControl and projects adopted prompt state.

## Dependencies

| Service | Purpose |
| --- | --- |
| `B:ConfigurationPublisher` | Atomically publish configuration and final input content. |
| `S:PromptHost` | Load named prompt sources. |
| `S:InitialPromptConfiguration` | Initial source/model selection. |
| `B:BranchLifecycle` | Coordinate opening and closing. |
| `B:TimelineIdentity` | Scoped dependency/lifecycle gate. |
| `B:Dispatcher` | Order configuration adoption and state projection. |
| `B:Timeline` | Write controls and rebuild adopted context. |
| `B:Diagnostics` | Record prompt-loading diagnostics. |
| `S:FaultReporter` | Report owned work failures. |
| `B:BranchOpening` | Register initial prompt completion receipt. |

## Events and lifetime

Consumes prompt configure-request/result/changed, `context/configured`, turn
boundaries, branch start/resume, `context/scope`, `stop/interrupt`. Produces
configure controls and, through ConfigurationPublisher, prompt changes,
context configuration and final adopted `input/message` content. Scope/turn
changes can produce retirement inputs. A tracked adoption worker serializes
work; opening handlers register receipts and return. Cleanup cancels pending
loads, joins tasks and settles waiters. Constructed host-load futures are joined
even after cancellation. Reload projects committed context; runtime waiter IDs
do not resume host operations. Source loading alone is not adoption.

## Tests

[prompt_lifecycle.rs](../../../../tests/integration/tests/prompt_lifecycle.rs):
`configuration_is_atomic_failure_safe_and_inherited`,
`overlapping_load_is_superseded_and_close_joins_cancellation`,
`startup_opening_dispatch_is_free_and_close_or_deadline_joins_the_host`.

<!-- generated:plugin Prompt — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:BranchLifecycle`、`B:BranchOpening`、`B:ConfigurationPublisher`、`B:Diagnostics`、`B:Dispatcher`、`B:Timeline`、`B:TimelineIdentity`、`S:FaultReporter`、`S:InitialPromptConfiguration`、`S:PromptHost` |
| 提供 | `B:Prompt`、`B:StateCell<PromptSnapshot>`、`B:dyn ConfigureControl` |
| 被注入于 | `PromptSourcesBranch`、`SkillsBranch`、`ToolBinding` |
<!-- /generated -->
