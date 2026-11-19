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
| `sqliteStorage` | Store implements [§5](../resource-faces.md#5-store-and-timelinestore). The engine supplies owner-specific persistence operations for Settings, Provider and repository records. The storage owner serves storage.report({}) → StorageReport. Transport declares a load-order dependency on the database lock for the native profile. SQL connections, read closures, transaction boundaries and migrations stay within the backend. |
| `settingsService` | Plugins use [settings](../appserver/settings-service.md) for declared registration/read/update/projects. Native source parsing, secret-bearing document edits and source authorization execute at the source owner. Public settings replies preserve redaction, revision, CAS and notices. |

## Additional file operations

On the scope returned by pluginFiles.open, directory(name) creates the authorized directory.
It returns its root-relative name. openRead(name) returns {readId,size}, or null for a missing file;
readAt(readId, offset, limit) and closeRead(readId) use the pinned-reader contract.
writeStream(name, next, cancelled) requests at most 64 KiB per pull; next returns bytes or an explicit end-of-stream result. It holds the destination lease until EOF, then publishes
the replacement. Failure, scope close or cancellation releases the reader or temporary
writer and preserves the prior file on a failed replacement. The native binding streams
directly. A native media adapter needing a local directory works at the resource owner.
Scope and lease paths keep the existing plugin namespace and symlink checks.

## Compatibility and delivery

1. Land each fsHost family with the matching file-tool and storage consumers. Keep
   [Session path import and fileId reads](../appserver/session-files.md) and existing limits.
2. Land execHost with caller cancellation, provider disconnect, repeat waits, shell input,
   output cursor reset and terminal cleanup. Preserve legal helper requests at the maximum
   aggregate output size: a single bridge frame size must never become a smaller product
   limit. Verify byte-for-byte parity for the largest legal request and reply before rollout.
3. Migrate repository/bootstrap, Settings and plugin-file consumers by owner. Each storage
   operation preserves its existing atomic write and failure result. Validate restart and
   redaction with disposable SQLite and the file-backed profile where applicable.
