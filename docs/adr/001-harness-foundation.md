# 001: Extensible harness foundation

Decision status: Recovery design adopted for this branch. Configuration is implemented on
this branch but remains Proposed for maintainer alignment with issue #3. The model-context
boundary of dynamic events is decided: events enter model context only through adopted inputs
and tool results (issue #2, revised 2026-10-15). Dynamic-event registration and dependency-loss
refinements remain Proposed for issues #2 and #6. This ADR does not supersede those issues' decisions.

Implementation status: The recovery, configuration and common-assembly batch is implemented;
the broader foundation remains incomplete. The delivery table records implementation scope,
not maintainer approval or completion of the linked issues. Its acceptance column describes
the required behavior, not a claim that unimplemented units have passed verification.

## Context

Dim continues the extracted v4 harness as its own mainline. Its execution kernel already
has typed plugins, scoped services, committed Timeline facts, persistent Sessions and
generation-owned work. Extension events remain a closed Rust enum, general plugin management
and cross-language capabilities are incomplete, and the configuration authority differs
from issue #3. Resolve each capability's contract and dependencies before extending it.

The goal is a complete extensible harness, not a replacement event bus or plugin framework.
Use rutis and its loader. The standalone Rust library remains usable without the App Server,
SQLite, Node, a TUI or a private account plugin.

## Decisions

### Events and ownership (Proposed; issue #2)

Model-context boundary (Decided, 2026-10-15): events never enter model context directly. The
model sees only adopted inputs and tool results; a capability that needs content in context
writes an adopted input with the final content or returns it in a tool result. There are no
registered projection rules. Issue #2 was revised to the same boundary. The rest of this
section remains Proposed.

Runtime mechanism facts, public capability protocol events and domain events share one
registration and persistence mechanism. A Rust core-event enum is a convenience view, not
an extension admission list. New domain capabilities, including built-in ones, declare
their own events. Changes to shared protocol semantics require a decision record.

Distinguish three lifetimes:

1. A module declares a stable identity, event keys and payload schemas. Do not persist
   loader-generated names or process-local instance IDs as declaration identities.
2. An instance adopts declarations within a scope and generation. Stop new work before
   draining accepted work; retain the ability to write required terminal facts until drain
   completes. Reject stale-generation operations afterward.
3. History retains immutable declaration content and references. Reading or importing a
   declaration never installs code, grants permissions or resumes external work.

Use stable type keys and reject conflicting declarations. Namespaces express ownership conventions,
not authentication or a central approval process. The declaration owner defines the data contract;
legitimate producers use the appropriate public ports and work ownership, rather than being limited
to the type's declaring instance. Bind active registration and its cleanup to rutis, not a second
plugin lifecycle manager. Revoke registration after the generation's required terminal writes drain.

Validate the final payload at the write boundary. Persist a declaration and its first use
atomically. Hashes identify content, not schema or semantic compatibility. Preserve unknown
historical events without executing them. Declaration size and reference resolution are bounded;
declarations contain no resolved credentials. All stores and portable exports preserve the
reference closure, including branch history and trace records. Archived schemas provide contract
provenance, not executable business interpretation. Unknown payloads remain readable as data.

Preserve the existing v1-v3 protocol contracts. A new dynamic protocol should preserve unknown
event types without requiring a server-side adapter for every extension. Visibility, authorization
and sensitive-data filtering remain host responsibilities, independent of compatibility conversion.
Do not require a universal event version counter: storage, snapshot interpretation and business
semantics have separate compatibility boundaries. Incompatible semantics require an explicit contract.

### Model context and recovery

The proposed event extension boundary preserves the existing adopted-input contract: domain events
do not independently project into model context. Capabilities deliver final runtime
content through adopted inputs or public tool results. The host interprets public protocol
facts, including assistant output, input consumption, tool results, adopted configuration and
compression. Optional UI rendering does not change what the model sees.

Historical interpretation needs supported host semantics and complete referenced content,
not execution of the original plugin. This does not guarantee that missing tools or Providers
can continue running. End persistent rules with explicit retirement facts, not historical edits.

Compression changes only the model-context view. It does not delete Timeline facts or replace
the replay input of other capabilities. A completed compression records its source prefix,
inclusive cut, summary and baseline. Preserve the suffix, changes after the source prefix,
and branch ancestry limits. A baseline cannot overwrite later rule changes or retirement.

Separate compression facts from optional recovery snapshots. A snapshot identifies committed
history and an interpreter compatibility contract readable before decoding its state. Incompatible
or absent snapshots can be bypassed; valid compression facts cannot. Replay reconstructs the
same context without generating another model summary. Unsupported facts or damaged required
content fail explicitly and remain intact. Snapshot candidates are supplied by storage; the
context interpreter decides compatibility. Exported facts and references must suffice without
snapshots. Independently verify full replay and accelerated replay agree at a fixed legal prefix.

The default interpreter v2 owns a paired encoder/decoder and an integrity envelope binding the
exact checkpoint payload to its anchor identity and interpreter. Reject damaged envelopes before
using their state; bypass unprotected v1 snapshots without migrating the database or rewriting
facts. This detects corruption after encoding, not malicious writers or semantic bugs in trusted
checkpoint builders. Builder equivalence is verified against full replay in tests; a known builder
defect requires a corrected compatibility version. Valid snapshots retain suffix-only reads.

### Configuration and lifecycle

The source-first configuration model below describes this branch's implementation, proposed
as an alternative to issue #3's editable-layer authority. It is not an accepted replacement
for that issue. General scope and acknowledgement requirements extend beyond the implemented
MCP and user Skill configuration paths; see the delivery table and issue alignment below.

The loader owns plugin enablement and runtime configuration composed from explicit sources.
Management services retain business routing, not independent generation ownership.
Distinguish user, workspace and Session scopes, source provenance and temporary overrides.
Each source has an explicit persistence authority and one write path; management edits name their
target source rather than writing back a merged view. Preserve shared workspace files as sources.
Import legacy product-home configuration before switching authority, retain originals on failure,
and do not automatically delete them. Scope row identities and configuration mappings so one
workspace cannot change another's instances. The loader owns the composed runtime intent; its
single editable layer does not itself define multi-scope persistence. Providers are a selection
dimension, not different harness feature sets.

SQLite is the App Server's default configuration persistence implementation, with revision checks.
It is not a runtime dependency. Report accepted intent, persisted revision, applied revision,
runtime state and business settlement separately. Loader edits are not atomic transactions with
external effects; runtime convergence can precede a failed save. Notifications trigger state
refresh, not an assumption of durable success. Stable MCP identity survives process restarts;
connections and leases remain generation-owned.

The App Server uses source-first persistence: validate and CAS the explicitly named source, then
publish its composed intent to the existing root loader. A host publisher outside plugin
apply/dispose serializes publication and retains the immutable assembly layer. Configure that
loader with no editable layer; do not switch its single editable target between workspaces or
use its pending-edit queue as a multi-source transaction log. Failed persistence cannot change
runtime intent; failed application retains persisted intent for an explicit refresh. A mutation
reply certifies persistence, not loader application or business settlement. Generic loader rows
reference private revision-bound configuration rather than embedding MCP secrets. Management
services and leases do not own or shut down these configured rows.

User MCP publication uses distinct loader rows for overlapping generations. Replacing a single row
would dispose the old connection while an accepted model request still holds its tools; a loader
group instance does not pin its old template. Retire a superseded row when its last binding is
released. Project generations instead retire on configuration change, disablement or loss of
execution approval, even with retained bindings. Explicit disconnect removes every generation for
the selected source and server, waits for loader disposal, and reports cleanup errors even when
reconciliation itself succeeds.
The stable part of a row identity hashes the canonical workspace (or user scope) and server id;
the generation suffix is transient. Private immutable configuration remains available until the
loader removes its row. Network connection remains lazy and generation-owned.

User MCP and Skill documents have one SQLite authority after a successful lazy import. Store
the document and import marker atomically, leave originals intact, and permit retry after failed
import. Workspace sources keep file authority and file CAS. The standalone Settings plugin can
still use its file backend without SQLite. The native database format follows the existing strict
compatibility policy; importing legacy settings files does not authorize migrating old databases.

The dependency-loss refinement is Proposed for issue #6: distinguish loader state, cleanup
completion and successful business settlement. Pending can occur before a plugin ever becomes
active, and rutis can publish Pending after reporting cleanup errors. Capabilities must own and
join their background work; retained service handles need generation checks where stale operations
would be unsafe. Do not infer successful terminal commits from loader state or retry unknown
external outcomes. Use existing rutis lifecycle hooks and the harness fault path.

### Trust and cross-language calls

> Superseded in part by [ADR 003](003-approval-scope.md): project MCP execution approval
> (grants, fingerprints, `mcp.approve`/`mcp.revoke`) is removed; project entries start disabled
> and the user enables them. The rest of this section stands.

Project MCP execution approval is implemented at the host boundary. It is not a complete
workspace sandbox or a cross-language Capability implementation. Authorizing a FileStorage root,
selecting FullAccess tool permissions or restoring a Session never grants project MCP execution.
Reading configuration is inert: unapproved project entries shadow user entries but cannot start
stdio processes or connect HTTP/SSE transports, including during automatic tool discovery.

The Settings owner persists host-local grants in `mcp-approvals.json`, format version 1, using
FileStorage leases and atomic replacement (0600 on Unix). Native and standalone hosts use the
same file contract; no native database version change or Session migration is required. The file
contains only hashed workspace/server identities and configuration fingerprints. Project files,
Timeline history and portable Session data cannot carry grants. Missing grants deny execution;
unreadable or corrupt stores fail closed without silently resetting existing records.

A grant binds a canonical workspace path, server ID and effective typed configuration. The
versioned fingerprint includes transport, command, arguments, environment overrides, resolved cwd,
URL, headers and authentication; it excludes enablement. Relative project stdio cwd resolves from
the workspace, never the daemon cwd. A disabled server remains disabled after approval. Changes to
executable configuration require a matching grant; identical configuration can reuse its grant.
User configuration remains a separate trusted local source and cannot confer approval on a
project entry with the same ID.

Protected-local protocol v3 management exposes `mcp.approve {cwd,id,approvalToken}` and
`mcp.revoke {cwd,id}`. `mcp.catalog` exposes `approval` (`notRequired`, `required`, `approved`) and
`approvalToken`, not executable configuration or credentials. The token is a configuration digest,
not a bearer credential or evidence of human consent. Approval compares the supplied token with
current intent, persists first, and re-reads before publication. An active Session may discover an
approved server immediately; callers must review configuration before approving. A stale token
returns `mcp_settings_conflict`. WebSocket callers cannot use these management methods.

The existing publication actor serializes host approval and generation changes; the loader still
owns disposal. Every project connection operation rechecks current source and grants before
admission. Revocation removes the persisted grant, rejects new calls through retained bindings,
cancels and joins accepted generation work, and closes transports before successful acknowledgement.
Cleanup failure is reported separately from persisted revocation; callers must inspect state after
interruption rather than assuming rollback. External file edits are observed on refresh or the
next connection operation, not by a filesystem watcher. Revocation settles this host's connections;
other independent hosts sharing the file observe it on their next read. Prior remote side effects
are not undone.

This approves configuration, not immutable executable bytes: modifying a referenced script or
binary, inherited environment, DNS or remote server behavior does not change the fingerprint.
Trusted native plugins, explicitly permitted shell tools and same-user processes can access host
files and are not sandboxed by this gate. Constrained plugin execution and OS isolation remain
separate work. Host safety settings do not belong to replayable business facts: replay must never
restore authority from a Session that once had it.

Configuration precedence never overrides host safety limits or workspace trust. Store credential
references in generic configuration; exclude resolved secrets from management notifications,
diagnostics, declarations and event history. Trust, authorization and OS isolation are distinct.
Native trusted code is not isolated by `Deps`; constrained callers require host checks of caller,
scope and work ownership. Shared language runtimes are not security sandboxes.

Rust and other languages use the same public business semantics. Cross-process operations carry
serializable data, explicit ownership and acknowledgement levels: accepted, committed, completed.
Establish correlation before publishing results. Caller abandonment is not rollback; cancellation
of unstarted work differs from settlement of accepted work and unknown external outcomes. Do not
automatically retry an operation whose side effects may already have happened.

Do not await cross-process code inside synchronous Timeline commit critical sections. External
preparation or review has an explicit operation lifetime. Validate the public event path with a
real Node capability as soon as registration and append are available, before hardening a Rust-only
API. Validate runtime loss, reload, cancellation and commit-before-reply disconnection.

## Delivery and acceptance

| Unit | Status | Acceptance |
| --- | --- | --- |
| Public contracts and contributor rules | Documented; proposed contracts still require issue alignment | Current and planned behavior remain distinct; no private source content |
| Recovery facts and snapshots | Implemented: immutable completion facts and separately persisted optional interpreter snapshots, introduced in native storage v12; current database format is v13 and default snapshot interpreter is v2 | Multi-compression, rule replacement/retirement, branch cuts, incompatible/missing snapshots; no new model calls |
| Production compression host | Not implemented: the real executable binds `ContextModelHost(None)` | Real model compaction uses the existing host contract and records replayable completion facts |
| Dynamic events and declarations | Not implemented | External Rust capability owns durable events; declaration lifetime and stale writes are enforced |
| Cross-language public capability | Not implemented | Node declaration, append, subscription, cancellation, reload and process-loss flows |
| Scoped persisted loader configuration | Implemented for MCP: source-first publication to the root loader; user MCP/Skills authority and import use native storage v13. Pending issue #3 alignment. General plugin configuration, Session overrides, unified credential references and complete acknowledgement/status reporting are not implemented | Scope isolation, save failure, protected-local management, secret-free rows and diagnostics |
| Common execution assembly | Implemented: API-key and product profiles share public capabilities and protected-local management | Binary API-key flow, persistent settings across restart, model adoption, account isolation and transport restrictions |
| Loader-owned MCP instances | Implemented: scoped logical identities, overlapping generation rows, host publication, explicit disconnect and cleanup receipts. Uses distinct generation rows rather than issue #3's proposed instanced group | Bound user-source tools survive replacement; project tools obey current approval; all generations drain; logical identities survive restart and root reordering |
| Project MCP execution approval | Implemented: host-local scoped configuration grants, protected-local approval/revocation, per-operation checks and generation disposal. General workspace sandboxing and constrained plugins are not implemented | Unapproved project MCP cannot execute or connect; stale configuration, restart, retained tool bindings and revocation respect the grant |

Keep this implementation batch on one integration branch with reviewable commits. The persistence
format, runtime assembly and lifecycle have one coordinated owner. Whole-plugin documentation
coverage and generated checks are not evidence of runtime correctness. Further work includes
compression hosts, broader workspace isolation, extensions to existing collaboration and permissions,
queue controls, questions, scoped rules, file history, external protocols, plugin management
and long-running/remote capabilities. These are separate delivery scopes, not additions to the
completed batch. TUI implementation and company services remain outside this work. Security
boundaries apply before exposing untrusted execution.

Each delivered unit must be usable on its declared parent and pass its relevant public-flow tests. Integrated
boundaries run workspace, lint, graph and packaging-script checks. Review P0 through P3 issues and remove
unnecessary machinery before committing. Storage compatibility follows actual format changes,
not a preallocated version shared across incompatible branch commits. Use disposable databases;
never overwrite a user's existing store to make a test pass.

## Existing issue ownership

- [#2](https://github.com/arcships/dim/issues/2) tracks dynamic events. Resolve the Proposed contracts
  above before committing its envelope, declaration storage and append API as a public format.
- [#3](https://github.com/arcships/dim/issues/3) tracks loader configuration and MCP ownership;
  [#4](https://github.com/arcships/dim/issues/4) tracks cross-language capabilities. Validate the
  latter immediately after the public dynamic-event path is usable.
- [#5](https://github.com/arcships/dim/issues/5) tracks remote nodes;
  [#6](https://github.com/arcships/dim/issues/6) tracks typed-dependency questions and startup tests;
  [#7](https://github.com/arcships/dim/issues/7) tracks contributor documentation and graph checks.

### Maintainer alignment required

| Issue | Existing decision or open question | Branch position and merge requirement |
| --- | --- | --- |
| #2 | Events enter model context only through adopted inputs and tool results, with no projection rules (revised 2026-10-15); registration follows the plugin lifecycle | Decided: the model-context boundary. Proposed: archive declarations and preserve terminal-write eligibility while accepted work drains. Resolve these contracts before implementing the dynamic public event format |
| #3 | The persisted editable layer is the single configuration authority; management maps directly to loader operations; MCP uses an instanced group | Implemented alternative: each named source persists first, then a publisher reconciles root-loader layers without an editable target. Distinct rows retain overlapping MCP generations. Obtain maintainer agreement on these differences before merging the configuration implementation; otherwise revise the affected implementation and format decisions |
| #6 | Dependency loss returning a plugin to Pending needs settlement semantics; optional dependencies remain open; the process startup deadline lacks an injection test | Proposed: separate loader state, cleanup completion and committed business settlement. This batch does not settle the optional-dependency question or add the missing startup test |

The source-first recommendation preserves shared workspace files and explicit edit targets.
In rutis 0.10.0, a loader has at most one editable target and applies edits before persistence;
`Edited` can be emitted after a save failure. A group instance does not pin the old template's
resources. These constraints motivate this branch's alternative, not permission to replace
the issue decision without agreement. Revisit the integration if rutis supplies the required
source and generation semantics; do not introduce another plugin lifecycle manager.

Recovery facts/snapshots and common API-key assembly are additional work, not completion of those
issues. The configuration implementation is available for review without settling dynamic
events, but does not complete #3. Plugin documentation and coverage checks address part of #7;
they do not resolve all design-document migration and issue-link work. Do not equate implemented
behavior with maintainer approval of the proposed issue changes.

## Alternatives and consequences

Continuing to add domain variants to the closed enum would defer extension ownership and force
another migration for every feature. A universal executable projection language would create a
second context-adoption mechanism and make history depend on plugin interpreters. This proposal
rejects both; issue #2 adopts the same model-context boundary (revised 2026-10-15).

Stable public contracts require cross-crate work and explicit storage compatibility. Snapshots add
storage maintenance but remain disposable; immutable facts and reference closure remain authoritative.
The foundation must be verified through public flows, not inferred from a successful compile or
agreement between design documents.
