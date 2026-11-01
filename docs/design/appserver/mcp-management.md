# MCP configuration management

Status: Draft.

Scope: process.
Issues: [#453](https://github.com/arcships/dim/issues/453), [#738](https://github.com/arcships/dim/issues/738).
Rules: [design rules](../../architecture/rust-agent/principles.md), [ADR 002](../../adr/002-plugin-scope.md).

## 1. Scope

This document owns the configuration surface of the `mcp` management namespace: how a client
reads one entry in order to edit it and writes the result. It owns those methods' requests and
replies, the values they redact, and how a source revision orders an edit. It also owns the
user↔project migration (`mcp.move`) and the user-local secret overlay that migration uses.

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
  A project entry moved from user scope also has a **user-local secret overlay** (`mcp-secrets`);
  its declared configuration is the project file's public skeleton merged with that overlay. The
  overlay is not a second entry and is never returned to a client.
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

Two properties of that surface prevent a client from editing an entry:

1. **The read carries no revision.** `mcp.save`, `mcp.update`, `mcp.remove` and `mcp.set_enabled`
   all require the target source's revision as `expectedVersion`, and the read does not return
   it. A client pairs the read with a separate `mcp.catalog` call, so the entry it renders and
   the token it writes with come from two snapshots taken at different times.
2. **The read and the patch are not the same configuration.** Both go through the effective
   entry, so a value the read shows is not a value the client may write back. For a project stdio
   entry that declares no `cwd`, the read shows the resolved workspace path, and a patch that
   leaves `cwd` alone persists that absolute path into `.mcp.json`. For a project entry the patch
   also persists the user record's enablement as the file's `enabled` field, which the same
   design ignores when it runs the entry.

Nothing moves an entry between sources. The client-side workaround is `mcp.save` on the
destination and `mcp.remove` on the source, built from the redacted read: it loses every withheld
value, or writes a token into `.mcp.json`, does not check the destination id or revision, and
does not handle enablement. `mcp.move` is the owner operation; D8–D14 and §4.5 state its
contract.

## 3. Decisions

### D1. One edit read, with the source revision and the enablement

`mcp.configuration` is the read an edit is based on: the reply carries the source revision, the
enablement the entry runs with, and the configuration projection, all taken from one source
snapshot. A client never combines a projection from one read with a revision from another; this
is the rule [Provider management](provider-management.md#31-read-models) states for the sole
edit-view read.

Creation has no entry to read yet, so `mcp.save` takes its token from the source revisions
`mcp.catalog` returns: `userRevision` for a user entry, `projectRevision` for a project one. That
is those two fields' only remaining use; a client editing an existing entry takes the token from
the entry read.

`mcp.catalog` stays the list read: no entry configuration and no per-entry revision, because a
list of entries and a token for writing one answer different questions. `mcp.inspect` stays the
run view: it reports the effective configuration with the entry's connection generations, and a
client must not write its values back.

### D2. The management surface speaks the declared configuration

Every configuration a client reads for editing, patches, or submits in a draft is the declared
entry. The daemon applies its own resolution when it runs an entry or inspects a running one.
Echoing a read value back therefore changes nothing, and no host path enters a source through an
edit: a project entry's `cwd` is returned as the file declares it (`null` when the file declares
none), and a patch that does not mention `cwd` leaves the file's `cwd` as it was.

The one field whose value is not the source's is `enabled`, and it is returned beside the
configuration, never inside it: a user or plugin entry is enabled unless its own `enabled` field
is `false`, and a project entry is enabled exactly when the user's local record names it, since
project files cannot set it (ADR 003 D3). It is written only by `mcp.set_enabled`. Enablement
and credentials therefore change without a configuration write (`mcp.set_enabled`, `mcp.auth.*`);
a client never rewrites an entry to change either.

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

### D8. The move is between user and project sources

`mcp.move` moves one entry by id from `user` to `project` or the reverse. The request names the
source and destination; `cwd` is required for the project side and authorized as today. Plugin
entries are not movable (`invalid_params`). The move never changes another source's entry, and
source precedence (project over plugin over user) is unchanged: if a higher-precedence source
holds the same id, the moved entry is shadowed, and the client states that before the move. A
destination **source** that already holds the id is a collision, not a shadow (`mcp_move_conflict`).

### D9. The move is copy-before-delete and not atomic

Settings gives one revision and compare-and-set per source document
([Settings service](settings-service.md#33-persistence-and-revisions)); there is no cross-source
transaction. The move commits the destination side first and removes the source last, and
reports success only after the source is removed. A failure between those writes is
`mcp_move_partial`, not success: the committed state stays, and the client rereads both sources
and follows the direction's recovery in §4.5. §4.5 also fixes the step order and the partial
states.

### D10. Enablement belongs to the destination's owner

Into project, the destination starts disabled. The move clears the user's `mcp-projects` record
for the workspace and id before writing the project entry, so a stale enablement cannot grant
execution, and the source's `enabled` value is not carried into `.mcp.json`. Into user, the
destination user entry's `enabled` field gets the project entry's current effective enablement,
and the project record is cleared. The move never enables a project entry; only the user's
`mcp.set_enabled` can.

### D11. Secrets stay in user scope

`.mcp.json` never receives a value the management read withholds. The move splits the source
entry: `.mcp.json` gets the public skeleton (type, command, declared `cwd`, URL origin and the
non-secret OAuth fields), and the user-local `mcp-secrets` source gets the withheld part (args,
environment and header values, the bearer token, the full URL when it carries userinfo, path,
query or fragment, hidden OAuth fields and unknown extension fields), keyed by canonical
workspace and id. The declared configuration merges the overlay before the effective resolution;
the overlay is never returned to a client and is a user-scope document under Settings' own
storage and access rules. A moved project entry is a Dim-managed skeleton, not a complete
standalone configuration for another tool; the user keeps the entry in user scope if that
portability is needed.

### D12. Secret fields of a moved project entry are edited in user scope

While an entry has an overlay, `mcp.configuration` reports `secretsInUserScope: true`.
`mcp.update` refuses a patch that sets `args`, `env`, `headers`, `bearer`, `url` when the overlay
holds it, or the hidden OAuth fields with `mcp_credentials_are_user_scoped`; `mcp.save` on the
entry is refused entirely, because a complete configuration would have to omit or rewrite what
the overlay holds. A non-secret patch is applied to the merged entry but written back as the
public skeleton only; the overlay-held fields stay in `mcp-secrets`. Non-secret fields remain
patchable; moving the entry back to user scope restores full editing. The read still shows the
merged names, counts and `authConfigured`/`oauthConfigured`.

### D13. Overlay life cycle follows the project entry

A move into project writes the project skeleton first and the overlay second, then removes the
source. A move back to user writes the user entry first, clears the project enablement, deletes
the overlay and removes the project skeleton last. `mcp.remove` of a project entry deletes its
overlay before removing the skeleton. The normal order therefore leaves a disabled destination
without an overlay, never an overlay without a destination entry; a leftover overlay record is
inert and the next move into project replaces it. `mcp.update` of an existing project entry
preserves its overlay; `mcp.save` is refused (D12), and `mcp.remove` deletes the overlay.

### D14. OAuth identity is unchanged

The move never changes the server id or the effective URL, and it never reads, writes or deletes
the MCP OAuth credential record (`mcp-oauth.json`). That record is keyed by id and canonical URL
independent of the source ([MCP OAuth §4.1](mcp-oauth.md#41-credential-record-persisted)), so an
existing grant and client identity continue to apply. OAuth tokens stay in that record;
`.mcp.json` and `mcp-secrets` never hold them.

## 4. Contract

Namespace `mcp`, process scope, served on local connections
([Management methods § Dispatch and lifecycle](management-methods.md#dispatch-and-lifecycle)).
`source` is `"user"`, `"project"` or `"plugin"`; `cwd` is required for a project source and is
the workspace whose `.mcp.json` is read, authorized as today.

These are the methods that read or write one entry's configuration:

| Method | Request | Reply | Mutating |
| --- | --- | --- | --- |
| `mcp.configuration` | `{cwd?, id, source}` | `{revision?, enabled, secretsInUserScope, configuration: Configuration}` | no |
| `mcp.save` | `{cwd?, id, source, expectedVersion, configuration}` | `{}` | yes |
| `mcp.update` | `{cwd?, id, source, expectedVersion, patch}` | `{}` | yes |
| `mcp.move` | `{cwd?, id, from, to, expectedFromVersion, expectedToVersion}` | `{}` | yes |

`mcp.save` takes a complete stdio, HTTP or SSE definition in the existing `mcpServers` format and
creates or replaces that source's entry, not a merged one. The namespace's other methods are
listed in §6 and keep their references: `mcp.catalog`, `mcp.inspect`, `mcp.test`, `mcp.remove`,
`mcp.set_enabled` and `mcp.disconnect` in the App Server's
[MCP configuration reference](../../../crates/dim-app-server/README.md#mcp-configuration-management),
`mcp.auth.*` in [MCP OAuth §9.1](mcp-oauth.md#91-management-methods-namespace-mcp-local-transports-only),
and `mcp.temporary.capabilities` in [TemporaryMcp](../plugins/TemporaryMcp.md). §3 D5 states the
revision every write takes. `mcp.test` probes a draft configuration; [#452](https://github.com/arcships/dim/issues/452)
extends its reply, so this document does not state that reply and is the document that will state
it.

### 4.1 Entry read (`mcp.configuration`)

| Field | Type | Meaning |
| --- | --- | --- |
| `revision` | string, absent for a plugin entry | The source document's revision, the token `expectedVersion` takes (D5) |
| `enabled` | boolean | The enablement the entry runs with: for a user or plugin entry its own field, enabled when absent; for a project entry the user's record (D2) |
| `secretsInUserScope` | boolean | True when the project entry has an `mcp-secrets` overlay; its withheld fields are edited in user scope (D11, D12) |
| `configuration` | `Configuration` | The declared configuration projection below; `enabled` is reported above and is not part of it |

```text
client                                  Mcp (process)                     Settings
  | mcp.configuration {source,id,cwd?}     |                                 |
  |--------------------------------------->| read the entry's source         |
  |                                        |-------------------------------->|
  |                                        |<-- {revision, document} --------|
  |                                        | project entry: enablement record|
  |<-- {revision, enabled, secretsInUserScope, configuration} -|             |
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
| `cwd` | string or `null` | The declared working directory; a project or plugin entry's relative path, not a resolved one, and `null` when the source declares none |
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

A project entry with `secretsInUserScope: true` (D11, D12) refuses a patch that sets `args`,
`env`, `headers`, `bearer`, `url` when the overlay holds it, or the hidden OAuth fields
(`authServerMetadataUrl`, unknown `oauth` extension fields) with
`mcp_credentials_are_user_scoped`; `mcp.save` on that entry is refused entirely. The message
says to move the entry back to user scope. `command`, `cwd`, `oauth.clientId`,
`oauth.callbackPort` and `oauth.scopes` remain editable. A non-secret patch is applied to the
merged entry and written back to `.mcp.json` as the public skeleton only; the overlay-held fields
stay in `mcp-secrets` and are never copied into the file.

### 4.4 Errors

| `appError` | When |
| --- | --- |
| `mcp_unknown_server` | The target source holds no entry with that id, for a read or a write; a move whose source lacks the id |
| `mcp_invalid_config` | The patched or saved document, an empty `id`, or an unauthorized workspace is refused |
| `mcp_settings_conflict` | `expectedVersion` is not the source's current revision (D5), or an overlay/enablement compare-and-set lost |
| `mcp_settings_read` | The source could not be read |
| `mcp_move_conflict` | The destination source already holds an entry with that id; nothing is written |
| `mcp_move_partial` | A destination-side write committed and the source removal did not; the client rereads and follows the direction's recovery: into project remove the destination and retry, into user remove the project source (D9, §4.5) |
| `mcp_credentials_are_user_scoped` | A patch on a moved project entry sets a field held in `mcp-secrets`, or a save targets that entry; move the entry back to user scope (D12) |
| `invalid_params` (`-32602`) | A field of the wrong transport, a project target without `cwd`, a plugin source, the same `from` and `to`, or a write to a plugin entry, which answers `plugin MCP entries change through their plugin` |
| `local_transport_required` | The connection is not local |
| `service_reloading` / `service_unavailable` | Mcp or its row is not serving; from [management methods](management-methods.md#dispatch-and-lifecycle) |

A read serves a disabled entry: enablement is a field, not a condition for reading.

### 4.5 Move (`mcp.move`)

`mcp.move` moves one entry by id between the user and project scopes. It is the only operation
that reads the complete entry, including withheld values, and writes another source; no value it
moves is returned to the client.

| Field | Type | Meaning |
| --- | --- | --- |
| `cwd` | string, required for the project side | Workspace whose `.mcp.json` is read or written; authorized as today |
| `id` | string | Entry id; non-empty |
| `from`, `to` | `"user"` or `"project"` | Source and destination; must differ |
| `expectedFromVersion` | string | Source document revision |
| `expectedToVersion` | string | Destination document revision |

The owner reads both sources and the user-local `mcp-secrets` and `mcp-projects` records, checks
both revisions, and refuses before writing when the source lacks the id
(`mcp_unknown_server`), the destination already holds it (`mcp_move_conflict`), or the full entry
is invalid (`mcp_invalid_config`). Plugin entries are not movable, and the same `from` and `to`
is `invalid_params`. The destination source revision is `missing` when that source has no
document, as `mcp.catalog` reports it.

The complete entry (the overlay merged in, for a project source) is validated as one
`mcpServers` entry before any write. The move then writes in this order:

Into project:

1. clear `mcp-projects` for the workspace and id (CAS);
2. write the public skeleton to `.mcp.json` (CAS on `expectedToVersion`);
3. write the withheld part to `mcp-secrets` (CAS);
4. remove the entry from `mcp.json` (CAS on `expectedFromVersion`).

Into user:

1. write the complete entry, including the overlay, to `mcp.json` with `enabled` set to the
   project entry's current effective enablement (CAS on `expectedToVersion`);
2. clear `mcp-projects` for the workspace and id (CAS);
3. delete the overlay (CAS);
4. remove the entry from `.mcp.json` (CAS on `expectedFromVersion`).

The order is the recovery contract. Into project, the entry is disabled and its skeleton exists
before the overlay, so a failure leaves a disabled entry without secrets, never an enabled entry
without them. Into user, the complete user entry exists before the project side is withdrawn, so
a failure leaves both entries, with the project entry disabled and still effective; the user
entry is complete and becomes effective when the project entry is removed. A move reports `{}`
only after the source entry is gone. A failure before the enablement clear answers the ordinary
error and changes nothing; a failure after only the enablement clear leaves no destination entry
and can be retried. A failure after a destination-side write answers `mcp_move_partial`; the
client rereads both sources and recovers by direction:

| Direction | Safe recovery |
| --- | --- |
| into project | remove the destination with `mcp.remove` and retry the move; the source still holds the complete entry. Never remove the source while the destination may be incomplete |
| into user | remove the project source with `mcp.remove`; the user entry is complete, and removing the project source also deletes the overlay and skeleton |

The into-project recovery also covers the case where the overlay committed: removing the complete
destination and retrying is safe and avoids guessing whether the overlay is present. If no
destination entry exists, the client retries the move; the enablement clear alone is harmless.
The into-user recovery is safe at every phase after the user entry commit.

The source and destination revisions are checked once before the first write, and every write is
CAS. A concurrent write to either source, the overlay or the enablement record makes that write
fail (`mcp_settings_conflict`, or `mcp_move_partial` if a destination write already committed).
The move holds Mcp's operation gate, so calls through this daemon do not interleave; another
daemon on the same home is ordered only by the per-document CAS.

The overlay is a user-scope Settings document `mcp-secrets` (`mcp-secrets.json`), keyed by
canonical workspace and id. Its record is the withheld part of one project entry:

```json
{
  "entries": {
    "/canonical/workspace": {
      "github": {
        "args": ["--stdio"],
        "env": {"TOKEN": "…"},
        "headers": {"Authorization": "…"},
        "auth": {"type": "bearer", "token": "…"},
        "url": "https://user:pass@host/mcp?key=…",
        "oauth": {"authServerMetadataUrl": "…", "extra": {}},
        "extra": {}
      }
    }
  }
}
```

Only the members that exist on the source entry are present. When the withheld part is empty,
the move writes no overlay record and `secretsInUserScope` stays false. The project `.mcp.json`
skeleton contains the safe members: `type`, `command`, declared `cwd`, `url` reduced to
`scheme://host[:port]` when the full URL carries userinfo, path, query or fragment, and
`oauth.clientId`/`callbackPort`/`scopes`. The owner merges the overlay into the declared
configuration before `config::effective`; the merge replaces `args` and `url`, merges `env`,
`headers`, `auth`, `extra` and the hidden `oauth` members, and is validated as a whole. A record
for a workspace and id with no project entry is inert; the move into project may replace it when
the destination has no entry, because it can only be the same entry's leftover. The move never
creates an overlay without a destination entry.

The overlay is never read by a client method. `mcp.configuration`, `mcp.catalog`, `mcp.inspect`,
`mcp.test` and every error show only the redacted projection (D3); `secretsInUserScope` is the
only new fact. It is a user-scope Settings document under the storage and access rules of that
service.

`mcp.remove` of a project entry that has an overlay withdraws enablement, deletes the overlay,
then removes the project entry. A failure leaves the entry disabled and the overlay present or
absent, never an enabled entry without its secrets; the caller retries. `mcp.save` on a project
entry with an overlay is refused with `mcp_credentials_are_user_scoped`; use `mcp.update` for its
non-secret fields or move it back to user scope. Moving back to user deletes the overlay in the
same operation.

## 5. What changes for a client

- The reply of `mcp.configuration` stops being the bare projection and becomes
  `{revision?, enabled, secretsInUserScope, configuration}`. That is a breaking change on
  protocol v3 for any client that decodes the old reply: the projection is now under
  `configuration` and the new boolean is additive. This repository's Web settings page changes
  in the same implementation; the native CLI decodes `mcp.catalog` only and is unaffected; every
  other client sees the projection replaced by the wrapper.
- An empty `id` on the entry read answers `mcp_invalid_config`, the code `mcp.save` already
  returns, instead of `invalid_params`.
- The edit flow is `mcp.configuration` then `mcp.update` with the returned `revision`; creating an
  entry takes the token from `mcp.catalog`'s source revision (D1).
- A client renders `command`, `cwd` and `endpoint`, the name lists and the counts, and offers
  replace-or-keep for `args`, environment values, header values, the bearer token, the URL and
  the metadata URL. It shows `argsCount`, `envNames` and `headerNames` as the fields that a
  replacement would replace.
- When `secretsInUserScope` is true, the client shows that the withheld fields are stored in the
  user's local configuration and disables their editors; the owner refuses those patch fields.
  Moving the entry back to user scope restores full editing.
- The move flow takes both revisions from `mcp.catalog`, asks for confirmation, and calls
  `mcp.move`. The client explains that into project the entry starts disabled and its withheld
  values stay in the user's local configuration; into user the entry keeps its current enabled
  state. On `mcp_move_conflict` or `mcp_settings_conflict` it keeps the form and rereads; on
  `mcp_move_partial` it follows the direction's recovery in §4.5: into project remove the
  destination and retry, into user remove the project source; it never removes the source while
  the destination may be incomplete. On success it reloads the catalog and opens the destination
  entry. The client never displays a moved withheld value.
- A client must not write back `mcp.inspect` values (D1).
- The Desktop client (arcships/dim-electron#20) replaces its `mcp.get` and `mcp.update`
  `capability_unavailable` answers with `mcp.configuration` and `mcp.update`.

## 6. What does not change

- `mcp.catalog`, `mcp.inspect`, `mcp.test`, `mcp.set_enabled` and `mcp.disconnect` keep their
  requests and replies, including `mcp.inspect`'s effective configuration and its connection
  generations. `mcp.configuration` gains the `secretsInUserScope` field (§4.1); `mcp.update`,
  `mcp.save` and `mcp.remove` keep their requests and replies, but a moved project entry's
  withheld fields are refused or cleaned as §4.5 states.
- The namespace gains one method, `mcp.move`; `hello.features` advertises it and a client checks
  support before offering the control.
- Three error codes are added: `mcp_move_conflict`, `mcp_move_partial` and
  `mcp_credentials_are_user_scoped`. Existing codes keep their meanings.
- The `mcp`, `mcp-projects` and `mcp-plugins` documents, their file names, source precedence and
  the `mcp-oauth.json` credential record keep their formats. A new user-scope `mcp-secrets`
  source holds the migration overlay; it has no prior format and a missing file reads as empty.
  No existing persisted data moves or changes meaning.
- No new plugin, part, service, port, Timeline event or notification. Assembly, routing, the
  MCP Session part and its connections are untouched; the MCP row registers the new Settings
  source itself.

## 7. Events and state

None. Entry configuration is process configuration changed through its owner's typed writes, as
[design rules §4 rule 9](../../architecture/rust-agent/principles.md) requires; it never enters
the Timeline. Revisions come from the stored document, not from an event. Mcp registers the
user-scope `mcp-secrets` source and follows its changes like its other sources; the overlay is
business configuration, not an event.

## 8. Dependencies

| Dependency | Use |
| --- | --- |
| `managementMethods` ([management methods](management-methods.md)) | The MCP row registers the namespace and its methods during `apply` |
| `settings` ([Settings service](settings-service.md#42-service-settings-owner-path)) | Reading a source with its revision, compare-and-set writes, project authorization; the `mcp`, `mcp-projects` and `mcp-secrets` sources |
| PluginManager ([§3.7](../plugins/PluginManager.md#37-content-registration)) | A plugin entry is registered content; Mcp reads it as declared and writes nothing |
| `McpAuth` ([MCP OAuth](mcp-oauth.md)) | The `mcp.auth.*` methods of the same namespace; the credential record is untouched by a move |

## 9. Cross-module requirements

None raised to the runtime, rutis, Settings or Rpc: the revision and the source snapshot already
exist, and the change stays inside the MCP owner and its documents and reply shapes.

Client-side follow-ups, which are not requirements on another owner: the Web settings page (this
repository) adds the move control and admits exactly `mcp.move` for it, as
[Web workspace §4.3](../web-workspace.md#43-settings-and-account-management-on-the-local-web-entry)
states; the Desktop client (arcships/dim-electron#20) adopts §5.

## 10. Tests

Flow level, reusing the existing MCP flows:

| File | What it protects |
| --- | --- |
| `crates/dim-app-server/tests/transport_flow/product_protocol.rs` | The wire contract on local v3: the read returns the revision the catalog reports for the same source; creating an entry with the catalog's source revision succeeds and a stale revision is `mcp_settings_conflict`; a plugin entry's read has no revision and a write to it is `invalid_params`; a project entry patched on one unrelated field leaves `.mcp.json` without a resolved `cwd` and without an `enabled` key; no withheld value appears in the read, including after an edit; `mcp.move` checks both revisions, refuses a destination id and admits no secret into its reply. The Web entry admits `mcp.move` and still refuses an unauthenticated or bare connection |
| `crates/dim-app-server/tests/mcp_flow.rs` | The service level: a source's read returns its declared entry with the source revision, and a project entry's enablement comes from the user's record, while the effective entry a Session runs resolves the workspace `cwd`; a user entry's read carries its declared configuration and revision; a move in each direction preserves args, env, header, bearer, URL and unknown fields and leaves `.mcp.json` without a withheld value; into project the destination starts disabled and into user it keeps its effective enablement; a destination id is `mcp_move_conflict`; a moved project entry refuses withheld patches with `mcp_credentials_are_user_scoped`; `mcp.remove` cleans the overlay; a partial move after an injected write failure and a restart leaves the documented safe state and is finished by the direction's recovery |
| `crates/dim-app-server/tests/mcp_oauth_flow.rs` | An OAuth entry moved in each direction keeps its `mcp-oauth.json` record and sign-in state; no token appears in `.mcp.json` or `mcp-secrets`; `mcp.inspect` and `mcp.update` keep their replies for the moved entry |

The redaction assertions of
`mcp_editor_preserves_redacts_and_removes_credentials_with_source_cas` extend to the new reply
shape rather than moving to a new test: the same fixture writes argument, environment, header,
bearer and URL secrets and reads the entry back. The move tests assert the same fixture's
secrets never appear in the reply, `.mcp.json`, a Web page response or a log.

The failure test uses a disposable home and fails one source write at the documented step (an
unwritable target or a test Settings wrapper), then restarts the App Server on the same home. It
asserts the partial state, the `mcp_move_partial` answer, and the direction's documented recovery
without credential loss or duplicate execution.

## 11. Rule check

| Rule | Answer |
| --- | --- |
| §4 rule 1 (commands for another owner's state) | No event. Entry configuration is process configuration written through the MCP owner's typed methods; the MCP owner is the writer of its own state |
| §4 rule 2 (handover) | Not applicable: no object is handed between modules |
| §4 rule 3 (`before` callbacks) | None |
| §4 rule 4 (core without business meaning) | No core change; Rpc dispatches the namespace as it does today |
| §4 rule 5 (batch) | Not applicable: no events are written |
| §4 rule 6 (model context) | Not applicable: nothing enters the model context; MCP tool declarations are unchanged |
| §4 rule 7 (public events and services only) | The MCP row uses Settings' public service and reads the entries PluginManager registered; it reads no other part's internal state. The `mcp-secrets` overlay is MCP's own user-scope document |
| §4 rule 8 (infrastructure unchanged) | No change to the Timeline, Dispatcher, event envelope, assembly, Session or branch facades; no requirement is raised |
| §4 rule 9 (only facts later decisions use) | Configuration stays out of the Timeline; a Session broadcast notice is not added |
| §1 (Capability and the App Server boundary) | No new part and no new Capability. The MCP process row and its Session part keep their state; the change is in the process row's management methods |
| ADR 002 D1 (scope follows the owner of state) | Entry configuration is shared across Sessions, so it stays in the process owner |
| ADR 002 D2 (no wiring modules) | No new plugin, port or adapter; `mcp.move` is one method on the existing MCP row |
| ADR 002 D3 (dependencies point to the core) | No dependency changes; Settings is already declared |
| ADR 002 D4 (structural versus business configuration) | Business configuration stays in its source, read on use; the `mcp-secrets` overlay is user-scope business configuration and reloads nothing |
| ADR 002 D5 (language-neutral contracts) | The request, reply and error shapes are JSON data; the revision is an opaque string |
| ADR 003 D3 (project entries start disabled) | A move into project clears the user's enablement record before the project entry exists and never writes `enabled` into `.mcp.json`; project content cannot enable itself. A move into user preserves only the user's existing enablement choice |

Scope migration adds no new rule answer: it writes MCP-owned Settings documents through the
owner's typed writes, uses the public Settings and OAuth contracts, and starts a project
destination disabled. It is explicitly not a cross-source transaction; §4.5 is the recovery
contract.

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
| Client-side save plus remove for a move | The redacted read loses every withheld value, and a complete save would have to carry them; the destination id, both revisions and enablement would be handled by the client, not the owner |
| Write the complete entry into `.mcp.json` | It writes env/header values, bearer tokens, URL credentials and argument values into a workspace file that is commonly committed. D11 keeps them in the user-local overlay |
| Refuse moves of an entry with withheld values | It makes the required both-direction move impossible for ordinary entries; the overlay preserves them without a readback |
| A cross-source transaction in Settings | Settings owns per-document revisions and CAS; a cross-document transaction is a new mechanism for one operation. The copy-before-delete order and `mcp_move_partial` give explicit recovery instead (D9) |
| Merge a project entry with the lower-precedence user entry that has the same id | It changes the source precedence and makes a hidden user entry affect a project entry implicitly. The explicit `mcp-secrets` overlay has one meaning and one writer |
| Reuse `mcp-oauth.json` for static secrets | That record is keyed by id and URL, not workspace, and its lifecycle is OAuth sign-in and refresh. Static env/header/bearer values have a different key and lifetime; a separate source is smaller than changing the OAuth format |
| Route a project entry's secret patch into `mcp-secrets` | It would make every edit of a moved entry a two-document, two-revision write, and the migration does not need it. Moving the entry back to user scope is the supported edit path (D12) |
