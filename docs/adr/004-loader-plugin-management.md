# 004: Session-owned MCP and business configuration

Status: implementation direction approved by the repository owner, following
[ADR 002](002-plugin-scope.md) D4. This record supersedes ADR 001's
process-shared MCP ownership, not issue #3's structural editable layer. It does
not claim completion of issue #3 or of the full
[methods contract](../design/contracts/session-boundary.md).

## Ownership and updates

MCP connections belong to a Session, not to a workspace or the process. Each
connection generation is an instanced group in the existing Session loader
subtree. Immutable private configuration passes through `create_instance().with(...)`,
not serialized loader entries or mutable shared templates. Closing a Session
closes only its generations. The loader owns instances and disposal; the MCP
publication owner coordinates source selection, authorization and retirement.
Transport tasks belong to the server plugin and settle before disposal completes.
No second loader or lifecycle manager is introduced. Session discovery starts
only after the Agent publishes the Session (`SessionLifecycle`): a Session that
fails assembly never connects, and assembly readiness never observes a
generation retired meanwhile.

Configuration changes are adopted automatically. New bindings use replacement
generations; a binding keeps a superseded generation while the entry stays
enabled, until release, Session closure or explicit disconnect, so a replacement
never breaks a request in flight. Preparing replacements does not await an old
model request. Disabling or removing an entry in a readable source retires its
generations in every Session, even with bindings; an unreadable source retires
nothing. A generation that no lease uses closes even while current, such as a
user server that a project entry now shadows. Disconnect drains the selected
server's generations across this host's Sessions and reports cleanup failure
separately.
Discovery failures remain visible in Session MCP state; persistence alone does
not certify discovery, model adoption or cleanup.

## D4: configuration authority

Structural configuration (plugin enablement, package/runtime paths and node
binding) belongs to loader rows and may reload a row. General
editable structural configuration remains issue #3 work. Business configuration,
including MCP server and Skill documents and future Gateway listener settings,
belongs to the owner's business source. Typed owner writes publish notifications
without reloading the owner or Rpc. Process configuration is not a Timeline fact.

SettingsService provides generic `BusinessDocument` read/update operations with
per-source CAS. A business owner supplies its namespace, source filenames,
payload type and validation. MCP owns its types in `mcp/config.rs`; the Skills
owner implements the existing `SkillSettingsHost`. Settings owns persistence,
not domain interpretation, connections or discovery. It does not yet provide
the [overall design](../design/overview.md)'s P0 Settings supplement: client-facing typed write operations under
plugin-declared schemas. Until then it has no
management-method namespace and exposes no generic document bodies over RPC.

Note: the overall design now splits that supplement ([§6.4](../design/overview.md#64-conventions)).
S8a keeps `BusinessDocument` as the Rust binding and adds change notification
for both user and project scope. S8b (implemented, declarations on C4 rows)
adds the language-neutral named `settings` service and the client
management methods `settings.read`/`settings.update`. It adds no credential
service: each owner handles its own secrets
([settings-service §1.2](../design/appserver/settings-service.md#12-decision-secrets-belong-to-their-owner)).
MCP bearer tokens, `env` and `headers` values stay inline in the `mcp` documents.

Native user sources retain the exact original v13 `settings_documents` contract:
`user/mcp`, `user/mcp-projects` and `user/skills`, plain payload JSON (no `{kind, document}` envelope),
positive source-local versions and opaque `native:<source>:<version>` CAS tokens.
The first successful read imports that source's legacy file, committing the
document and provenance atomically. Concurrent imports adopt one winner. Originals
remain intact; invalid files or failed imports do not create an authoritative row
or empty fallback and can be repaired and retried. SQLite then owns that source;
ordinary edits preserve its import provenance. Project `.mcp.json` retains file
authority and content CAS; standalone Settings may use the file backend.

Successful writes acknowledge persistence and only then notify consumers. Failed
persistence does not publish unsaved runtime intent. A file replacement that is
in place but could not be synced is not unsaved: consumers are notified and the
write reports that its durability is uncertain. Interrupted replies require
rereading before retry. There is no UserConfiguration service, editable-user
document row, configuration RPC namespace, queued flush, v14 envelope, migration
API or two-stage bootstrap. Full process assembly is reconciled once. Unsupported
databases are rejected and preserved, never reinterpreted as v13. See the
[native storage contract](../design/appserver/native-storage-format.md).

There is no host-local approval ([ADR 003](003-approval-scope.md)): a configured
server runs when it is enabled. Project entries start disabled, and a project
file, including its own `enabled` field, cannot enable them. The user's choice is
an MCP business document in the user source `mcp-projects`, keyed by canonical
workspace and server id, written by `mcp.set_enabled` for a project entry. That
call checks its `expectedVersion` against the project source; the record keeps no
configuration, so later edits to an enabled entry take effect (ADR 003 D4).
Disabling, or removing the entry through `mcp.remove`, withdraws the record first
and does not depend on the project file. Only these calls withdraw it: an entry
that leaves the file through an edit or a checkout keeps its record and is
enabled again if it returns. An unreadable record
disables every project entry and is reported in the project application; user
entries stay available. A disable returns once the entry's running generations
have closed in every Session, including while the project file is unreadable. A
record outlives a deleted workspace; a different project later cloned at the
same path with the same id is treated as enabled.
`mcp-approvals.json` is no longer read, migrated or removed. The record is
configuration, not replayable authority: file-root access, tool permission
presets and Session replay enable nothing. This is an additive source in the v13
format, not a schema change; older binaries ignore its row.

## Management and installation boundaries

Process owners provide named Rust `ManagementMethods` contributions that Rpc
looks up per call, without a dependency on any owner
([methods contract §3.2](../design/contracts/session-boundary.md#32-management-method)).
Owners retain declarations, wire validation, business calls and the calls they
accept: an owner's unload interrupts its `Report` calls and waits for its
`Drain` calls. Rpc owns entry admission and reply delivery. There is no
Methods/Management plugin, registration port or adapter row. MCP and Skills
publish their existing management methods; Settings publishes none: S8b's
`settings.read`/`settings.update` belong to SettingsMethods, which depends on Settings (see D4). Local v3
hello lists the methods of the active owners. A namespace whose owner row is
not running and has not failed (Pending, not started yet, or between
generations) returns the retryable `service_reloading`, as before
contributions; one whose owner row cannot run again until it is fixed or
restarted (Failed, Unresolved or Disposed) returns the non-retryable
`service_unavailable`; one with no enabled owner, and non-serving
entries, return `unknown_method`. Rpc
tells them apart from the loader's row status, with no requirement raised to
rutis. An owner that is inactive or reloading affects only its namespace;
business edits do not reload plugins. See
[Management methods](../design/appserver/management-methods.md).

PluginManager's proposed responsibility remains package acquisition, manifest
admission and installation/removal. Installation records, structural enablement,
business source revisions, project enablement and Session instances are distinct.
PR #13 integration must follow D4 without moving business documents into the
editable layer. This branch's approval does not accept another contributor's PR.

## Delivery and deferred work

| Unit | Implementation scope |
| --- | --- |
| Session-owned MCP instances | Independent connections, retained bindings, Session close, cross-Session disconnect and cleanup reporting |
| Business settings persistence | Generic source-local CAS, lazy atomic import and unchanged v13 payloads; MCP's user-local project enablement |
| Skill content | Session-owned content instances with selective replacement; Skills owner supplies SkillSettingsHost |
| Management | Rust owner contributions for existing business methods, not generic plugin installation or structural configuration management |

Declaration-driven method discovery, cross-language plugin method declarations
and the named `managementMethods` describe/call contract, contributed Session
methods, CallChannel, rutis-link export and RemoteGateway remain deferred. There is no
generic credential service; each owner handles its own secrets, and private
credentials never enter catalogs, notifications, diagnostics or Timeline facts.
Package marketplace, Hooks, TUI, dynamic events and production compression are
outside this batch.

Verification uses public persistence, transport and Session flows: original v13
payload compatibility, source-local CAS/import failure, two Sessions sharing
configuration without sharing connections, replacement with retained bindings,
revocation and cleanup failures. Documentation describes implementation scope,
not a substitute for those tests or proof of completing the linked proposals.

`xtask check` guards the source dependency boundary: Settings may use its own
modules, native storage and FileStorage, but not business owners or wire methods,
and it implements no `BusinessDocument` itself; shared protocol must not import
management modules. At run time, a namespace or file belongs to the first
document type that uses it, so two owners cannot share a persisted source. Built-in Session contracts
and Connection's Session-specific MCP calls remain allowed. This source analysis
resolves aliases and re-exports but does not expand macros or infer receiver types.
