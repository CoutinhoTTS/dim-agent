# Subagent model policy

Status: **Proposed**, not implemented. Business owner: Collaboration.
Scope: the model-policy part of [#263](https://github.com/arcships/dim/issues/263)
and [the feature list](../feature-list.md#providers-and-models).

The confirmed scope applies inherit/fixed policy to **every newly created native
member Session**, including a named Agent such as `reviewer`. Existing member
Sessions retain their configuration. The concrete configuration and interfaces
below require architecture review before implementation.

## 1. Behavior

| Policy | Model for a new native member |
| --- | --- |
| `agent` (default) | Existing behavior: use the temporary/named Agent's profile model, or its existing host/workspace default when the profile has none |
| `inherit` | The model selection currently adopted by the leader's **requesting branch**, including its model options and metadata |
| `fixed` | Resolve the configured `{providerId, modelId}` through the same model-selection path as an explicit Agent profile model |

Inherit/fixed takes precedence over both a temporary `profile.model` and a named
Agent's stored model. For example, a leader using A with an inherited policy
creates a `reviewer` member using A even when that Agent's profile names B.
The stored Agent profile is unchanged; the choice belongs to the new Session.

A request addressed to an existing `agentId + sessionId` does not create a member
and does not apply this policy. Changing policy or the leader's model never
reconfigures existing members. Provider-owned/external members retain their own
contract; this policy does not interpret their opaque identities or set their models.

The default preserves current model selection. It introduces no additional model
decision by an LLM. `agent` means the existing request/profile selection behavior.

## 2. Use existing mechanisms

| Need | Existing mechanism |
| --- | --- |
| Own the decision | `Collaboration::establish`, reached for both named and temporary new members |
| Identify the requesting branch | `CollaborationCapability::request` already receives `branch_id`; pass it within Collaboration, without changing an event or tool request |
| Read the adopted leader model | `Session::get<PromptSnapshot>(branch_id)`; F3 already uses this public read in `connection.rs` |
| Resolve an explicit model | The existing `AgentIndex` model-selection host and configured/non-catalog profile paths |
| Persist and edit policy | Native `BusinessDocument`, `SettingsService::read/update`, source CAS and owner management methods, as used by `permission_rules.rs` |
| Start/restore and retain members | Existing Agents/Collaboration Session creation, usage holds and cleanup, including idle unloading |

Do not use `leader.config().model` or `session.list` as the adopted leader model:
they describe creation configuration rather than a subsequent F3 adoption.
The inherited selection consists only of the existing `ModelSelection` fields:
provider/model IDs, capability snapshot, model config and request metadata.
The member keeps its own prompts, tools and existing permission/workspace behavior.
No leader tool table or prompt content is copied.

Inheritance preserves existing model options and metadata, including any Provider
route stamp or its absence. It does not resolve a different default model or read
Provider's saved client preferences. Fixed/agent selection also retains its current
profile-resolution semantics; saved client preferences are not implicitly applied.
Actual execution still checks availability through the existing model owner.

## 3. User configuration and management

Register one native Settings source, namespace `collaboration`, with
`USER_FILE = "collaboration.json"` and no project file. Its document is the policy
itself, not another configuration wrapper:

```json
{"mode":"agent"}
```

```json
{"mode":"inherit"}
```

```json
{"mode":"fixed","model":{"providerId":"provider-id","modelId":"model-id"}}
```

This is one tagged type. Reject unknown fields/modes, a missing fixed model,
empty IDs, and a model supplied with another mode at the external boundary.
A missing source uses `agent`; malformed persisted data fails the read and is
not rewritten as a default. Existing Settings storage chooses native persistence
or its file-backed path; there is no new database table, store or migration.

Project files and tool arguments cannot set the policy. Use the local management
boundary for this user setting, not the generic `settings.*` methods, which do not
serve Rust `BusinessDocument` sources.

The existing Collaboration process plugin provides these methods under
`managementMethods:collaboration` when its Settings source is configured:

| Method | Request | Reply | Mutating | Interruption |
| --- | --- | --- | --- | --- |
| `collaboration.model_policy` | `{}` | `{revision: string, policy: Policy}` | false | `Report` |
| `collaboration.set_model_policy` | `{expectedRevision: string, policy: Policy}` | `{revision: string}` | true | `Report` |

`Policy` is exactly the tagged source type above. Use the existing local v3
management authorization; a paired device has no access. Invalid request shape
or policy is `invalid_request`; source CAS conflict is
`collaboration_model_policy_conflict`; source closure is `service_reloading`;
cancellation is `operation_interrupted`; other read/persistence errors are
`collaboration_model_policy_failed`. Follow the existing Settings/Permission
owner error handling. A write error can leave its outcome uncertain: reread
before another CAS, without automatically replacing the user's selection.

App Server configurations without a product/execution Settings root continue
using existing `agent` behavior and expose no policy management methods.
Collaboration must not gain an unconditional Settings dependency: declare the
real rooted-host dependency using existing configuration-dependent `When`/`Deps`,
and obtain it from `Deps`. The composition root reserves this source beside the
other native sources and registers its management owner only in that configured
case. This needs no new user enable flag, optional lookup loop or wiring plugin.
When configured, a Pending Settings provider follows normal dependency waiting;
it is not treated as an absent source or a reason to skip policy.

## 4. Creation and lifetime

For each new native member, read one Settings snapshot in Collaboration's
existing owned request task. Do not cache policy or subscribe to configuration
changes: only new creation uses it. Resolve `agent`/`fixed` through the existing
profile paths; for `inherit`, read the requesting branch's adopted Prompt snapshot
and use its model selection in the new Session configuration.

If the configured source, adopted snapshot or requested model cannot be read or
resolved, use the existing `member_create_failed` result and temporary-Agent
cleanup. Do not silently use another model. Keep existing visibility checks,
Session admission holds, cancellation, terminal facts and Agent cleanup.

The sample governs that creation even if policy or the leader's model changes
before Session publication. A later creation samples again; two concurrent
creations may therefore use different snapshots. There is no cross-owner atomic
check, retry fence or guarantee that an already sampled creation uses a later
edit. Each member's existing `context/configured` records what it actually adopted.
Read-only detail/cards display that observed model, not a guess from current policy.

## 5. Native CLI interaction

Add one action to the existing `/models` picker. It edits policy, not the leader's
current model. The fixed option reuses the model picker with a policy-selection
purpose; choosing it must not send the leader's F3 model update.

```text
Select a model                  Subagent models
  Current provider / model        Agent choice (default)
  Other provider / model          Inherit requesting branch
  Edit models                     Fixed model > select from existing picker
  Subagent models: Agent choice

Applies to new members, including named Agents.
Existing members keep their models.
```

Read the source revision on opening and after a successful own write. On conflict,
keep the selection visible and require reread before another save. Cancel performs
no write. A failed/empty fixed-model listing leaves the existing policy intact.
Hosts without the advertised management methods omit this action; no legacy or
in-memory write fallback is added. No independent settings screen or second
command route is required for this TUI scope.

## 6. Implementation and acceptance

The implementation touches Collaboration's process plugin and existing creation
path, its native Settings registration/management owner, and the CLI model picker.
Prompt, Provider, Agents' public contracts, Rpc dispatch, event types and rutis
remain unchanged. Register acquired effects immediately and settle existing owned
work before unload. Regenerate architecture output when dependency/assembly
registration changes, using `xtask graph`; do not hand-edit generated artifacts.

Extend existing Collaboration/native CLI public flows to cover:

- Named and temporary new members under all three modes; inherit/fixed overrides
  a conflicting profile model, while `agent` preserves current defaults.
- A requesting child branch and a later F3 adoption: inheritance uses that
  branch's actual model/options/metadata, preserves member prompts/tools, and
  does not reconfigure an existing member on reuse or after a policy edit.
- Settings persistence/restart, two-client CAS, invalid source preservation,
  missing fixed models and source reload failure without model fallback or orphan
  temporary Agents. Unrooted hosts retain current behavior and assembly.
- Actual CLI selection, cancellation and conflict, confirming that policy editing
  leaves the leader's adopted model and draft intact.

These are implementation acceptance requirements, not completed runtime evidence.
Native transcript cards still follow their separate integration dependency on
[#324](https://github.com/arcships/dim/pull/324).

## 7. Requirements raised to mechanism owners

None. The public Prompt snapshot, model-selection host, native Settings CAS and
management binding already provide the required operations. This design requests
no core, Prompt, Provider, Settings, Rpc or rutis mechanism extension.

## 8. Rule check

| Rule | Application |
| --- | --- |
| §4.1 | Collaboration chooses a model only for the Session it is creating; other owners' state is unchanged |
| §4.2 | Policy edits affect later creation samples; accepted work and existing Sessions are not revoked |
| §4.3 | No before callback; validate policy at its external boundary, perform I/O in the owned request task |
| §4.4 | No business branch in core, Dispatcher, Rpc or loader |
| §4.5 | Settings owns the policy CAS; member configuration uses existing Session creation without a cross-owner transaction |
| §4.6 | Only existing adopted model configuration reaches context; management replies are not injected |
| §4.7 | Read Prompt through public Session state and resolve models through the existing host; no private state access |
| §4.8 | Existing mechanisms suffice; no new port or speculative concurrency guarantee |
| §4.9 | No policy Timeline event, notice stream or duplicate model history; existing configuration facts identify the adopted model |
| §1 | Extend Collaboration at its process scope; no extra Capability or policy plugin |
| ADR 002 D1 | User policy belongs to Collaboration; Prompt/model execution and clients retain their own state |
| ADR 002 D2 | Existing Settings and management registration, no adapter or wiring module |
| ADR 002 D3 | Root-dependent Settings is declared in Deps; unrooted hosts and normal Pending behavior are preserved |
| ADR 002 D4 | Business policy is a native user source, never loader-row configuration |
| ADR 002 D5 | Tagged JSON, method fields, errors and interruption behavior are language-neutral |

## 9. Ablation review

One tagged document is necessary to persist the three user choices; no containing
configuration type is needed. Two owner methods provide the read and CAS write
that generic Settings methods intentionally do not expose for native sources.
One picker action provides the requested TUI control. The existing creation path
and Prompt snapshot supply the behavior without a cache, listener, per-Agent
override map, new Timeline fact, model-policy service or additional lock.
