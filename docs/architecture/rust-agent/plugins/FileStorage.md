# FileStorage

Owner: [file_storage.rs](../../../../crates/dim-agent-capabilities/src/file_storage.rs)
and [windows.rs](../../../../crates/dim-agent-capabilities/src/file_storage/windows.rs).
Provides authorized-root-relative file operations and leases with secure path
traversal and serialized mutation.

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

## Tests

[file_storage_flow.rs](../../../../crates/dim-agent-capabilities/tests/file_storage_flow.rs):
`late_roots_preserve_indices_and_generation_boundaries_on_reload` and
`file_storage_flow` (path, lock, permission and platform checks).

<!-- generated:plugin FileStorage — 由 `cargo run -p xtask -- graph` 从真实装配生成，勿手改 -->
| 项 | 值 |
| --- | --- |
| 作用域 | 进程 |
| 注入 | — |
| 提供 | `P:FileStorage` |
| 被注入于 | `HostPlatform` |
<!-- /generated -->
