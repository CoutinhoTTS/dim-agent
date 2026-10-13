# Agent v4 upstream sync verification

## Result

Pass. The public Rust workspace incorporates the complete applicable Rust delta from Agent v4 commit `e79aa2257f77d2ace1041c768160f3514e839733` to fixed commit `3bc0cace829a144c2c9b4baae3fbddf0f5dd3ccb`. The reviewed public implementation is `df7a0e38ccda76b06b171177732a73f5c2ea4c76`, including the public host's generic managed-JS naming adaptations. No blocking finding remains in the integrated implementation or local setup instructions.

The public host retains its generic managed-provider bridge and the private plugin remains a separately supplied source-built runtime package. No Dim OAuth implementation, legacy TypeScript agent, OpenTUI or Desktop code is imported. Plugin release CI and remote downloads remain deferred. All four GitHub publishing/build workflow templates retain their `.yml.disabled` filenames.

## Independent code verification

An assert-based comparison enumerated exactly seven changed upstream Rust files. Four synchronized files match the fixed latest upstream files byte-for-byte: FileStorage implementation, context publication flow, typed capability declarations and settings flow. Assembly, App Server registration and transport reload tests preserve the public host adaptations while implementing the upstream contracts. The public neutral managed-provider test additionally covers identities resolved outside Builtins.

| Contract | Evidence |
| --- | --- |
| Authorized roots survive reload | `FileStoragePlugin` clones share only the append-only authorized path table. Each `apply` opens a new `FileStorage` with a fresh generation UUID, file handles, lease registry, cancellation and drain state. The process factory captures one plugin and returns clones, so dynamically authorized root indices survive reload without retaining old I/O resources. |
| Process factories are registered first | The App Server registers its native and generic bridge modules, Agent modules, connection, RPC and transport before awaiting scoped installation. The cloned builder captured by `AgentsPlugin` preserves the completed assembly. Independent Agent placement registers `AgentAnchor` before scoped installation as well. |
| Scoped names cannot replace host identities | `AgentBuilder::install` queries the shared Builtins registry for each proposed module and increments `#n` until unused. Its reserved set contains internal row/group identities and host identities absent from Builtins, rather than duplicating registered module state. The App Server derives both the managed JS row ID and canonical entry module path from the actual row and passes them through `reserved_names`. Independent Agent placement supplies no extra external identities. |
| Public groups remain stable | Extension group IDs are public instance selectors and cannot be silently renamed. Collisions with internal selectors, registered host modules, managed JS row IDs or entry module paths fail construction before loading. Pinned rutis-loader 0.10.0 `Builtins::resolve` reads an in-memory map; scoped registration does not invoke an external resolver. |
| Async installation preserves failure behavior | Both actual `AgentBuilder::install` callers await its result and supply the correct reservations. `AssertUnwindSafe` wraps the entire async body and `FutureExt::catch_unwind` catches construction and metadata panics while polling, returning a build error. Failure paths retain root shutdown and cleanup-error propagation. |
| Typed dependency API remains coherent | No consumer imports the removed `Named` wrapper. `InSession`, `InBranch`, scoped gates and `Contributions` remain the typed dependency contract. Factories construct disposable metadata samples, preserve names/keys and acquire I/O only during `apply`; captured configuration owns state that must survive reload. |
| Progress cleanup follows its owner | The context flow waits on the ModelProvider-owned progress watch after Loop becomes idle. It retains the preceding publication assertions and surrounding timeout instead of assuming idle has already cleared another capability's state. |

The public architecture entry expresses the applicable typed dependency, shared registry, external reservations, factory, reload and startup contracts without importing source-project documents that reference unavailable internal packages. The private README states the fixed source provenance; no private OAuth implementation changed in the upstream delta.

## Runtime and build evidence

The independent reviewer inspected the executor's final integrated logs and reused applicable unchanged-file evidence without starting duplicate Cargo work.

| Check | Result and evidence |
| --- | --- |
| Integrated reload flows | `cargo test --locked -p dim-app-server --test transport_flow reload::`: 9 pass. Includes process-name collisions at Session and branch scope, two live sessions retained across RPC restart, conflicting extension-group rejection and startup failure while the real FileStorage worker is blocked. `/tmp/dim-agent-v4-sync-integrated-reload-20261013.log` |
| Actual generic Node bridge | The opt-in `node_managed_provider_recovers_credentials_and_failure_is_local` flow passes against actual Node/rutis and Rust implementations. It loads Session capabilities named after both the JS row ID and canonical module path, verifies both execute, exercises model HTTP credential recovery and bridge failure isolation, and rejects extension groups using either external identity. The fixture contains no private Dim OAuth implementation. `/tmp/dim-agent-v4-sync-integrated-node-20261013.log` |
| External capability contract | `cargo test --locked -p dim-agent-integration-tests --test extensions`: 10 pass on the integrated host, including first/subsequent factory panics and rejection before session publication. `/tmp/dim-agent-v4-sync-integrated-extensions-20261013.log` |
| Settings and authorized projects | The 5 settings flows pass, including authorization, old root identity and correct project settings across a FileStorage reload. FileStorage, settings flow and the shared-clone process registration retain the tested implementation. `/tmp/dim-agent-v4-sync-server-20261013.log` |
| FileStorage generation contract | `cargo test --locked -p dim-agent-capabilities --test file_storage_flow`: 2 pass, including root-index persistence and generation boundaries on reload. This implementation is unchanged in the reviewed integration. `/tmp/dim-agent-v4-sync-files-20261013.log` |
| Model progress publication | `cargo test --locked -p dim-agent-capabilities --test context_flow progress_keeps_committed_output_until_it_is_published`: 1 pass; the tested file matches the integrated implementation. `/tmp/dim-agent-v4-sync-progress-20261013.log` |
| Workspace checks and binaries | Formatting check, locked all-target workspace Clippy with `-D warnings`, and actual CLI/App Server builds pass. `/tmp/dim-agent-v4-sync-integrated-clippy-20261013.log`, `/tmp/dim-agent-v4-sync-integrated-build-20261013.log` |
| Architecture | `xtask check` passes at 147 modules and 990 edges. Generated module references include the existing CLI model/stop commands; no extra runtime module or dependency boundary is introduced. `/tmp/dim-agent-v4-sync-integrated-boundaries-20261013.log` |
| Actual source-package development flow | Private `test/development.test.mjs`, pointed at the integrated public checkout and rebuilt host, passes against the actual private/public launchers, private source package, Node bridge and Rust binaries. An external local HTTP fixture verifies device login, catalog refresh, actual model HTTP, normal restart, same-session resume, retained identity and normal stop. 1 pass, 0 failures in about 16 seconds. `/tmp/dim-agent-v4-sync-integrated-dev-20261013.log` |

The public and private README instructions match the development entrypoints: pinned prerequisites, sibling/custom source location, preparation without Rust compilation or daemon startup, login, model refresh, explicit/default selection, restart, resume and stop. API-key mode bypasses private source and JS preparation. The private wrapper delegates to the public entry and preserves the caller's working directory and arguments. No tests run on normal startup.

## Simplification and limits

The implementation uses the existing registration, resolver and lifecycle mechanisms. It removes the unused dependency wrapper and redundant module-name bookkeeping rather than adding a second registry or reload state container. Explicit reservations cover actual host identities that Builtins cannot know. FileStorage retains only authorization configuration across generations; normal resource acquisition and cleanup remain owned by each generation. No further removable structure was identified within the affected contract.

Production Dim credentials/cloud calls, other-platform execution and release/npm artifact rebuilding were not repeated. The actual Node and source-package flows use external HTTP fixtures on macOS arm64. No GitHub synchronization, publishing workflow activation, release tag, CI dispatch or npm publication was performed.
