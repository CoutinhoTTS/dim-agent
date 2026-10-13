# App Server

The Rust CLI uses this server over a private Unix socket (NDJSON) or Windows named pipe. The server owns Agent v4 execution, approvals, model/provider selection and native storage. WebSocket transport remains available for compatible clients; account/provider management is limited to local transports.

Commands below run from the repository root:

```sh
cargo run --locked -p dim-app-server -- \
  --product-home /absolute/product-home \
  --home /absolute/product-home/rust \
  --unix /absolute/product-home/rust/daemon.sock --no-ws
```

A configured model file uses the same real workspace tools and approval policy:

```sh
cargo run --locked -p dim-app-server -- \
  --model-config /absolute/models.json \
  --home /absolute/development-home \
  --unix /absolute/development-home/daemon.sock --no-ws
```

`--mock-model` selects the explicit deterministic test profile. It uses mock model/tool hosts. Product, configured and mock daemons identify themselves in `hello.daemonProfile`:

```json
{"kind":"product"}
{"kind":"configured","modelConfigSha256":"<64 lowercase hexadecimal characters>"}
{"kind":"mock"}
```

The configured digest is SHA-256 of the same original bytes parsed and validated for that server launch. It excludes neither whitespace nor environment-variable reference names, and never prints file contents or resolved environment values. A client must refuse another profile or configuration rather than reuse, delete or terminate its daemon.

`--home` and `--database` are mutually exclusive. SQLite format v11 includes normalized managed-provider records. Unsupported formats, including the source project's v10, are rejected and preserved; there is no automatic migration. [Native storage format](../../docs/design/appserver/native-storage-format.md) defines the SQL and JSON contract.

Unix endpoint directories must be private (0700), sockets use 0600, and cleanup removes only the inode acquired by this listener. Named pipe ownership and local identity checks remain in the transport. Ordinary client disconnect detaches; it does not delete a session. See the [public architecture](../../docs/architecture/README.md).

## Optional managed provider

The public server runs without account plugins. API-key and supported native subscription providers do not require Node. An explicitly supplied plugin adds managed providers through rutis 0.10.0:

```sh
dim-app-server --product-home /absolute/product-home \
  --home /absolute/product-home/rust --unix /absolute/product-home/rust/daemon.sock --no-ws \
  --plugin-dir /absolute/plugin-package \
  --node /absolute/node-executable \
  --js-runtime /absolute/runtime/node_modules/@arcships/rutis-runtime
```

All three plugin flags are required together. `--js-runtime` names the installed package directory, not a bootstrap script. Node is started by absolute executable path. The runtime receives no terminal stdin, stdout or stderr; business output is returned through the server protocol. The runtime and plugin are separate rutis loader rows in the existing process tree.

The plugin package contains `package.json`, its ESM entry and `dim-plugin.json`:

```json
{
  "schemaVersion": 1,
  "id": "my-managed-provider",
  "version": "1.0.0",
  "hostApiVersion": 1,
  "rutisVersion": "0.10.0",
  "entry": "dist/index.js",
  "files": {"dist/index.js": "<sha256>"}
}
```

The host checks compatibility and confines the canonical entry path to the plugin directory. Release staging verifies download and file digests before loading or packaging code. An explicitly configured invalid or incompatible plugin fails startup. Without plugin configuration, no Node process starts.

The ESM entry is a `definePlugin` export from `@arcships/rutis` 0.10.0. It declares `managedProvider` in `provides`, with every method `async`, and receives `{home: absoluteProductHome}` as its configuration. Bundle its SDK/business runtime dependencies or supply them inside its own package. Plugin module resolution must not rely on this repository's development installation.

```ts
import { definePlugin } from '@arcships/rutis'
export default definePlugin({
  name: 'ManagedProvider',
  provides: { managedProvider: { providers: 'async', status: 'async' } },
  apply(ctx, config) {
    ctx.provide('managedProvider', accountService)
    ctx.effect(() => accountService.close())
  },
})
```

This fragment shows declaration and ownership; a usable account plugin implements the full service contract below. It owns account persistence, authentication, token refresh and catalog normalization. The Rust provider host does not interpret supplier-specific responses or own refresh tokens.

## Service contract

Every call returns one JSON envelope:

```ts
type Result<T> =
  | { ok: true; value: T }
  | { ok: false; error: { code: string; message: string } }

type Provider = { providerId: string; displayName: string; loginMethods: Array<'browser' | 'device'> }
type Account = {
  credentialEpoch: string
  generation: number
  account?: { issuer: string; subject: string; grantedScopes: string[] }
}
```

`providers()` has no arguments. Every other method receives one DTO, carried as `[dto]` by rutis. `providerId` is always explicit. There is no default managed supplier.

| Service method | DTO | Successful value |
| --- | --- | --- |
| `providers` | None | `Provider[]` |
| `status`, `recover`, `logout` | `{providerId}` | `Account` |
| `beginBrowser` | `{providerId,directory,email?,redirectUri}` | `{attemptId,authorizationUrl,expiresAt}` |
| `beginDevice` | `{providerId,directory,email?}` | `{attemptId,verificationUri,verificationUriComplete?,userCode,expiresAt}` |
| `completeBrowser` | `{providerId,attemptId,callbackUrl}` | `Account` |
| `attemptStatus` | `{providerId,attemptId}` | `{status,account?,error?}` |
| `cancelAttempt` | `{providerId,attemptId}` | `{}` |
| `catalog` | `{providerId}` | `{account,models:CatalogModel[]}` |
| `credential` | `{providerId,modelId}` | `CredentialLease` |
| `recoverRejected`, `finishRejected`, `release` | `{providerId,leaseId}` | Lease, `{}`, `{}` respectively |

Attempt statuses are `pending`, `completed`, `failed`, `cancelled` and `expired`. A completed attempt includes its non-secret account snapshot. Account and login operations are exposed as `account.providers`, `account.status`, `account.recover`, `account.logout`, `account.login.begin`, `account.login.browser.begin`, `account.login.browser.complete`, `account.login.status` and `account.login.cancel`. Existing `account.codex.*` and `account.grok.*` retain their native contracts.

Internal model operations use these normalized types:

```ts
type Protocol = 'openai-compatible' | 'openai-responses' | 'anthropic'
type CatalogModel = {
  entry: ModelEntry
  protocol: Protocol
  unavailableReason: string | null
  zdr: boolean
}
type CredentialLease = {
  leaseId: string
  account: Account
  protocol: Protocol
  baseUrl: string
  headers: Record<string, string>
  bearer: string
}
```

`ModelEntry`, capabilities and metadata are defined in [`model_catalog.rs`](src/model_catalog.rs) and `dim-agent-types`. A plugin supplies final public model descriptions and public diagnostics. An unavailable model keeps a reason and remains unselectable. The host validates duplicate IDs, supported protocols and safe positive model limits once at the cross-language boundary. It does not reproject normalized plugin models through bundled provider rules.

`baseUrl` is the final protocol base URL; the existing model adapter adds its protocol endpoint. Headers and bearer are request-local secrets. `leaseId` is an opaque short-lived handle bound to the plugin runtime, account and credential revision. A plugin rejects expired/old handles and bounds their count and lifetime. `recoverRejected` performs one controlled recovery, and `finishRejected` settles rejection of the recovered credential. Host recovery is limited to HTTP 401; unrelated failures retain the account.

Credential methods are internal to the model resolver and never appear in terminal RPC, history, Timeline, configuration replies or logs. Refresh tokens stay inside the plugin. Rust releases a lease when its last request/recovery owner drops it. A withdrawn binding rejects new calls, closes its generation's task tracker and drains already admitted promises. A caller disappearing does not drop an accepted token rotation. Runtime loss invalidates old handles and is not automatically replayed or restarted; API-key providers keep working.

Plugin error messages are not forwarded. The host maps canonical `invalid_store`, `unsupported_schema`, `identity_rejected`, `invalid_input`, `invalid_callback`, `config_error`, `attempt_not_found`, `attempt_closed`, `token_missing`, `refresh_rejected`, `account_changed`, `account_recovery_required`, `account_reset_blocked` and `service_reloading` into fixed public diagnostics. Malformed responses fail explicitly. Account corruption and unavailable plugins are not represented as a signed-out account.

## Catalog and storage consistency

The provider host stores normalized catalog caches, user model overrides, disabled model IDs and their revision in the existing `providers` table. Managed records have `kind: managed` and an account snapshot; token data is absent. Static records retain their own schema.

A catalog refresh captures the current SQL record version before its plugin request. Its commit uses that version as a compare-and-swap guard, so a late request cannot overwrite a newer catalog. Account generation is checked before and after commit. An account change fails the operation rather than returning successful stale data. Cache lookup and request credential resolution require the same account epoch/generation; a previously stored snapshot can never authorize the next account.

There is no atomic transaction spanning the plugin's account file and SQLite. A generation change during an admitted SQL write can leave an older, unusable cache record; the next explicit refresh replaces it. The host does not hold an account lock across IPC or retry state-changing operations implicitly.

## Verification

Public API-key, provider, native subscription and transport flows do not need Node:

```sh
cargo test --locked -p dim-app-server --test provider_flow --test managed_oauth_flow --test model_catalog_flow --test transport_flow
```

One separately enabled test loads a neutral SDK plugin through the actual rutis Node runtime. It covers normalized catalog selection, HTTP 401 recovery, lease release, secret-free replies/history and a runtime crash followed by an API-key request. It does not require a private account package:

```sh
npm ci --prefix runtime --ignore-scripts
DIM_TEST_NODE="$(command -v node)" \
DIM_TEST_JS_RUNTIME="$PWD/runtime/node_modules/@arcships/rutis-runtime" \
DIM_TEST_RUTIS_SDK="$PWD/runtime/node_modules/@arcships/rutis/src/index.mjs" \
  cargo test --locked -p dim-app-server --test transport_flow \
  node_managed_provider_recovers_credentials_and_failure_is_local -- --ignored
```
