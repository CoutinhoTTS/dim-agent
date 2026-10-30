# 011: Execution settings

Decision status: Proposed.

Extends [ADR 010](010-sandbox-opt-in.md): the sandbox is opt-in. This decision completes the
storage and management ADR 010 defers, and qualifies its “the model cannot widen the sandbox”
sentence. ADR 003 is unchanged.

## Context

ADR 010 fixes the default, the two switches and the `/settings` surface. The switches still need
one owner, schema and read path. Main carries the old isolation policy surface; the new one must
not become another required file.

## Decision

### D1. One declared source, one read path

The App Server's Settings service registers a built-in declared source `execution`, user scope
only, with two boolean fields: `sandbox` and `network`, both `false`. It has the source's CAS
revision and is read and written through `settings.read` and `settings.update`; the `/settings`
Execution row uses those methods. It has no project file.

`HostPlatform` does not name the Settings service. A read-only host port
`ExecutionSettingsHost` returns the two values; the App Server provides it and passes it through
`HostPlatformConfig` at process start. This is the requirement raised under §4 rule 8. The
in-process reader uses the named `settings` service; the client management methods are
`settings.read`/`settings.update`.

A two-boolean HostPlatform file such as `exec-shell.json` is not enough: the switch must be one
user-scope, revisioned value that `/settings`, web, ACP and paired clients read from the same
daemon, and it must go through the Settings surface those clients already use.

### D2. Full access may write the source

ADR 010 says the model cannot widen the sandbox by calling a tool. This decision narrows that
sentence to the `exec` argument: a full-access model can write the `execution` source through
file tools or a shell, including the native SQLite store, and the next process uses the new
value. That is the accepted exception. A user who wants a boundary does not use `full-access`.

## Consequences

- `host.isolation.get/update/reset`, `isolation_policy.rs`, `IsolationPolicy`,
  `host-isolation.json` and the Command isolation row are removed by ADR 010; this decision adds
  the `execution` source, the Execution row and the host port that replaces them.
- `settings-service.md`, `crates/dim-cli/README.md`, `docs/design/cli-commands.md`, the
  `HostPlatform.md` page, `dev`, the Chinese CLI mirror, `host_platform.rs` and the F5 design are
  updated under their owners after acceptance. `readOnlyPaths`, `writablePaths` and `environment`
  have no replacement.
- An existing `host-isolation.json` is not read; ADR 010's startup diagnostic is the visible
  channel, and the user removes the file.
- No Timeline event or wire field is added. The source is business configuration.

## Non-goals

- Session grants, command-output ownership and file-access facts.
- A project-scoped execution source.
- Migration from the old file.

## Supersession

Extends [ADR 010](010-sandbox-opt-in.md) and completes its deferred storage and management. It
qualifies ADR 010 D2 for a full-access source write; the two decisions land together. ADR 003
remains as written.

## Rule check

| Rule | Application |
| --- | --- |
| §1, ADR 002 D1 | The source is App Server settings; `HostPlatform` remains a process part and reads the host port. No part moves. |
| §4 rule 1 | The source is configuration with typed `settings.read/update`; no command event. |
| §4 rule 2 | The source applies to the next process; an admitted call is unchanged. |
| §4 rule 3 | No `before` callback. |
| §4 rule 4 | No core mechanism change. |
| §4 rule 5 | No Timeline batch. |
| §4 rule 6 | No context-input change. |
| §4 rule 7 | `HostPlatform` reads its host port; the App Server reads the named `settings` service. |
| §4 rule 8 | **Raises a host-port requirement**: `ExecutionSettingsHost`, provided by the App Server, returns the two values to `HostPlatform`. |
| §4 rule 9 | No new Timeline fact. |
| ADR 002 D2 | No wiring plugin. |
| ADR 002 D3 | The source is user-scoped; `HostPlatform` depends on the host port. |
| ADR 002 D4 | Business configuration, read at use time. |
| ADR 002 D5 | The client surface is `settings.read/update`; the in-process surface is the named `settings` service and `ExecutionSettingsHost`. |
