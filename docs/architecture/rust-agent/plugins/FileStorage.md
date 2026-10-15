# FileStorage

Owner: [file_storage.rs](../../../../crates/dim-agent-capabilities/src/file_storage.rs)
and [windows.rs](../../../../crates/dim-agent-capabilities/src/file_storage/windows.rs).
Provides authorized-root-relative file operations and leases with secure path
traversal and serialized mutation, plus directory listing and removal.

## Dependencies

No injected services (`Deps = ()`).

## Events and lifetime

No Timeline events. Each `apply` opens fresh root-directory handles; blocking
operations and leases are tracked by that generation. Close stops admission,
drains operations/leases and releases resources. Retained old service handles
reject new operations. Clone the plugin to preserve append-only authorized-root
indices across reload; rebuilding from the original list could reuse indices.
Cancellation is checked before commit, not treated as rollback after installation.
Roots reject symlinked traversal; this is a trusted host storage boundary, not
isolation from the host or same-user processes. Do not automatically retry an
uncertain committed mutation.

## Leases and lock keys

A lease locks a key file with the target's name in the reserved
`.dim-file-storage-locks` directory beside the target, so the filesystem's own
case and Unicode aliases of one name share one key. Releasing a lease keeps its
key; only directory removal unlinks keys, one at a time while holding each. A
lock counts only if its key is still linked once held. A key linked when locked
stays linked while held, because only its holder unlinks it. A key unlinked
before it was held is stale: the current key is resolved once more, and a
second stale key returns `Locked`. Without that check, a lock racing a failed
removal admitted a second holder:

```text
B: lock(d/x)              remove(d)                    C: lock(d/x)
open key x (inode 1)
                          lock, unlink every key,
                          rmdir the lock directory
                                                       recreate the lock directory,
                                                       create and lock key x (inode 2)
                          rmdir d fails, release keys
lock inode 1              -> before: B and C both hold d/x
                          -> now: inode 1 is unlinked, so B reopens key x,
                             finds inode 2 held and returns Locked
```

A replacement writes its temporary file in
`.dim-file-storage-locks/.dim-file-storage-temps` under its key's name, so only
that key's holder writes or removes it. A leftover from an interrupted
replacement is discarded by the next replacement of that name.

`auth.json` also takes the TypeScript-compatible `auth.json.lock` marker, which
is reclaimed only while holding the `auth.json` key. A marker is stale once it
is older than 30 seconds and names a process that is no longer running; a
marker naming this process counts as live. The exception is a marker this
process failed to release: it keeps the owner record, whose nonce only that
marker carries, and reclaims the marker at once, so removal and locking need
not wait for the process to exit.

## Listing and removal

`list` returns the entries of a directory below an authorized root, sorted by
name, each a `File`, `Directory` or `Other` (a link, reparse point or special
file, never followed). FileStorage's lock, temporary and account-marker
resources are omitted, including names the filesystem equates with them.

`remove` makes an entry absent. Files, including those beneath a directory,
are removed under the lock leases use, so removing a leased file, or a directory
with a lease held anywhere beneath it, returns `Locked`. A link is removed
itself. A directory is emptied depth first through directory handles, without
following links or entering a filesystem mounted at or inside it (a Linux bind
mount of the same filesystem is not detected). An explicit stack replaces
recursion, so depth costs no thread stack. On Unix, removal holds one directory
descriptor at a time: it returns to each parent through `..` and fails if that
is no longer the directory it descended from, or starts again from the target if
a concurrent removal deleted it. Windows holds a handle per level, since it
cannot reopen a parent from a handle without a path.

In each directory, leftovers go first: entries the listing hides by name, other
than the lock directory and a regular account marker. Examples are a resource
from an earlier version, or a marker that is a directory or a link. They follow
the same rules as business entries, and going first means a stray marker cannot
block locking `auth.json` beside it. The directory's FileStorage resources go
last and hold one key at a time, however many names the directory has used:

```text
remove(d), once d's other entries are gone:
  auth.json.lock: holding key auth.json, reclaim it if stale; live -> Locked
  for each key k in .dim-file-storage-locks:
    lock k                                         held -> Locked
    k unlinked by a concurrent removal?            -> skip it
    unlink temps/k, unlink k, release k
  for each temps/t still left (written under a key taken since,
  or named at random by an earlier version): the same, under key t
  rmdir temps, rmdir .dim-file-storage-locks, rmdir d
                    an entry or key created meanwhile -> DirectoryNotEmpty
```

Removal therefore never unlinks a key or temporary file that a holder has
locked. An absent entry counts as removed. Removal is neither atomic nor
synced: an error or cancellation can leave part of a tree, retrying is safe,
and an owner that must not leak files after a crash reconciles by listing. The
owner orders removal after the directory's writers; entries or leases created
meanwhile make it fail. A replacement racing the removal of its directory still
commits: if removal deleted the empty temporary directory first, the
replacement creates it again. Roots, the reserved directory and the account
marker cannot be listed or removed.

On Windows, every reparse point is listed as `Other` and never entered. Removal
deletes a junction or symbolic link itself, but refuses a volume mount point
(a mount-point reparse point whose target is a `\??\Volume{…}` path) with
`CrossesDevices`, as Unix refuses a directory on another filesystem; Unix lists
such a directory as `Directory`. Listing and removal need a volume that reports
file IDs during enumeration, such as NTFS or ReFS. Elsewhere, such as on FAT,
exFAT or some network shares, they fail with an `Unsupported` I/O error rather
than list entries whose aliases of reserved names could not be recognized.

## Tests

[file_storage_flow.rs](../../../../crates/dim-agent-capabilities/tests/file_storage_flow.rs):
`late_roots_preserve_indices_and_generation_boundaries_on_reload`,
`file_storage_flow` (path, lock, permission and platform checks),
`directory_listing_and_removal_stay_inside_the_authorized_root` (Session-file
layout, reserved resources and aliases, leases held during partial removal,
links, cancellation, leftovers with reserved names) and
`removal_scales_with_directory_size` (a directory that
used 300 names and a 1,000-level tree, removed under a 64-descriptor limit on a
256 KiB worker stack, where recursion overflowed at a few hundred levels). The unit test
`lock_keys_stay_exclusive_across_concurrent_directory_removal` in the owner
module drives the lock, removal and temporary-file interleavings that the public
API cannot produce reliably.

<!-- generated:plugin FileStorage — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 进程 |
| 注入 | — |
| 提供 | `P:FileStorage` |
| 被注入于 | `HostPlatform` |
<!-- /generated -->
