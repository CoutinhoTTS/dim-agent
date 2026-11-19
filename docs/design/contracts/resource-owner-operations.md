# Resource owner operations

Proposed plugin boundaries for process execution, files and persistence. Request, result,
error, cancellation and cleanup types for the resource operations are defined in the
[resource contract](../resource-faces.md). An ID belongs to the owner that issued it.

| Catalog entry | Public operation and result |
| --- | --- |
| `hostPlatform` | `execHost` implements [§3](../resource-faces.md#3-exechost). Ship the task family together: start/wait/kill/settle, output/tail/removeOutputDirectory and shell/write/resize share task identity and terminal settlement. Ship info/confined/configured against the same resource owner. |
| `fileStorage` | `fsHost` implements [§4](../resource-faces.md#4-fshost). The storage family adds stat/readBytes, lock/unlock/backup, createDirectory/replace/list/remove to the existing pinned-reader operations. The tool family provides resolve/read/write/edit/glob/grep/mentions. Run traversal and matching at the file owner. |
| `mentionIndex` | Workspace completion calls `fsHost.mentions({path,query,limit?}) → {candidates}`. The file owner retains the walk, cache and invalidation policy. |
| `pluginFileTransactions` | Scheduler, EventHub and Modality open their authorized [plugin file scope](../../../crates/dim-app-server/README.md). A text transaction uses withLock and lease.read/replace. Directory listing and removal use scope.list/remove. Extend the same scope with directory creation and streaming reads/writes for media. |
| `sessionStore` | Session rows use store, metadata uses sessionMetadata, files use sessionFiles, timeline storage uses the Session timelineStore, and worktree ownership uses worktreeAssociations. Bootstrap binds these operations at Session construction. The storage engine and its open/close sequence belong to the repository owner. |
| `sqliteStorage` | Store implements [§5](../resource-faces.md#5-store-and-timelinestore). Agents uses agentRecords, diagnostic reads use storage.report and Transport gates on storage in the native profile. SQL connections, read closures, transaction boundaries and migrations stay within the backend. |
| `settingsService` | Plugins use [settings](../appserver/settings-service.md) for declared registration/read/update/projects. Native source parsing, secret-bearing document edits and source authorization execute at the source owner. Preserve the redaction policy of each entry, revisions, CAS and notices. |

## Plugin files and local media paths

`pluginFiles.open()` selects the existing configured root. Scope arguments are relative
names. The additional scope methods return the existing FileResult envelope:

| Method | Successful value |
| --- | --- |
| directory(name) | The absolute directory path on the file owner's node. Create missing parents with the existing private permissions; reject a link or non-directory component. Native callers retain the lossless OS path. A path that cannot be encoded on the wire fails with invalid_path. |
| openRead(name) | {readId,size}, or null if absent. Pin the scoped file and bind retention to this caller and scope. Use existing fsHost.readAt({readId,offset,limit}) and fsHost.closeRead({readId}) on the same node. Closing the scope releases its remaining IDs. |
| writeStream(name,next,cancelled) | null after atomic replacement. Each pull requests up to 64 KiB; next returns bytes or end. The lease covers the whole write. Failure, interruption or scope close discards the temporary file. |

Modality and its native adapter execute on the same node as the task files. The following
are paths in that node's filesystem. A path is never interpreted relative to a caller's cwd. Non-local clients use the existing task result pages to retrieve artifacts.

| Consumer | Handoff |
| --- | --- |
| Provider adapter submit | Modality calls directory(task input) and directory(task work). For each stored input it joins the validated single-component filename to the input directory and supplies Job.files and Job.work_dir to the existing adapter submit. New node_path submissions retain the bounded import into task storage. An old record containing node_path uses its original node-local path. |
| Host OCR | Supply the existing input file, work, tmp and cache directories to the confined helper on the control node. Preserve the executable, argv, environment and read/write/service grants of the selected helper. Other nodes retain node_unavailable. |
| Result settlement and export | Validate result names and regular files at the work root, pin each source, and publish through the existing counted/digested write and revision check. Task export joins the validated result name to directory(task result) and returns the existing TaskFileSource. Task result reads use bounded pages. |

The native binding calls the same directory operation in process. No additional path lookup,
file upload, Session file ID or cross-node file mapping is required.
See [Modality](../plugins/Modality.md) and [plugin storage](../../../crates/dim-app-server/README.md#trusted-plugin-storage).

## SQLite consumers

The native storage profile keeps one database, its existing read connections, writer and
schema. A feature's SQLite repository code executes at the database owner. Each feature
continues to own validation, notifications, task cancellation and external credentials.
Bind feature-specific handlers there at profile construction; activating and withdrawing
their operations follows the feature's current load lifetime. No plugin receives a database
key, raw SQL callback, standalone connection or second database opener. The repository code remains in its feature module; the storage
profile owns the SQLite driver. A separately mounted consumer reaches the operations
in the table via a data face. Only code running inside the owning component may call its
private repository functions.

| Caller | API |
| --- | --- |
| Session bootstrap, Timeline, Session files and metadata | Use store, timelineStore, sessionFiles, sessionMetadata and worktreeAssociations with their [existing contracts](../resource-faces.md#5-store-and-timelinestore). At Session construction install the Session-scoped timelineStore binding from the storage owner; Agents stops calling native.timeline_store(). Readers, writers, file sweeps, migrations and export inside the backend retain private engine access. |
| Agents startup, repair and deletion | Use the operation-only agentRecords binding below in the persistent profile. The volatile profile keeps its current in-memory behavior. |
| StorageMethods and Transport | The SQLite owner exposes storage.report({}) → StorageReport. The existing client storage.report forwards to it with its present storage_unavailable mapping. Native Transport declares a load-order gate on storage; starting a listener performs zero report reads. |
| Provider management and runtime | The native Provider repository serves the existing [Provider operations](../appserver/provider-management.md#3-management-contract) at the storage node. Reads use provider.list/catalog/selection, provider.management.list and provider.models.query; writes use provider.create/update/delete/select/refresh, provider.preferences.update and provider.models.create/edit/delete/reset/set_enabled with their current request, result, version and conflict types. Legacy import retains its existing operation. In-process selection, model lookup and credential-backed execution enter through modelSelectionHost, selectableModelsHost, auxiliaryModel and the invocation owner. The Provider implementation executes its existing sparse queries, per-provider transactions, adoption and credential resolution locally. Credentials and imported source bodies stay off the repository interface. |
| Settings persistence | The file/native profile selects the Settings document backend once. Its private read(source,scope) and compare-and-replace(source,scope,expectedRevision,document) support the operations below. A missing native user row imports its validated file and provenance in one transaction; concurrent import returns the winning row. A write checks the revision and protected fields against the stored row, commits once, then emits the same ordered notification. Project settings retain their file leases. |

The Provider runtime maps saved-preference reads to savedPreferences(providerId,modelId),
one-model inspection to catalogTarget(providerId,modelId), and selection to selection(cwd)
on that same owner. Source activation, lazy per-provider adoption, profile edits and legacy
import each retain their original transaction. Public dispatch returns the owner's existing
view or version. A connection-bearing lookup ends at the native driver on the owner node.

agentRecords uses the existing AgentRecords owner methods with data arguments:

| Method | Result |
| --- | --- |
| agents({}) | Ordered records {id,profile,default,owner}, with the existing per-row profile decode success/error. |
| save({id,profile,default,owner?}), delete({id}) | Empty success. Keep existing insertion, duplicate/profile checks and delete semantics. |
| ownedAgents({owner}), ledMembers({leader}), memberSessions({agent}) | Ordered arrays of IDs. |
| agentOwner({agent}) | Owning Session ID or null. store.owner({sessionId}) returns the Agent that owns a Session. |
| members({}) | Ordered {sessionId,leaderSessionId} pairs. |
| unfinishedOwned({}) | {incomplete,orphaned}, from the existing single recovery query. |
| owners({}) | Existing per-Session ownership snapshot with sessionId, optional agentId/protocolVersion pair and optional row error. |

## Settings callers

Declared sources continue to use [settings.register/read/update/projects](../appserver/settings-service.md#42-service-settings-owner-path).
The client settings namespace retains its local admission and client-write policy. The
plugin config tool retains its writeOnly redaction and preservation of userOnly members.

| Caller | API and ownership |
| --- | --- |
| PluginManager | Add settings.declaredBy({instanceId}) → SourceDeclaration[] for the live row returned by the loader. The instance ID is process-local, ephemeral and never persisted. Return only live declared sources, in the existing registry order. An absent or unloaded row has no declarations. Use existing settings.read/update for owner edits. Keep the existing redaction, nested writeOnly refusal and CAS. SettingsMethods implements settings.sources at the registry owner. |
| Agents create/restore; Hooks and MCP local management | Authorize the selected cwd inside the existing trusted admission entry, before the dependent action. The host invokes Settings' private authorizeProject(cwd,cancelled) → root with the existing canonicalization, idempotency and notice order. These entry handlers own that admission step. Read-only projectSources/projectRoot/projectPath callers use the root/cwd pairs from settings.projects and its change notice. Match cwd exactly, retain the invalid-scope error for an unknown root, and reuse the same snapshot within a call. Project lookup alone never registers a root. |
| Permission and Collaboration | Use commandRules.read for the Session rule projection. permission.rules/set_rules and collaboration.model_policy/set_model_policy retain their current request/reply and validation. Collaboration runtime uses the same policy read operation as the management handler, with the current optional-profile fallback. |
| Hooks | The source owner supplies sources(cwd,cancelled) → HookSourcesSnapshot to the Hooks runtime. hooks.catalog/save/set_enabled and hooksService.plugins/refusal/register/withdraw use that owner's hooks, hooks-projects and hooks-plugins records. Project hooks files use fsHost.stat/readBytes. |
| MCP | mcp.catalog, configuration, save, update, remove, move, set_enabled, import and auth management keep their existing wire records. The MCP publication owner reads authorized projects from settings.projects and consumes its existing effective-source projection inside MCP. It reads enablement/secrets only on the paths that currently require them. MCP alone constructs the live connection configuration. mcp, mcp-projects, mcp-plugins and mcp-secrets are decoded there; authenticated network operations receive credentials on the executing node. Plugin contributions use mcpService.register/withdraw. |
| Skills | The Skills settings owner serves skills(cancelled) → SkillSettingsSnapshot and updateSkills({expected,id,enabled},cancelled) → SettingsVersion. skills.catalog/installations/install/update/remove/set_enabled and plugin registration keep their existing result and failure types. |
| Scheduler | The schedule owner reads its validated schedule snapshot and CAS-updates it through the same ScheduleDocument codec on file and SQLite profiles. scheduleJobs.call and schedule.list/save/delete/set_enabled/run_now retain their existing payloads and conflict behavior. |
| Modality | Evaluation reads the owner's defaults snapshot {revision,document:ModalitySettings}; modality.configure performs the existing revision-checked update. modalityTargets.resolve and modality.capabilities consume that same selection policy. |
| Workflow, prompts, execution defaults, model log, Provider preferences and Web configuration | Reuse workflowFiles and the declared settings bindings each already uses. Keep their existing source identifiers, defaults and cancellation. |

For native source families the feature's registered operations above are the plugin boundary.
Their shared, **private** source binding is read(scope,cancelled) → {revision,document}
and update(scope,expectedRevision,document,cancelled) → {revision,document}, specialized
to the existing BusinessDocument codec. Construct these handlers with the selected Settings
backend inside the persistence owner. The source registry keeps registration collision
checks and unload cleanup for both source kinds. Native-only sources stay absent from
settings.sources and unavailable through generic settings.read/update. Management RPCs
and their native consumers call the same owner operation directly.

## Helper output across frames

Ordinary execHost.output/tail retain their [text cursor contract](../resource-faces.md#34-output).
confined/configured return independent, bounded **raw bytes** for stdout and stderr,
including invalid UTF-8. Sizes and limits apply separately to captured stdout and stderr. The following is their Dim wire binding. The native result
remains the existing two byte buffers plus status, flags and diagnostics.

```text
confined(request, receivePages)   → InlineResult | {kind:"pagesReceived"}
configured(request, receivePages) → InlineResult | {kind:"pagesReceived"}
InlineResult = existing result with stdout and stderr as base64 strings
receivePages(metadata, {stdoutSize:u64, stderrSize:u64}, readAt) → void
readAt({stream:"stdout"|"stderr", offset:u64, limit:u32}) → {bytes:base64}
```

- Return InlineResult directly when the combined captured length is at most 64 KiB.
  Otherwise invoke receivePages once after settlement with the existing status and flags
  and one reader over both immutable buffers. Return pagesReceived only after the receiver
  acknowledges completion. The SDK assembles the original result and exposes it only when
  the outer call succeeds. Small replies add zero calls; an execution failure never calls
  receivePages.
- Offsets and sizes count decoded bytes. Use the pinned-reader paging semantics:
  clamp each reply to 64 KiB; zero limit or offset at/past EOF returns empty. Repeated
  reads at the same offset return identical bytes. Each base64 page is at most 87,384
  characters. Readers can alternate streams. The raw buffers already captured by the
  helper stay in memory for this call; no path, temp file or second capture is required.
- The outer call awaits receivePages. On receipt completion, failure, caller unload, link
  close or cancellation, invalidate readAt and release both buffers. A retained reader
  fails closed. Callback/read failure fails the call without rerunning the command.
  Cancellation before process settlement follows the existing kill-and-join behavior.
  Execution errors and configured/confined output limits keep the respective owner semantics.
- For configured stdin above the same inline bound the wire argument is
  {size,readAt(offset,limit) → {bytes:base64}}. Pull bounded pages from offset zero into
  the existing stdin writer, observe backpressure and close on completion. A short final page must end at the declared size. Premature EOF, an oversized page or a failed input callback cancels and settles the child. Inline
  stdin and absent stdin preserve the existing byte and EOF behavior.
