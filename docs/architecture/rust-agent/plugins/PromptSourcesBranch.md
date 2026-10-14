# PromptSourcesBranch

Owner: [prompt_sources.rs](../../../../crates/dim-agent-capabilities/src/prompt_sources.rs).
Periodically compares checked instruction, environment and date sources against
the branch's adopted prompt entries and asks Prompt to adopt differences.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:PromptHost` | Load the current source contents. |
| `B:BranchLifecycle` | Activation and closing gates. |
| `B:StateCell<PromptSnapshot>` | Compare with already adopted entries. |
| `B:dyn ConfigureControl` | Request adoption of changed sources. |

## Events and lifetime

No direct event production or subscription. The tracked polling task starts at
activation, checks immediately and then at the configured interval. Closing is
observed between checks; generation cancellation can end an in-flight check.
Cleanup joins the task. Reload restarts comparison from current adopted state.
Source/check errors are logged and retried next time. This is polling of the
configured checked set, not automatic adoption of every possible source.

## Tests

[prompt_sources.rs](../../../../tests/integration/tests/prompt_sources.rs):
`agents_md_changes_arrive_as_rules_from_the_periodic_check`,
`unreadable_project_instructions_keep_the_adopted_version`,
`a_loaded_branch_appends_a_date_reminder_when_the_day_changes`.

<!-- generated:plugin PromptSourcesBranch — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:BranchLifecycle`、`B:StateCell<PromptSnapshot>`、`B:dyn ConfigureControl`、`S:PromptHost` |
| 提供 | — |
| 被注入于 | — |
<!-- /generated -->
