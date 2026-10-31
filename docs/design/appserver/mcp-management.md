# MCP configuration management

Status: Draft.

Scope: process.
Issue: [#453](https://github.com/arcships/dim/issues/453).
Rules: [design rules](../../architecture/rust-agent/principles.md), [ADR 002](../../adr/002-plugin-scope.md).

## 1. Scope

This document owns the configuration surface of the `mcp` management namespace: how a client
reads one entry in order to edit it and writes the result. It owns those methods' requests and
replies, the values they redact, and how a source revision orders an edit.

It does not own:

| Surface | Owner |
| --- | --- |
| `mcp.auth.start`/`complete`/`cancel`/`logout`, the catalog's `auth` field, `session.mcp` | [MCP OAuth](mcp-oauth.md#9-interfaces) |
| `mcp.temporary.capabilities` | [TemporaryMcp](../plugins/TemporaryMcp.md) |
| Entry admission, dispatch, withdrawn-namespace answers, interruption settlement | [Management methods](management-methods.md) |
| Revisions, compare-and-set, project authorization | [Settings service](settings-service.md#33-persistence-and-revisions) |
| Which plugin registers which entry, and how | [PluginManager](../plugins/PluginManager.md#37-content-registration) |

Two terms are used below:

- The **declared** configuration is what an entry's source holds for it: `mcp.json` for a user
  entry, `.mcp.json` for a project entry, the plugin's registered document for a plugin entry.
- The **effective** configuration is the declared one with the daemon's own resolution applied:
  a project entry's stdio `cwd` is resolved against the workspace and its enablement is taken
  from the user's record (ADR 003 D3), and a plugin entry's stdio `cwd` is resolved against the
  plugin directory.

## 2. Current behavior

`mcp.catalog` lists the merged entries with `id`, `source`, `enabled`, `transport` and the
optional `auth`, plus both source revisions and each source's application record. `mcp.save`
creates or replaces one source's entry, `mcp.update` applies a typed patch, `mcp.remove` removes
the entry and `mcp.set_enabled` writes enablement. `mcp.configuration` returns a redacted
projection of one entry, `mcp.inspect` returns that projection with the entry's connection
generations, and `mcp.test` probes a draft configuration.

Three properties of that surface prevent a client from editing an entry:

1. **The read carries no revision.** `mcp.save`, `mcp.update`, `mcp.remove` and `mcp.set_enabled`
   all require the target source's revision as `expectedVersion`, and the read does not return
   it. A client pairs the read with a separate `mcp.catalog` call, so the entry it renders and
   the token it writes with come from two snapshots taken at different times. Creating a new
   entry has the same shape of problem: it reads the token from the catalog, which is the list
   read, not the entry read.
2. **The read and the patch are not the same configuration.** Both go through the effective
   entry. For a project stdio entry that declares no `cwd`, the read shows the resolved
   workspace path, and a patch that leaves `cwd` alone persists that absolute path into
   `.mcp.json`. For a project entry the patch also persists the user record's enablement as the
   file's `enabled` field, which the same design ignores when it runs the entry.
3. **The projection does not say which values are safe to write back.** A client cannot tell a
   value it may echo from one it must leave alone, so an editor either drops stored fields or
   writes effective values into a source.

## 3. Decisions

### D1. One edit read: the entry, its source revision and its enablement

`mcp.configuration` is the single read a client uses before editing one entry, and the single
read `mcp.save` and `mcp.update` are built from. Its reply carries the source revision, the
enablement the entry runs with, and the configuration projection, all taken from one source
snapshot. A client never combines a projection from one read with a revision from another; this
is the rule [Provider management](provider-management.md#31-read-models) states for the sole
edit-view read.

`mcp.catalog` stays the list read: no entry configuration and no per-entry revision, because a
list of entries and a token for writing one answer different questions. `mcp.inspect` stays the
run view: it reports the effective configuration with the entry's connection generations, and a
client must not write its values back.

### D2. The management surface speaks the declared configuration

Every configuration a client reads for editing, patches, or submits in a draft is the declared
entry. The daemon applies its own resolution when it runs an entry or inspects a running one.
Echoing a read value back therefore changes nothing, and no host path enters a source through an
edit: a project entry's `cwd` is returned as the file declares it (absent when the file declares
none), and a patch that does not mention `cwd` leaves the file's `cwd` as it was.

The one field whose value is not the source's is `enabled`: for a project entry it is the user's
record, which project files cannot set. It is returned because it is what the entry runs with,
and it is written only by `mcp.set_enabled`. Enablement and credentials therefore change without
a configuration write (`mcp.set_enabled`, `mcp.auth.*`); a client never rewrites an entry to
change either.

### D3. Redaction covers every value a client does not need

The projection returns the executable, the endpoint's origin, names, counts and presence flags.
Argument values, environment and header values, the bearer token, and the URL's userinfo, path,
query and fragment are withheld: a stdio command line and a request header commonly carry a
token, and a URL can carry one in its userinfo or query. This design adds no field that exposes a
stored value; an edit preserves every withheld field by omitting it (D4).

### D4. An edit is a patch; an omitted field keeps its stored value

`mcp.update` takes the source revision and a typed `patch`. Omission preserves, an explicit value
replaces, and a map changes only the keys it names. This is what lets a client edit one field
without holding another field's secret. `mcp.save` remains the complete-configuration write for
creating an entry, or replacing one whose withheld values the user re-enters; a client does not
use it to round-trip an existing entry, because a complete configuration would have to carry the
secrets the read withholds.

### D5. One revision per source document, and a conflict is a reread

The revision is Settings' opaque revision for the entry's source document
([Settings service](settings-service.md#33-persistence-and-revisions)): the same string
`mcp.catalog` reports as `userRevision` or `projectRevision`, and the same one every write takes
as `expectedVersion`. All entries of one source share it, so a write to another entry of that
source makes the token stale and the next write answers `mcp_settings_conflict`. A client then
re-reads the entry it is editing and shows the conflict; it does not retry the same write.

A plugin entry has no Settings source and no revision. The read returns no `revision` for it,
which is the client's signal that the entry is read-only: `mcp.save`, `mcp.update`, `mcp.remove`
and `mcp.set_enabled` refuse a plugin target. Sign-in, `mcp.test`, `mcp.inspect` and
`mcp.disconnect` still serve one, as they do today.

### D6. Writes keep their reply

`mcp.save`, `mcp.update`, `mcp.remove` and `mcp.set_enabled` keep replying `{}`. A client that
has written re-reads the entry and the catalog: the server normalizes what it stores (map merges,
URL and OAuth validation, a project entry's declared `cwd`), so the result is read back rather
than predicted from the request. Returning the new revision instead would not remove that read.

### D7. No server-side copy of an entry

Duplicating an entry is a client-side draft built from the redacted read. Secrets are not copied:
the credential record is keyed by server id and URL ([MCP OAuth](mcp-oauth.md#41-credential-record-persisted)),
so a copy either shares that credential or needs its own, and neither is decided here. The new
entry starts without the environment, header and bearer values of its original, and the user
enters them again.

## 4. Contract

Namespace `mcp`, process scope, served on local connections
([Management methods § Dispatch and lifecycle](management-methods.md#dispatch-and-lifecycle)).
`source` is `"user"`, `"project"` or `"plugin"`; `cwd` is required for a project source and is
the workspace whose `.mcp.json` is read, authorized as today.

These are the methods that read or write one entry's configuration:

| Method | Request | Reply | Mutating |
| --- | --- | --- | --- |
| `mcp.configuration` | `{cwd?, id, source}` | `{revision?, enabled, configuration: Configuration}` | no |
| `mcp.save` | `{cwd?, id, source, expectedVersion, configuration}` | `{}` | yes |
| `mcp.update` | `{cwd?, id, source, expectedVersion, patch}` | `{}` | yes |

`mcp.save` takes a complete stdio, HTTP or SSE definition in the existing `mcpServers` format and
creates or replaces that source's entry, not a merged one. The namespace's other methods are
listed in §6 and keep their references: `mcp.catalog`, `mcp.inspect`, `mcp.test`, `mcp.remove`,
`mcp.set_enabled` and `mcp.disconnect` in the App Server's
[MCP configuration reference](../../../crates/dim-app-server/README.md#mcp-configuration-management),
`mcp.auth.*` in [MCP OAuth §9.1](mcp-oauth.md#91-management-methods-namespace-mcp-local-transports-only),
and `mcp.temporary.capabilities` in [TemporaryMcp](../plugins/TemporaryMcp.md). §3 D5 states the
revision every write takes.

### 4.1 Entry read (`mcp.configuration`)

| Field | Type | Meaning |
| --- | --- | --- |
| `revision` | string, absent for a plugin entry | The source document's revision, the token `expectedVersion` takes (D5) |
| `enabled` | boolean | The enablement the entry runs with; for a project entry the user's record |
| `configuration` | `Configuration` | The declared configuration projection below |

```text
client                                  Mcp (process)                     Settings
  | mcp.configuration {source,id,cwd?}     |                                 |
  |--------------------------------------->| read the entry's source         |
  |                                        |-------------------------------->|
  |                                        |<-- {revision, document} --------|
  |                                        | project entry: enablement record|
  |<-- {revision, enabled, configuration} -|                                 |
  | user edits one field                   |                                 |
  | mcp.update {expectedVersion: revision, |                                 |
  |             patch}                     | patch the declared entry, then  |
  |--------------------------------------->| compare-and-set that revision   |
  |                                        |-------------------------------->|
  |<-- {} ---------------------------------|                                 |
  | re-read to show what is stored         |                                 |
```

### 4.2 `Configuration`

The declared entry's projection. A stdio entry:

| Field | Type | Meaning |
| --- | --- | --- |
| `type` | `"stdio"` | Transport |
| `command` | string | The executable as declared |
| `argsCount` | integer ≥ 0 | How many declared arguments are stored; the values are withheld |
| `cwd` | string, absent when none | The declared working directory; a project or plugin entry's relative path, not a resolved one |
| `envNames` | string[] | Declared environment names, ascending; values withheld |

A remote entry:

| Field | Type | Meaning |
| --- | --- | --- |
| `type` | `"http"` or `"sse"` | Transport |
| `endpoint` | string | `scheme://host[:port]` of the declared URL; userinfo, path, query and fragment withheld |
| `headerNames` | string[] | Declared header names, ascending; values withheld |
| `authConfigured` | boolean | A static bearer token is stored |
| `oauthConfigured` | boolean | An `oauth` object is stored |
| `oauth` | object, present when `oauthConfigured` | `clientId?` string, `callbackPort?` integer 1 to 65535, `scopes?` space-separated string, `metadataConfigured` boolean; the metadata URL itself is withheld |

Withheld from every reply and error message: environment and header values, the bearer token,
URL userinfo, path, query and fragment, the authorization server metadata URL, argument values,
unknown extension fields of an entry and of its `oauth` object, and every credential of
[MCP OAuth](mcp-oauth.md) (D3).

### 4.3 Edit (`mcp.update`)

The patch fields and what an edit does with them; `mcp.save` takes a complete server definition
in the `mcpServers` file format instead.

| Patch field | Applies to | Omitted | Present |
| --- | --- | --- | --- |
| `command` | stdio | kept | replaced |
| `args` | stdio | kept | replaces the whole list, including an empty list |
| `cwd` | stdio | kept | replaced; an empty string clears it |
| `env` | stdio | kept | per key: a string sets the value, `null` removes the key |
| `url` | remote | kept, with everything the projection withholds | replaced |
| `headers` | remote | kept | per key, as `env` |
| `bearer` | remote | kept | `{action:"set",token}` replaces, `{action:"remove"}` clears |
| `oauth` | remote | kept | `{action:"remove"}` clears the object; `{action:"set"}` changes only the fields it names, `null` clearing an optional one |

A field of the other transport is `invalid_params` (`-32602`). The patched entry is validated as
a whole document before it is stored, so an edit cannot persist a configuration that `mcp.save`
would refuse, including one that sets `oauth` beside static credentials.

### 4.4 Errors

| `appError` | When |
| --- | --- |
| `mcp_unknown_server` | The target source holds no entry with that id, for a read or a write |
| `mcp_invalid_config` | The patched or saved document, the entry's id on a write, or an unauthorized workspace is refused |
| `mcp_settings_conflict` | `expectedVersion` is not the source's current revision (D5) |
| `mcp_settings_read` | The source could not be read |
| `invalid_params` (`-32602`) | A field of the wrong transport, an empty id on a read, a project target without `cwd` |
| `local_transport_required` | The connection is not local |
| `service_reloading` / `service_unavailable` | Mcp or its row is not serving; from [management methods](management-methods.md#dispatch-and-lifecycle) |

A read serves a disabled entry: enablement is a field, not a condition for reading.

## 5. What changes for a client

- The edit flow is `mcp.configuration` then `mcp.update` with the returned `revision`. No second
  call is needed to obtain the token (D1).
- The reply of `mcp.configuration` gains a wrapper: `{revision?, enabled, configuration}`. The
  Web settings page reads and writes the projection through it; the native CLI, which lists and
  toggles entries only, is unaffected.
- A client renders `command`, `cwd` and `endpoint`, the name lists and the counts, and offers
  replace-or-keep for `args`, environment values, header values, the bearer token, the URL and
  the metadata URL. It shows `argsCount`, `envNames` and `headerNames` as the fields that a
  replacement would replace.
- A client must not write back `mcp.inspect` values (D1).
- The Desktop client (arcships/dim-electron#20) replaces its `mcp.get` and `mcp.update`
  `capability_unavailable` answers with `mcp.configuration` and `mcp.update`.

## 6. What does not change

- `mcp.catalog`, `mcp.inspect`, `mcp.test`, `mcp.save`, `mcp.remove`, `mcp.set_enabled` and
  `mcp.disconnect` keep their requests and replies, including `mcp.inspect`'s effective
  configuration and its connection generations.
- The namespace's method set is unchanged, so `hello.features` advertises the same names and no
  client needs a capability check for a new method.
- No error code is added, removed or renamed.
- The `mcp` business documents, their file names, source precedence, project enablement record,
  plugin registration and the credential store keep their formats. No persisted data moves, so no
  format version changes.
- No new plugin, part, service, port, Timeline event or notification. Assembly, routing, the
  MCP Session part and its connections are untouched.

## 7. Events and state

None. Entry configuration is process configuration changed through its owner's typed writes, as
[design rules §4 rule 9](../../architecture/rust-agent/principles.md) requires; it never enters
the Timeline. Revisions come from the stored document, not from an event.

## 8. Dependencies

| Dependency | Use |
| --- | --- |
| `managementMethods` ([management methods](management-methods.md)) | The MCP row registers the namespace and its methods during `apply` |
| `settings` ([Settings service](settings-service.md#42-service-settings-owner-path)) | Reading a source with its revision, compare-and-set writes, project authorization |
| PluginManager ([§3.7](../plugins/PluginManager.md#37-content-registration)) | A plugin entry is registered content; Mcp reads it as declared and writes nothing |
| `McpAuth` ([MCP OAuth](mcp-oauth.md)) | The `mcp.auth.*` methods of the same namespace; unchanged |

## 9. Cross-module requirements

None raised to the runtime, rutis, Settings or Rpc: the revision and the source snapshot already
exist, and the change stays inside the MCP owner and its reply shapes.

Client-side follow-ups, which are not requirements on another owner: the Web settings page (this
repository) and the Desktop client (arcships/dim-electron#20) adopt §5.

## 10. Tests

Flow level, reusing the existing MCP flows:

| File | What it protects |
| --- | --- |
| `crates/dim-app-server/tests/transport_flow/product_protocol.rs` | The wire contract on local v3: the read returns the revision the catalog reports for the same source, a write with it succeeds and a repeat write with the same token is `mcp_settings_conflict`, a plugin entry's read has no revision and refuses a write, a project entry's read returns its declared `cwd`, and no withheld value appears in the read, including after an edit |
| `crates/dim-app-server/tests/mcp_flow.rs` | The service level: a project entry patched on one field leaves `.mcp.json` unchanged otherwise, with no resolved `cwd` and no `enabled` field written, while the effective configuration a connection runs still resolves the workspace; a user entry's read and patch round-trip |
| `crates/dim-app-server/tests/mcp_oauth_flow.rs` | Existing evidence, unchanged: `mcp.inspect` and `mcp.update` keep their replies for an OAuth entry |

The redaction assertions of
`mcp_editor_preserves_redacts_and_removes_credentials_with_source_cas` extend to the new reply
shape rather than moving to a new test: the same fixture writes argument, environment, header,
bearer and URL secrets and reads the entry back.

## 11. Rule check

| Rule | Answer |
| --- | --- |
| §4 rule 1 (commands for another owner's state) | No event. Entry configuration is process configuration written through the MCP owner's typed methods; the MCP owner is the writer of its own state |
| §4 rule 2 (handover) | Not applicable: no object is handed between modules |
| §4 rule 3 (`before` callbacks) | None |
| §4 rule 4 (core without business meaning) | No core change; Rpc dispatches the namespace as it does today |
| §4 rule 5 (batch) | Not applicable: no events are written |
| §4 rule 6 (model context) | Not applicable: nothing enters the model context; MCP tool declarations are unchanged |
| §4 rule 7 (public events and services only) | The MCP row uses Settings' public service and reads the entries PluginManager registered; it reads no other part's internal state |
| §4 rule 8 (infrastructure unchanged) | No change to the Timeline, Dispatcher, event envelope, assembly, Session or branch facades; no requirement is raised |
| §4 rule 9 (only facts later decisions use) | Configuration stays out of the Timeline; a Session broadcast notice is not added |
| §1 (Capability and the App Server boundary) | No new part and no new Capability. The MCP process row and its Session part keep their state; the change is in the process row's management methods |
| ADR 002 D1 (scope follows the owner of state) | Entry configuration is shared across Sessions, so it stays in the process owner |
| ADR 002 D2 (no wiring modules) | No new plugin, port or adapter; no method is added to the namespace |
| ADR 002 D3 (dependencies point to the core) | No dependency changes; Settings is already declared |
| ADR 002 D4 (structural versus business configuration) | Business configuration stays in its source, read on use; editing reloads nothing and changes no loader row |
| ADR 002 D5 (language-neutral contracts) | The request, reply and error shapes are JSON data; the revision is an opaque string |
| ADR 003 D3 (project entries start disabled) | Unchanged: a project entry's enablement is the user's record, and this design reads it back instead of writing an ignored field |

## 12. Alternatives considered

| Alternative | Why not |
| --- | --- |
| Add `mcp.get` beside `mcp.configuration` | Two reads of one entry; `mcp.configuration` already is the read. The Desktop client maps its own name onto it |
| Include each entry's configuration in `mcp.catalog` | The list would carry every entry's redacted configuration and cannot carry a per-entry token that means anything: the revision belongs to the source, and the list is not the entry snapshot a client writes with. It would also make every list render the widest possible projection |
| Return secrets to local clients (`settings.read` precedent) | A stored token would then travel to every local UI, be cached and be written back by an editor. Each owner protects its own secrets ([Settings service §1.2](settings-service.md#12-decision-secrets-belong-to-their-owner)); the read withholds them and the patch preserves them |
| Return `args` values | A stdio command line may carry a token in an argument, and the redaction flow test treats argument values as secret. A replacement remains possible, with the count shown |
| Return the new revision from the writes (`provider.update` precedent) | A client reads the entry back after a write anyway, because the server normalizes what it stores; the four writes keep one reply shape, and `mcp.remove` has no entry to return |
| Keep the read and the patch on the effective configuration | Then a client cannot echo a value it read: a project entry's resolved `cwd` and the user record's `enabled` would be persisted into `.mcp.json` by an edit of another field |
| A server-side `mcp.duplicate` | It would carry secrets between entries without deciding whether the credential record is shared or copied; the copy's enablement and source would need their own decision |
