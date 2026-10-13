# Agent v4 upstream sync verification

## Result

Pass. The public Rust workspace incorporates the complete Rust delta from Agent v4 commit `e79aa2257f77d2ace1041c768160f3514e839733` to fixed commit `3bc0cace829a144c2c9b4baae3fbddf0f5dd3ccb`. The reviewed public baseline is `5b1f969b07864b5650f3f8288cc323e857cd8264`. No blocking finding remains in the synchronization or the accompanying local setup instructions.

The public host retains its generic managed-provider bridge and the private plugin remains a separately supplied source-built runtime package. No Dim OAuth implementation, legacy TypeScript agent, OpenTUI or Desktop code is imported. Plugin release CI and remote downloads remain deferred. All four GitHub publishing/build workflow templates retain their `.yml.disabled` filenames.

## Independent code verification

An assert-based comparison enumerated exactly seven changed upstream Rust files. For the six direct copies, the public baseline equals the previous upstream baseline, and the synchronized files equal the fixed latest upstream files byte-for-byte. For `crates/dim-app-server/src/lib.rs`, the complete sequence of added/deleted lines equals the upstream delta; the existing public managed JS integration is retained.

| Contract | Evidence |
| --- | --- |
| Authorized roots survive reload | `FileStoragePlugin` clones share only the append-only authorized path table. Each `apply` opens a new `FileStorage` with a fresh generation UUID, file handles, lease registry, cancellation and drain state. The process factory captures one plugin and returns clones, so dynamically authorized root indices survive reload without retaining old I/O resources. |
| Process factories are registered first | The App Server registers its native and generic bridge modules, Agent modules, connection, RPC and transport before awaiting scoped installation. The cloned builder captured by `AgentsPlugin` preserves the same completed assembly. |
| Scoped names cannot replace process modules | `AgentBuilder::install` queries the shared Builtins registry for each proposed module and increments `#n` until unused. Reserved internal row identities and extension groups remain reserved. Extension groups are public selectors and fail construction when colliding with registered host modules. Pinned rutis-loader 0.10.0 `Builtins::resolve` reads an in-memory map; registration does not call an external resolver. |
| Async installation preserves failure behavior | Both actual `AgentBuilder::install` callers await its result. `AssertUnwindSafe` wraps the entire async body and `FutureExt::catch_unwind` catches construction and metadata panics while polling, returning a build error. Existing failure paths retain root shutdown and cleanup-error propagation. |
| Typed dependency API remains coherent | No consumer imports the removed `Named` wrapper. The existing `InSession`, `InBranch`, scoped gates and `Contributions` remain the typed dependency contract. Factories construct disposable metadata samples and acquire I/O only during `apply`; the new factory contract documents stable names/keys and shared reload configuration. |
| Progress cleanup follows its owner | The context flow waits on the ModelProvider-owned progress watch after Loop becomes idle. It retains the preceding publication assertions and the surrounding timeout instead of assuming idle has already cleared another capability's state. |

The public architecture entry expresses the applicable upstream typed dependency, shared registry, factory, reload and startup contracts without importing source-project documents that reference unavailable internal packages. The private README updates the fixed source provenance; no private OAuth implementation changed in the upstream delta.

## Runtime and build evidence

The root executor ran the affected checks once; this independent review inspected their logs and reused that evidence without starting another Cargo build.

| Check | Result and evidence |
| --- | --- |
| App Server flows | `cargo test --locked -p dim-app-server --test transport_flow --test settings_flow`: 88 transport and 5 settings tests pass; one unrelated transport test remains ignored. Includes process-name collisions at Session and branch scope, two live sessions, RPC restart, conflicting extension-group rejection, authorized-project settings across FileStorage reload, and startup failure while the real file worker is blocked. `/tmp/dim-agent-v4-sync-server-20261013.log` |
| External capability contract | `cargo test --locked -p dim-agent-integration-tests --test extensions`: 10 pass, including first/subsequent factory panics and failure before session publication. `/tmp/dim-agent-v4-sync-extensions-20261013.log` |
| FileStorage generation contract | `cargo test --locked -p dim-agent-capabilities --test file_storage_flow`: 2 pass, including root-index persistence and generation boundaries on reload. `/tmp/dim-agent-v4-sync-files-20261013.log` |
| Model progress publication | `cargo test --locked -p dim-agent-capabilities --test context_flow progress_keeps_committed_output_until_it_is_published`: 1 pass. `/tmp/dim-agent-v4-sync-progress-20261013.log` |
| Workspace checks and binaries | Formatting check, locked all-target workspace Clippy with `-D warnings`, and actual CLI/App Server builds pass. `/tmp/dim-agent-v4-sync-clippy-20261013.log`, `/tmp/dim-agent-v4-sync-build-20261013.log` |
| Architecture | `xtask check` and graph regeneration pass at 147 modules and 990 edges. Generated module-reference changes only record existing CLI model/stop commands and related references; no extra runtime module or dependency boundary is introduced. `/tmp/dim-agent-v4-sync-boundaries-20261013.log`, `/tmp/dim-agent-v4-sync-graphs-20261013.log` |
| Actual source-package development flow | Private `test/development.test.mjs`, pointed at the synchronized public checkout, passes against the actual private/public launchers, private source package, Node bridge and Rust binaries. An external local HTTP fixture verifies device login, catalog refresh, actual model HTTP, normal restart, same-session resume, retained identity and normal stop. 1 pass, 0 failures in about 15 seconds. `/tmp/dim-agent-v4-sync-dev-20261013.log` |

The public and private README instructions match the unchanged development entrypoints: pinned prerequisites, sibling/custom source location, preparation without Rust compilation or daemon startup, login, model refresh, explicit/default selection, restart, resume and stop. API-key mode bypasses private source and JS preparation. The private wrapper delegates to the public entry and preserves the caller's working directory and arguments. No tests run on normal startup.

## Simplification and limits

The synchronization uses the existing registration, resolver and lifecycle mechanisms. It removes the unused dependency wrapper and redundant module-name bookkeeping rather than adding a second registry or reload state container. FileStorage retains only authorization configuration across generations; normal resource acquisition and cleanup remain owned by each generation. No further removable structure was identified within the affected contract.

Production Dim credentials/cloud calls, other-platform execution and release/npm artifact rebuilding were not repeated. The real developer flow uses an external HTTP fixture on macOS arm64. No GitHub synchronization, publishing workflow activation, release tag, CI dispatch or npm publication was performed.
