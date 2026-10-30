# 013: Command output ownership

Decision status: Proposed.

Extends ADR 010 and the session-file ownership model in
[`session-files.md`](../design/appserver/session-files.md) §11.1. This decision makes command
output, offloaded `read` results and background logs session-owned artifacts, so copied history
can read them after the source session is deleted. ADR 003 is unchanged.

## Context

A host-produced output file is handed to the model by path under
`<database dir>/command-output/<session>/`. It has no `session_files` row or id, it can grow while
a command runs, and delete removes the directory. A fork's copied history quotes those paths, so
ownership has to follow a row, not the source directory.

## Decision

### D1. Two tables, one fact

Attachments keep `session_file_owners (session_id, file_id)`. Output uses
`session_output_owners (session_id, path)`. Both feed the `session-owned` fact defined by
ADR 014.

Two keys are deliberate. Output has no `session_files` row or id, is appended while running and
is removed as a directory. A single polymorphic table would need a nullable key, could not use
the `session_files` foreign key and would filter every sweep by kind. The second table exists so
a fork can read output after the source is deleted; without that requirement the current
directory rule is cheaper.

### D2. Registration

The output owner calls `register(session_id, path)` before it returns the tool result that hands
the path to the model. If registration fails, the call fails; a path with no row is unowned.
Registration accepts only a canonical path under `<output root>/<session_id>/`; it guards misuse
among trusted parts, not isolation.

`exec` output, offloaded `read` results and a running background log all register. A log may
grow after registration; ownership is stable.

### D3. Queries and escapes

`owns(session_id, path)` resolves the path, then:

- an attachment requires the path to stay inside
  `<db>.session-files/<fileId>/` for an owned file id;
- output requires the exact registered path.

`..`, symlinks and an unresolvable path are denied. `owned_paths(session_id)` queries the rows
and returns the attachment path from its file-id directory and the output path from its row; it
is not one primary-key read. A query failure denies.

### D4. Fork, delete and growth

A fork copies the source's rows in both tables at the fork's commit. The fork owns those files; a
file the source adds later is not owned.

Delete releases the session's rows. The output directory is removed only when no row names any
path under it; the last owner removes it. This replaces the unconditional directory removal in
`session-deletion-and-fork.md`, which recorded the old rule because no row named the output.

The new table is part of the next native storage format version; its upgrade creates it empty,
and a failed upgrade rolls back.

Command output may grow after a fork. The fork owns the file and reads its current content; a
point-in-time copy is a separate concern.

### D5. Process start

A process start asks the ownership owner for the session's artifact paths and mounts them
read-only, together with the workspace, platform roots and grants. A running persistent shell
keeps the mounts it started with.

## Consequences

- A new `session_output_owners` table, a native format version and `register`, `owns`,
  `owned_paths`.
- `native-storage-format.md`, `session-deletion-and-fork.md`, `session-files.md` §11, the
  Attachments and WorkspaceTools designs and the CLI tables are updated under their owners after
  acceptance.
- No Timeline event is added; the table is storage state.

## Non-goals

- A new file-access rule; output uses ADR 014's `session-owned` fact.
- Copying output at fork; growth is a separate concern.
- Registering paths outside the caller's output directory.

## Rule check

| Rule | Application |
| --- | --- |
| §1, ADR 002 D1 | Ownership stays in the session-files owner; WorkspaceTools keeps producing output. |
| §4 rule 1 | Registration is a typed owner operation; no command event. |
| §4 rule 2 | The model reads the registered path; no call handover changes. |
| §4 rule 3 | No `before` callback. |
| §4 rule 4 | No core mechanism change. |
| §4 rule 5 | No Timeline batch. |
| §4 rule 6 | No context-input change. |
| §4 rule 7 | Parts call the session-files owner; no other plugin state. |
| §4 rule 8 | No infrastructure change; the native table is a storage format version owned by SessionStore. |
| §4 rule 9 | No new Timeline fact. |
| ADR 002 D2 | No wiring plugin. |
| ADR 002 D3 | Ownership is session-files state; file tools and process start read it. |
| ADR 002 D4 | Business state in the native store, read at use time. |
| ADR 002 D5 | `register`, `owns` and `owned_paths` are language-neutral contracts. |
