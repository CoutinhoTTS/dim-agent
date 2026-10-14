# PluginManager

Status: Draft. Overall design: [overview B.12](../overview.md#b12-pluginmanager), [Settings, Mcp, SkillsCatalog changes](../overview.md#c2-app-server-process-plugins), [conventions](../overview.md#64-conventions), [multiple nodes](../overview.md#63-multiple-nodes), [ADR 002](../../adr/002-plugin-scope.md). Phase: P3 (depends on [#3](https://github.com/arcships/dim/issues/3); non-Rust capability plugins on [#4](https://github.com/arcships/dim/issues/4); peer nodes on [#5](https://github.com/arcships/dim/issues/5)). Language: Rust.

PluginManager installs plugin packages from Git marketplaces, local paths or Git URLs onto a target node, records plugin intent (install, enable, node binding, package paths) through typed Settings operations into loader rows, routes each plugin's business configuration to that plugin's own business source, and publishes the typed content of enabled packages through `PluginContents`. Content owners (Skills, Hooks, MCP, workflows) define their own content type keys and discover the content they understand; PluginManager knows none of them. Package code runs only after approval through the Settings host-local approval service bound to node and package fingerprint. Packages are never upgraded automatically.

## 1. Structure

```mermaid
flowchart LR
  subgraph CN["App Server process, control node"]
    PM["PluginManagerPlugin<br/>plugin.* methods, package store"]:::new
    PC["PluginContents<br/>stable port, no dependencies"]:::new
    ROW["PluginPackage row<br/>one per installed package"]:::new
    SET["Settings<br/>typed operations, plugin business source,<br/>host-local approvals, credential resolver"]
    LOADER["rutis Loader"]
    MR["Method port<br/>level management"]:::contract
    NS["Node-services port<br/>lookup by node id"]:::contract
    CONS["Content owners<br/>SkillsCatalog, Mcp, Hooks, Workflow"]
  end
  subgraph NODE["Each node, local or peer"]
    SVC["FileStorage, HostPlatform<br/>of that node"]
    CODE["Package code rows<br/>gated by PluginApproval"]:::new
  end
  PM --> SET
  PM --> LOADER
  PM -->|"registers plugin.*"| MR
  PM -->|"per call"| NS
  ROW -->|"verify per load"| NS
  NS --> SVC
  ROW -->|"register sources"| PC
  ROW -->|"PluginApproval when approved"| CODE
  CONS -->|"read and watch sources by own type key"| PC
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
  classDef contract stroke-dasharray:4 3
```

`PluginContents` is a stable port (ADR 002 D2) hosted in its own plugin with no dependency on PluginManager or package rows: package rows register into it, content owners read and watch it, so a PluginManager reload or a package row change never reloads a content owner. Node services are resolved per call by node id through the node-services port; an offline node fails only operations on that node with `node_unavailable`. Package code rows that contribute tools, methods or other extensions register into each consumer's port like any contributor; when one unloads, its registrations disappear and only its own in-flight calls fail.

| Part | Scope | Depends on | Provides |
| --- | --- | --- | --- |
| `PluginManagerPlugin` | Process, control node | `SettingsService` (typed operations, plugin business source, host-local approvals), `Arc<Loader>`, node-services port, method port | `plugin.*` methods (level management) |
| `PluginContentsPlugin` | Process, control node | — | `PluginContents` (stable port; empty without PluginManager) |
| `PluginPackage` row | Process, one per installed package | `PluginContents`, node-services port, host-local approval service | Content sources; `PluginApproval` keyed `<node>/<id>` |
| Package code rows | Process, on the package's node | what the code declares, plus `PluginApproval` | what the code declares; reads its business configuration at use time |
| Content owners | Process | `PluginContents` | — (each defines its own type key) |

Contracts defined here: package manifest `dim-plugin.json` schema 2, marketplace index `dim-marketplace.json`, the `PluginContents` publication contract (`ContentSource`), the package directory layout, the `PluginApproval` gate key, `plugin.*` methods. Contracts depended on: typed Settings operations on the loader editable layer (#3) for plugin intent, the Settings business source for per-plugin configuration (ADR 002 D5), the Settings host-local approval service (R11) and credential-reference resolver (R10), process storage on the control node, the rutis-loader `Loader` (`entries`, `reload`, `restart`, `LoaderChanged`), `NodeServices` (`FileStorage`, `HostPlatform`) of the package's node through the node-services port, the [method contribution](../contracts/session-boundary.md#a1-method-port) contract. PluginManager never calls a consumer and never names a content type key in code.

## 2. Data and state

```mermaid
flowchart TB
  subgraph LEL["Loader editable layer, via Settings"]
    MK["Marketplace registrations"]
    PR["Package rows plugin id per node<br/>version, source, commit, fingerprint,<br/>contents, enabled"]
  end
  BC["Plugin business configuration<br/>Settings business source, per node and id"]
  subgraph HLA["Settings host-local approval service"]
    AP["owner plugin, node, source, id, fingerprint"]
  end
  subgraph PS["Package root on each node, via its FileStorage"]
    PKG["Package directories<br/>content-addressed by fingerprint"]
    STG["Staging"]
  end
  CACHE["Marketplace clones and indexes<br/>control node process storage"]
  MEM["PluginContents sources and revision<br/>memory"]:::new
  TL["Timeline"]
  PR -->|"row apply verifies"| PKG
  PR -->|"publishes"| MEM
  AP -->|"gates code rows"| PR
  BC -.->|"read at use time by code rows"| PR
  TL -.-|"never written"| MEM
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
```

The package row is the persisted intent; `plugin.list` is projected from loader entries, never from a separate store. Per-plugin business configuration is not in rows: `plugin.configure` writes it to the business source with a revision and a change notification, and the plugin reads it at use time, so configuring never reloads a row (A16). Published sources are rebuilt by row `apply` on every load; package bytes live under the package root of their node, marketplace caches in control-node process storage; approvals live only in the host-local approval service. Nothing is a Session fact.

## 3. Flows

### 3.1 Install and publish

```mermaid
sequenceDiagram
  participant C as Client
  participant PM as PluginManager
  participant PS as Package store on node
  participant S as Settings
  participant L as Loader
  participant R as PluginPackage row
  participant PC as PluginContents
  participant O as Content owner
  C->>PM: plugin.install source, node
  PM->>PM: lock node and id
  PM->>PS: stage fetch and verify digests
  PM->>PS: commit atomic rename
  PM->>S: plugins.install with expected revision
  S->>L: create package row and code rows
  L->>R: apply
  R->>PS: verify
  R->>PC: register sources, revision bump
  PM-->>C: id, fingerprint, approval required or notRequired
  PC-->>O: changed
  O->>PC: sources of its own type key
```

Content-only packages are active once their row applies; owners reconcile on `changed` and never receive pushes. Code rows stay Pending until approval.

### 3.2 Approve, revoke and code lifecycle

```mermaid
sequenceDiagram
  participant C as Client
  participant PM as PluginManager
  participant A as Host-local approval service
  participant R as PluginPackage row
  participant K as Code row
  participant TR as Consumer port e.g. tool source port
  C->>PM: plugin.approve id, approvalToken
  PM->>A: grant owner plugin, node, id, fingerprint
  R->>R: matching grant, provide PluginApproval
  K->>TR: apply registers contributions
  C->>PM: plugin.revoke
  PM->>A: revoke
  R->>K: withdraw PluginApproval
  K->>TR: disposer settles own calls, unregisters
  TR->>TR: revoke its sets, in-flight call gets tool provider unavailable
  PM-->>C: done after disposal
```

Revoking or disabling code that contributes tools affects only that contributor: the batch and turn continue, branches adopt the tool set without it (fixes the earlier "settled as a contributor reload" chain).

### 3.3 Explicit update, never automatic

```mermaid
sequenceDiagram
  participant C as Client
  participant PM as PluginManager
  participant PS as Package store
  participant S as Settings
  C->>PM: plugin.marketplace.update
  PM->>PS: refetch indexes only
  PM-->>C: commits, updateAvailable flags
  C->>PM: plugin.update id, expectedRevision
  PM->>PS: stage and commit new fingerprint directory
  PM->>S: plugins.update row version, fingerprint
  Note over PM,S: old approval no longer matches, code rows Pending until re-approved
```

A marketplace refresh only marks `updateAvailable`; changing an installed package always takes an explicit `plugin.update` (decision E22).

## 4. Timing and conflicts

```mermaid
sequenceDiagram
  participant PM as PluginManager
  participant PS as Package store
  participant S as Settings
  participant C as Client
  PM->>PS: commit
  alt interrupted before Settings write
    PM-->>C: operation_interrupted, no row, directory collected later
  else interrupted after Settings write
    PM-->>C: operation_interrupted, row exists, retry gives plugin_exists
  end
```

Install is ordered stage, commit, Settings write; the Settings revision is the commit point, so an interruption never leaves an approved or half-registered package.

| Scenario | Outcome | Owner |
| --- | --- | --- |
| Code row contributing tools revoked or disabled while a call runs | That call gets "tool provider unavailable"; batch and turn continue | [Tool source port](../contracts/process-tool-contribution.md), code row |
| Approval granted, then package updated | New fingerprint does not match; code rows Pending until re-approved | `PluginPackage` row |
| Node goes offline after install | Only that node's rows Pending; its sources withdrawn | rutis lifecycle |
| `plugin.configure` while the plugin runs | Business data update and notification; no row reload; next use reads the new value | Plugin |
| Two installs of one id | Second waits on the lock, then `plugin_exists` | PluginManager |
| Package disabled while a branch uses its Skill | Branch keeps its adopted prompt until it re-adopts | Skills owner |

Full table: [Appendix B](#appendix-b-full-conflict-table).

## 5. Context mapping

No direct model-visible effect. PluginManager writes no input, contributes no tool and registers no prompt source. Package content reaches the model only through its owner, in that owner's form: Skills through the Skill prompt source (adopted per branch, replayed after compaction as configuration, inherited by child branches from the adopted prefix), MCP and code-row tools through tool declarations and `ToolResult.content`, Hooks output through Session input. Non-interactive Sessions differ only as owners define. The model never sees package ids, fingerprints, approvals, marketplace data, configuration or diagnostics.

## 6. Interfaces (summary)

| Interface | Kind | Purpose |
| --- | --- | --- |
| `plugin.marketplace.list/add/remove/update/search/view` | Methods, level management | Marketplaces; `update` refreshes indexes only |
| `plugin.install/update/list/set_enabled/configure/reload/approve/revoke/remove` | Methods, level management | Package lifecycle |
| `PluginContents::sources(type_key)`, `changed()` | Stable port | Generic content publication |
| `PluginApproval` keyed `<node>/<id>` | Gate key | Code rows load only when approved |
| `dim-plugin.json` v2, `dim-marketplace.json` v1 | File formats | Package and marketplace descriptions |
| Typed Settings operations `plugins.*`, `marketplaces.*` | Depended on (#3, ADR 002 D5) | Persist intent; route business configuration |
| CLI | `/plugins`, `/plugin`, `/reload-plugins`, `dimcode plugin …` | User entries |

Details: [Appendix A](#appendix-a-interface-details).

## 7. Security

Installing, approving and removing are `level: management` methods, available only on entries that serve that level. Package code is trusted native-process code once loaded (AGENTS rule 11), so loading requires a host-local approval of the exact fingerprint on that node; replay, fork, FullAccess, project files and installation itself never grant it. Git fetch runs no repository hooks or submodules; package files are never executed at install time; integrity is rechecked on every load. Content owners apply their own host-local approval to executable content they interpret (Hooks, MCP, workflows). Plugin business configuration holds credential references only, resolved by the plugin through the Settings credential resolver; resolved secrets never enter rows, the business source, `plugin.list`, diagnostics or content sources. A package that contributes `level: management` methods extends the host administration surface only after approval.

## Appendix A. Interface details

### A.1 Package manifest `dim-plugin.json` (schema 2)

Version 1 is the existing managed-provider manifest ([App Server contract](../../../crates/dim-app-server/README.md#optional-managed-provider)); it stays for `--plugin-dir` and is not installable here (A3).

```json
{
  "schemaVersion": 2,
  "id": "acme-review",
  "version": "1.2.0",
  "name": "Acme review kit",
  "description": "Review Skills and hooks",
  "files": {"skills/review/SKILL.md": "<sha256>", "dist/index.js": "<sha256>"},
  "contents": [
    {"type": "skills", "path": "skills"},
    {"type": "hooks", "path": "hooks/hooks.json"}
  ],
  "code": [
    {"runtime": "node", "entry": "dist/index.js", "hostApiVersion": 2, "rutisVersion": "0.10.0"}
  ]
}
```

| Field | Rule (validated once at the install boundary) |
| --- | --- |
| `id` | `[a-z][a-z0-9-]{0,63}`; unique per node (A4) |
| `version` | SemVer; displayed and compared for `updateAvailable` only |
| `files` | Every file except `dim-plugin.json` and `.git/`; relative POSIX paths, no `..`, no symlinks; unlisted file or digest mismatch rejects |
| `contents` | Opaque `type` `[a-z][a-z0-9-]*` (key defined by its consumer: `skills`, `hooks`, `mcp`, `workflows`, R14), `path` inside the package, optional `options` ≤ 64 KiB; only this shape is validated |
| `code` | Optional; `runtime: "node"` loads `entry` as a rutis runtime row; `hostApiVersion`/`rutisVersion` checked; other runtimes rejected until #4 (A1) |
| Fingerprint | SHA-256 of the stored manifest bytes; the manifest lists every file digest, so it binds all package bytes. This is the canonical executable content PluginManager declares to the host-local approval service (R11) |

### A.2 Marketplace index `dim-marketplace.json` (A5)

At the repository root: `{schemaVersion: 1, name, plugins: [{id, version, description, keywords?, source: {path} | {git, ref?}, fingerprint?}]}`. `source.path` is inside the marketplace repository; `fingerprint`, when present, must equal the fetched package's fingerprint.

### A.3 Plugin content sources

```rust
/// Stable port in PluginContentsPlugin; key TypeKey::of::<PluginContents>().
impl PluginContents {
    /// Sources of enabled, integrity-verified packages whose node store is Active.
    pub fn sources(&self, type_key: &str) -> Vec<Arc<ContentSource>>;
    pub fn changed(&self) -> tokio::sync::watch::Receiver<u64>; // revision
    pub fn register(&self, ctx: &Ctx, source: ContentSource);  // used only by PluginPackage rows
}
pub struct ContentSource {
    pub package: String,
    pub version: String,
    pub fingerprint: String,             // consumers bind their own approvals to it
    pub node: Option<String>,            // None = control node
    pub root: PathBuf,                   // package directory on `node`
    pub path: PathBuf,                   // declared path, relative to root
    pub options: serde_json::Value,       // from the manifest
}
```

A consumer owns its type key and format, reads files through services valid for `node`, and binds its own host-local approval to `node + package + fingerprint` ([overview conventions](../overview.md#64-conventions)). Node rule (A6): a source on node N is content of node N. A consumer that executes content against a Session (for example Hooks) uses only sources whose node equals that Session's resource node (the `resource_node` field of `SessionConfig`); a consumer that only reads content on the control node ignores sources it cannot reach and reports them in its own catalog.

### A.4 Package store

Internal to PluginManager and `PluginPackage` rows, not a service: `stage` (fetch, verify), `commit` (atomic rename), `verify` (re-hash), `remove`, `fetch_marketplace`, each run per call against the `NodeServices` of the target node looked up by node id; sources are a local path or a Git URL with optional reference.

Packages live under PluginManager's package root on their node, written through that node's `FileStorage` (A13). Git runs through the node's `HostPlatform` with hooks and submodules disabled, `https://` and `ssh://` only (A7). Local paths are copied, never referenced (A8). Directories are content-addressed by fingerprint, so an update never rewrites files a running row uses.

### A.5 Methods (namespace `plugin`, level management)

| Method | Params | Result | Interruption |
| --- | --- | --- | --- |
| `plugin.marketplace.list` | `{}` | `[{name, url, ref, commit, fetchedAt, error?}]` | Drain |
| `plugin.marketplace.add` | `{name, url, ref?}` | `{commit}` | Report |
| `plugin.marketplace.remove` | `{name}` | `{}` | Report |
| `plugin.marketplace.update` | `{name?}` | `[{name, commit, error?}]`; refreshes indexes only | Report |
| `plugin.marketplace.search` | `{query, marketplace?}` | `[{marketplace, id, version, description, installed: [{node, version}]}]` | Drain |
| `plugin.marketplace.view` | `{marketplace, id}` | entry plus manifest summary (`contents` types, `code` presence) | Drain |
| `plugin.install` | `{source: {marketplace, id} \| {path} \| {git, ref?}, node?, enabled?}` | `{id, version, node, fingerprint, approval, approvalToken?}` | Report |
| `plugin.update` | `{id, node?, version?, expectedRevision}` | `{version, fingerprint, approval}` | Report |
| `plugin.list` | `{node?}` | `[{id, version, node, enabled, status, approval, approvalToken?, contents, source, updateAvailable, error?}]` | Drain |
| `plugin.set_enabled` | `{id, node?, enabled, expectedRevision}` | `{}` | Report |
| `plugin.configure` | `{id, node?, configuration, expectedRevision}`; `expectedRevision` of the plugin's business configuration | `{}` | Report |
| `plugin.reload` | `{id?, node?}` | `[{id, node, status, error?}]` | Report |
| `plugin.approve` | `{id, node?, approvalToken}` | `{}` | Report |
| `plugin.revoke` | `{id, node?}` | `{}` after code rows are disposed | Report |
| `plugin.remove` | `{id, node?}` | `{cleanupError?}` | Report |

`status`: `active`, `pending` (node offline or awaiting approval), `failed` (reason), `disabled`. `approval`: `notRequired`, `required`, `approved`; `approvalToken` is the fingerprint (a digest, not a credential). `install`, `update` and `marketplace.update` report phases (`fetch`, `verify`, `commit`) through the call's progress sink ([method contribution](../contracts/session-boundary.md#a2-call-channel), A14). Additions to overview B.12: `configure`, `approve`, `revoke`, `update`, `marketplace.search/view` (A9).

### A.6 Typed Settings operations used (#3, proposed Settings supplement)

| Operation | Effect |
| --- | --- |
| `plugins.install(entry, expected_revision)` | Creates package row and code rows on the editable layer |
| `plugins.update(id, node, version, fingerprint, expected_revision)` | Updates the package row; code rows follow |
| `plugins.set_enabled(id, node, enabled, expected_revision)` | `set_disabled` on the package row (code rows follow by dependency) |
| `plugins.configure(id, node, configuration, expected_revision)` | Writes the plugin's business configuration to the Settings business source keyed `<node>/<id>` and notifies; never edits rows (A16) |
| `plugins.remove(id, node, expected_revision)` | Removes all rows of the package |
| `marketplaces.save/remove` | Marketplace registrations |
| Host-local approval service `grant/revoke {owner: "plugin", node, source, id, fingerprint}` | Outside loader and Timeline (R11) |

`reload`, `restart` and status call the `Loader` directly.

### A.7 Capability mechanics

| Step | Path |
| --- | --- |
| Timeline | No writes, no Dispatcher subscription, no `StateCell`; rechecks are Settings revisions and approval-record matches |
| Install | Lock `(node, id)` (A10), `stage`, compare with intent, `commit`, `plugins.install`, rows created, reply; a failed Settings write leaves an unreferenced directory collected later (A11) |
| Row load | `PluginPackage::apply` looks up the node's services by node id, `verify`, registers sources, provides `PluginApproval` when a matching grant exists; its `ctx.effect` withdraws both |
| Code rows | Pending until approved; disposed on revoke or disable; Pending while the node is offline (`peer:<node>/…` rows); contributions go through consumer ports; business configuration read at use time |
| PluginManager reload | Only its methods leave the method port (`plugin.*` `unknown_method` until re-registered); `PluginContents`, rows, code rows and content owners are unaffected; in-flight install/remove return `operation_interrupted` |
| Process restart | Loader restores rows; rows re-verify; staging deleted; orphans collected after the loader settled |

## Appendix B. Full conflict table

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Turn or branch interrupt, branch close, concurrent inputs, `input/withdrawn`, unsettled batch, compaction, child cut | — | No effect on PluginManager; owners' adoption rules apply | Content owners, ToolBinding |
| Package disabled while a Session uses its Skill | revision bump < next adoption | Branch keeps its adopted prompt until it re-adopts | Skills / PromptSourcesBranch |
| Code row contributing tools revoked or disabled mid-call | — | That call "tool provider unavailable"; batch and turn continue; branches adopt without it | Tool source port |
| Two installs of one id on one node | lock, then Settings revision | Second `plugin_exists` (or `plugin_settings_conflict` across processes) | PluginManager, Settings |
| Two `configure` calls race | business source revision | Stale `expectedRevision` fails; `set_enabled` uses the row revision and does not conflict with `configure` | Settings |
| Approval granted, then package updated | new fingerprint | Code rows Pending until re-approved | `PluginPackage` row |
| Revoke while code runs | revoke persisted < gate withdrawn | Code rows disposed after settling their work; reply after disposal | Loader, code rows |
| Install interrupted after commit, before Settings | — | `operation_interrupted`; no row; directory collected | PluginManager |
| Install interrupted after Settings write | — | `operation_interrupted`; row exists; retry `plugin_exists` | Client reads state |
| Duplicate or late `plugin.install` | — | `plugin_exists` | PluginManager |
| Marketplace update finds a newer version | — | `updateAvailable` only; nothing changes | PluginManager |
| Target node offline before install | — | `node_unavailable`; nothing written | PluginManager (node-services port) |
| Node offline after install | peer rows Pending | Its rows Pending; its sources withdrawn; other nodes unaffected | rutis lifecycle |
| Package node differs from a Session's resource node | — | Executing consumers ignore the source for that Session | Content owner |
| Files modified on disk | next load | `integrity_failed`; row failed; sources withdrawn | `PluginPackage` row |
| PluginManager reload or restart mid-operation | — | Per A.7; never re-executes a fetch | PluginManager |
| Session unload/restore | — | No effect | — |
| Client detach | — | Operation continues; reply and progress dropped | Method contribution |

## Appendix C. Errors and recovery

| Code (`appError`) | Meaning |
| --- | --- |
| `manifest_invalid` | Missing or malformed manifest or index |
| `integrity_failed` | Digest mismatch, unlisted file, path escape or symlink |
| `incompatible_host` | `hostApiVersion`/`rutisVersion` or unsupported `runtime` |
| `source_unavailable` | Git fetch or path read failed (stderr only in diagnostics) |
| `node_unavailable` | Target node's services not available |
| `plugin_exists`, `plugin_not_found`, `marketplace_not_found` | Identity errors |
| `plugin_settings_conflict` | Stale `expectedRevision` |
| `approval_stale` | `approvalToken` differs from the current fingerprint |
| `runtime_unavailable` | Node runtime not configured; code rows Pending |
| `operation_interrupted` | `Report` rule of the method contribution; read `plugin.list` before retrying |

Removal disposes rows, then deletes the directory; a deletion failure returns `cleanupError` and garbage collection retries.

## Appendix D. Acceptance

Flow tests through `plugin.*` methods with a local test marketplace and a test consumer of a custom type key:

1. Add, update, search, view a marketplace; invalid index `manifest_invalid`; update marks `updateAvailable` without changing installed packages.
2. Install from marketplace, path and Git URL; tampered, unlisted and fingerprint-mismatched packages are rejected with nothing written.
3. A content-only package publishes sources to the test consumer; disable withdraws them; reload bumps the revision; remove deletes rows then files; a PluginManager reload does not reload the test consumer.
4. A package with Node code stays Pending until `plugin.approve`; `plugin.update` requires re-approval; revoke disposes the code row.
5. A code row contributing a test tool is revoked mid-call: that call returns "tool provider unavailable", the turn continues, other Sessions are unaffected.
6. Concurrent installs; stale `expectedRevision`; interruption after commit and after the Settings write.
7. Peer test node: install targets it; offline makes only its rows Pending and withdraws sources; recovery restores them.
8. Restart: rows restored, integrity verified, staging removed, orphans collected.

## Appendix E. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Package code is Node runtime rows until #4; no dynamic Rust code | Overview §5.12, §11.1 | More `code.runtime` values |
| A2 | Code approvals use the Settings host-local approval service with owner `plugin` | Decision R11; overview conventions | A plugin-owned approval file |
| A3 | Manifest schema 2 extends the existing format; v1 only for `--plugin-dir` | App Server README; judgment | Compatibility mapping |
| A4 | One installed version per `(node, id)` | Judgment | Side-by-side versions |
| A5 | Marketplace index format and name | Judgment; feature list "Git plugin marketplaces" | Format replaced |
| A6 | Sources are node content; executing consumers match the Session's resource node | Overview multiple nodes; judgment | Consumers read peer content through imported services |
| A7 | Git via the node's `git`, hooks/submodules disabled, `https`/`ssh` only | Judgment | Library fetcher or more schemes |
| A8 | Local paths copied, not linked | Judgment (integrity) | A development link mode |
| A9 | Additional methods `configure`, `approve`, `revoke`, `update`, `marketplace.search/view` | Feature list; overview B.12 | Folded into other methods |
| A10 | Operations serialize per `(node, id)` | Judgment | Settings revisions alone |
| A11 | Unreferenced directories garbage-collected after the loader settled | Judgment | Explicit cleanup command |
| A12 | Content sources use a stable port; node services are looked up per call | ADR 002 D2 | Offline peers or reloads propagate to owners |
| A13 | Package directories are written through the node's `FileStorage` under a PluginManager package root (atomic rename available there) | Judgment | A per-node storage service is needed |
| A14 | Install and update report progress phases | Method contribution progress sink; judgment | Result only |
| A15 | No automatic upgrade, including content-only packages | Decision E22 | Opt-in auto-upgrade setting |
| A16 | Per-plugin business configuration lives in a Settings business source keyed `<node>/<id>`, read by the plugin at use time | ADR 002 D5; judgment | Each plugin keeps its own store and contributes its own configure method |

## Appendix F. Open questions

| Question | Description |
| --- | --- |
| #3 typed operations | The Settings operations in A.6 are proposed here; their final shape belongs to the Settings supplement and #3 (intent only; `plugins.configure` writes business configuration, ADR 002 D5) |
| Overview SkillsCatalog change | "Installation and removal are initiated by PluginManager" is read as: PluginManager publishes and withdraws sources, SkillsCatalog discovers them; PluginManager makes no SkillsCatalog call |
| Peer-node content | Whether control-node consumers should read content on peer nodes through imported resource services (#5) |
