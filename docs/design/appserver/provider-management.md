# Provider management

Status: **Proposal; not Ready and not implemented**. Extends the existing process
`ProviderPlugin` for [#329](https://github.com/arcships/dim/issues/329).
[中文](../../zh/design/appserver/provider-management.md).
The implemented [S4 auxiliaryModel](auxiliary-model.md) remains unchanged.
New methods and fields below require implementation and client acceptance before use.

## 1. Scope and current implementation

Desktop and CLI need one App Server contract for the
[Providers and models](../../feature-list.md#providers-and-models) flows. At
`main` @ `78dd8c5e`:

| Flow | Implemented | Proposed completion |
| --- | --- | --- |
| List/edit | `provider.list`, `account.providers`, `provider.catalog` | One safe management snapshot, including connection fields and editable model descriptors |
| Connection | `provider.save` writes complete records | Create and partial update replace save; preserve secrets the client cannot read |
| Disconnect/delete | Custom delete removes a record; builtin delete disables it | Disconnect atomically clears static credentials and disables; account logout stays with its owner |
| Discovery | `provider.refresh` needs a saved record | Probe an unsaved draft without creating or modifying a Provider |
| Models | Refresh, enable, edit and reset default models | Separate user-added models, persistent overrides and explicit New API routes |
| Preferences | F3 accepts `ModelPreferences` on Session adoption | Persist per-model preferences for clients to pass explicitly |
| Accounts/usage | Native login and optional `account.usage` already exist | Reuse those contracts and their actual errors; no static-provider usage settings |
| Multiple clients | Replies and record CAS | Reread on view open and after own writes; no pushed notices |
| Legacy import | Explicit read-only import with per-entry results | Specify field mapping, preserve source bytes and reject unsupported nonempty fields |

Source contracts: [Provider owner](../../../crates/dim-app-server/src/provider.rs),
[management methods](../../../crates/dim-app-server/src/provider/management.rs),
[model descriptors](../../../crates/dim-app-server/src/model_catalog.rs),
[legacy import](../../../crates/dim-app-server/src/provider/legacy.rs),
[account methods and Usage](../../../crates/dim-app-server/README.md), and
[F3](../model-configuration.md).
Media execution belongs to [Modality](../plugins/Modality.md); consumer-specific
choices, such as SessionTitle and sub-agent models, remain with their owners.

## 2. Ownership and affected surfaces

Provider owns its process configuration, model catalog, overrides, preferences,
validation, safe projections, writes and network tasks. Account owners retain login,
tokens and credential leases. SqliteStorage owns transactions and format admission.
Clients own unsaved drafts and presentation. Adopted Session configuration remains
with its existing owner; management never writes another owner's state.

| Surface | Consumers and change |
| --- | --- |
| `managementMethods`, process, key `provider` | Local v3 Desktop/CLI and trusted process callers; §3 extends this existing named binding |
| Existing `account`, `account.codex`, `account.grok` namespaces | Existing account operations; no new account method, event or managed host capability |
| `ProviderConnectionResolver`, `ModelSelectionHost` | Provider implements catalog/route decisions through existing types; no new core port |
| `auxiliaryModel.complete` | Existing caller-selected S4 contract, unchanged |
| Native CLI Provider flows | Migrate discovery, create, update and disconnect together with the new owner methods |

Business logic stays in Provider, not `connection.rs`, `protocol.rs` or a wiring
plugin. No Session Capability, notification stream, second catalog, credential
service or registry is added. The [managed-provider boundary](../../architecture/development.md#managed-provider-boundary)
continues to exclude private account implementations and credentials from this repository.

## 3. Management contract

### 3.1 Read models

| Method | Request | Reply | mutating | interruption |
| --- | --- | --- | --- | --- |
| `provider.management` (new) | `{providerId?}` | `{drivers: ManagementDriver[], providers: ManagementProvider[], errors: ProviderRecordError[]}` | false | `Drain` |
| `provider.selection` (existing) | `{cwd?}` | Existing `{selection}` and its separate scope/version | false | `Drain` |
| `provider.catalog` (existing) | `{providerId}` | Existing `{catalog}` with `ProviderModel`; **unchanged** | false | `Drain` |

Without an ID, management returns all known entries, including signed-out accounts.
With an ID, it returns that entry or a typed error. It is the sole edit-view read:
its record version, model descriptors and annotations come from the same Provider record
snapshot. Clients do not combine `provider.catalog` from another version with its
`expectedVersion`. Selection and read-only web clients keep using the unchanged
catalog. Aggregation does not promise an atomic snapshot across account owners.

| `ManagementProvider` field | Type and meaning |
| --- | --- |
| `providerId`, `displayName` | Strings; stable identity and user-visible name |
| `source` | `builtin` / `custom` / `managed`; record origin, not login state |
| `credentialAuthority` | `static` / `openai_codex` / `grok` / `account`; the credential owner |
| `version` | Record version or `null` only when an account entry has no local catalog record; never a fabricated zero |
| `enabled` | Boolean for a static record; `null` when the account-owned record has no Provider enable flag |
| `account` | Optional `{namespace, loginMethods, state}`; namespace is `account`, `account.codex` or `account.grok`, login methods are `browser`/`device`, and state is that namespace's existing public status reply; absent for static authority |
| `actions` | Available operation names: `update`, `delete`, `refresh`, `models.create`, `models.edit`, `models.reset`, `models.delete`, `models.set_enabled`, `preferences.update`, `login`, `logout`, `usage`; no per-action reason codes |
| `connection` | Safe static-record connection below; `null` when connection configuration exists only in the optional account package |
| `models` | `{model: ModelEntry, source: "default" \| "override" \| "user", enabled: boolean, endpoint: ModelProtocol \| null, unavailableReason: string \| null, actions: string[], endpointOverride?: NewApiChatEndpoint}[]`; `null` only for an account entry with no local catalog record. Bad catalog data is an error. Model actions are `set_enabled`, `edit`, `reset`, `delete`, restricted by origin/authority |
| `preferences` | Map from model ID to the existing `ModelPreferences` shape |

`ManagementDriver` fields are `driverKind: string`, `authTypes: StaticAuth["type"][]`
and `remoteModelList: boolean`, derived from executable drivers. `StaticAuth` is the
existing tagged type, not another auth schema. Only drivers allowed for custom
creation appear. There is no dynamic form-schema or second driver registry.

| Safe `connection` field | Type and meaning |
| --- | --- |
| `driverKind`, `auth`, `baseUrl` | Driver string, existing `StaticAuth`, validated URL |
| `credentialConfigured` | Boolean; neither the credential nor secret reference is returned |
| `headerNames` | String array; header values are never returned |
| `proxyUrl` | String without user info, or `null`; display only, never automatically written back |
| `proxyAuthConfigured` | Boolean; proxy authentication remains secret |
| `timeoutMs` | Positive integer or `null` for the driver default |

List errors use existing `ProviderRecordError`; healthy entries remain available.
Actions describe current availability, not authorization or a promise that a later
write succeeds. Model origin is derived from the distinct persisted lists in §6.
A disappeared default's override cannot become a user-added model. Templates copy
a complete public `ModelEntry`, identified by `{providerId, modelId}`; no template
reference or bundled catalog is copied into client storage.

### 3.2 Writes and field operations

`ManagementProvider` below means the same edit snapshot as §3.1. Existing method
request/reply shapes remain except the explicit `ModelEdit.endpoint` addition and
typed errors in §3.4. Existing writes retain `mutating:true`, `interruption:Drain`.

| Method | Request | Reply / behavior | mutating | interruption |
| --- | --- | --- | --- | --- |
| `provider.create` (new) | `{providerId, displayName, connection, models: ModelEntry[], enabled}` | `ManagementProvider`; create-only, caller-generated fresh UUID, existing ID conflicts | true | `Drain` |
| `provider.update` (new) | `{providerId, expectedVersion, patch}` | `ManagementProvider`; validate and commit one partial update transaction | true | `Drain` |
| `provider.probe` (new) | `{providerId?, expectedVersion?, connection}` | `{models: ModelEntry[]}`; §3.3, no record writes | false | `Drain` |
| `provider.models.create` (new) | `{providerId, expectedVersion, model: ModelEntry}` | Existing catalog; separate user-added list; duplicate default, override or user ID rejected | true | `Drain` |
| `provider.models.delete` (new) | `{providerId, expectedVersion, modelId}` | Existing catalog; only user-added items; clean owned preferences/routes/selections in the same transaction | true | `Drain` |
| `provider.preferences.update` (new) | `{providerId, expectedVersion, modelId, patch}` | `ManagementProvider`; effort/tier independently set/clear, validated against the model | true | `Drain` |

The client generates and retains the UUID before sending; the server validates it and never derives
identity from the name. After an uncertain reply, reread that exact ID. A repeat
create with the same ID returns `provider_conflict`, never overwrites or duplicates
the record. A new creation after deletion uses a fresh UUID. No identity tombstone,
cross-deletion version clock or new idempotency service is introduced.

Creation `connection` contains required `driverKind`, `auth`, `baseUrl`, optional
`network: ModelNetwork`, `headers: Record<string,string>` and `credential` using
existing keep/clear/set/set_aws/set_ref operations. Absent headers/network mean no
overrides for a new record; absent credential means none. Enabled keyed records
require a valid credential. Model discovery and templates already return `ModelEntry`;
clients do not convert the execution projection `ProviderModel` back into descriptors.

| `patch` field | Operation | Writable records |
| --- | --- | --- |
| `displayName` | Nonempty string | Custom only |
| `baseUrl` | Validated URL string | Static-authority builtin/custom |
| `enabled` | Boolean | Local builtin/custom records |
| `credential` | Existing keep/clear/set/set_aws/set_ref; omitted keeps | Static authority only |
| `headers` | `{name, action:"set", value}` / `{name, action:"remove"}` array; omitted names keep | Static-authority builtin/custom |
| `network.proxy` | `{action:"set", value:URL}` / `{action:"clear"}` | Local builtin/custom; existing authority validation applies |
| `network.timeoutMs` | `{action:"set", value:positive integer}` / `{action:"clear"}` | Local builtin/custom |

Omitted fields keep their value; null, unsupported fields, duplicate/conflicting
header names and invalid operations fail at the input boundary. Header names are
case-insensitive there. Proxy and timeout update independently; replacing a proxy
URL explicitly replaces its authentication. Builtin identity/name/driver/auth and
default descriptors remain readonly. Custom `driverKind` and `auth` are fixed at
creation too; changing them requires a new Provider and explicit selection.
Account-owned credentials and connection fields use their owner, not this patch.
Preference patch fields `reasoningEffort` and `serviceTier` each use set/clear;
clear means the catalog default at subsequent explicit adoption.

Existing `delete` keeps its builtin-disable/custom-delete behavior. `models.edit`
keeps `ModelEdit` and adds `endpoint: {action:"set",value:NewApiChatEndpoint} |
{action:"clear"}` for static New API models only. `NewApiChatEndpoint` is the
existing executable subset `openai`, `openai-response`, `anthropic`, `gemini`.
Override wins over discovery; clear returns to discovery without deleting metadata.
Unknown upstream endpoint strings remain descriptive, not executable routes.
`models.reset` restores an existing default; user entries use delete. `refresh`
updates default models while preserving user models, overrides, preferences and
explicit disables, with its existing admitted-configuration conflict check.
Managed account catalogs do not gain user-added routes/models through these methods.

Deleting a Provider clears only Provider-owned selections in its transaction.
Deleting a user model likewise clears its owned selection/preferences/route.
Agents profiles, SessionTitle choices and adopted Sessions retain their exact
IDs: their later use reports not found/unavailable. Provider neither rewrites other
owners' records nor silently substitutes a model. A new Provider uses a fresh ID
and does not rebind those references.

### 3.3 Probe, preference adoption and usage

Probe omits both ID/version for a new complete connection, or supplies both for an
existing record with `connection` as the §3.2 patch. Provider merges that snapshot
in memory and obtains credentials through the original authority. Apply the same
URL/header/network validation as create/update **before** any request. A changed
base URL sends the stored credential to that new host even without saving; the
UI states this when testing the edit. Local management already holds that authority.
No temporary record, catalog, selection or credential is persisted. Unsupported
listing returns `provider_unsupported` and still permits manual models. Results and
errors disclose no secrets or raw response body. A stale draft probe continues to
its request timeout; the client discards its reply. Disconnect/navigation cannot
cancel it. Owner unload uses the existing cancellation token and drains the call.

Provider persists the existing `ModelPreferences` for clients only, superseding
[F3's](../model-configuration.md) no-persisted-preference scope. The client sends
the exact `{providerId, modelId}` with preferences read for that pair to
`session.create` or `session.model.update`. An unavailable explicit target fails;
report it and do not retry without the target or fall back to another model.
The server never applies saved preferences implicitly. Omitted preferences with
an explicit target still start at catalog defaults; untargeted F3 updates still
change only supplied fields on the current adopted model. `/fast` remains
Session-only; saved `serviceTier` is a persisted fast/tier choice applied only by
an explicit client request. Consumer-specific models do not inherit chat preferences.

Usage reuses implemented `account.usage {providerId} -> Usage` in the `account`
namespace, forwarding one call to the optional package's existing `usage` method.
The [public Usage DTO](../../../crates/dim-app-server/README.md) and account errors
remain unchanged. Provider adds no usage settings, probe, CAS, cache or states for
static providers, and no account generation recheck or managed host-API extension.
Missing metrics remain absent; reported unlimited buckets retain the DTO's explicit
`unlimited` convention.

`hello.features` advertises `account.usage` whenever the namespace is active; it
does **not** prove the loaded package implements usage. For an entry returned by
`account.providers`, the client may offer a query when that method is advertised.
An absent package/method can return `account_provider_unavailable`; display that
safe error without relabeling it "unsupported" or affecting model/login behavior.
`unknown_method` describes an older daemon, not package capability discovery.
Any more precise discovery/error contract remains with the account owner.

### 3.4 Errors, admission and compatibility

Provider business errors use JSON-RPC code `-32000`, a safe message and
`data.appError`: `provider_not_found`, `provider_conflict`,
`provider_invalid_config`, `provider_unsupported`, `provider_auth_required`,
`provider_auth_failed`, `provider_transport_failed`, `provider_cancelled`,
`provider_failed`. Classify typed owner errors, not their message text. This also
maps not-found, CAS/admitted-refresh conflict and validation errors from retained
`delete`, `models.edit`, `models.reset`, `models.set_enabled`, `select` and
`refresh`; their requests/replies and JSON-RPC code do not change. Unknown methods
and malformed RPC parameters retain their existing generic boundary errors.
Never include tokens, secret references, headers, proxy auth or raw upstream bodies.

New methods, endpoint editing and typed owner errors ship as the discoverable
`provider.management` interface unit. `provider.save` is replaced by create/update
and removed with its CLI callers; there is no fallback save/direct-database path.
Rpc retains `service_reloading`/`service_unavailable` and ordinary admission.
All accepted calls remain owner-tracked and drain on unload. Disconnect only loses
the reply; it never rolls back committed writes. Reread uncertain writes rather
than replaying them. Provider updates do not reload the loader row.

## 4. Client integration

```text
Desktop / CLI              Local App Server             Provider / account
    | hello: actual methods       |                            |
    | management / selection ---->|---- safe snapshot --------->|
    | edit unsaved draft          |                            |
    | probe --------------------->|---- validated I/O --------->|
    | create(UUID) / CAS patch --->|---- atomic owner write ---->|
    | reread after own write ---->|                            |
    | explicit target + saved preferences -> existing Session adoption
    | reopen / reconnect: reread  |                            |
```

Use the local socket/pipe for the same product home; paired devices gain no
management authority. A CAS conflict preserves the draft and requires reread and
user confirmation, not automatic overwrite with a fresh version. One bad record
does not empty the list. Replies from an obsolete view/draft do not replace current
edits. Other clients' changes appear at the next read; no watch/replay is assumed.
`provider.select` changes a scoped default, not an existing Session; Session
adoption is a separate explicit action with its own outcome and exact target.

Native CLI migration includes `PROVIDER_METHODS` in
`crates/dim-cli/src/session_host.rs`, all save callers, and actual-daemon flows:
Add probes the unsaved connection **before** create, retains its generated UUID
for recovery, and never saves then refreshes as a test. Connection edits probe a
draft when requested, then update changed fields. Disconnect sends credential
clear **and** `enabled:false` atomically, fixing the enabled-key-required rejection
of clear alone. Enable/disable use update; account login/logout retain their owners.

```text
Providers                         Provider detail
[ Search... ] [+ Add]             Name / source / connection status
  Builtin A       Connected       [Edit] [Disable] [Disconnect]
  Custom B        Disabled        Credential: Stored [Keep | Replace | Clear]
  Account C       Signed out      Models [Search...] [Add] [Refresh]
  Broken D        Error           Model / Enabled / [Edit] [Reset/Delete]
                                  Account usage: result or actual query error
                                  Draft [Test connection] [Cancel] [Save]
                                  Conflict: reread / retain draft
```

Reuse existing layouts/components. CLI provides equivalent actions without
copying Desktop layout or inferring capabilities from hardcoded Provider names.

## 5. Requirements raised to mechanism owners

None. Existing named management calls, CAS, Provider storage, metadata and owner
lifetimes suffice. No core, Rpc, rutis, assembly or account host-API expansion is
requested. S4 remains implemented. Client notifications, if wanted, require a
separate decision covering Settings too, not a prerequisite here.

## 6. Persistence, existing Sessions and migration

**Format decision: v18, unchanged SQL schema.** Add only these optional top-level
fields; missing maps/lists mean no corresponding user data:

| Struct / JSON field | Stored value and use |
| --- | --- |
| `StaticProviderRecord.user_models` / `userModels` | `ModelEntry[]` for user-added models; `models` remains defaults and `modelConfigOverrides` remains overrides only |
| `StaticProviderRecord.model_preferences` / `modelPreferences` | Map of model ID to existing `ModelPreferences` |
| `ProviderRecord.model_preferences` / `modelPreferences` | Same preference map for managed catalogs |
| `StaticProviderRecord.endpoint_overrides` / `endpointOverrides` | Map of model ID to the existing `NewApiEndpoint` executable subset; separate from discovered `endpointTypes` |

No fields are added to `ModelMetadata`, `ModelNetwork`, `StaticCredential` or the
closed endpoint enum: old v18 readers reject unknown nested fields there. Model
source needs no extra tag: persisted membership in defaults/overrides/userModels
defines it. Build the effective static catalog **and** driver routes from defaults
plus userModels, apply descriptor overrides to their owned defaults and endpoint
overrides before deriving routes. Preserve these fields on refresh, writes and
bundled-definition reseeding. User IDs must not collide with default/override IDs;
a refresh introducing a collision fails without replacing the admitted catalog.

Old v18 binaries can read these records but drop new top-level fields on rewrite,
including automatic `seed_bundled` rewrites when fingerprints change. That downgrade
loss is accepted explicitly, not limited to user edits. Disposable-database
acceptance must run an old v18 reader and its rewrite path on a new record.
No storage-owner gate, deletion history or format bump is added. Unknown database
formats retain their existing rejection and byte-preservation rules.

| Edit | Effect on already-adopted Sessions |
| --- | --- |
| Credential, base URL, headers, proxy, timeout | Current connection is resolved on the next request; already-issued I/O uses its admitted connection |
| Descriptor/capability edits or saved preferences | Adopted snapshot stays unchanged; changes apply on the next explicit adoption, preferences only when passed |
| Disable/delete Provider or model | Next request/adoption fails unavailable/not found; no forced switch or cross-owner reference rewrite |
| Effective route change | An older adopted request fails with a diagnostic to select the model again; never reinterpret protocol-specific effort/tier options |

For static New API selection, Provider stamps the chosen `ModelProtocol` string
in existing `ModelSelection.metadata.extensions["providerProtocol"]`. Prompt and
request preparation already carry metadata through adoption to `ModelRequest`.
Provider compares that value with the current effective route before resolving
credentials/network I/O. Missing metadata in an older New API Session also requires
explicit reselection. Untargeted F3 preference updates preserve and check the
adopted route before mapping options; only explicit target selection creates a new
route stamp. This is Provider-owned data in the existing metadata extension, not a
new core field, epoch or registry. Managed route/lease checks remain unchanged.

Legacy TypeScript import stays explicit, read-only and per-entry:
`imported`/`already_imported`/`rejected`. Map connections, defaults, user overrides,
user models, disables, preferences and scoped selections explicitly. `activeModelId`
is only a hint and cannot overwrite an existing scoped choice. Nonempty static
usage settings, unsupported `retryConfig`/`capabilityDefaults` and other unmapped
fields reject the entry and preserve its source; freeze the mapping before Ready.
Never overwrite an existing target, copy OAuth grants or resurrect deletion on
repeated import. Drafts, templates, usage replies and resolved credentials are not
persisted. Register cleanup at acquisition, close admission, cancel owned I/O,
release leases and settle accepted calls before removing services.

## 7. Acceptance and delivery

| Gate | Required observable evidence |
| --- | --- |
| Ready | DTOs, client migration, explicit preference targets, route handling and legacy mapping accepted; proposal is not implementation |
| Secrets/patches | Header edit preserves others, timeout edit preserves proxy auth, keep/set/clear/ref and authority boundaries, no secret in RPC/log/errors |
| Models/routes | Real calls to user-added builtin/custom models; edit/reset/delete/refresh/restart; route change rejects an older Session and reselection restores correct effort/tier |
| Probe | Unsaved discovery, validated new host with stored credential, failure/timeout/unload without writes, stale reply discarded, no client-cancel claim |
| Preferences | Save/clear/restart; exact target with preferences; unavailable target fails without fallback; untargeted F3 and consumer choices unchanged |
| Accounts | Existing login/logout/usage, actual advertised-method versus missing-package-method behavior, unchanged Usage DTO, no static usage config |
| Concurrency/recovery | Two-client typed CAS conflicts on new and retained writes; lost create reply recovered by UUID; repeated create conflicts; read errors preserve healthy entries/drafts |
| Storage/import | New record read and rewritten by old v18 code including reseed; valid/invalid/repeated imports, collisions and source preservation in disposable databases |
| Client flow | Actual source-built App Server and Desktop/CLI: draft → probe → create → model/select → call → edit → call → restart → disconnect/delete; no fake-daemon-only evidence |

Reuse existing Provider, transport, account and CLI flow tests for implementation.
This documentation change checks references/consistency only; no runtime acceptance
is claimed. Tests use isolated homes, never an existing user's store.

## 8. Rule check

| Rule | Application |
| --- | --- |
| §4.1 | Provider writes its own process state; Session adoption uses existing owner commands |
| §4.2 | Disconnect does not retract accepted work or committed writes |
| §4.3 | No before hook; boundary validation and owner business validation; no network wait under commit locks |
| §4.4 | No Provider branch in core/Rpc/Dispatcher |
| §4.5 | Owned related selections/models/preferences/routes commit together; existing adoption remains atomic |
| §4.6 | Management data does not enter model context; only existing adoption paths change configuration |
| §4.7 | Accounts use public methods; other owners' references are not rewritten |
| §4.8 | §5 raises no mechanism requirement; existing metadata and v18 top-level JSON suffice |
| §4.9 | No new Timeline event or pushed process notification |
| §1 | Extend the existing process Provider; no Capability or wiring plugin |
| ADR 002 D1 | Process records belong to Provider; adopted branch state to its owner; drafts to clients |
| ADR 002 D2 | Reuse namespaces/resolver; no management plugin or second registry |
| ADR 002 D3 | Optional account failure affects its own operations; no core dependency |
| ADR 002 D4 | Business configuration is owner state; updates do not reload |
| ADR 002 D5 | §3 names fields, methods, mutability, interruption and errors independently of Rust bindings |

## 9. Ablation review

The single management read supplies one edit snapshot; the unchanged catalog serves
selection. Create, update and probe respectively handle recoverable identity, secret-
preserving writes and unsaved I/O. Disconnect reuses update. Separate user-model
storage is required to distinguish it from an orphaned default override and make
it executable. The route stamp preserves the adopted protocol meaning using
existing metadata. No notification, version-history service, template entity,
credential abstraction or second client configuration store is needed.
