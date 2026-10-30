# 011: Execution settings

Decision status: Proposed.

Extends [ADR 010](010-sandbox-opt-in.md): the sandbox is opt-in. This decision gives the two
switches one owner, schema and management surface, and removes the old isolation policy file.
ADR 003 is unchanged.

## Context

ADR 010 fixes the default and the user surface, not the storage. Main still carries
`host.isolation.get/update/reset`, `isolation_policy.rs`, `IsolationPolicy`, `host-isolation.json`
and the `/settings` Command isolation row. Those manage the wrong policy: a required file with a
tier and an escalation rule.

## Decision

### D1. The switches are one declared Settings source

The App Server's Settings service owns a declared source `execution`, user scope only, with two
boolean fields: `sandbox` and `network`, both `false`. It has the source's CAS revision and is
read and written through `settings.read` and `settings.update`. `/settings` has an Execution row
over it. It has no project file, so a cloned repository cannot turn the network on.

The daemon reads the source when a Session or process starts. A running process keeps its
environment.

### D2. The old policy is removed

`host.isolation.get/update/reset`, `isolation_policy.rs`, `IsolationPolicy` and
`host-isolation.json` are removed. The `/settings` Command isolation row is replaced by the
Execution row. There is no node policy file and no `host-platform.json`.

An existing `host-isolation.json` is not read. The client shows a startup diagnostic while the
file exists: it is no longer used and the sandbox is off. No migration is kept.

### D3. No backend means no sandbox

On a platform with no sandbox backend (Windows, or Linux without `bwrap`), turning the sandbox on
fails as `isolation_unavailable`. The user leaves it off or installs the backend.

### D4. `full-access` can write the source

A full-access model can write the `execution` source through the file tools. That is the accepted
trust consequence: full access is not a sandbox boundary. A user who wants a boundary does not
use `full-access`.

## Consequences

- The switch has one storage, one schema, one revision and one `/settings` surface.
- `settings-service.md`, the CLI settings panel, `host_platform.rs`'s model-facing text and the
  F5 isolation design are updated under their owners after acceptance.
- No Timeline event, wire field or event fixture is added. The source is business configuration.

## Non-goals

- Session grants, command-output ownership and file-access facts.
- A project-scoped execution source.
- Migration from the old file.

## Rule check

| Rule | Application |
| --- | --- |
| §1, ADR 002 D1 | The Settings source is App Server configuration; `HostPlatform` remains a process part and reads it. No part moves. |
| §4 rule 1 | The source is configuration with typed `settings.read/update`; no command event. |
| §4 rule 2 | The source applies to the next process; an admitted call is unchanged. |
| §4 rule 3 | No `before` callback. |
| §4 rule 4 | No core mechanism change. |
| §4 rule 5 | No Timeline batch. |
| §4 rule 6 | No context-input change. |
| §4 rule 7 | Parts read the source through the Settings service. |
| §4 rule 8 | No infrastructure change. |
| §4 rule 9 | No new Timeline fact. |
| ADR 002 D2 | No wiring plugin. |
| ADR 002 D3 | The source is user-scoped; `HostPlatform` and the CLI read it. |
| ADR 002 D4 | Business configuration, read at use time. |
| ADR 002 D5 | `settings.read/update` is the existing language-neutral contract; the source schema is two booleans. |
