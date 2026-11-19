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
| Provider management handlers, ProductResolver and auxiliary model binding → ProviderService | Same Provider row constructs these handlers. Provider owns selection policy, validation, retries and credential resolution. |
| Provider → providerRecords | Provider uses the storage operations below for records, selections, preferences, conditional edits and legacy import. |
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

## Native persistence interfaces (proposal)

The host mounts Provider, Settings and storage independently. Provider injects
`providerRecords`. Settings injects `settingsRecords` in the native SQLite profile.
Storage publishes both as ordinary process data faces and owns their SQLite transactions.
Each consumer drains its own work on unload. Storage drains its writer on shutdown.

### Settings → settingsRecords

| Operation | Contract |
| --- | --- |
| readUser({source}) | Return null or {version,document}. The document is the stored text. Settings decodes it using the registered format. |
| importUserIfAbsent({source,document,importedRevision}) | In one transaction insert version 1 and provenance if absent, read the winning row and return it. Settings reads and validates the legacy file only after a miss. |
| replaceUser({source,expectedVersion,decision}) | Check the expected version first. For an accepted document, write it, increment the version and return the committed version. For a rejected protected-field change, return the existing rejection without writing. Missing row, conflict, overflow and storage failure keep their existing mappings. |

Settings forms the existing revision token. It uses the snapshot from its existing
native read/import step to compare protected fields after defaults, with the existing
owner/client distinction. Check the expected revision before protected-field comparison.
If protected-field validation rejects an edit, pass that decision through the same version check. A concurrent change must still yield conflict before a stale validation error.
A successful native write enqueues the existing source/scope/revision notification in writer
commit order before acknowledging it; dispatch outside the transaction. Do not publish
from independently arriving RPC replies. An import sends no update notice. A cancelled
or conflicting write cannot generate a change notice for an uncommitted revision.
Project and file-profile operations retain the existing lease and notification path.

### Provider → providerRecords

The following are typed repository operations. The selectors and mutation variants are
closed data types. They name existing query shapes and write transactions, never SQL text,
a connection or a callback to execute under a lock.

| Operation family | Current call sites and proposed inputs/results |
| --- | --- |
| readSelection(scope); writeSelection(scope,expectedVersion,selection) | selection/select. Return the stored selection and version; the write checks the scope version and commits the selected model atomically. |
| readPreferences(providerId,modelId) | saved_preferences. Read the one model preference field and keep the existing absent-value behavior in Provider. |
| readRecord(providerId,projection) | catalog, catalog_for, catalog_target, resolve_connection, probe, discovery and edit preparation. Select header/version, one target model, managed catalog or full editable record as required. Return missing or the matching persisted projection. A projection containing an inline credential returns its stored value. |
| findCandidates(criteria); listProviders(filter); queryModels(page); managementPage(page) | Candidate IDs/ordered models, provider list, filtered model pages and management windows use their existing indexed selectors, limits, ordering and row-error behavior. Pass bundle identity, required catalog inputs and account facts as data. Keep transaction-local adoption and page selection together wherever the existing query does both. |
| activateSource(revision,baseFacts); adoptProvider(id,revision,baseFacts) | Existing source activation and per-provider adoption transactions. Supply only the bundle facts required by the existing operation. |
| commitEdit(providerId,precondition,edit,replyShape) | Closed edit variants: create, delete, connection/header patch, model create/edit/reset/delete, preferences, single/bulk enablement and discovery refresh. Preserve each existing transaction's touched columns, indexes, write precondition and projection. Create checks absence; versioned edits check the expected version. Refresh and connection changes that compare the prior record keep that exact comparison. Return the committed version and requested existing projection. |
| importLegacy(batch) | The existing legacy import commits its related records together and returns the existing item results. Cancellation and rollback keep the original transaction boundary. |

Provider retains validation, bundled model semantics, selection policy, secret resolution
and HTTP requests. For probe/refresh it reads its snapshot, performs I/O outside SQLite,
then conditionally commits if a write is called for. A failed precondition is the current
conflict; the binding does not replay a network request. Ordinary provider management
RPCs, selection and model invocation keep their existing or separately specified faces.

### Credential values in replies

Any trusted plugin in the same process-service scope can inject either repository face.
`providerRecords.readRecord` returns the stored fields selected by its projection,
including inline credentials when selected. A stored secret reference is returned as a
reference. Provider resolves it through its configured secret host and obtains managed
OAuth credentials from their existing owner.

`settingsRecords.readUser` and `importUserIfAbsent` return the complete stored document,
including original values in `mcp-secrets`. `replaceUser` returns the committed version.
MCP reads its credential document through Settings and uses it to establish connections.
Provider and MCP business replies omit credentials. The business API `settings.read`
serves declared sources and excludes the native `mcp-secrets` document.

## Settings

| Caller → Settings | Current operation and contract |
| --- | --- |
| PluginManager; declared-source consumers | Filter declarations by live loader instance using declaredBy above. Reuse settings.read/update/projects for owner edits and reads. SettingsMethods uses its client path and lists only declared sources. |
| Agents create/restore, Hooks and MCP management | authorize_project(cwd,cancelled) → root registers the authorized root and publishes the existing notice. It is a write performed at trusted admission. project_root/path/sources are reads covered by settings.projects. Binding authorizeProject as an unrestricted plugin method would broaden admission. Keep the operation inside the trusted host entry or retain this as an explicit host-boundary decision. |
| PermissionRules and CommandRules; CollaborationModelPolicy and optional Collaboration runtime | User permission and collaboration policy reads/writes. Expose the existing commandRules.read projection to rule consumers; use the policy owner's read returning {revision,policy} for Collaboration. Preserve absent-profile fallback. |
| HooksSettings and HookSources | hooks, hooks-projects and hooks-plugins. The runtime consumes sources(cwd,cancelled) → HookSourcesSnapshot. Source resolution, revisions and enablement belong to Hooks. |
| Mcp and McpPublication | mcp, mcp-projects, mcp-plugins and mcp-secrets. Publication reads effective server configuration and constructs live connections. Keep partial decoding of individual server entries and credential use at the MCP owner. A list/status response cannot start an authenticated connection. |
| SkillsCatalog, Scheduler, Modality | Each registers and reads/updates its own native source. Keep the existing SkillDocument, ScheduleDocument and ModalitySettings behavior, including duplicate-job checks and validated model defaults. Their public management methods run at the feature owner. |
| Settings → settingsRecords in the native user profile | Read the existing row; on first use import validated content and provenance atomically; replace at expected revision and publish after commit. File-backed and project settings keep their existing file path and lease behavior. |

There are two different crossings for a native document. The feature calls
Settings.register<D>, read<D> and update<D>. Settings then runs the chosen persistence
backend. Inside a feature, decoding, defaulting and validation belong to its codec.
The native persistence calls and their placement are specified above.

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
Keep server decoding, secret use and connection setup on the MCP side. MCP consumes its own credential-bearing configuration through its existing trusted path.
A native type parameter, storage closure or another row's database handle is never a
wire argument.
