# 001: Extensible harness foundation

Decision status: Accepted (2026-10-14)

Implementation status: In progress; the table below is authoritative for delivery status.

## Context

Dim continues the extracted v4 harness as its own mainline. Its execution kernel already
has typed plugins, scoped services, committed Timeline facts, persistent Sessions and
generation-owned work. Extension events remain a closed Rust enum, configuration is not
yet unified under editable loader intent, and general cross-language capabilities are not
yet implemented. These gaps must be addressed before expanding the domain feature set.

The goal is a complete extensible harness, not a replacement event bus or plugin framework.
Use rutis and its loader. The standalone Rust library remains usable without the App Server,
SQLite, Node, a TUI or a private account plugin.

## Decisions

### Events and ownership

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

Domain keys use their declaring module's namespace. Check duplicate identities and conflicting
declarations at admission and activation. A namespace is not authentication. The declaration
owner defines the data contract; legitimate producers use the appropriate public ports and
work ownership, rather than being limited to the type's declaring instance.

Validate the final payload at the write boundary. Persist a declaration and its first use
atomically. Hashes identify content, not schema or semantic compatibility. Preserve unknown
historical events without executing them. Declaration size and reference resolution are bounded;
declarations contain no resolved credentials. All stores and portable exports preserve the
reference closure, including branch history and trace records.

### Model context and recovery

Domain events do not directly project into model context. Capabilities deliver final runtime
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

### Configuration and lifecycle

The loader's editable persisted intent is the single authority for plugin enablement and
configuration. Management services retain business routing, not independent generation ownership.
Distinguish user, workspace and Session scopes, source provenance and temporary overrides.
File-backed configuration remains an explicit source; migration has one writer and no silent
conflicting authority. Providers are a selection dimension, not different harness feature sets.

SQLite is the App Server's default configuration persistence implementation, with revision checks.
It is not a runtime dependency. Report accepted intent, persisted revision, applied revision,
runtime state and business settlement separately. Loader edits are not atomic transactions with
external effects; runtime convergence can precede a failed save. Notifications trigger state
refresh, not an assumption of durable success. Stable MCP identity survives process restarts;
connections and leases remain generation-owned.

### Trust and cross-language calls

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
| Public contracts and contributor rules | Implemented as documentation | Current and planned behavior remain distinct; no private source content |
| Recovery facts and snapshots | Not implemented | Multi-compression, rule replacement/retirement, branch cuts, incompatible/missing snapshots; no new model calls |
| Dynamic events and declarations | Not implemented | External Rust capability owns durable events; declaration lifetime and stale writes are enforced |
| Cross-language public capability | Not implemented | Node declaration, append, subscription, cancellation, reload and process-loss flows |
| Scoped persisted loader configuration | Not implemented | Scope isolation, save failure, trust limits, secret-free notifications |
| Common assembly and MCP instances | Not implemented | Real API-key path includes public capabilities; generation drain survives reload and restart |

Keep implementation on one integration branch with reviewable commits. The persistence format,
runtime assembly and lifecycle have one coordinated owner. Whole-plugin documentation coverage
and generated checks may progress alongside implementation; they are not evidence of runtime
correctness. After these foundations, deliver compression hosts, collaboration, permissions,
queue controls, questions, scoped rules and file history; then external protocols, plugin
management and long-running/remote capabilities. TUI implementation and company services remain
outside this work. Security boundaries apply before exposing untrusted execution.

Each unit is usable on its declared parent and passes its relevant public-flow tests. Integrated
boundaries run workspace, lint, graph and packaging-script checks. Review P0–P3 issues and remove
unnecessary machinery before committing. Storage compatibility follows actual format changes,
not a preallocated version shared across incompatible branch commits. Use disposable databases;
never overwrite a user's existing store to make a test pass.

## Alternatives and consequences

Continuing to add domain variants to the closed enum would defer extension ownership and force
another migration for every feature. A universal executable projection language would create a
second context-adoption mechanism and make history depend on plugin interpreters. Both are rejected.

Stable public contracts require cross-crate work and explicit storage compatibility. Snapshots add
storage maintenance but remain disposable; immutable facts and reference closure remain authoritative.
The foundation must be verified through public flows, not inferred from a successful compile or
agreement between design documents.
