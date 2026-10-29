# PromptSources

The PromptSources plugin registers PromptRegistry and PromptSources in every Session and PromptSourcesBranch in every branch.

## PromptRegistry

Owner: [prompt_registry.rs](../../../../crates/dim-agent-capabilities/src/prompt_registry.rs).
Keeps the Session's prompt sources, which parts register with it by name, and
serves them as the Session PromptHost and as the initial configuration that
adopts every registered source plus the Session-selected model.

### Dependencies

| Service | Purpose |
| --- | --- |
| `S:SessionConfig` | Initial model selection. |

### Events and lifetime

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

### Tests

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

## PromptSources

Owner: [prompt_sources.rs](../../../../crates/dim-agent-capabilities/src/prompt_sources.rs).
Provides named base, Session, environment/date and user/project instruction
sources for PromptRegistry; source loading is separate from branch adoption.

### Dependencies

| Service | Purpose |
| --- | --- |
| `P:HostPlatform` | Home, environment and shell/platform context. |
| `S:SessionConfig` | Working directory and Session context. |
| `S:PromptRegistry` | Register each of its sources by name, until this part unloads. |

The policy texts — `dim/base`'s identity and rules, the shell guidance and the
file-encoding guidance — are the plugin's factory configuration: an optional
member each, the built-in text in force when none is set. The product installs
the plugin without a configuration, so its texts are the built-in ones; a
candidate is installed through the shipped assembly with one, never by patching
text at a request boundary
([PromptSources design §2](../../../design/plugins/PromptSources.md#2-policy-at-the-factory-configuration)).
The shell and file-encoding guidance stay inside the environment state block,
where the model reads them today.

### Events and lifetime

No Timeline subscriptions/production or background tasks in this Session plugin.
Each source shares a generation cancellation token and checks it on load.
Reload publishes new source handles; PromptSourcesBranch checks mutable sources.
Instructions come from the public AGENTS.md search logic, not private agent
documentation. Sources must be loaded then adopted by Prompt before reaching
model context. Polling does not guarantee immediate visibility of file changes.
The environment source reads `HostPlatform::exec_shell()` on each load, so the
existing refresh/adoption flow also picks up exec shell preference changes.
Explicit refresh, like periodic checks, refreshes only instructions, environment
and date. Base and Session System entries stay adopted. Profile adoption uses
`session_entry` to construct or revoke the same `dim/session` entry as creation.

### Rule check

§1 and ADR 002 D1–D3: the existing Session source retains its owner, dependencies
and lifecycle; no wiring module is added. §4 rules 1–6 and 9: the existing prompt
refresh and adoption path is unchanged, with no new events or context path.
§4 rule 7 and ADR 002 D4: shell data comes from HostPlatform's public read-at-use
API, not its private source. ADR 002 D5: no new service or wire contract is added.

### Requirements raised to mechanism owners

None (§4 rule 8). Existing source refresh and adoption cover shell changes.

### Tests

[prompt_sources.rs](../../../../tests/integration/tests/prompt_sources.rs):
`system_is_static_and_instructions_and_environment_arrive_as_inputs`,
`user_instructions_appear_once_and_absent_files_produce_no_entry`,
`a_restored_session_receives_the_changed_environment_once`.

<!-- generated:plugin PromptSources — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | Session |
| 注入 | `P:HostPlatform`、`S:PromptRegistry`、`S:SessionConfig` |
| 提供 | — |
| 被注入于 | — |
<!-- /generated -->

## PromptSourcesBranch

Owner: [prompt_sources.rs](../../../../crates/dim-agent-capabilities/src/prompt_sources.rs).
Periodically compares checked instruction, environment and date sources against
the branch's adopted prompt entries and asks Prompt to adopt differences.

### Dependencies

| Service | Purpose |
| --- | --- |
| `S:PromptHost` | Load the current source contents. |
| `B:BranchLifecycle` | Activation and closing gates. |
| `B:StateCell<PromptSnapshot>` | Compare with already adopted entries. |
| `B:dyn ConfigureControl` | Request adoption of changed sources. |

### Events and lifetime

No direct event production or subscription. The tracked polling task starts at
activation, checks immediately and then at the configured interval. Closing is
observed between checks; generation cancellation can end an in-flight check.
Cleanup joins the task. Reload restarts comparison from current adopted state.
Source/check errors are logged and retried next time. This is polling of the
configured checked set, not automatic adoption of every possible source.

### Tests

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
