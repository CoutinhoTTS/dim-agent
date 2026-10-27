# dim-oauth

The Dim account provider (`dimcode-api-oauth`) as a Rust rutis plugin. It provides the host's `ManagedProvider` service: browser and device login, the `auth.json` account record, token refresh, the relay model catalog, usage and request credential leases. It replaces the TypeScript `dim-oauth-plugin` 0.2.0 and the Node runtime the account build embedded for it; signed-in accounts carry over unchanged. See the [runtime contract](docs/runtime-contract.md) and [design](docs/design.md).

## Account build

`dimcode` installs the plugin with `dim_oauth::row(ClientIds { cn, global })` when it is compiled with both `DIM_OAUTH_CLIENT_ID_CN` and `DIM_OAUTH_CLIENT_ID_GLOBAL`; without them it is the API-key build. The client IDs never enter the repository: put them in `.env.dev` for `./dev`, and in the repository secrets of the same names for release CI. See the [managed provider boundary](../../docs/architecture/development.md#managed-provider-boundary).

```bash
./dev login dimcode-api-oauth --directory https://dimagent.cn --device
./dev models dimcode-api-oauth --refresh
./dev --provider dimcode-api-oauth --model "<model-id>"
```

## Verification

`tests/dim_oauth_flow.rs` drives the plugin through the public `ManagedProviders` against a local account service (`tests/fixtures/dim_oauth.rs`): device and browser login with userinfo identity, catalog projection, leases, usage, logout and revocation, 401 recovery and compare-and-delete, a single refresh across two processes, background renewal of an idle process's grant, TypeScript 0.2.0 account files (including their digests), reset backups and unknown schema. The host side of the hook is covered in `managed_provider_flow` and `transport_flow`.
