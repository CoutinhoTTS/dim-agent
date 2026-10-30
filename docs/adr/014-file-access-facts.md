# 014: File access facts

Decision status: Proposed.

Extends ADR 010. It uses the grants of ADR 012 and the host-artifact ownership of ADR 013
(sibling PRs). This decision gives `Permission` one fixed set of facts for file reads and writes.
ADR 003 is unchanged.

## Context

Reads are path-independent today, writes ask outside the workspace, and the platform's system
roots are reachable for writes under `workspace-write`. A path alone cannot say whether a file is
the session's own artifact. The facts have to be named once, with an owner for each.

## Decision

### D1. Fixed facts

`Permission` decides a file operation from:

| Fact | Source |
| --- | --- |
| workspace | `SessionConfig` |
| system roots | HostPlatform's platform-roots list, independent of the sandbox switch |
| session-owned | ADR 013's ownership fact |
| user grant | ADR 012's grants |
| preset | the dispatched branch's permission mode |

The platform-roots list is HostPlatform state; the facts service and the sandbox builder read the
same list, so the fact is non-empty when the sandbox is off. The facts service calls the
session-files owner for the ownership fact; it does not import SessionStore. `Permission` passes
the branch preset into the query; the preset is branch state, not the Session's initial config.

### D2. Read and write

| Mode | Inside the facts | Outside the facts |
| --- | --- | --- |
| `read-only` | reads allowed | reads ask; writes, processes and network denied |
| `workspace-write` | reads allowed; writes only in the workspace and write grants | reads and writes ask |
| `full-access` | file-tool writes allowed | reads and writes allowed without a prompt |

The table is about file reads and writes. Processes and network keep their existing rules: the
sandbox setting and the execute-process approval.

- Under `read-only`, the workspace is read-only and a write grant is inert. Under
  `workspace-write`, the workspace and write grants are writable.
- System roots and session-owned artifacts are read-only under `read-only` and `workspace-write`.
  Under `full-access`, file-tool writes are admitted anywhere, including them; the process mounts
  in ADR 013 are unchanged.
- A write grant also permits reads; a read grant never permits writes.

### D3. Matching

Read and write calls resolve the path before admission, like `write` and `edit` do today: cwd,
`~`, `..` and symlinks are settled, and `Permission` compares the canonical path. A workspace
symlink to a path outside the workspace does not match the workspace fact.

The path check runs before the tool lists and `allowedTools`, so an allowed tool does not bypass
a path fact. A directory operation is decided on the directory path; a prefix covers its
children; `grep`/`glob` over a directory use that directory. ADR 012 owns the grant's directory
rule.

There are no path deny rules or glob patterns.

### D4. The read change is visible

Reads outside the facts now ask under `read-only` and `workspace-write`, where today they never
ask. A non-interactive run turns the prompt into the ordinary denial with exit code 3. The change
is announced in the CLI documentation and release notes and tracked in #691.

## Consequences

- `Permission.md` and the CLI permission tables describe the fixed facts, the pre-admission read
  resolution and the read change.
- HostPlatform owns the platform-roots list used by both the facts service and the sandbox
  builder.
- AGENTS.md rule 11's “file-root authorization” clause is superseded: workspace, system-root,
  session-owned and grant facts now decide access.
- No Timeline event or wire field is added; the facts service is Session configuration.
- ADR 012, 013 and 014 land together; the mounts are owned by ADR 010 (sandbox) and ADR 013
  (artifacts).

## Non-goals

- A path-pattern engine or path deny rules.
- Changing the sandbox mounts; ADR 010 and ADR 013 decide them.
- Cross-session grants.

## Supersession

Supersedes ADR 010's “file access is otherwise unchanged” and ADR 009's “file-root authorization
is unchanged”, and AGENTS.md rule 11's “file-root authorization never confers authority”. It
extends ADR 010 and depends on ADRs 012 and 013. ADR 003 remains as written.

## Rule check

| Rule | Application |
| --- | --- |
| §1, ADR 002 D1 | `Permission` remains the branch Capability; the facts service is Session configuration; tool owners keep execution. |
| §4 rule 1 | The facts service is typed configuration; no command event. |
| §4 rule 2 | The admitted call is unchanged. |
| §4 rule 3 | Read and write resolution happen before admission; no `before` callback. |
| §4 rule 4 | No core mechanism change. |
| §4 rule 5 | No Timeline batch. |
| §4 rule 6 | No context-input change. |
| §4 rule 7 | `Permission` reads the facts service; no other plugin state. |
| §4 rule 8 | **Raises two requirements**: HostPlatform's platform-roots list must be available independently of the sandbox, and read calls need the same pre-admission resolution as writes. |
| §4 rule 9 | No new Timeline fact. |
| ADR 002 D2 | No wiring plugin. |
| ADR 002 D3 | Facts are Session-scoped; `Permission` depends on them. |
| ADR 002 D4 | Business configuration, read at use time. |
| ADR 002 D5 | The facts query is a language-neutral contract; grants and ownership carry paths/modes. |
