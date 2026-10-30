# 012: Session grants

Decision status: Proposed.

Extends [ADR 010](010-sandbox-opt-in.md): a file operation outside the facts asks. This decision
lets a user answer “for this session” and keeps that answer across a short interruption.
ADR 003 is unchanged.

## Context

An approval outside the workspace asks once or for the session. A session can be interrupted and
resumed, and the user should not have to answer the same path again. The grant is a user action;
a model answering a member's approval is not a user.

## Decision

### D1. The grant and its storage

A grant is `{session_id, path, mode}` in a native table keyed by session. `path` is an exact file
or a directory prefix; `mode` is `read` or `write`, and a write grant also permits reads. A grant
applies only to its session, survives a restore, and is removed when the session is deleted. A
fork is a new session and does not inherit grants.

### D2. Approval writes the grant

`ApprovalDecision` stays `Allow | Deny`. The Session grants service exposes
`add(session_id, path, mode)` and `remove(session_id, path)`. When the user chooses “this
session”, the client calls `add` and then answers allow; if the write fails, the answer is
allow-once. Approving a directory writes a prefix.

### D3. User clients only

Grant writes come from a user client. Local and remote clients are treated the same; the
connection or identity layer decides whether that user may act on the session. A leader model
answering a member's approval never writes a grant, and a member session does not inherit the
leader's grants.

### D4. Directory operations

A directory operation is decided on the directory path. A prefix covers its children;
`grep`/`glob` over a directory use that directory. A path outside the facts still asks.

### D5. Full access

A full-access model can write the grant table; that is the accepted trust consequence. A user
who wants a boundary does not use `full-access`.

## Consequences

- A native `session_grants` table and the `add`/`remove` Session methods.
- The client approval surface, `overview.md` B.1/B.2, `Permission.md` and the CLI tables are
  updated under their owners after acceptance.
- No Timeline event or approval wire field is added; `ApprovalDecision` is unchanged.

## Non-goals

- Cross-session or user-scoped grants.
- Model-written grants.
- A path-pattern engine.

## Rule check

| Rule | Application |
| --- | --- |
| §1, ADR 002 D1 | `Approval` remains the branch Capability; grants are Session state; the Session grants service is its owner. |
| §4 rule 1 | Grants are configuration with typed owner operations; no command event. |
| §4 rule 2 | A grant applies to the next operation; the admitted call is unchanged. |
| §4 rule 3 | No `before` callback. |
| §4 rule 4 | No core mechanism change. |
| §4 rule 5 | No Timeline batch. |
| §4 rule 6 | No context-input change. |
| §4 rule 7 | `Approval` calls the Session grants service; no other plugin state is read. |
| §4 rule 8 | No infrastructure change. |
| §4 rule 9 | No new Timeline fact; grants stay outside the Timeline. |
| ADR 002 D2 | No wiring plugin. |
| ADR 002 D3 | Grants are Session-scoped; `Approval` depends on the service. |
| ADR 002 D4 | Business configuration, read at use time. |
| ADR 002 D5 | `add`/`remove` are language-neutral contracts carrying paths and modes. |
