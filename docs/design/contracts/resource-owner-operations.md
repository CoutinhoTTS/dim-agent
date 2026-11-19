# Resource owner operations

Rust and foreign callers use the same owner operation. The native binding calls the owner
directly. The rutis binding dispatches its data arguments and returns its result.

## Interfaces already specified

| Dependency | Caller and contract |
| --- | --- |
| `hostPlatform` | Exec and Shell use the task and PTY operations in [execHost §3](../resource-faces.md#3-exechost). Hooks calls configured(request); OCR calls confined(request). Both return the result of that call. Process handles remain with the platform. |
| `fileStorage`, `mentionIndex` | File tools and workspace completion use [fsHost §4](../resource-faces.md#4-fshost), including mentions. Storage clients use the owner's file and lease operations. An opened reader uses openRead/readAt/closeRead. |
| `sessionStore` | Session repository, metadata, attachments, Timeline and worktree associations use store, sessionMetadata, sessionFiles, timelineStore and worktreeAssociations respectively. Session construction binds the existing Session-scoped timelineStore. Path import and fileId reads use the [Session files contract](../appserver/session-files.md). |
| `pluginFileTransactions` | Scheduler and EventHub use pluginFiles.open(), scope.withLock and lease.read/replace for the same atomic text update. Scope.list/remove and lease.readBytes/replaceBytes have their existing contracts. |
| `settingsService` | Declared sources use settings.register/read/update/projects. PluginManager uses the owner-write form for CAS edits and applies its existing writeOnly redaction. SettingsMethods retains the local client-write path. [Settings contract](../appserver/settings-service.md#42-service-settings-owner-path). |

## Additional public bindings

| Target and signature | Existing operation and consumer |
| --- | --- |
| FileScope.directory(name) → FileResult of node-absolute path | PluginFileTransactions.directory already validates the scoped name, creates parents and returns the directory. Modality passes the node-local input filenames and work directory directly to its native adapter. OCR uses its current work/tmp/cache paths and grants. Existing exports use the recorded result filename. |
| FileScope.openRead(name) → FileResult of {readId,size} or null | PluginFileTransactions.open_read already opens a pinned file by relative name and returns None if missing. Bind that reader to the existing fsHost reader identity for the caller on the same node. Use fsHost.readAt/closeRead and the existing caller-unload cleanup. A scoped name never requires a caller-side root lookup. |
| FileScope.writeStream(name, next) → FileResult of null | PluginFileTransactions.write_stream already consumes a reader to EOF under the destination lease. A rutis callback supplies bytes or EOF for the native reader parameter. Preserve atomic replacement and cleanup on failure or caller shutdown. The existing bounded whole-file replaceBytes remains available for small files. |
| settings.declaredBy({instanceId}) → SourceDeclaration[] | SettingsService.declared_by already selects live declarations by registering instance. PluginManager passes the loader's current instance ID. settings.sources lists declarations across owners and omits that identity. |

The OS path, pinned reader and input reader above use a path value, an existing reader ID
and a callback respectively. Existing local media submission/import, legacy node paths,
result settlement and result paging keep the [Modality contract](../plugins/Modality.md).

## Agent records

In the persistent profile Agents injects store, SessionStore, SqliteStorage and
sessionMetadata. SessionStore supplies the native Timeline bootstrap binding.
Agents constructs AgentRecords with the injected database. AgentRecords is private
to Agents today; each of its repository operations uses the SQLite owner's reader or
writer. In the volatile profile there is no such database dependency.

With Agents and SQLite remaining independently mounted, place **only the repository
operations** on the storage side and bind these existing calls directly through rutis:

| Caller in Agents | Repository API; IDs and records are data |
| --- | --- |
| Load the directory and Session ownership | agents({}) → AgentRecord[]; owners({}) → SessionOwner[]. Keep ordered results, invalid-profile rows, ownership errors and validation of the unique default. |
| Install/forget a persisted Agent | save({id,profile,default,owner?}) and delete({id}) → void. |
| Look up delegation and decide if removal is permitted | members({}) → ordered Session/leader pairs; ledMembers({leader}), ownedAgents({owner}), memberSessions({agent}) → ordered IDs; agentOwner({agent}) → owning Session ID or null. |
| Startup repair | unfinishedOwned({}) → {incomplete,orphaned}. Preserve the existing single query and the two distinct cleanup paths. |

These signatures preserve the existing call granularity; they do not require a
new service per method. store.owner maps a **Session to an Agent**. store.query
queries Timeline data. Neither returns the Agent-to-owner relation or orphan scan.
Live Agent hosts, admission/stop, cleanup decisions and the order of calls remain with
Agents. The repository methods can remain private only in an assembly that actually
places their database access inside the same owner. Removing a catalog name alone does
not change the boundary.

## Provider

| Caller → owner | Operation |
| --- | --- |
| Provider management handlers, ProductResolver and auxiliary model binding → ProviderService | Same Provider row constructs these handlers. Catalog adoption, sparse reads, model preferences, validation, transaction contents, retries and credential lookup are private implementation. |
| Provider → SqliteStorage | Explicit injection in Provider.apply. Reads and writes use the shared database for model/connection records, selection, preferences, CAS edits and legacy import. A client provider.* RPC calls Provider and cannot serve as a storage operation in the opposite direction. |
| Rpc → Provider | Ordinary Session model configuration calls configuration(explicit?,cwd?,preferences) → ModelSelection and applyPreferences(current,update) → ModelSelection. modelSelectionHost uses configuration-at-defaults for a different purpose. Bind these existing selection operations for the Rpc caller. |
| Rpc and Collaboration → Provider | Rpc admits a temporary startup selection, checks an inherited connection and retains it after successful adoption/creation. Collaboration retains the inherited configuration after creating a member Session. Provider owns pending admission, rollback, Session-close observation and credential destruction. |

For the last row the required data are the selected model, an admission identity while
creation/adoption is pending, and the target Session identity. Use the existing pending startup binding ID when present. The minimum proposed
operations at that boundary are prepareStartup(existing startup request) →
{model,admissionId?}, checkStartup(source) → void, retainStartup({sessionId,model,
admissionId?}) → void and releaseStartup({admissionId}) → void. Failed or abandoned
admission releases the pending binding, including on caller unload. A successful
retention attaches to the same Session generation. The caller that checks an inherited
connection receives success or the existing error; the HTTP connection stays inside
Provider. This startup contract belongs with the process/model face in #1857.

## Native storage placement

The native product mounts the database-backed implementations of Provider and Settings
**at the owner of the open database**. The existing store owner opens the database once
and holds its lock, readers and writer. At that owner, a private constructor builds the
Provider persistence implementation and another builds the Settings persistence
implementation. They execute the existing SQL on that engine. Only those implementations
access it. Their source code and business rules stay in their respective modules.

The product profile selects the same Provider and Settings rows as before. In the native
profile their factories come from the native owner: they mount each feature's public
bindings in its own rutis lifetime. Each binding publishes the feature operations directly
from its owner. It receives only its domain implementation; it cannot obtain a connection
or submit SQL. There is one registered implementation per face and instance. Replace the
old top-level factories that require SqliteStorage. Do not pass the same database object
to those factories under a different key or in an assembly closure.

| Caller | Face and result | Private work at the native owner |
| --- | --- | --- |
| Management client → registered Provider handler | Existing provider.list/catalog, provider.models.query, provider.create/update and the other [Provider requests](../appserver/provider-management.md#3-management-contract), with their current versioned replies. | Provider validates changes and executes the existing selective queries and atomic edits through its private repository. |
| Session/model consumer → Provider | Session selection and startup operations above; full invocation uses the model operation defined with #1857. | Provider resolves the credential locally and invokes its native driver. Aimux keeps its own driver lifetime and pool. Managed OAuth retains its account lifecycle. |
| Declared-source plugin → Settings | settings.register/read/update/projects. An update still returns its committed snapshot or the existing Settings error. | Settings selects file or native user persistence. Native import writes content and provenance together. CAS and protected-field comparison run against the same stored revision. Notify after commit in commit order. |
| Native-source feature → its owner | Use the feature operation or source-bound operation below. Local management retains the current client contract. | For a secret-bearing source the native owner mounts its feature-specific codec and secret-using operation together. For example, MCP connection setup consumes the effective credentials there. No general source lookup returns raw MCP credentials. |

The three Settings repository operations below are private functions. Provider's repository
likewise has no public wire contract. Neither feature injects sqliteStorage, store.run,
a connection factory or a replacement SQL service. File-only Settings uses its existing
factory without starting SQLite. Provider is mounted only in the profiles that select it.

Feature enable, disable and reload address each binding's own row. Closing Provider stops
admission, settles that generation's work and temporary credentials, and withdraws its faces;
Settings and the database remain usable. Closing Settings drains its admitted updates and
withdraws its registrations while Provider remains usable subject to its declared dependencies.
Native store shutdown closes its dependent rows and joins their work before releasing the
database lock. A new generation binds to the same engine without reopening the file.

Acceptance for these two edges: the assembled inject/read graph has no Provider or Settings
read of SqliteStorage; foreign and native calls hit the same owner; CAS, secret handling,
selective query counts and independent reload retain their current behavior. Once the other
native consumers in #1292 are migrated, delete the SqliteStorage catalog registration.
The database driver remains a private implementation of the storage owner.

## Settings

| Caller → Settings | Current operation and contract |
| --- | --- |
| PluginManager; declared-source consumers | Filter declarations by live loader instance using declaredBy above. Reuse settings.read/update/projects for owner edits and reads. SettingsMethods uses its client path and lists only declared sources. |
| Agents create/restore, Hooks and MCP management | authorize_project(cwd,cancelled) → root registers the authorized root and publishes the existing notice. It is a write performed at trusted admission. project_root/path/sources are reads covered by settings.projects. Binding authorizeProject as an unrestricted plugin method would broaden admission. Keep the operation inside the trusted host entry or retain this as an explicit host-boundary decision. |
| PermissionRules and CommandRules; CollaborationModelPolicy and optional Collaboration runtime | User permission and collaboration policy reads/writes. Expose the existing commandRules.read projection to rule consumers; use the policy owner's read returning {revision,policy} for Collaboration. Preserve absent-profile fallback. |
| HooksSettings and HookSources | hooks, hooks-projects and hooks-plugins. The runtime consumes sources(cwd,cancelled) → HookSourcesSnapshot. Source resolution, revisions and enablement belong to Hooks. |
| Mcp and McpPublication | mcp, mcp-projects, mcp-plugins and mcp-secrets. Publication reads effective server configuration and constructs live connections. Keep partial decoding of individual server entries and credential use at the MCP owner. A list/status response cannot start an authenticated connection. |
| SkillsCatalog, Scheduler, Modality | Each registers and reads/updates its own native source. Keep the existing SkillDocument, ScheduleDocument and ModalitySettings behavior, including duplicate-job checks and validated model defaults. Their public management methods run at the feature owner. |
| Settings → SqliteStorage in the native user profile | Read the existing row; on first use import validated content and provenance atomically; replace at expected revision and publish after commit. File-backed and project settings keep their existing file path and lease behavior. |

There are two different crossings for a native document. The feature calls
Settings.register<D>, read<D> and update<D>. Settings then runs the chosen persistence
backend. Inside a feature, decoding, defaulting and validation belong to its codec.
Inside persistence, the minimal database operations are readUser(source) → row or
missing, importUserIfAbsent(source,document,importedRevision) → winning row, and
replaceUser(source,expectedRevision,document) → committed revision or conflict.
These are storage-owner operations, **not** a proposed public way to read every
native document. Secret-bearing records must stay within their credential owner.

For a non-secret source whose complete existing decode/default/validation and
visibility contracts are representable by the declared-source face, reuse
register/read/update. For other native sources the missing contract is a source-specific
owner read(scope) → {revision,document} and update(scope,expectedRevision,document)
→ {revision,document}, with registration/withdrawal tied to the source's row.
Reuse these operation shapes at the source's boundary; declaring every native source as a
JSON-schema source would change settings.sources and client access. For a source that must retain custom decoding and stay outside settings.sources, the
candidate registration is registerDocument({source,userFile,projectFile?},codec) →
source-bound {read,update}. The codec belongs to the registering feature; preserve its
missing-file defaults, partial decoding and write validation. Withdrawal follows the
registering row. Register only once per source and use its existing file/row and CAS
revision. This is a proposed binding for non-secret data, not an additional global lookup.
A second feature row uses its owner's projection (for example commandRules.read), rather
than registering the same source twice. For MCP, publication refresh already runs inside
McpPublication; no other plugin needs to call refresh to support its settings subscription.
Keep server decoding, secret use and connection setup on the MCP side. Mount the secret-bearing persistence/codec with its consuming operation as specified above.
A native type parameter, storage closure or another row's database handle is never a
wire argument.
