# 003: Session-owned MCP and business configuration

Status: implementation direction approved by the repository owner for this branch,
corrected to the authoritative D4 in PR #15 (`origin/pr-15-design`, `34af2f1`).
PR #15 owns ADR 002, Plugin scope assignment and configuration placement; this
branch record is numbered 003 to avoid that collision. This record supersedes
ADR 001's process-shared MCP ownership, not issue #3's structural editable layer.
It does not claim completion of issue #3 or the full PR #15 method contract.

## Ownership and updates

MCP connections belong to a Session, not to a workspace or the process. Each
connection generation is an instanced group in the existing Session loader
subtree. Immutable private configuration passes through `create_instance().with(...)`,
not serialized loader entries or mutable shared templates. Closing a Session
closes only its generations. The loader owns instances and disposal; the MCP
publication owner coordinates source selection, authorization and retirement.
Transport tasks belong to the server plugin and settle before disposal completes.
No second loader or lifecycle manager is introduced.

Configuration changes are adopted automatically. New bindings use replacement
generations; retained user bindings keep their old generation until release,
Session closure or explicit disconnect. Preparing replacements does not await an
old model request. Project changes, disablement, unreadable authority and
revocation retire old generations even with retained bindings. Each project
operation checks current authorization. Revocation persists the grant removal,
rejects new work and drains affected instances across this host's Sessions before
successful acknowledgement. Cleanup failure does not roll back revocation.
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
not domain interpretation, connections or discovery. It has no management-method
namespace and does not expose generic private document bodies over RPC.

Native user sources retain the exact original v13 `settings_documents` contract:
`user/mcp` and `user/skills`, plain payload JSON (no `{kind, document}` envelope),
positive source-local versions and opaque `native:<source>:<version>` CAS tokens.
The first successful read imports that source's legacy file, committing the
document and provenance atomically. Concurrent imports adopt one winner. Originals
remain intact; invalid files or failed imports do not create an authoritative row
or empty fallback and can be repaired and retried. SQLite then owns that source;
ordinary edits preserve its import provenance. Project `.mcp.json` retains file
authority and content CAS; standalone Settings may use the file backend.

Successful writes acknowledge persistence and only then notify consumers. Failed
persistence does not publish unsaved runtime intent. Interrupted replies require
rereading before retry. There is no UserConfiguration service, editable-user
document row, configuration RPC namespace, queued flush, v14 envelope, migration
API or two-stage bootstrap. Full process assembly is reconciled once. Unsupported
databases are rejected and preserved, never reinterpreted as v13. See the
[native storage contract](../design/appserver/native-storage-format.md).

Settings also provides generic namespaced host-local grants in
`<namespace>-approvals.json`, retaining MCP's existing version-1
`mcp-approvals.json` format. Business owners supply scoped identity and effective
configuration fingerprints; Settings validates/persists digests. Grants are not
business documents, loader intent or replayable authority. Missing grants deny
execution and damaged stores fail closed. Package acquisition, file-root access,
tool permission presets and Session replay never confer execution approval.

## Management and installation boundaries

Process owners provide named Rust `ManagementMethods` contributions collected
by Rpc via `Contributions<T>`. Owners retain declarations, wire validation and
business calls; Rpc owns admission, cancellation, accepted-task drainage and
generation-fenced delivery. There is no Methods/Management plugin, registration
port or adapter row. MCP and Skills publish their existing management methods;
Settings publishes none. Local v3 hello advertises the collected methods;
non-serving entries return `unknown_method`. Structural reloads follow ordinary
dependencies; business edits do not reload plugins. See
[Management methods](../design/plugins/management.md).

PluginManager's proposed responsibility remains package acquisition, manifest
admission and installation/removal. Installation records, structural enablement,
business source revisions, execution grants and Session instances are distinct.
PR #13 integration must follow D4 without moving business documents into the
editable layer. This branch's approval does not accept another contributor's PR.

## Delivery and deferred work

| Unit | Implementation scope |
| --- | --- |
| Session-owned MCP instances | Independent connections, retained user bindings, Session close, cross-Session project revocation and cleanup reporting |
| Business settings persistence | Generic source-local CAS, lazy atomic import, unchanged v13 payloads and host-local namespaced grants |
| Skill content | Session-owned content instances with selective replacement; Skills owner supplies SkillSettingsHost |
| Management | Rust owner contributions for existing business methods, not generic plugin installation or structural configuration management |

Declaration-driven method discovery, cross-language plugin method declarations
and the named `managementMethods` describe/call contract, contributed Session
methods, CallChannel, rutis-link export and RemoteGateway remain deferred. Generic
credential references and their resolver are also future work; current private
credentials never enter catalogs, notifications, diagnostics or Timeline facts.
Package marketplace, Hooks, TUI, dynamic events and production compression are
outside this batch.

Verification uses public persistence, transport and Session flows: original v13
payload compatibility, source-local CAS/import failure, two Sessions sharing
configuration without sharing connections, replacement with retained bindings,
revocation and cleanup failures. Documentation describes implementation scope,
not a substitute for those tests or proof of completing the linked proposals.

`xtask check` guards the source dependency boundary: Settings may use its own
modules, native storage and FileStorage, but not business owners or wire methods;
shared protocol must not import management modules. Built-in Session contracts
and Connection's Session-specific MCP calls remain allowed. This source analysis
resolves aliases and re-exports but does not expand macros or infer receiver types.
