# Settings service

Status: Draft (for review). Roadmap item S8b ([#118](https://github.com/arcships/dim/issues/118)).
Builds on S8a ([#54](https://github.com/arcships/dim/issues/54), PR [#90](https://github.com/arcships/dim/pull/90));
implementation depends on C4 ([#63](https://github.com/arcships/dim/issues/63)).
Overview: [§6.4 Configuration](../overview.md#64-conventions). Scope:
[ADR 002](../../adr/002-plugin-scope.md) D4 and D5 (process, inside the existing Settings plugin);
[ADR 004](../../adr/004-loader-plugin-management.md) D4.

## 1. Summary

Settings serves sources owned by plugins that are not written in Rust. Such a plugin declares each
source in its package manifest: the source name, its files, a JSON Schema (with `default`
values) and its user-only fields. Settings then offers three things over JSON documents:

- the process service `settings`, which the owner uses: `read`, `update` (CAS on the revision)
  and `projects`;
- the client management methods `settings.read` and `settings.update`;
- the same rutis events as S8a, `settings.changed` and `settings.project_authorized`. Their
  payloads become serializable. There is no `subscribe(source)`.

There are two write paths. A client write (`settings.update`) may change any field. An owner write
(the `settings` service) may not change a user-only field. A write the owner makes on behalf of
its tool is an owner write, so it is rejected when it changes a user-only field.

Credentials are `*Ref` fields holding opaque strings. One process service,
`credentials.resolve(owner, reference)`, resolves them. Provider's `secretRef` resolver is replaced
by it, and Provider's AWS case goes through it too (§4.4).

`BusinessDocument` stays the Rust binding and is unchanged. Rust sources keep their owners'
management methods. F2's `permission.rules`/`permission.set_rules` stay (§8.1).

No new plugin, no Timeline event, no change to Timeline, Dispatcher, rutis or assembly.

### 1.1 What this does not do

- No `subscribe(source)`. The issue text lists it, but S8a moved change notification to the rutis
  event bus (issue comment). Settings keeps no watch channel of its own.
- No generic client access to Rust sources (`mcp`, `mcp-projects`, `skills`, `permission`).
  Their validation is Rust code that Settings cannot run on client JSON.
- No source discovery method and no settings UI. See §10.
- No secret store. Today no public plugin provides one (§4.4, §10).
- MCP's inline tokens, `env` and `headers` stay where they are (ADR 004 D4). Moving them to
  references is not part of S8.

## 2. Structure

```
 Process scope
 +-------------------------------------------------------------------------+
 |  Settings (existing plugin)                                             |
 |   SettingsService                                                       |
 |     BusinessDocument sources (Rust owners, unchanged)                   |
 |     declared sources (from package manifests): schema, defaults,        |
 |       files, user-only pointers                                         |
 |   named service `settings` (HostDispatch)  <-- owner writes             |
 |   managementMethods `settings`             <-- client writes            |
 |   emits settings.changed on settings/<source>,                          |
 |         settings.project_authorized on settings/projects                |
 +------^-------------------------------^----------------------------------+
        | read/update/projects          | settings.read / settings.update
        |                               |
  TypeScript owner (rutis runtime row)  Rpc  <--  local client
        |
        | resolve(owner, reference)
        v
  `credentials` (provided by the host secret store owner, if any)
        ^
        | resolve(owner = "provider", reference)
  Provider (existing)
```

Dependencies point to the core. Settings depends on no plugin and names no owner. Provider keeps
its configuration-gated dependency on the secret store (it changes type, not shape). A
TypeScript owner depends on `settings` (and on `credentials` only if it has `*Ref` fields).

## 3. Data and state

### 3.1 Source declaration

Each package that owns sources lists them in its manifest:

```json
"settings": [
  {
    "source": "web",
    "userFile": "web.json",
    "projectFile": null,
    "schema": {
      "type": "object",
      "properties": {
        "searchService": {
          "type": "object",
          "properties": {
            "kind": { "type": "string" },
            "endpoint": { "type": "string" },
            "secretRef": { "type": "string" }
          },
          "required": ["kind"]
        },
        "maxResults": { "type": "integer", "minimum": 1, "default": 5 }
      },
      "additionalProperties": false
    },
    "userOnly": ["/searchService"]
  }
]
```

| Field | Rule (checked once, where the host reads the manifest) |
| --- | --- |
| `source` | `[a-z0-9_-]`, as for Rust sources. Unique among all sources of the process: a second declaration, or a declaration naming a Rust source, fails assembly |
| `userFile` | Same rule as `BusinessDocument::USER_FILE` (a plain filename in the home root) |
| `projectFile` | Optional, same rule as `PROJECT_FILE`. Present means the source also has project scope. This is the "allowed scopes" declaration |
| `schema` | JSON Schema (draft 2020-12). It must compile, and its root must be `type: object` |
| `userOnly` | JSON Pointers (RFC 6901) to object members. Each must name a member reachable through `properties` |
| owner | The package `id` from the same manifest. Not written in the entry |

The default document is `{}` with defaults applied (§3.2). It must be valid against the schema,
or the manifest is rejected. So a missing file always reads as a valid document.

### 3.2 Documents, validation and defaults

A declared source holds one JSON object. Settings handles it at its boundary, as S8a does for
Rust documents:

- **Defaults**: for every object whose schema has `properties`, a missing member with a `default`
  is filled in. This recurses through `properties` only (not through `$ref`, `allOf`, `anyOf`,
  `oneOf`), the same reach as serde's `#[serde(default)]` on nested structs.
- **When**: on every write, and when reading a persisted file or native row. A missing file is
  `{}` with defaults applied. The written and returned document is the filled one.
- **Validation**: after defaults, against the schema. Then every member whose name ends in `Ref`
  must be a non-empty string.
- Consumers use the document as delivered. They do not validate again or add defaults. A rule
  that JSON Schema cannot express (such as a CIDR) is the owner's diagnostic at use time.

### 3.3 Persistence and revisions

Unchanged from S8a. User scope uses the user file, or the additive native row `user/<source>` in
v13 storage. Project scope uses the project file and content CAS. Revisions are the same opaque
strings (`missing`, a SHA-256 of the file, or `native:user/<source>:<version>`). No format change.

### 3.4 Runtime state

Settings keeps the compiled declarations in memory, built from its factory configuration. Nothing
else is new. Nothing enters the Timeline.

## 4. Interfaces

### 4.1 Common shapes

| Shape | JSON |
| --- | --- |
| `Scope` | `"user"` or `{"project": <root>}`. `root` is the project root number Settings assigns in this process (`SettingsScope::Project`). It is not a durable identity and is never persisted |
| `Snapshot` | `{revision: string, document: object}` |
| `Error` | `{code, message}` |

### 4.2 Service `settings` (owner path)

Process scope, no instance key, provided by Settings.

| Method | Mode | Request | Reply |
| --- | --- | --- | --- |
| `read` | async | `{source, scope: Scope}` | `Snapshot` |
| `update` | async | `{source, scope: Scope, expectedRevision, document}` | `Snapshot` (the stored document) |
| `projects` | sync | `{}` | `{projects: [{root, cwd}]}`: the authorized project roots now |

- `update` is CAS: it fails with `conflict` unless `expectedRevision` is the current revision.
- `update` is an owner write. It fails with `user_only_field` when the stored document and the new
  one differ at any `userOnly` pointer (a value changed, added or removed). The comparison uses
  the documents after defaults.
- `projects` is what an owner reads first. Afterwards it follows `settings.project_authorized`.
- Cancellation: the caller drops the call. A write that has already committed stays committed,
  as in S8a.
- Errors (§6): `unknown_source`, `invalid_scope`, `invalid_document`, `user_only_field`,
  `conflict`, `unsynced`, `failed`, `closed`, `cancelled`.

The service has no caller identity. Ownership comes from the manifest: each source has one owner,
and only that package is written against it. This is the trust model of in-process plugins
(AGENTS.md rule 11): dependency declarations are not a sandbox.

### 4.3 Management methods `settings.*` (client path)

Namespace `settings`, provided by Settings through `managementMethods`. They are never served to
paired devices (overview §6.2 rule 9).

| Method | Request | Reply | Errors |
| --- | --- | --- | --- |
| `settings.read` | `{source, scope}` | `{revision, document}` | invalid params (bad scope), `settings_unknown_source`, `settings_failed` |
| `settings.update` | `{source, scope, expectedRevision, document}` | `{revision, document}` | invalid params (bad scope, schema violation, empty `*Ref`), `settings_unknown_source`, `settings_conflict`, `settings_unsynced`, `settings_failed` |

- Only declared sources are served. A Rust source (for example `mcp`) is `settings_unknown_source`.
  Its owner's methods stay the way to change it.
- A client write may change user-only fields. There is no other difference from the owner path.
- `closed` maps to the existing retryable reloading error.

### 4.4 Service `credentials`

Process scope, no instance key, provided by the plugin that owns the host secret store.

| Method | Mode | Request | Reply |
| --- | --- | --- | --- |
| `resolve` | async | `{owner, reference}` | `{value: string}` |

- `owner` is the id of the plugin whose document holds the reference: the package `id`, or
  `"provider"` for Provider. The store may keep references per owner.
- `reference` is the opaque string from a `*Ref` field. One spelling everywhere: a member named
  `secretRef` or `<name>Ref` whose value is a non-empty string. Dim never parses it.
- Errors: `not_found`, `failed`, `cancelled`. Messages never contain a value. A consumer reports
  its own message, as Provider does today ("Provider secret lookup failed").
- Values are resolved per use and never written to Settings, the Timeline or a reply.

**Provider.** Provider's `ResolveProviderSecret` (`provider/secrets.rs`) is replaced by this
service. A `{"secretRef": "..."}` credential resolves with `owner = "provider"`. For a token,
`value` is the token. For AWS Bedrock auth, `value` is the JSON text
`{"accessKeyId", "secretAccessKey", "sessionToken"?}`, which Provider parses into its existing
`AwsCredentials` at that boundary and validates as today. So the AWS case needs no second resolver.
Inline credentials (`ProviderSecret::Token`/`Aws`) are unchanged.

Today no product plugin provides a secret store. `ProviderPlugin::default` has no secret host, and
a `secretRef` fails with "No provider secret store configured". Only tests provide one. S8b keeps
that state; it defines the contract and moves the test store to it.

### 4.5 Events

Unchanged channels and names from S8a. The payloads become serializable data:

| Event | Channel | Payload |
| --- | --- | --- |
| `settings.changed` | `settings_changes(source)` = `settings/<source>` | `{source: string, scope: Scope, revision: string}` |
| `settings.project_authorized` | `project_authorizations()` = `settings/projects` | `{root: number, cwd: string}` |

A declared source emits `settings.changed` exactly as a Rust source does, after the commit, for
both write paths. The payload never carries the document. A consumer reads once, then follows the
events.

A TypeScript part receives these events through the C4 binding for rutis events (§10, question
2). S8b does not add a delivery path of its own. Until that binding exists, a TypeScript owner
reads at use time, which is what most owners do anyway (F2's Permission does).

### 4.6 Rust binding

In `dim-app-server::settings` (S8a types, changed only where noted):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsScope { User, Project(usize) }            // + Serialize/Deserialize

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsChanged { pub source: String,           // was &'static str
                             pub scope: SettingsScope, pub revision: String }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAuthorized { pub root: usize, pub cwd: PathBuf }   // + Serialize/Deserialize

/// One manifest entry, parsed and checked where the host reads the manifest.
pub struct SourceDeclaration { pub owner: String, pub source: String, pub user_file: String,
                               pub project_file: Option<String>, pub schema: serde_json::Value,
                               pub user_only: Vec<String> }

pub enum Writer { Owner, Client }

impl SettingsService {
    pub async fn read_declared(&self, source: &str, scope: SettingsScope,
                               cancel: &CancellationToken) -> Result<SettingsSnapshot<Value>>;
    pub async fn update_declared(self: &Arc<Self>, source: &str, scope: SettingsScope,
                                 expected: &str, document: Value, writer: Writer,
                                 cancel: &CancellationToken) -> Result<SettingsSnapshot<Value>>;
}
```

- `SettingsPlugin::new`/`native` take the declarations as factory configuration, next to
  `project_roots`. `apply` compiles each schema with `jsonschema` (already in `Cargo.lock` through
  the vendored aimux) and records each declared name in the existing `claims` map. A Rust type
  that later names a declared source fails with the existing "belongs to another document type".
- The file and native paths are shared with `BusinessDocument`, keyed by source name instead of
  `D::NAMESPACE`. CAS, the gate, leases, the unsynced case and `publish` stay as they are.
- `SettingsError` gains `UnknownSource` and `UserOnlyField`. A schema failure is `Invalid` with
  the validator's message (the instance path, never the value).
- The `settings` service is a `HostDispatch` under `host_key("settings")`, and assembly registers
  `settings` in the `ServiceCatalog`, as `pluginFiles` is done today.
- Settings provides `ManagementMethods` for `settings`, and assembly lists Settings' row as the
  owner of that namespace.
- Credentials: `pub trait ResolveCredential { fn resolve(&self, owner: String, reference: String,
  cancel: CancellationToken) -> HostFuture<'_, String>; }` and `pub struct
  CredentialsHost(pub Arc<dyn ResolveCredential>)` replace `ResolveProviderSecret` and
  `ProviderSecretHost`. Provider's `Deps` change from `When<Arc<ProviderSecretHost>>` to
  `When<Arc<CredentialsHost>>`, behind the same configuration switch. A Rust store also provides
  `host_key("credentials")` for TypeScript consumers.

## 5. Flows

### 5.1 Owner reads and updates

```
 TS owner                 settings (HostDispatch)      SettingsService            storage
   | read {web, "user"} ---->| decode request           |                          |
   |                         | read_declared ---------->| file / user/web row ---->|
   |                         |                          | defaults, schema check   |
   |<-- {revision r1, document} -------------------------|                          |
   | update {web, "user", r1, doc'} ->| update_declared(Owner)                       |
   |                         |                          | defaults, schema, *Ref   |
   |                         |                          | user-only diff vs current|
   |                         |                          |   differs: user_only_field
   |                         |                          | CAS r1 ------------------>| replace
   |                         |                          | emit settings.changed {web, "user", r2}
   |<-- {revision r2, document} -------------------------|                          |
```

The user-only diff is checked under the same lease (or native transaction) as CAS, against the
document it replaces.

### 5.2 Client edits a user-only field

```
 Client            Rpc              Settings (managementMethods)
   | settings.read {web, "user"} ---->| {r2, doc}
   | settings.update {web, "user", r2, doc with searchService} -> update_declared(Client)
   |<-- {r3, doc}                     | emit settings.changed {web, "user", r3}
 TS owner (its tool asks to change searchService)
   | update {web, "user", r3, doc with another searchService} -> user_only_field, nothing written
```

### 5.3 Credential use

```
 TS owner (at use time)       credentials (store)
   | read {web, "user"} -> document.searchService.secretRef = "web-search"
   | resolve {owner: "web", reference: "web-search"} ----->|
   |<-- {value} | not_found | failed                        |
   | use value for this call only; never stored or logged
```

### 5.4 Manifest boundary

```
 assembly reads package manifest
   -> parse `settings` entries, check names, files, pointers
   -> compile schema; check that {} with defaults is valid
   -> SettingsPlugin factory configuration (declarations)
 Settings apply: claims each declared name; provides `settings`, `settings.*`
```

An invalid declaration fails assembly with the package id and source in the message, like an
invalid manifest today.

## 6. Errors

| Code (service / wire) | When | Retry |
| --- | --- | --- |
| `unknown_source` / `settings_unknown_source` | Not a declared source (including Rust sources) | No |
| `invalid_scope` / invalid params | Project scope for a source without `projectFile`, or a root not authorized | After authorizing the project |
| `invalid_document` / invalid params | Schema violation, a non-object document, or an empty `*Ref` | No; nothing written |
| `invalid_document` / `settings_failed` | The persisted file or row is invalid (on read) | After the user repairs it |
| `user_only_field` | An owner write changes a user-only field | No; the user changes it through the client |
| `conflict` / `settings_conflict` | Stale `expectedRevision` | Read and retry |
| `unsynced` / `settings_unsynced` | The new content is in place and announced, but could not be synced | Read before retrying |
| `failed` / `settings_failed` | Storage, persistence or worker failure | Read before retrying |
| `closed` / reloading | Settings is unloading | Yes |
| `cancelled` | Call dropped or cancelled | Caller's choice |

## 7. Security

- Management methods are local only; paired devices never reach `settings.*` (overview §6.2 rule 9).
- User-only fields protect user decisions from the owner's own writes, including writes made for
  the model's tool. They are not a boundary against a hostile plugin. In-process plugins are
  trusted and not sandboxed (AGENTS.md rule 11, ADR 003).
- Project files keep their own authority (ADR 001). A declared project file is read only for an
  authorized root, as for Rust sources. Settings is not a security setting, and no Settings
  document confers approval (ADR 003).
- Documents hold references, not secrets. Resolved values exist only inside the consumer's call.
  Error messages and validator messages carry paths, never values.
- Settings documents never enter the Timeline. Replay and fork do not restore or widen them.

## 8. Relationship to existing parts

| Part | Relationship |
| --- | --- |
| S8a Settings | Supplement: declared sources, the named service, the `settings` namespace, serializable event payloads. `BusinessDocument`, CAS, native rows and channels are unchanged |
| MCP, Skills | Unchanged. Their sources stay Rust documents with their own management methods |
| Permission rules (F2) | Unchanged; see §8.1 |
| Provider | `ResolveProviderSecret` is replaced by `credentials` (§4.4). The secret store stays optional by configuration, as today |
| ADR 004 D4 | It says Settings publishes no management methods until S8b. S8b adds the `settings` namespace, limited to declared sources. ADR 004 is updated with the implementation |
| C4 (TypeScript bridge) | Provides package loading for TypeScript parts and the binding that delivers rutis events to them. S8b's implementation waits for it |
| PluginManager (P3) | Its manifest design has no `settings` entry yet; see §10 question 1 |
| Rpc | Unchanged; it dispatches the new namespace like any owner's |
| Timeline, Dispatcher, rutis, loader | Unchanged |

Public additions: the `settings` service, the `settings` management namespace (two methods), the
`credentials` service (replacing a Rust-only trait), the manifest `settings` entry, and the JSON
shapes of the two existing events. Each one is needed for a non-Rust owner to read, write and be
told about its source, for a client to edit it, and for a reference to resolve.

### 8.1 Decision: keep `permission.rules`/`permission.set_rules`

F2 left open whether to fold its methods into `settings.read/update`. They stay:

- `permission` is a Rust source. Its validation (no separator characters, at least one word) is
  Rust code. `settings.update` validates against a declared JSON Schema, so serving Rust sources
  would need a type-erased registry of Rust documents in Settings. Nothing else needs that.
- `permission.set_rules` already has a typed contract and error codes, and F6 is designed against
  it.
- The rule is the same for MCP and Skills: a Rust owner keeps its own methods, and
  `settings.*` serves declared sources.

## 9. Rule check

| Rule | Answer |
| --- | --- |
| Principles §1 (one event bus) | Notification stays on the rutis event bus with S8a's events. No `subscribe`, no watch channel, no second bus. TypeScript delivery uses C4's binding of the same events |
| Principles §3 (dependency direction) | Settings depends on no plugin. Owners depend on Settings. Provider's store dependency keeps its shape. No core part depends on an optional plugin |
| §4 rule 1 (command events) | Not used. Settings is process configuration written through its owner's typed write (overview §6.2 rule 2) |
| §4 rule 2 (handover) | Not applicable; no handover between modules |
| §4 rule 3 (before callback) | Not used |
| §4 rule 4 (core has no business meaning) | Settings validates against declared schemas and names no owner |
| §4 rule 5 (batches) | No Timeline writes |
| §4 rule 6 (context) | Nothing enters model context. A tool's view of settings is its owner's business |
| §4 rule 7 (public interfaces only) | Owners use the `settings` service; clients use management methods; consumers use `credentials` |
| §4 rule 8 (infrastructure unchanged) | No change to Timeline, Dispatcher, event envelope, rutis, loader, Agent assembly or Session facade. Existing `HostDispatch`, `ServiceCatalog`, `managementMethods` and rutis events are used as they are |
| §4 rule 9 (only needed facts in Timeline) | No event is added; settings are not Timeline facts |
| ADR 002 D1 (scope) | Settings is a process plugin; the sources outlive Sessions |
| ADR 002 D2 (no wiring modules) | No new plugin. The service and methods live in Settings, the owner of the data |
| ADR 002 D3 (dependencies) | No new optional dependency. Provider's existing configuration-gated store dependency changes type only |
| ADR 002 D4 (configuration) | Business configuration in the owner's named source, read at use time, changed with notification, no reload |
| ADR 002 D5 (language-neutral) | §4.1–§4.5 are the contract; §4.6 is the Rust binding |

Requirements raised to mechanism owners: none. S8b uses C4's TypeScript binding when it lands
(§10 question 2); that is a dependency on C4's planned work, not a new requirement on dim runtime
or rutis.

## 10. Open questions

1. **Which manifest carries the `settings` entry.** Manifest schema 1 is the fixed managed-provider
   format. Schema 2 is PluginManager's design and is not implemented. C4 has not chosen how
   TypeScript parts are packaged. The entry's shape is defined here; its home is decided with C4's
   loading path.
2. **How a TypeScript part listens to rutis events.** rutis 0.10's `definePlugin` context offers
   only `use`, `provide` and `effect`. rutis-bridge can emit a rutis event into the Node runtime
   (`EmitToCordis` with a mount's `emits`), but only for mounted Cordis plugins. The C4 spike lists
   event subscription as an open binding item. S8b only needs that binding to carry the §4.5
   payloads and to end the listener with the part.
3. **Packages installed while running.** Declarations are Settings factory configuration, so a
   runtime install (PluginManager, P3) would reload Settings and its dependents (MCP, Skills).
   ADR 002 D3 accepts reloads after structural operations, but whether this one is acceptable is
   decided with PluginManager. Today TypeScript packages are loaded only at startup.
4. **Which plugin provides `credentials` in the product.** No public plugin does today. The
   choice (an OS keychain plugin, a private package) and how an owner that needs it behaves when
   it is absent are not decided here.
5. **Source discovery for a settings UI.** A generic `/settings` page would need the declared
   sources and their schemas (for example a `settings.sources` method). Nothing requires it for
   S8b; it is decided with the CLI settings design.
6. **User-only fields for Rust sources.** No Rust owner writes its source on behalf of a tool
   today. If one does (Scheduler, Workflow), it needs a user-only declaration on its
   `BusinessDocument`. That is decided with that owner's design.

## 11. Acceptance (flow tests)

App Server flow tests (`crates/dim-app-server/tests/settings_flow.rs` and a TypeScript row loaded
through C4's path):

1. A TypeScript test package declares source `example` (user file, project file, schema with a
   `default`, one user-only pointer). Its first `read` returns the default document with revision
   `missing`. An `update` with that revision stores the document and returns a new revision. A
   stale revision fails with `conflict`. A schema violation or an empty `*Ref` fails and writes
   nothing.
2. A client `settings.update` changes the user-only field. The owner's next `update` that changes
   it fails with `user_only_field`; one that keeps it succeeds.
3. A Rust listener on `settings_changes("example")` receives `{source, scope, revision}` after
   each successful write on both paths, and none after a rejected one. The payload serializes to
   the §4.5 JSON shape.
4. Native profile: the user scope persists in row `user/example`; existing v13 rows are unchanged.
   Project scope on an unauthorized root fails with `invalid_scope`.
5. `settings.read` of `mcp` returns `settings_unknown_source`. A manifest declaring `mcp`, or two
   packages declaring the same source, fails assembly.
6. With a test `credentials` store: the TypeScript owner resolves the `*Ref` from its document,
   and Provider resolves a `secretRef` token and an AWS JSON value through the same store. No
   value appears in replies, errors or traces.
7. A paired-device connection is not served `settings.read`.
