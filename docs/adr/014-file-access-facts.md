# 014: File access facts

Decision status: Proposed.

Extends [ADR 010](010-sandbox-opt-in.md). It uses the grants of ADR 012 and the host-artifact
ownership of ADR 013. This decision gives `Permission` one fixed set of facts for file reads and
writes; ADR 003 is unchanged.

## Context

Reads are path-independent today, writes ask outside the workspace, and the platform's system
roots are writable under `workspace-write`. A path alone cannot say whether a file is the
session's own artifact. The facts have to be named once.

## Decision

### D1. Fixed facts

`Permission` decides a file operation from:

| Fact | Source |
| --- | --- |
| workspace | `SessionConfig` |
| system roots | HostPlatform's platform roots; read-only |
| session-owned | the session-files ownership rows; read-only |
| user grant | the session's grants (ADR 012) |
| preset | the branch's permission mode |

A Session-scoped facts service composes them. `Permission` passes the dispatched branch's preset
into the query; the preset is branch state, not the Session's initial config. It does not know
about attachments, SessionStore or the sandbox.

### D2. Read and write

| Mode | Inside the facts | Outside the facts |
| --- | --- | --- |
| `read-only` | reads allowed | reads ask; writes, processes and network denied |
| `workspace-write` | reads allowed; writes only in the workspace and write grants | reads and writes ask |
| `full-access` | allowed | reads and writes allowed without a prompt |

- Under `read-only`, the workspace is read-only. Under `workspace-write`, the workspace and
  write grants are writable. System roots and session-owned artifacts are read-only under both.
- Under `full-access`, file-tool writes are admitted anywhere, including system roots and
  session-owned artifacts. The process mounts are unchanged; full access is trust, not a
  sandbox boundary.
- A write grant also permits reads; a read grant never permits writes.

### D3. Matching

Paths are canonicalized before matching; a symlink is judged by its target. A workspace symlink
to a path outside the workspace does not match the workspace fact. The tool lists and
`allowedTools` decide whether a tool may be called; they do not grant a path. A directory
operation is decided on the directory path; a prefix covers its children; `grep`/`glob` over a
directory use that directory.

There are no path deny rules or glob patterns.

## Consequences

- `Permission.md` and the CLI permission tables describe the fixed facts and the read change:
  reads outside the facts now ask under `read-only` and `workspace-write`.
- AGENTS.md rule 11's “file-root authorization” clause is updated by this decision.
- No Timeline event or wire field is added; the facts service is Session configuration.

## Non-goals

- A path-pattern engine or path deny rules.
- Changing the sandbox mounts; they are decided by ADR 010 and the platform defaults.
- Cross-session grants.

## Rule check

| Rule | Application |
| --- | --- |
| §1, ADR 002 D1 | `Permission` remains the branch Capability; the facts service is Session configuration; tool owners keep execution. |
| §4 rule 1 | The facts service is typed configuration; no command event. |
| §4 rule 2 | The admitted call is unchanged. |
| §4 rule 3 | No `before` callback; write resolution keeps its pre-admission contract. |
| §4 rule 4 | No core mechanism change. |
| §4 rule 5 | No Timeline batch. |
| §4 rule 6 | No context-input change. |
| §4 rule 7 | `Permission` reads the facts service; no other plugin state. |
| §4 rule 8 | No infrastructure change. |
| §4 rule 9 | No new Timeline fact. |
| ADR 002 D2 | No wiring plugin. |
| ADR 002 D3 | Facts are Session-scoped; `Permission` depends on them. |
| ADR 002 D4 | Business configuration, read at use time. |
| ADR 002 D5 | The facts query is a language-neutral contract; grants and ownership carry paths/modes. |
