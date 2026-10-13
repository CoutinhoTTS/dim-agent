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

The public host exposes provider/account operations and leases request-local credentials. It does not implement a particular account realm, PKCE flow, token exchange, refresh policy, private catalog projection or provider-specific error classification. Those behaviors belong to the separately supplied plugin. Refresh tokens never cross the public bridge. Terminal/history projection excludes credentials, authorization codes and PKCE state.

Timeline and Dispatcher deliver both business events and interrupts. Capabilities cancel their own work and emit terminal outcomes after cleanup. A stopped turn must settle before the CLI accepts another prompt. Client detach does not delete or close a persisted session. Storage rejects incompatible data explicitly; see the [storage format](../design/appserver/native-storage-format.md).

Model adapters validate incoming HTTP/SSE data at their boundary. Event serialization is covered by frozen compatibility fixtures; the original TypeScript source generator is not needed at build time. `ProviderPresentation` transports generic nonempty source/reason/action strings and public diagnostics; private classification policies are not part of the Rust contract.

## Generated architecture

```sh
cargo run --locked -p xtask -- graph
cargo run --locked -p xtask -- check
```

Graphs are generated from actual runtime bindings and Cargo/syn analysis into `docs/architecture/rust-agent/generated/`. Do not edit generated graphs by hand. `tests/extension-a`, `tests/extension-b` and `tests/integration` verify that independent external capabilities can participate without depending on internal implementation details.

## Daemon profile identity

`hello` declares a product, configured or mock daemon profile. Configured identity includes the SHA-256 of the exact validated model configuration bytes, without credential values or raw configuration. The CLI requires the expected profile and matching configuration digest before any session/account operation. A missing or incompatible identity is rejected without deleting its endpoint or stopping that process. Using separate homes for different configurations remains recommended.
