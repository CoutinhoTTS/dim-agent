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

## Listing and removal

`list` returns the entries of a directory below an authorized root, sorted by
name, each a `File`, `Directory` or `Other` (a link, reparse point or special
file, never followed). FileStorage's lock, temporary and account-marker
resources are omitted, including names the filesystem equates with them.

`remove` makes an entry absent. Files, including those beneath a directory,
are removed under the lock leases use, so removing a leased file, or a directory
with a lease held anywhere beneath it, returns `Locked`. A link is removed
itself. A directory is emptied through held directory handles, without following
links or entering a filesystem mounted at or inside it (a Linux bind mount of
the same filesystem is not detected). Its FileStorage resources go last: every
lock key is locked before any is unlinked and stays locked until the directory's
own removal returns, and a live account marker returns `Locked`, so removal
never unlinks a key another holder has locked. An absent entry counts as
removed. Removal is neither atomic nor synced: an error or
cancellation can leave part of a tree, retrying is safe, and an owner that must
not leak files after a crash reconciles by listing. The owner orders removal
after the directory's writers; entries or leases created meanwhile make it
fail. Roots, the reserved directory and the account marker cannot be listed or
removed.

## Tests

[file_storage_flow.rs](../../../../crates/dim-agent-capabilities/tests/file_storage_flow.rs):
`late_roots_preserve_indices_and_generation_boundaries_on_reload`,
`file_storage_flow` (path, lock, permission and platform checks) and
`directory_listing_and_removal_stay_inside_the_authorized_root` (Session-file
layout, reserved resources and aliases, leases held during partial removal,
links, cancellation).

<!-- generated:plugin FileStorage — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 进程 |
| 注入 | — |
| 提供 | `P:FileStorage` |
| 被注入于 | `HostPlatform` |
<!-- /generated -->
