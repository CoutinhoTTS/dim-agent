# AgentSettings

Status: Draft. Parent: [agent environment awareness](../agent-environment.md). Issue: [#1022](https://github.com/arcships/dim/issues/1022). Rules: [design rules](../../architecture/rust-agent/principles.md) §1, §4 and §5, [ADR 002](../../adr/002-plugin-scope.md), [ADR 003](../../adr/003-approval-scope.md), [ADR 009](../../adr/009-permission-decides.md), [settings service](../appserver/settings-service.md).

AgentSettings is the write counterpart of [EnvironmentSettings](EnvironmentSettings.md): the model can read a model-facing setting as state, and — only for the settings an owner explicitly allows — change it through a tool. It is not a general settings editor and not a second Settings store.

## 1. Scope and non-goals

Owns:

- the rule by which an owner declares a setting agent-readable or agent-writable;
- the model-facing tool contract for the allowed settings;
- the requirements a tool or CLI satisfies to write through the owner.

Does not own:

- the settings documents, their schemas, CAS or validation; those stay with each owner and the Settings service;
- permissions, approvals or grants; those stay with [ADR 003](../../adr/003-approval-scope.md), [ADR 009](../../adr/009-permission-decides.md) and Permission;
- structural plugin configuration (enable/disable, runtime, installation); that is PluginManager;
- reading values into model context; that is [EnvironmentSettings](EnvironmentSettings.md).

## 2. Options

| Option | Form | Strength | Cost |
| --- | --- | --- | --- |
| A. Per-owner tools | Each allowlisted setting's owner registers a tool with `toolRegistry` (Session) or a management method (process) | Owner keeps its schema, validation and wording; the tool is the allowlist; follows the existing tool route | Tool count grows with the allowlist |
| B. One generic `settings` tool | One tool with `op: list/get/update`, `source`, `scope`, `values`; a registry of allowed sources/fields | One schema; discoverable | Needs a second allowlist registry and a dispatcher to owner writers; risks looking like an arbitrary settings editor |
| C. Built-in CLI + Skill | `dimcode settings ...` run through the shell tool; a Skill explains the commands | Small prompt footprint; batch/scripting | Depends on shell and its approval; output parsing; the CLI must still use the same owner path; no shell means no access |

**Decision for v1: A, per-owner tools, for a small allowlist.** The owner already owns the value, its validation and its wording; a per-owner tool adds no central registry and no second interpretation of a settings document. A generic `settings` tool is rejected for v1: it either exposes arbitrary documents or needs the same allowlist registry plus a writer dispatcher, and it invites the model to treat settings as an API surface. The CLI + Skill is a later adapter for batch or inspection if a concrete need appears; it is not the first path and never edits a file directly.

The decision is about the first path, not a permanent exclusion: if the allowlist grows enough that tool declarations dominate the prompt, a CLI + Skill over the same owner methods can replace the tools, but not both at once.

## 3. Allowlist

### 3.1 Who decides

Each setting's owner decides:

- `read`: the effective value may be shown to the model (usually already a [EnvironmentSettings](EnvironmentSettings.md) source);
- `write`: the model may change it through a tool;
- `none`: neither; the default.

The owner states the reason in its own design or page. An owner that does not declare a setting leaves it at `none`. There is no inferred allowlist from "the model can already write the file" and no allowlist from a Settings schema.

### 3.2 Defaults and exclusions

Default `none`. The following are `none` unless a separate decision changes them:

| Setting | Why |
| --- | --- |
| Execution `sandbox` / `network` | A safety boundary; a model that can widen it defeats the boundary ([ADR 011](../../adr/011-execution-settings.md)) |
| Permission preset, approval rules, command rules | Self-escalation; Permission is the decision point |
| Session grants | A grant is a user action ([ADR 012](../../adr/012-session-grants.md)) |
| Hooks, MCP server configuration, plugin enablement | Code or tool execution configuration; not a model preference |
| Remote pairing, device records | Host security material |
| Credentials, tokens, `writeOnly` fields | Never readable or writable by the agent |
| TUI-only preferences | The model does not act on them |

A setting that is merely inconvenient for the agent is not automatically writable. The owner has to want the agent to change it.

### 3.3 First candidates

These are candidates for an owner decision, not an approved list:

- per-session model, reasoning effort and fast mode — a session preference the user can already change; a tool needs approval and a clear effect on the next request;
- subagent model policy — affects delegated work; the owner states whether a mid-session change is allowed;
- a plugin's own business setting only if it has no security boundary and no secret, and its owner provides a typed action.

The implementation PR names the first owner and its reason; this design does not open a setting on the owner's behalf.

## 4. Tool contract

A per-owner tool uses the existing `toolRegistry` and `ToolContribution`, so it is declared, admitted and executed like every tool.

| Element | Rule |
| --- | --- |
| Name | Owner's, `[A-Za-z0-9_-]{1,64}`; a new name, not an alias of a method |
| Description | States what changes, the scope, when it takes effect, and that it is an owner-validated write |
| Input | The owner's schema; a closed object; no free-form document and no path |
| Result | A short text stating what changed and when it takes effect; structured fields for persistence/UI. The result is not a substitute for the [EnvironmentSettings](EnvironmentSettings.md) state |
| Read | A `get` form may return only the allowlisted effective fields; never the whole document, a secret or a `writeOnly` member |
| Errors | `invalid`, `not_writable`, `unknown`, `conflict`, `closed`; a refused write changes nothing |
| Approval | The tool's risk category and Permission decide; a write is an agent action, not a configuration that runs by itself |
| Cancellation | The tool's existing cancellation; a cancelled write leaves the previous value |

The tool does not take a file path, a JSON Pointer into an arbitrary document, or a raw Settings document. It takes the fields the owner declared.

## 5. Write path

1. The tool validates the input shape and calls the owner's typed action.
2. The owner validates against its schema and writes through the Settings service (CAS on the revision) or its own typed store.
3. The committed write emits the existing `settings.changed` for a Settings source; the [EnvironmentSettings](EnvironmentSettings.md) source adopts the new effective value before the next request.
4. The tool replies with the new revision and the effect. The model sees the state block on its next request; it does not assume the write succeeded from its own text.
5. No plugin reload. A structural change (enable/disable, runtime, install) is not this path.
6. A concurrent write by the user or another client yields `conflict` with the current revision; the tool does not overwrite.

The owner's write is the only authority. A CLI later uses the same typed action; it never edits `plugins.json`, a settings file or the Timeline directly.

## 6. CLI + Skill (later)

If a batch or inspection need appears, a built-in CLI may offer `list`, `get` and `update` over the same owner actions. Requirements:

- It uses the same Settings/owner validation and CAS; no direct file writes.
- It reports the same revision and effect; JSON output for scripting.
- It is admitted by the shell tool's existing permission and approval; it is not a second authority.
- A Skill explains the commands, the allowlist and the effect. A Skill is a tool result, not a permission; loading it grants nothing.
- A Session without a shell does not get the CLI path; it fails explicitly, not silently.
- The tool path and the CLI path are not both offered for the same setting by default: one first path per setting.

## 7. Verification

| Scenario | Expected |
| --- | --- |
| Read an allowed setting | The allowlisted effective fields; no secret |
| Read a disallowed setting | `not_writable` / `unknown`; not an empty value |
| Write an allowed setting | Approval/Permission applies; the owner validates; the result states the effect; the next request carries the new state; no reload |
| Write sandbox, permission, a grant or a token | Refused; nothing written |
| Write with a stale revision | `conflict`; the current value is not overwritten |
| Two clients write concurrently | One wins; the other gets a readable conflict |
| Cancel during a write | The previous value remains |
| A non-model field of the same document | The tool cannot change it; the state block does not move |
| CLI without a shell | Explicit failure |
| Secret | Never in schema, result, CLI output, log or model context |

## 8. Rule check

| Rule | How this design complies |
| --- | --- |
| §1 Parts bounded | The tool belongs to the setting's owner; the value stays in the owner's source. |
| §4 rule 1 Command events | The owner writes its own state; the tool does not write another module's state. |
| §4 rule 2 Handover | No handover. |
| §4 rule 3 Before callbacks | None. |
| §4 rule 4 Core carries no business meaning | No core branch; owner tools and Settings validation. |
| §4 rule 5 Batch | A settings write is one CAS write; no Timeline batch. |
| §4 rule 6 Context from consumed events | The tool result enters as a tool result; the effective value enters as the state input from [EnvironmentSettings](EnvironmentSettings.md). |
| §4 rule 7 Public services | The owner's typed write; tools registered with `toolRegistry`; no second container. |
| §4 rule 8 Infrastructure unchanged | No Timeline, Dispatcher, envelope, assembly or facade change. |
| §4 rule 9 Timeline holds decision facts | Configuration does not enter the Timeline; a tool result is enough for the model, `settings.changed` for listeners. |
| §4 rule 10 Adopted inputs | The state is an adopted input; the tool result is separate. |
| ADR 002 D1 | The setting's owner owns the write; the tool is a Session entry. |
| ADR 002 D2 | No adapter plugin; each owner's tool is its own. |
| ADR 002 D3 | The tool depends on the owner's service; no core depends on the tool. |
| ADR 002 D4 | Business configuration stays with its owner; no loader row. |
| ADR 002 D5 | The tool input/result and the owner action are data; Rust is a binding. |
| ADR 003 / ADR 009 | The write is an agent action admitted by Approval and Permission; no configuration-time bypass. |

## 9. Open decisions

1. The first owner and setting to open; this design does not choose for the owner.
2. Whether a read-only `get` is a tool, a state source, or both; the state source is the default.
3. Whether the allowlist later moves to one generic `settings` tool.
4. Whether a CLI + Skill is needed at all, and for which batch/inspection case.
5. The risk category of an allowed settings write and the approval wording.
