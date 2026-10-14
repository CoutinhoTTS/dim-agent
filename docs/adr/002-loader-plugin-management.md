# 002: Session-owned MCP and editable plugin configuration

Status: implementation direction approved by the repository owner for this branch.
This record supersedes ADR 001's proposed process-shared MCP ownership and
source-first user configuration direction. It does not claim completion of issue #3.

## Ownership and updates

MCP connections belong to a Session, not to a workspace or the process. Each
connection generation is an instance of an instanced group in the existing
Session loader subtree. The group template describes the plugin; immutable,
private connection configuration is passed through `create_instance().with(...)`.
It is not serialized into loader entries, and changing one Session's connection
does not edit the shared template. Closing the Session closes all its generations.
There is still one loader and one lifecycle implementation.

Configuration changes are adopted automatically, as in the current public
contract; they are not deferred until a new Session or an explicit refresh.
New tool bindings adopt the replacement generation. Existing user-source bindings
retain their generation until released, unless the Session closes or the source
is explicitly disconnected. Preparing a replacement does not wait for an old
model request to finish. Failed discovery remains visible through the existing
Session MCP state. Configuration updates do not guarantee successful connection
or adoption merely by returning a persistence acknowledgement.

Project configuration changes, disablement, unreadable authority and revocation
retire affected old generations even if bindings remain. Retained handles check
current authorization before execution. Revocation removes the host-local grant,
rejects new work and awaits disposal of all affected instances in this host before
returning success. A cleanup failure must not disguise the persisted revocation.
Ordinary Session closure cannot remove another Session's connections.

The loader owns instances and disposal. The MCP coordinator owns source selection,
authorization and retirement eligibility, not a parallel plugin tree. The server
plugin owns transport tasks and waits for their cancellation and cleanup. Calls
to loader mutation APIs that acquire its operation lock stay outside plugin
apply/dispose; instance creation uses the framework's lock-free instance API.

## Configuration and management

User-managed plugin intent uses the root loader's editable layer, persisted by an
application `Persist` implementation backed by SQLite. Project `.mcp.json` stays
an external source, and `mcp-approvals.json` stays host-local authorization. A
persisted intent, an authorization grant and a runtime generation are different
identities. Private credentials must not enter generic catalog or event output.

The editable layer must not be replaced by an MCP publisher's whole-tree
reconcile. Session instance creation does not itself solve this: initial assembly,
editable intent and external-source projections have explicit owners. Shared
group templates do not contain mutable per-Session connection configuration.

Management uses loader operations, with stable serialized projections rather
than framework handles. Persistence, applied state and cleanup results remain
distinct; a loader event does not prove persistence or successful business work.
No automatic replay of uncertain authorization operations is introduced.

The database format policy remains explicit: no automatic migration of an old
native database. Any changed persistent representation requires a format decision
and disposable-data tests; existing settings are not silently reinterpreted.

## Delivery and verification

Implement and verify Session ownership before changing user configuration
authority. Then add editable persistence and management, followed by Skill content
rows. Hooks, the TUI, dynamic events and the compression model host are outside
this batch. Update the delivery status as each complete unit lands.

| Unit | Status |
| --- | --- |
| Session-owned MCP instances | Implemented; independent connections, Session close, retained bindings, cross-Session revocation and cleanup failures covered by public flows |
| Editable user persistence and management | In progress; native settings still use the v13 source-first format |
| Skill content rows | Not implemented |

Public-flow checks cover two Sessions using one configured server, closing only
one Session, automatic replacement with a retained old binding, cancellation
during instance creation, cross-Session project revocation, failed cleanup,
restart/import behavior and secret-free management output. Run the workspace
tests, Clippy, architecture checks and regenerated graph checks before completion.
