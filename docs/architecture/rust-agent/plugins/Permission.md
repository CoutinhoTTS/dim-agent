# Permission

The Permission plugin registers two branch parts: Permission, which decides each call, and Approval, which admits it or asks the user.

## Permission

Owner: [permission.rs](../../../../crates/dim-agent-capabilities/src/permission.rs).
Evaluates tool risk, the fixed file-access facts, explicit allow/deny policy and
the user's command rules, and exposes versioned branch-local permission state.
The facts service is owned by the Session part in
[file_access.rs](../../../../crates/dim-agent-capabilities/src/file_access.rs)
([ADR 014 D1](../../../adr/014-file-access-facts.md#d1-fixed-facts)).

`PermissionState` is serializable for read-only generated host exports. Agents'
`sessionPermissions.permission` reads this same dispatched state through the
Session facade and holds its Session lease; it adds no Permission dependency,
cache or event.

### Dependencies

| Service | Purpose |
| --- | --- |
| `S:SessionConfig` | Seed the initial permission policy. |
| `S:CommandRulesHost` | Read the user's command rules when deciding a command call. |
| `S:SessionIdentity` | Name the Session in the facts query. |
| `B:Dispatcher` | Replay local policies, apply committed policy changes and settle update waiters. |
| `B:Timeline` | Write changes and read ancestor policies. |

`S:FileAccessFacts` is read optionally for a file write; a composition without
the App Server facts part decides a write by the workspace alone. It is not an
injected dependency, so its absence does not gate the part.


### Events and lifetime

Produces and consumes `permission/changed`. Updates become effective when
dispatched, not merely appended. Replay starts with Session policy, folds the
fixed ancestor prefix and then local changes. No background tasks. Cleanup
closes admission and drops pending waiters, which report Closed. Reload builds
a new state cell from history. Policy is admission logic, not OS isolation;
explicit disallow overrides allow.

### Model-facing texts

`Command denied by command rule …`: the refusal an admitted-and-denied call
returns, and it is protocol
([model-facing prompts §4](../../../design/model-prompts.md#4-protocol-and-policy)).
The text names the rule and what it matched, which the model uses to change the
command rather than retry it; `Command rules could not be read: <error>` is
protocol too, because it says the rules were not applied and a retry may be
decided differently.

### File-access facts

[ADR 014](../../../adr/014-file-access-facts.md) fixes what a file operation is
decided from. The `FileAccessFacts` Session part registers `fileAccessFacts`
with `facts(session_id, preset, path, mode) -> FileFacts`: `workspace` from
`SessionConfig`, `system` from HostPlatform's platform roots, `owned` from
`SessionFiles.owns`, and `grant` from the Session's grants. The part holds no
state; it reads at use time and imports no SessionStore. A query failure denies.

For a write, Permission resolves the call's path to the same canonical target
the tool owner admitted and calls the service with the branch preset and
`write`. It uses WorkspaceTools' `resolve_path` for both absolute and relative
targets, so cwd, `~`, `..` and symlinks are settled before the query; a
composition without HostPlatform cannot expand `~` or join a relative target
and keeps the fallback rule below. A system root or Session-owned artifact is
denied, not asked, under `read-only` and `workspace-write`, including when
`allowedTools` names the tool. A `read-only` write is otherwise denied by its
preset; `full-access` admits file-tool writes everywhere without a query.
`workspace-write` allows a workspace target, allows a target covered by a write
grant, and asks otherwise. A write grant is inert under `read-only`; a read
grant never permits a write. A composition without the facts service keeps the
workspace-only rule below. Reads are unchanged until
[ADR 014 D2/D4](../../../adr/014-file-access-facts.md#d2-read-and-write) lands
for them.

### Workspace write targets

Decision for [#294](https://github.com/arcships/dim/issues/294): under
`workspace-write`, a `write_filesystem` call runs automatically only when the
facts say its resolved `path` is inside the Session workspace. Every outside
write asks for approval for that call, including a model write to a host-owned
output directory. `allowedTools` does not bypass this question;
`disallowedTools` still denies. A write tool without a path, or with a relative
target in a composition without HostPlatform, asks because its workspace scope
cannot be established. This is the fallback rule when the facts part is absent.

The tool's owner resolves its target before admission. WorkspaceTools expands
home-relative paths using its own HostPlatform facts and applies its shared
path boundary: platform path handling, lexical components and canonicalization
of the longest existing prefix, including a new file's parent. ToolExecutor
passes that resolved call to both Approval and execution, so retargeting an
original symlink while approval is pending does not redirect the approved write.
The model's original tool-call fact stays unchanged. Permission applies the same
resolver to a call it receives unresolved, and the facts compare the workspace,
system roots, owned artifacts and grants against that canonical target. Compare
path components, not string prefixes. Resolution failures refuse the call; an
unreadable prefix, symlink loop or dangling symlink is not a missing directory
to skip. I/O runs outside the async executor. Read the dispatched policy after
this I/O so a policy change while it waits takes effect. Approval rechecks the
path and policy before an approved call starts, using its existing
call-specific request and response.

### Session grants

[ADR 012](../../../adr/012-session-grants.md) stores the user's
"Allow for this session" answers in `session_grants`, owned by the App Server
`SessionGrants` part. The facts service reads them and matches the resolved
target: a file grant matches the exact path, a directory grant matches by
component, and a write requires a write grant. Permission applies no grant by
itself, so there is one path resolver. A grant read failure denies; a cancelled
read propagates. No grant becomes a Timeline fact, preset or
output-directory exemption. Reads stay path-independent under the current
presets, including Session attachments and command output; ADR 014's outside
read ask will use the same `grant` fact. Host-generated output and configured
commands retain their own execution contracts under ADR 003. This is tool
admission, not an OS filesystem sandbox against concurrent filesystem mutation.

The CLI uses its existing approval prompt, with the resolved outside target in
the reason and a Session-scoped grant action for file tools:

```text
[approval] write: Writing outside the session workspace requires approval: /home/user/.zshrc
  path: /home/user/.zshrc
› Allow once   Allow for this session   Reject   Full access
```

### Rule check

- §1 and ADR 002 D1–D3: the existing branch Permission and Approval Capabilities
  own admission and approval facts. WorkspaceTools owns file execution; no
  plugin, wiring part, lifecycle or isolation dependency is added. The grant
  store and `session.grants.add/remove` belong to the App Server
  `SessionGrants` Session part (ADR 012), and the fixed facts belong to the
  `FileAccessFacts` Session part (ADR 014 D1); Permission only reads them.
- §4 rules 1–6 and 9: existing approval requests/responses and tool results
  carry the decision and outcome; handlers retain their existing tasks,
  cancellation and cleanup. No new event or context input path is introduced.
- §4 rule 7: Permission reads its declared Session configuration, command rules,
  Session identity and the optional facts service; WorkspaceTools uses its
  existing HostPlatform dependency. The bound handler's `resolve_write`
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
`a_policy_tightened_during_the_rules_read_decides_the_call`,
`a_composition_without_facts_builds_and_asks`,
`file_write_facts_decide_each_path_by_preset`,
`a_session_grant_matches_only_the_canonical_target_and_mode`.

[session_files_flow.rs](../../../../crates/dim-app-server/tests/session_files_flow.rs)
checks `sessionFiles.owns` against the attachment owner rows.

[session_grants.rs](../../../../crates/dim-app-server/tests/transport_flow/session_grants.rs)
covers the `session.grants.add/remove` methods, a write grant admitting the
next outside write without an approval request, and its removal making the
next write ask.

[local_product.rs](../../../../tests/integration/tests/local_product.rs) verifies
real workspace writes, per-call external write/edit approvals despite explicit
tool allows, outside reads, denied edits, host output targets, parent symlinks,
unresolvable targets and the unchanged read-only/full-access presets.

<!-- generated:plugin Permission — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 分支 |
| 注入 | `B:Dispatcher`、`B:Timeline`、`S:CommandRulesHost`、`S:SessionConfig`、`S:SessionIdentity` |
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

### Model-facing texts

`Tool execution denied by user` is the result of a call the user denied, and it
is protocol
([model-facing prompts §4](../../../design/model-prompts.md#4-protocol-and-policy)):
the model recognises it as a denial and does not retry the call.

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
