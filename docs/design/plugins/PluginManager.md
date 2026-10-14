# PluginManager

Status: Draft. Overall design: [overview §5.12](../overview.md#512-pluginmanager), [§7.2 Settings, Mcp, SkillsCatalog](../overview.md#72-app-server-process-plugins), [§9 Trust](../overview.md#9-cross-cutting-conventions), [§3.1 Multiple nodes](../overview.md#31-multiple-nodes). Phase: P3 (depends on [#3](https://github.com/arcships/dim/issues/3); non-Rust capability plugins on [#4](https://github.com/arcships/dim/issues/4); peer nodes on [#5](https://github.com/arcships/dim/issues/5)). Language: Rust.

PluginManager installs plugin packages from Git marketplaces, local paths or Git URLs onto a target node, records user plugin intent and configuration through typed Settings operations into the loader editable layer, and publishes the typed content of enabled packages as generic plugin-content sources. Content owners (Skills, Hooks, MCP, workflows) discover the content types they understand; PluginManager knows none of them. Executable code in a package runs only after host-local approval bound to node, scope and package fingerprint.

## 1. User flows

1. `/plugins` → add marketplace: the user adds a Git marketplace; PluginManager fetches its index and lists its plugins. Search filters the cached indexes.
2. Install from a marketplace: PluginManager resolves the entry, fetches the package on the target node, verifies the manifest and every file digest, stores it, and writes the plugin's loader rows through Settings. Content-only packages are active immediately; packages with code report `approval: required`.
3. The user reviews and approves the code (`plugin.approve` with the token shown by `plugin.list`); the code rows load.
4. A package declaring `skills` content: SkillsCatalog sees a new `skills` source on its next revision and lists the Skills; PluginManager did nothing Skill-specific.
5. Disable, reload (`/reload-plugins`), configure, remove: each maps to a typed Settings operation or a loader operation; content sources are withdrawn or republished; consumers refresh.
6. Install on a peer node: the package is fetched and stored on that node; its code rows run there as `peer:<node>/…` rows; its content sources carry the node.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Marketplaces (Git sources): add, remove, update, list, search, view | Interpreting content types (each content owner) |
| Package fetch, manifest validation, integrity, storage per node, removal | Approval of executable content owned by a consumer (Hooks, MCP, workflows apply their own host-local approval) |
| Plugin intent and configuration via typed Settings operations; reload/restart via the loader | Plugin runtime behavior, tools, events (the plugin itself) |
| Host-local approval for package code rows it writes | Loading non-Rust capability plugins before #4 |
| `PluginContents`: generic typed content sources | Project-declared plugin requirements |
| Management methods `plugin.*` | Dynamically loading Rust code (no dynamic Rust plugins, A1) |

## 3. Scope and dependencies

**Contracts defined here** (owner: PluginManager): the package manifest `dim-plugin.json` schema version 2, the marketplace index `dim-marketplace.json`, the `PluginContents` publication contract (`ContentSource`), the `PackageStore` node service, the `PluginApproval` gate key, and the `plugin.*` management methods. **Contracts depended on**: typed Settings operations on the loader editable layer and the host-local approval source (Settings supplement per #3, proposed), the rutis-loader `Loader` (`entries`, `reload`, `restart`, `LoaderChanged`; verified in rutis-loader 0.10), `HostPlatform` process start on the package's node, `FileStorage` leases, the [Management contribution contract](Management.md). Consumers depend on `PluginContents` only; PluginManager never calls a consumer and never names a content type in code.

| Part | Scope | Injects | Provides | Status |
| --- | --- | --- | --- | --- |
| `PluginManagerPlugin` | Process (control node) | `SettingsService` (typed plugin operations), `Arc<Loader>` | `PluginContents`, `PackageNodes` (registry of node stores), `ManagementContribution` (`plugin`) | Proposed |
| `PluginPackages` | Process, one row per node: local, or `peer:<node>/PluginPackages` | `HostPlatform`, `FileStorage` of that node | `PackageStore` keyed by node | Proposed |
| `PackageNodeMount` (instanced, one per node) | Process (control node) | `PackageNodes`, keyed `PackageStore` of its node | — (registers the store) | Proposed |
| `PluginPackage` row (one per installed package, written through Settings) | Process | `PluginContents` registry, the node's `PackageStore`, host-local approval source | its content sources; `PluginApproval` keyed `<node>/<id>` when approved | Proposed |
| Package code rows (zero or more per package) | Process, on the package's node | what the code declares, plus the `PluginApproval` gate | what the code declares | Proposed |
| Content owners (SkillsCatalog, Mcp, Hooks, Workflow) | Process | `When<Arc<PluginContents>>` | — | Supplement in each owner (overview §7.2) |

PluginManager reaches node stores only through `PackageNodes`, fed by one mount instance per node (the pattern of [Management](Management.md#3-scope-and-dependencies)), so an offline peer makes only its own mount Pending, never PluginManager (A12). The `PluginPackage` row realizes #3's "content plugins become loader rows once installed": an installed package is a row; the row is the persisted intent (enabled, configuration, version, source, fingerprint); `plugin.list` is projected from loader entries, never from a separate intent store.

## 4. Events

None. Installation, enablement and approval are host configuration, not Session facts; they live in the loader editable layer, the node package store and host-local approvals. Session-visible effects arise only through consumers (a Skill list change is adopted through prompt sources; a contributed tool through ToolRegistry), whose own documents define them.

## 5. State

| State | Owner, storage | Rebuild |
| --- | --- | --- |
| Marketplace registrations (name, URL, ref) | Settings, loader editable layer | Persisted |
| Marketplace clones and parsed indexes | `PackageStore` of the control node, cache directory | Re-fetched; derivable |
| Installed package intent: rows `plugin:<id>` per node with id, version, node, source, resolved commit, fingerprint, contents, enabled, configuration | Loader editable layer (through Settings) | Persisted by #3 |
| Package bytes | `PackageStore` of the package's node, content-addressed directory | Persisted on that node |
| Code-row approvals `{node, scope, id, fingerprint}` | Settings host-local approval source (`plugin-approvals.json`, version 1, private file) (A2) | Persisted |
| Published content sources and revision | `PluginContents` registry | Rebuilt from `PluginPackage` rows' `apply` |
| Staging directories, per-package operation lock | `PackageStore`, PluginManager in memory | Staging deleted on `PackageStore` start |

## 6. Interfaces

### 6.1 Package manifest `dim-plugin.json` (schema version 2)

Version 1 is the existing managed-provider manifest ([App Server contract](../../../crates/dim-app-server/README.md#optional-managed-provider)); it is still accepted by the `--plugin-dir` path and is not installable through PluginManager (A3).

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
| `version` | SemVer string; displayed and compared for updates only |
| `files` | Every file of the package except `dim-plugin.json` and `.git/`, relative POSIX paths, no `..`, no symlinks; an unlisted file or a digest mismatch rejects the package |
| `contents` | Opaque `type` `[a-z][a-z0-9-]*`, `path` inside the package and listed (or a directory whose files are listed), optional `options` object ≤ 64 KiB. PluginManager validates only this shape |
| `code` | Optional. `runtime: "node"` loads `entry` as a rutis runtime row (precedent `--plugin-dir`); compatibility checks `hostApiVersion` and `rutisVersion` as today. Other runtimes are rejected until #4 (A1) |
| Fingerprint | SHA-256 of the manifest bytes as stored; since the manifest lists every file digest, it binds all package bytes |

### 6.2 Marketplace index `dim-marketplace.json` (A5)

At the repository root of a marketplace: `{schemaVersion: 1, name, plugins: [{id, version, description, keywords?, source: {path} | {git, ref?}, fingerprint?}]}`. `source.path` is inside the marketplace repository; `fingerprint`, when present, must equal the fetched package's fingerprint.

### 6.3 Plugin content sources

```rust
/// Process service; key TypeKey::of::<PluginContents>().
impl PluginContents {
    /// Sources of enabled, integrity-verified packages whose node is available.
    pub fn sources(&self, content_type: &str) -> Vec<Arc<ContentSource>>;
    /// Bumps on any change; carries the new revision.
    pub fn changed(&self) -> tokio::sync::watch::Receiver<u64>;
}
pub struct ContentSource {
    pub package: String,        // id
    pub version: String,
    pub fingerprint: String,    // binds host-local approvals of consumers
    pub node: Option<String>,   // None = control node
    pub root: PathBuf,          // absolute package directory on `node`
    pub path: PathBuf,          // declared path, relative to root
    pub options: serde_json::Value,
    pub configuration: serde_json::Value, // user configuration of the package (credential references only)
}
```

A consumer owns its type key and format, reads files through the services valid for `node`, and applies its own host-local approval for executable content, binding `node + package + fingerprint` (overview §9). A consumer that cannot access `node` ignores the source and reports it in its own catalog (A6).

### 6.4 Node package store

```rust
impl PackageStore {   // one per node
    async fn stage(&self, source: PackageSource) -> Result<Staged, PluginError>; // fetch, verify
    async fn commit(&self, staged: Staged) -> Result<StoredPackage, PluginError>;  // atomic rename
    async fn verify(&self, stored: &StoredPackage) -> Result<(), PluginError>;    // re-hash all files
    async fn remove(&self, stored: &StoredPackage) -> Result<(), PluginError>;
    async fn fetch_marketplace(&self, url: &str, reference: Option<&str>) -> Result<Index, PluginError>;
}
pub enum PackageSource { Path(PathBuf), Git { url: String, reference: Option<String> } }
```

Git runs through the node's `HostPlatform` as `git` with hooks and submodules disabled; only `https://` and `ssh://` URLs (A7). Local paths are copied into the store, never referenced in place (A8). Stored directories are content-addressed by fingerprint, so an update never rewrites files a running row uses.

### 6.5 Management methods (namespace `plugin`)

| Method | Params | Result | Interruption |
| --- | --- | --- | --- |
| `plugin.marketplace.list` | `{}` | `[{name, url, ref, commit, fetchedAt, error?}]` | Drain |
| `plugin.marketplace.add` | `{name, url, ref?}` | `{commit}` | Report |
| `plugin.marketplace.remove` | `{name}` | `{}` | Report |
| `plugin.marketplace.update` | `{name?}` | `[{name, commit, error?}]` | Report |
| `plugin.marketplace.search` | `{query, marketplace?}` | `[{marketplace, id, version, description, installed: [{node, version}]}]` | Drain |
| `plugin.marketplace.view` | `{marketplace, id}` | entry plus the fetched manifest summary (`contents` types, `code` presence) | Drain |
| `plugin.install` | `{source: {marketplace, id} \| {path} \| {git, ref?}, node?, enabled?}` | `{id, version, node, fingerprint, approval, approvalToken?}` | Report |
| `plugin.list` | `{node?}` | `[{id, version, node, enabled, status, approval, approvalToken?, contents, source, updateAvailable, error?}]` | Drain |
| `plugin.set_enabled` | `{id, node?, enabled, expectedRevision}` | `{}` | Report |
| `plugin.configure` | `{id, node?, configuration, expectedRevision}` | `{}` | Report |
| `plugin.reload` | `{id?, node?}` | `[{id, node, status, error?}]` | Report |
| `plugin.approve` | `{id, node?, approvalToken}` | `{}` | Report |
| `plugin.revoke` | `{id, node?}` | `{}` | Report |
| `plugin.remove` | `{id, node?}` | `{cleanupError?}` | Report |

`status` is projected from the loader entry: `active`, `pending` (node offline or awaiting approval), `failed` (with reason), `disabled`. `approval` is `notRequired` (no code), `required` or `approved`; `approvalToken` is the fingerprint, a digest, not a credential. `plugin.configure`, `plugin.approve`, `plugin.revoke` and the `view`/`search` names are additions to the overview list (A9). CLI entries: `/plugins`, `/plugin`, `/reload-plugins` (calls `plugin.reload {}`), `dimcode plugin …`.

### 6.6 Typed Settings operations used (proposed Settings supplement, #3)

| Operation | Loader effect |
| --- | --- |
| `plugins.install(entry, expected_revision)` | Creates `plugin:<id>` and its code rows (`create`) on the editable layer, persisted |
| `plugins.set_enabled(id, node, enabled, expected_revision)` | `set_disabled` on the package row (code rows follow by dependency) |
| `plugins.configure(id, node, configuration, expected_revision)` | `update` of the package row configuration |
| `plugins.remove(id, node, expected_revision)` | `remove` of all rows of the package |
| `marketplaces.save/remove` | Marketplace registrations |
| `approvals.plugins.grant/revoke` | Host-local approval source, outside the loader and Timeline |

Non-persistent operations (`reload`, `restart`, status) call the `Loader` directly.

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Timeline writes | None by PluginManager, `PluginPackages` or `PluginPackage` rows |
| Install | Per `(node, id)` operation lock (A10) → `stage` → compare with existing intent → `commit` → `plugins.install` with expected revision → loader creates rows → reply. A failed Settings write leaves a committed but unreferenced directory, removed by garbage collection (A11) |
| Package row load | `PluginPackage::apply` resolves the node's `PackageStore`, runs `verify` (integrity on every load), registers its sources in the `PluginContents` registry and, when a matching approval grant exists, provides `PluginApproval` keyed `<node>/<id>`. Its `ctx.effect` withdraws both. Revision bumps on each change |
| Code rows | Inject `PluginApproval` for their package: Pending until approved, disposed when the grant is revoked or the package row is disabled, Pending while the node is offline. They follow the loader lifecycle; their in-flight work is settled by their own disposers |
| Consumers | Observe `changed()`, read `sources(type)`, reconcile in their own state. They decide only from what they read; PluginManager never pushes into them |
| Reload | `plugin.reload` calls `Loader::reload` on the package row (re-verify, republish, revision bump) and on its code rows |
| PluginManager reload | `PluginContents` consumers reload through `Deps`; `PluginPackage` rows reload and republish; in-flight install/remove operations return `operation_interrupted` and leave state per the steps already committed |
| Process restart | Loader restores rows from the editable layer; package rows re-verify; staging is deleted; unreferenced stored directories are collected after the loader settled |

No `StateCell`, Dispatcher subscription or recheck against Timeline exists; rechecks are revision checks on Settings and the approval source.

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Turn `stop/interrupt`, branch interrupt or close, concurrent inputs, unsettled tool batch, compaction, child branch cut | — | No effect on PluginManager; consumers' adoption rules apply to content already adopted by a branch | Consumers (e.g. Skills, ToolBinding) |
| Package disabled while a Session uses its Skill | revision bump < next adoption | Branch keeps its adopted prompt until it re-adopts | Skills / PromptSourcesBranch |
| Code row contributing tools removed while a call runs | — | Settled as a contributor reload per [process tool contribution](../contracts/process-tool-contribution.md) | ToolExecutor, Loop |
| Two installs of the same id on the same node | operation lock, then Settings revision | Second waits, then fails `plugin_exists` (or `plugin_settings_conflict` across processes) | PluginManager, Settings |
| `plugin.set_enabled` races `plugin.configure` | Settings revision | Later write with a stale `expectedRevision` fails | Settings |
| Approval granted, then package updated | new fingerprint | Old grant does not match; code rows Pending until re-approved | `PluginPackage` row |
| Revoke while code runs | revoke persisted < gate withdrawn | Code rows disposed; their work settled by their own disposers; reply after disposal | Loader, code rows |
| Install interrupted after commit, before Settings | — | `operation_interrupted`; no row; directory collected; `plugin.list` shows not installed | PluginManager |
| Install interrupted after Settings write | — | `operation_interrupted`; row exists; `plugin.list` shows installed; retry returns `plugin_exists` | Client reads state |
| Duplicate or late `plugin.install` | — | `plugin_exists` | PluginManager |
| Target node offline before install | — | `node_unavailable`; nothing written | `PackageStore` lifecycle |
| Node goes offline after install | `PackageStore` Pending | Package rows and code rows Pending; sources withdrawn; consumers drop them | rutis lifecycle |
| Files modified on disk after install | next load | `integrity_failed`; row failed; sources withdrawn | `PluginPackage` row |
| PluginManager reload or process restart mid-operation | — | As §7; never re-executes a fetch automatically | PluginManager |
| Session unload/restore | — | No effect | — |
| Client detach | — | Operation continues; reply discarded | Management |

## 9. Context mapping

No direct model-visible effect. PluginManager writes no input, contributes no tool and registers no prompt source. Package content reaches the model only through its consumer, in the form that consumer defines: Skills through the Skill prompt source (adopted per branch and replayed after compaction as configuration), MCP and code-row tools through tool declarations and `ToolResult.content`, Hooks output through Session input. Non-interactive Sessions differ only as the consumers define. The model never sees package ids, fingerprints, approvals, marketplace data or diagnostics.

## 10. Errors and recovery

| Code (`appError`) | Meaning |
| --- | --- |
| `manifest_invalid` | Missing or malformed `dim-plugin.json` or marketplace index |
| `integrity_failed` | Digest mismatch, unlisted file, path escape or symlink |
| `incompatible_host` | `hostApiVersion`/`rutisVersion` or unsupported `runtime` |
| `source_unavailable` | Git fetch or path read failed (stderr only in diagnostics) |
| `node_unavailable` | Target node's `PackageStore` not Active |
| `plugin_exists`, `plugin_not_found`, `marketplace_not_found` | Identity errors |
| `plugin_settings_conflict` | Stale `expectedRevision` |
| `approval_stale` | `approvalToken` differs from the current fingerprint |
| `runtime_unavailable` | Node runtime not configured; code rows stay Pending |
| `operation_interrupted` | Management `Report` rule; read `plugin.list` before retrying |

Removal disposes rows first and then deletes the directory; a deletion failure returns `cleanupError` and the directory is retried by garbage collection.

## 11. Security

Installing is a management operation: only entries that mount Management can install, approve or remove. Package code is trusted native-process code once loaded (AGENTS rule 11), so loading requires host-local approval of the exact fingerprint on that node; replay, fork, FullAccess and project files never grant it. Git fetch runs no repository hooks or submodules; package files are never executed at install time. Integrity is rechecked on every load. Content consumers keep their own approval rules for executable content and must not treat installation as approval. Plugin configuration carries credential references only; resolved secrets never enter rows, `plugin.list`, diagnostics or content sources. Host-local approvals and the package store are node-local and never enter Timeline.

## 12. Acceptance

Flow tests through management methods with a local test marketplace repository and a test consumer of a custom content type:

1. Add, update, search and view a marketplace; an invalid index yields `manifest_invalid`.
2. Install from marketplace, path and Git URL; tampered file, unlisted file and fingerprint mismatch are rejected with nothing written.
3. A content-only package publishes sources of its declared types to the test consumer; disable withdraws them; reload bumps the revision; remove deletes rows and then files.
4. A package with Node code stays Pending until `plugin.approve`; update requires re-approval; revoke disposes the code row.
5. Concurrent installs of one id; stale `expectedRevision`; interruption after commit and after the Settings write leave the states in §8.
6. With a peer test node: install targets it; taking it offline makes rows Pending and withdraws sources; recovery restores them.
7. Restart: rows restored, integrity verified, staging removed, orphans collected.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Package code is limited to Node runtime rows until #4; no dynamic Rust code | Overview §5.12, §11.1 | More runtimes add `code.runtime` values |
| A2 | Code approvals live in a host-local `plugin-approvals.json` owned by Settings | Overview §7.2 Settings, §9 Trust; precedent `mcp-approvals.json` | Another host-local store |
| A3 | Manifest schema version 2 extends the existing format; version 1 stays for `--plugin-dir` only | App Server README manifest; judgment | Both installable with a compatibility mapping |
| A4 | One installed version per `(node, id)`; user scope per node | Judgment | Side-by-side versions or project scope need identity changes |
| A5 | Marketplace index format and file name | Judgment; feature list "Git plugin marketplaces" | Format replaced |
| A6 | Consumers ignore sources on nodes they cannot access | Overview §3.1; judgment | Consumers read peer files through imported services |
| A7 | Git via the node's `git` executable, hooks and submodules disabled, `https`/`ssh` only | Judgment | A library fetcher or more schemes |
| A8 | Local paths are copied, not linked | Judgment (integrity) | A development link mode bypasses fingerprints |
| A9 | Additional methods `plugin.configure/approve/revoke`, `plugin.marketplace.search/view` | Feature list (search, view details, configuration); overview §5.12 names | Folded into other methods |
| A10 | Operations serialize per `(node, id)` | Judgment | Settings revision checks alone |
| A11 | Unreferenced stored directories are garbage-collected after the loader settled | Judgment | Explicit cleanup command |
| A12 | Node stores are mounted per node into a registry instead of injected directly | Judgment (offline isolation) | PluginManager Pending whenever any configured node is offline |

## 14. Open questions

| Question | Description |
| --- | --- |
| #3 typed operations | The Settings operations in §6.6 are proposed here; their final shape belongs to the Settings supplement and #3 |
| Overview §7.2 SkillsCatalog | "Installation and removal are initiated by PluginManager" is read as: PluginManager publishes and withdraws sources, SkillsCatalog discovers them; PluginManager makes no SkillsCatalog call |
| Peer-node content | Whether control-node consumers should read content on peer nodes through imported resource services (#5) |
| Updates | Whether marketplace updates may auto-upgrade content-only packages |
| Management contributions from packages | Packaged code may provide a `ManagementContribution`; installing it extends the host administration surface after approval only |
