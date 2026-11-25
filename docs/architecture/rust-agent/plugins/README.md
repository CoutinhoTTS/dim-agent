# Local-execution business plugins

These pages document the business plugins mounted by the [local-execution profile](../../../../crates/dim-agent-capabilities/src/defaults.rs), with Attachments and Questions enabled by the [verification host](../../../../xtask/src/plugins.rs). Each page describes the plugin's use of its dependencies and its obligations under the [plugin contract](../../../design/plugin-contract.md).

## Reading and maintaining these pages

Dependency tables describe the exact injected services in this profile:
`B:` means the calling branch, `S:` its Session, and `P:`
the process. Named contributions retain their `#name`. Conditional dependencies
are listed only when enabled in this assembly. A dependency used only as a
lifecycle gate is still a dependency. Per-call branch service lookups are
explained separately from injected services.

The heading immediately above each `Dependencies` or `依赖` section names the part. Run `cargo run --locked -p xtask -- check` to check coverage and dependency tables against the live local-execution assembly. Update the affected page when a documented part or its dependencies change.
