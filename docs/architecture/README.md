# Architecture

```text
Terminal / npm launcher
          |
     Rust dimcode
          | authenticated local NDJSON RPC
  Rust dim-app-server
     |            |
  Agent v4     Provider host
     |            |-- API-key/native model connections
  Timeline        |-- optional managedProvider service
  Dispatcher              |
  Capabilities       rutis Node bridge
  Storage                 |
                  separately supplied account plugin
```

`dim-agent-types` owns serialized data. `dim-agent-runtime` owns instance construction, timeline ordering and close coordination. Capabilities own their work, cancellation and state. Model and tool adapters provide typed ports; transport and product configuration belong to the App Server. The CLI is a client, not another agent execution engine.

Services use rutis 0.10.0 TypedPlugin/Capability contracts. Dependencies are declared once; each instance scope uses the appropriate keys. Managed JS services are optional gated bindings on the same rutis tree. The process owns the Node runtime and drains its work at close. If an account plugin is unavailable, requests for its managed providers fail explicitly while API-key providers remain available.

Capability factories are pure and preserve plugin names and dependency keys. Configuration that must survive reload belongs to the captured plugin configuration; each `apply` acquires fresh I/O resources and registers their cleanup. FileStorage clones share the append-only authorized-root identities, while directory handles, operations and leases belong to one service generation. Rebuilding from the initial root list would let a later project reuse an earlier project's index.

Workspace tool hosts register `FileWriteCoordinatorPlugin` at process scope and clone it across reloads. `write` and `edit` share per-path admission across Sessions, retain their lock through blocking I/O and binding publication, and stage complete bytes before atomic replacement. Existing revision, identity and metadata are checked immediately before commit; new targets use create-if-absent publication. This is not compare-and-swap against unrelated processes or parent-directory replacement. Native metadata is preserved (macOS ACL/xattrs/resource forks, Linux xattrs/ACL, Windows DACL); Windows alternate streams fail unchanged until supported. An error after installation reports that the target may already have changed. Re-read before retrying.

Hosts register process modules before awaiting `AgentBuilder::install`. Scoped module names are allocated against rutis `Builtins` with `#n` suffixes on collision; the assembly only keeps reserved identities, not a second module registry. Hosts also pass identities absent from `Builtins` to `install`: external module names and row IDs (both the managed JS entry path and its row ID, for example). Extension group IDs are public instance selectors and cannot be silently renamed: collisions are rejected before loading.

One startup deadline covers loader readiness, reconcile and process activation. Scoped dependencies use `InSession`, `InBranch`, gates and `Contributions`; the unused `Named` wrapper is not part of the API.

Session and child-branch opening also have a single startup budget. Opening handlers register `BranchOpening` receipts and return without awaiting host I/O on the Dispatcher. Each capability owns and joins its work; the tree waits for the receipts and their committed dispatch fence before publishing readiness or the first child input. Closing signals initial prompt loading before dispatch drains. Deadlines request cancellation, not abandonment: a non-cooperative host still has to settle before cleanup completes.

The public host exposes provider/account operations and leases request-local credentials. It does not implement a particular account realm, PKCE flow, token exchange, refresh policy, private catalog projection or provider-specific error classification. Those behaviors belong to the separately supplied plugin. Refresh tokens never cross the public bridge. Terminal/history projection excludes account credentials, authorization codes and PKCE state. A credential or recovery result acquires its release guard inside the accepted, generation-tracked operation, before validation or handoff to a caller; a dropped waiter cannot orphan it. Release uses that generation's dispatch, not whichever plugin is currently installed.

Transport admission is bounded per connection: 16 MiB per decoded frame (excluding NDJSON delimiters), 64 ordinary in-flight requests plus 8 reserved control slots, and 1024 outbound frames with a 32 MiB encoded-byte budget held through writing. Saturation closes a slow output consumer rather than dropping durable events on a healthy connection. Clients reconnect and use persisted history to resume. Product model selection uses explicit request, Agent profile, then product defaults; collaboration members use the same declared model-selection host to resolve their capability snapshot.

Timeline and Dispatcher deliver both business events and interrupts. Capabilities cancel their own work and emit terminal outcomes after cleanup. A stopped turn must settle before the CLI accepts another prompt. Client detach does not delete or close a persisted session. Storage rejects incompatible data explicitly; see the [storage format](../design/appserver/native-storage-format.md).

The CLI disables terminal echo before inviting callback input and restores it on every exit. Built-in shell approvals intentionally show the command and cwd for informed consent; commands may themselves contain sensitive values. Arbitrary extension arguments are not printed. The npm launcher requires Node 22.19+ on the 22.x line or 23.11+ so POSIX execution can replace the wrapper without duplicate terminal signals.

Model adapters validate incoming HTTP/SSE data at their boundary. Event serialization is covered by frozen compatibility fixtures; the original TypeScript source generator is not needed at build time. `ProviderPresentation` transports generic nonempty source/reason/action strings and public diagnostics; private classification policies are not part of the Rust contract.

## Generated architecture

```sh
cargo run --locked -p xtask -- graph
cargo run --locked -p xtask -- check
```

Graphs are generated from actual runtime bindings and Cargo/syn analysis into `docs/architecture/rust-agent/generated/`. Do not edit generated graphs by hand. `tests/extension-a`, `tests/extension-b` and `tests/integration` verify that independent external capabilities can participate without depending on internal implementation details.

## Daemon profile identity

`hello` declares a product, configured or mock daemon profile. Configured identity includes the SHA-256 of the exact validated model configuration bytes, without credential values or raw configuration. The CLI requires the expected profile and matching configuration digest before any session/account operation. A missing or incompatible identity is rejected without deleting its endpoint or stopping that process. Using separate homes for different configurations remains recommended.
