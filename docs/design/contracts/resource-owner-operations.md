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

## Native consumers and existing entry points

| Dependency | Operation boundary |
| --- | --- |
| `sqliteStorage`: diagnostics and startup | The client method storage.report({}) calls SqliteStorage.storage_report(). Transport requires database readiness in the persistent profile and never calls report. |
| `sqliteStorage`: Agents | Agents constructs its private AgentRecords repository. Startup, repair and removal use its existing agents/save/delete, ownership, member and unfinished-owned operations. store.query takes a Timeline query. It cannot supply the Agent profile records or classify incomplete/orphaned ownership rows. |
| `sqliteStorage`: Provider and Settings | Provider owns credential-bearing reads and its per-provider writes. Settings owns document import, revision CAS and post-commit notices. Their database transactions remain private. The [Provider management](../appserver/provider-management.md#3-management-contract) and Settings client methods retain their current callers and results. |
| `settingsService`: native documents | Permission, Collaboration, Hooks, MCP, Skills, Scheduler and Modality continue to use their owner-specific reads and writes and the existing BusinessDocument codecs. MCP secret/configuration reads stay in the MCP path. These sources remain outside the declared-source listing. |
| `settingsService`: project admission | Agents create/restore, Hooks and MCP management keep the existing authorize_project operation at their trusted entry. Read-only consumers can use settings.projects. Listing roots does not authorize a new cwd. |

Private SQL closures, the live engine and type-parameterized BusinessDocument operations
are outside the plugin API. The existing client RPCs do not replace these native
dependencies. Removing a remaining native dependency requires an actual owner binding
for each cross-part caller or private wiring confined to one owner.
