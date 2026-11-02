# 018: Permission is the preset; file scope and isolation are sandbox settings

Decision status: Proposed.

## Context

[ADR 017](017-independent-permission-controls.md) D3 put File access, Tool approval and Process
isolation on one `/permissions` panel. Its reason was legibility: a preset name such as Ask or
Full access did not say whether a choice changed files or tool questions.

The panel now asks the user to make three choices, and two of them do not decide permission:

- A preset already decides both questions. `read-only` cannot write, `workspace-write` writes
  inside the workspace and asks outside it, `full-access` does not ask. An independent File
  access control can therefore only produce combinations nobody asks for — paths unrestricted
  while every call still asks, or paths read-only while tool questions are skipped.
- What is genuinely independent of "may this run, and must it ask" is the **path boundary**:
  workspace, system roots, a Session's own files. That is an execution boundary, owned by the
  facts and the sandbox ([014](014-file-access-facts.md), [010](010-sandbox-opt-in.md)).
- Process isolation is an execution backend ([011](011-execution-settings.md), owned by
  ExecutionSettings). ADR 017 brought its control onto the permission panel to replace the
  Settings > Execution entry, which put two owners behind one menu.

## Decision

### D1. One permission control: the preset

`/permissions` offers the three presets and nothing else. It is the decision of what may run and
what must be asked before it runs; the panel states the consequence of the chosen preset rather
than splitting it into dimensions the user cannot act on independently.

### D2. File scope and process isolation move under the sandbox settings

The path boundary and the process isolation backend appear under Settings > sandbox, beside the
settings that own them. `/permissions` links to that section instead of hosting it. A user who
needs "this path is allowed without asking" or "run without a sandbox" acts on the sandbox, not
on permission.

### D3. The two overrides are removed, and this type stops rejecting unknown keys

`PermissionPolicy` loses `fileAccess` and `toolApproval`. Admission follows the preset and nothing
else, and a record that does not name a preset takes the default.

The type also loses `deny_unknown_fields`. A record written before this decision carries the two
keys, and strict parsing would refuse it; with the attribute gone the keys are unknown and are
ignored, like any other key the type does not name. The cost is deliberate and stated: a
misspelling in this type no longer fails to parse, so a typo in a policy reads as the default.

This is a breaking format decision, not a silent reinterpretation: client options, tests and
ADR 017 in the current alpha still write the keys, and this decision stops reading their meaning.

### D4. A stored limit that no longer exists is stated, not dropped

Where a stored policy is read — Session configuration, `permission/changed`, a job's cap — the
reader compares the ignored keys with what the preset implies. When the ignored value was
stricter than the preset, the client says so and asks for a preset instead of starting the
Session under wider authority.

That comparison is the only place the old names appear, and it exists because
[003](003-approval-scope.md) and [009](009-permission-decides.md) forbid widening authority
without saying so. It produces a notice and a choice; it does not map a value back to a preset.

### D5. Cap and containment compare presets and tool lists only

Scheduler's `cap`, `within` and `effective` and Workflow's launch cap keep their authority
comparison, expressed in the presets they still have: the lower preset wins, allowed tools are the
ones both sides allow, disallowed tools are the union. The dimension comparison — `fileRank`, the
derived approval, and the `schedule_invalid` case for explicit Ask under a legacy read-only bound —
disappears with the fields. `schedule_invalid` keeps its other validation of a job.

## Consequences

- ADR 017 D3 is superseded: there is one visible permission control, and the two others leave
  the panel.
- `/permissions`, the CLI design and the Permission page describe the preset and link to the
  sandbox for the rest; the Permission page stops documenting an independent file-access
  override as a user choice.
- Clients stop rendering and writing the two overrides; a value they send under those names is
  ignored like any unknown field.
- A Session whose stored policy named a stricter override is not started under the wider preset
  without a notice (D4); once the user picks a preset, the record holds that preset alone.
- Policy parsing in this type no longer rejects unknown keys, which also removes typo detection
  there. Other types keep `deny_unknown_fields`.
- The preset keeps its exact legacy meaning, so nothing about admission changes.

## Non-goals

- Not changing what a failed admission does, and not changing Session grants or approvals.
- Not changing the sandbox's own contract, its opt-in default or its file facts (010, 014).
- Not changing which calls ask: the preset decides that, as before.
