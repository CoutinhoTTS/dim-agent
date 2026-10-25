# Permission

The Permission plugin registers two branch parts: Permission, which decides each call, and Approval, which admits it or asks the user.

## Permission

Owner: [permission.rs](../../../../crates/dim-agent-capabilities/src/permission.rs).
Evaluates tool risk, explicit allow/deny policy and the user's command rules,
and exposes versioned branch-local permission state.

`PermissionState` is serializable for read-only generated host exports. Agents'
`sessionPermissions.permission` reads this same dispatched state through the
Session facade and holds its Session lease; it adds no Permission dependency,
cache or event.

### Dependencies

| Service | Purpose |
| --- | --- |
| `S:SessionConfig` | Seed the initial permission policy. |
| `S:CommandRulesHost` | Read the user's command rules when deciding a command call. |
| `B:Dispatcher` | Replay local policies, apply committed policy changes and settle update waiters. |
| `B:Timeline` | Write changes and read ancestor policies. |

### Events and lifetime

Produces and consumes `permission/changed`. Updates become effective when
dispatched, not merely appended. Replay starts with Session policy, folds the
fixed ancestor prefix and then local changes. No background tasks. Cleanup
closes admission and drops pending waiters, which report Closed. Reload builds
a new state cell from history. Policy is admission logic, not OS isolation;
explicit disallow overrides allow.

Command rules are read on each decision of an `execute_process` call with a
string `command`, never cached, stored or replayed. A matching deny rule denies
under every preset; a matching allow rule only turns an approval question into
an allow, for a command without shell separators; a failed read denies the
command call. The policy is taken after the read, so a change dispatched while
it waits decides the call. See [permission command rules](../../../design/appserver/permission-command-rules.md).

### Tests

[permission_flow.rs](../../../../crates/dim-agent-capabilities/tests/permission_flow.rs):
`presets_decide_by_risk_and_denied_calls_do_not_run`,
`tool_lists_override_presets_and_disallowed_wins`,
`updates_are_versioned_per_branch_take_effect_when_dispatched_and_survive_restore`,
`allow_rules_answer_approval_only_for_simple_commands_within_the_preset`,
`deny_rules_match_any_segment_under_every_preset_and_are_read_at_use_time`,
`unreadable_rules_deny_command_calls_only`,
`a_policy_tightened_during_the_rules_read_decides_the_call`.

<!-- generated:plugin Permission — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:Dispatcher`、`B:Timeline`、`S:CommandRulesHost`、`S:SessionConfig` |
| 提供 | `B:Permission`、`B:StateCell<PermissionState>` |
| 被注入于 | `Approval` |
<!-- /generated -->

## Approval

Owner: [approval.rs](../../../../crates/dim-agent-capabilities/src/approval.rs).
Turns permission decisions into call admission or durable approval requests and
rechecks policy before allowing an approved call to start.

### Dependencies

| Service | Purpose |
| --- | --- |
| `B:Permission` | Evaluate current policy and recheck an allowed response. |
| `B:Dispatcher` | Replay requests to recover abandoned ones, then receive approval responses. |
| `B:Timeline` | Commit requests/cancellations. |

### Events and lifetime

Produces `approval/request` and `approval/cancelled`; consumes `approval/respond`.
Caller cancellation settles its waiter; branch close/reload releases remaining
requests. Recovery cancels unmatched historical requests because their execution
did not survive. The service owns request state, not tool execution tasks.
Responses are not unconditional authority: cancelled batches and changed policy
can prevent execution. Approval does not sandbox an admitted tool.

### Client notifications

Rpc projects the three events at outbound encoding; none reaches `session.event`
or `session.history`. `approval/request` → `approval.request`, also replayed on
attach for pending requests; `approval/cancelled` → `approval.cancelled`;
`approval/respond` → `approval.resolved {sessionId, branchId, requestId, decision}`
(protocol v3 only: the v1 and v2 notification sets are frozen,
[#278](https://github.com/arcships/dim/issues/278)).
`approval/respond` is the command, so every committed response is projected.
Approval accepts the first one it consumes in sequence order: the first
`approval.resolved` per `requestId` carries the accepted decision, a later one
was answered `Unknown`, and an `approval.cancelled` after it means the call did
not start. The decision is the user's answer, not the tool's outcome. A client
that attaches after a response is committed but before Approval consumes it is
replayed the request without that response; its own `approval.respond` then
fails with `invalid_params`, which ends the prompt.

### Tests

[permission_flow.rs](../../../../crates/dim-agent-capabilities/tests/permission_flow.rs):
`an_allowed_approval_is_rechecked_against_the_current_policy`,
`a_response_that_finds_its_batch_stopped_ends_as_an_interrupt`,
`a_deny_rule_written_while_approval_is_pending_cancels_the_allowed_call`;
[local_product.rs](../../../../tests/integration/tests/local_product.rs).

<!-- generated:plugin Approval — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:Dispatcher`、`B:Permission`、`B:Timeline` |
| 提供 | `B:Approval` |
| 被注入于 | `ToolExecutor` |
<!-- /generated -->
