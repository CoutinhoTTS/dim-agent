# ContextUsage

Owner: [context_usage.rs](../../../../crates/dim-agent-capabilities/src/context_usage.rs).
A branch part of the ContextPreparation feature. It owns persisted numeric context
contributions and request calibration under [ContextUsage](../../../design/plugins/ContextUsage.md).

## Dependencies

| Service | Purpose |
| --- | --- |
| `B:Timeline` | Read selected source properties and numeric nodes; atomically append nodes and recovery results. |
| `B:Dispatcher` | Observe source publication and report persistence or projection faults. |
| `B:BranchLifecycle` | Start after publication and cancel when the branch closes. |
| `S:ContextModelHost` | Resolve the declared additive counter and its algorithm. |
| `S:HistoryRenderer` | Initialize missing numeric state with the canonical projection. |

## Events and lifetime

Consumes `context/configured`, `input/consumed`, `step/model-output-end`,
`step/tool-result`, `context/compaction` and `model/usage`. Handlers retain only
the wanted sequence and notify the owned task. It reads at most 128 new source
identities per page, consumes necessary properties, then saves reachable new
`context-usage/node` facts and `context-usage/recovered` atomically.

Recovery reads the latest local result after the branch opening and processes
only subsequent sources. Missing old state initializes explicitly; unknown
schemas or damaged references fail instead of replaying silently. Numeric tree
nodes contain no messages. Current reads use the root summary; a fixed earlier
source boundary reads only its numeric prefix path. With no bound resolver the
part exposes an unavailable read and starts no work.

Recovery and request references write v2 schemas and accept the previous v1
shape. A request-specific fixed-content identity keeps its provider calibration
separate from the adopted configuration's current usage.

The source counter shares input, settled-output and tool-result projection rules
with HistoryRenderer. Output settlement selects the last complete content and
only later fragments of that request. Raw deltas do not produce statistics writes.
Provider calibration reads the private request reference by request ID. It uses
the latest compatible request; duplicate usage replaces that measurement.

ContextPreparation passes the exact projected request boundary and retains a
Weak request reference until ModelProvider confirms the final object after
before transforms and image adaptation. Confirmation saves
`context-usage/request-base` before invocation. Changed or unproved requests
remain subject to the final complete-request budget check.

## Tests

[context_flow.rs](../../../../crates/dim-agent-capabilities/tests/context_flow.rs):
`persisted_usage_reads_and_restores_without_projecting_history`,
`unconfigured_usage_does_not_project_legacy_messages`,
`provider_usage_baseline_counts_new_history_and_ignores_unusable_measurements`.

<!-- generated:plugin ContextUsage — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:BranchLifecycle`、`B:Dispatcher`、`B:Timeline`、`S:ContextModelHost`、`S:HistoryRenderer` |
| 提供 | `B:ContextUsage` |
| 被注入于 | `ContextPreparation` |
<!-- /generated -->
