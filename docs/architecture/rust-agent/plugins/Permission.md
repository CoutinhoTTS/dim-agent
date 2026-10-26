# Permission

The Permission plugin registers two branch parts: Permission, which decides each call, and Approval, which admits it or asks the user.

## Permission

Owner: [permission.rs](../../../../crates/dim-agent-capabilities/src/permission.rs).
Evaluates tool risk, workspace write targets, explicit allow/deny policy and the user's command rules,
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

### Workspace write targets

Decision for [#294](https://github.com/arcships/dim/issues/294): under
`workspace-write`, a `write_filesystem` call runs automatically only when its
resolved `path` is inside the Session workspace. Every outside write asks for
approval for that call, including a model write to a host-owned output directory.
`allowedTools` does not bypass this question; `disallowedTools` still denies.
A write tool without an absolute target also asks because its workspace scope
cannot be established. `read-only` and `full-access` retain their existing rules.

The tool's owner resolves its target before admission. WorkspaceTools expands
home-relative paths using its own HostPlatform facts and applies its shared
path boundary: platform path handling, lexical components and canonicalization
of the longest existing prefix, including a new file's parent. ToolExecutor
passes that resolved call to both Approval and execution, so retargeting an
original symlink while approval is pending does not redirect the approved write.
The model's original tool-call fact stays unchanged. Permission canonicalizes
the workspace and target for its current boundary check. Compare path
components, not string prefixes. Resolution failures refuse the call; an unreadable prefix,
symlink loop or dangling symlink is not a missing directory to skip. I/O runs
outside the async executor. Read the dispatched policy after this I/O so a policy
change while it waits takes effect. Approval rechecks the path and policy before
an approved call starts, using its existing call-specific request and response.

There is no remembered path grant, output-directory exemption, new permission
preset, event or persistence field. Reads stay path-independent, including
Session attachments and command output. Host-generated output and configured
commands retain their own execution contracts under ADR 003. This is tool
admission, not an OS filesystem sandbox against concurrent filesystem mutation.

The CLI uses its existing approval prompt, with the resolved outside target in
the reason and no permanent-allow action for file tools:

```text
[approval] write: Writing outside the session workspace requires approval: /home/user/.zshrc
  path: /home/user/.zshrc
Allow once or deny? [y/N] _
```

### Rule check

- §1 and ADR 002 D1–D3: the existing branch Permission and Approval Capabilities
  own admission and approval facts. WorkspaceTools owns file execution; no
  plugin, wiring part, lifecycle or isolation dependency is added.
- §4 rules 1–6 and 9: existing approval requests/responses and tool results
  carry the decision and outcome; handlers retain their existing tasks,
  cancellation and cleanup. No new event or context input path is introduced.
- §4 rule 7: Permission reads its declared Session configuration; WorkspaceTools
  uses its existing HostPlatform dependency. The bound handler's `resolve_write`
  prepares the actual call at its external path boundary before admission; it
  reads metadata and performs no file mutation or process execution.
- ADR 002 D4–D5: policy state and public request/reply formats stay unchanged.
  Clients display the existing language-neutral approval request.
- §4 rule 8: no new Timeline, Dispatcher, loader or rutis mechanism is required.

### Requirements raised to mechanism owners

None. Existing admission, approval facts and host process facts suffice.

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

[local_product.rs](../../../../tests/integration/tests/local_product.rs) verifies
real workspace writes, per-call external write/edit approvals despite explicit
tool allows, outside reads, denied edits, host output targets, parent symlinks,
unresolvable targets and the unchanged read-only/full-access presets.

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
