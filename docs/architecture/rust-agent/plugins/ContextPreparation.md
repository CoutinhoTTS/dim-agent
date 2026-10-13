# ContextPreparation

Owner: [context_preparation.rs](../../../../crates/dim-agent-capabilities/src/context_preparation.rs)
and [context_policy.rs](../../../../crates/dim-agent-capabilities/src/context_policy.rs).
Prepares a fixed-source model request, enforces context policy and provides the
atomic ConfigurationPublisher used by Prompt.

## Dependencies

| Service | Purpose |
| --- | --- |
| `S:HistoryRenderer` | Render supported history for preparation/budget checks. |
| `S:ContextModelHost` | Optional context-policy/recovery resolver. |
| `S:PrepareContextHost` | Host preparation hook. |
| `B:TimelineIdentity` | Identify configuration publication scope. |
| `B:Dispatcher` | Receive preparation/steer/interrupt transitions and report faults. |
| `B:Timeline` | Read fixed context, publish handoff and configuration facts. |
| `B:Diagnostics` | Preparation diagnostics. |

## Events and lifetime

Consumes `context/prepare`, `stop/interrupt`, `step/model-start`, `input/applied`,
turn start/end. Produces `model/call` on success or `step/model-output-end` on
failure. Its publisher commits prompt changes, adopted inputs and
`context/configured` atomically. Preparation is single-flight, tracked,
cancelled by stop/steer and joined during cleanup; cleanup aggregates failures.
Replay settles unmatched handoffs without rerunning host work. The mock profile
has no context resolver. Completed compaction facts remain authoritative across
checkpoint selection; optional snapshot separation is not yet implemented.

## Tests

[context_flow.rs](../../../../crates/dim-agent-capabilities/tests/context_flow.rs):
`restoring_each_preparation_handoff_settles_without_replaying_work`,
`before_modified_request_is_budget_checked_before_native_model_invocation`;
[checkpoint_projection.rs](../../../../crates/dim-agent-capabilities/tests/context_flow/checkpoint_projection.rs).

<!-- generated:plugin ContextPreparation — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:Diagnostics`、`B:Dispatcher`、`B:Timeline`、`B:TimelineIdentity`、`S:ContextModelHost`、`S:HistoryRenderer`、`S:PrepareContextHost` |
| 提供 | `B:ConfigurationPublisher`、`B:ContextPreparation` |
| 被注入于 | `ModelProvider`、`Prompt` |
<!-- /generated -->
