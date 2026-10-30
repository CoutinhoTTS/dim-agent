# 013: Command output ownership

Decision status: Proposed.

Extends [ADR 010](010-sandbox-opt-in.md) and the session-file ownership model in
[`session-files.md`](../design/appserver/session-files.md) §11.2. Attachments are already
session-owned; command output is not. This decision makes both readable by ownership rather than
by the directory a path happens to be in. ADR 003 is unchanged.

## Context

Command output is a host-produced file handed to the model by path. It lives under the session's
output directory, outside the workspace, and is not in `session_file_owners`. A fork inherits the
history that quotes it, so ownership must follow the row, not the source directory.

## Decision

### D1. Two tables, one fact

Attachments keep `session_file_owners (session_id, file_id)`. Command output uses
`session_output_owners (session_id, path)`. Both are the `session-owned` file-access fact.

### D2. Registration

The output owner calls `register(session_id, path)` when it hands the output to the model.
`register` accepts only a path under the output directory the caller owns. It is not a general
grant and cannot register `~/.ssh` or another session's output.

### D3. Queries

`owns(session_id, path)` extracts an attachment's file id or uses the output path and does a
primary-key lookup in the matching table. `owned_paths(session_id)` queries the rows and returns
the attachment path from its file-id directory and the output path from its row; it is not one
primary-key read. A query failure denies the operation.

### D4. Fork, delete and growth

A fork copies the source's rows in both tables at the fork's commit. The fork owns those files; a
file the source adds later is not owned. Deleting a session releases its rows; a file another
session still owns remains.

Command output may grow after a fork. The fork owns the file and reads its current content; a
point-in-time copy is a separate concern.

### D5. Process start

A process start asks the ownership owner for the session's artifact paths and mounts them
read-only, together with the workspace, platform roots and grants. A running persistent shell
keeps the mounts it started with.

## Consequences

- A new `session_output_owners` table, `register`, `owns` and `owned_paths` on the session-files
  owner.
- `session-files.md` §11, `session-deletion-and-fork.md`, the Attachments and WorkspaceTools
  designs and the CLI tables are updated under their owners after acceptance.
- No Timeline event or storage format beyond the new table is added.

## Non-goals

- A new file-access rule for command output; it uses the existing `session-owned` fact.
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
| §4 rule 8 | No infrastructure change. |
| §4 rule 9 | No new Timeline fact. |
| ADR 002 D2 | No wiring plugin. |
| ADR 002 D3 | Ownership is session-files state; file tools and process start read it. |
| ADR 002 D4 | Business state in the native store, read at use time. |
| ADR 002 D5 | `register`, `owns` and `owned_paths` are language-neutral contracts. |
