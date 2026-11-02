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

### D3. Stored explicit overrides stay honoured, and are no longer written

Policies already persisted with `fileAccess` or `toolApproval` keep the meaning ADR 017 D2 gave
them: the runtime honours them, and a preset write never clears or reinterprets them. New writes
from a client set the preset only. The fields stay in the type for that reason; nothing new reads
them as a UI dimension.

This is deliberate: removing the fields would silently reinterpret stored Session configuration
and `permission/changed` history, which ADR 017 D2 forbids.

### D4. The unrepresentable-combination rule stays where it already lives

A legacy policy whose explicit dimensions cannot be expressed as a preset keeps its current
handling — Scheduler and Workflow pass the stored policy through and surface the owner's error.
No plugin gains a second policy comparison.

## Consequences

- ADR 017 D3 is superseded: there is one visible permission control, and the two others leave
  the panel.
- `/permissions`, the CLI design and the Permission page describe the preset and link to the
  sandbox for the rest; the Permission page stops documenting an independent file-access
  override as a user choice.
- Clients that already render three controls keep reading the same fields; they stop writing the
  two overrides.
- The preset keeps its exact legacy meaning, so nothing about admission changes.

## Non-goals

- Not removing `fileAccess` or `toolApproval` from the persisted type (D3).
- Not changing the sandbox's own contract, its opt-in default or its file facts (010, 014).
- Not changing which calls ask: the preset decides that, as before.
