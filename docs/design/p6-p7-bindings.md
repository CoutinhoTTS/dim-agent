# Scheduler and Workflow native access

Status: Proposed for architecture and owner review; not implemented. Applies to
[Scheduler #70](https://github.com/arcships/dim/issues/70) and
[Workflow #71](https://github.com/arcships/dim/issues/71). Their full designs stay
Draft. This document specifies repository-local bindings and the evidence needed
before implementation can claim Ready.

## 1. Problem and existing mechanisms

| Consumer | Required behavior | Existing mechanism | Missing repository work |
| --- | --- | --- | --- |
| Scheduler authorizing saves and reuse | Read the dispatched branch policy and the host default bound | `Session::get<PermissionState>`, native `session.permission.get`, Agents' Session defaults | Read-only methods on the existing named `sessions` service |
| Workflow definitions | Read user, project and installed-package definitions; save project definitions within file authority | Settings owner reads/CAS, `settings.projects`, authorized-root `FileStorage` read/list/replace/create-directory | A named binding of FileStorage's existing operations and Workflow's source rules |
| Workflow report recovery | Know that an earlier report writer has ended before retrying an unknown open attempt | S2 input/lookup, rutis row-owned effects, `caller_row()` and `CallerRow::ctx()` | Retain admitted native input work through settlement; prove the replacement-row boundary in actual Node flows |

Creation configuration is not live Permission state. Published history is not a
replacement for the owner's dispatched state. `pluginFiles` is a versioned
private text store, not a project/package reader. An abandoned JavaScript Promise
does not prove that its native write ended.

The locked rutis 0.12.1 release already exposes
[`caller_row` and `CallerRow`](https://github.com/arcships/rutis/blob/ffd6e8b33556b9082d0d7f990ee47c68605c2d51/crates/rutis-bridge/src/session/services.rs).
The caller must be captured synchronously inside `HostDispatch::invoke`;
polling its returned future later does not retain that thread-local context.
An effect on that caller's `Ctx` belongs to its original loader row. No rutis
upgrade, new lifecycle event, cancellation protocol or host-epoch service is
part of this proposal.

## 2. Read-only permission access

Agents' stable directory keeps providing `sessions`. Add two asynchronous
methods, each taking one request object and using the existing
`{ok:true,value}` / `{ok:false,error:{code,message,retryable}}` envelope:

| Method | Request | Value |
| --- | --- | --- |
| `permission` | `{sessionId, branchId}` | `{version, policy}`, with the same fields and meanings as native `session.permission.get` |
| `permissionDefaults` | `{}` | The current index's `ServerConfig.session_defaults.permission_policy`, as the existing `PermissionPolicy` |

`permission` acquires/restores the target through Agents, retains its existing
per-call Session lease, and reads `PermissionState` from that branch. It reads
no private field, reconstructs no policy and adds no watcher or cache. A missing
Permission returns `reloading`; missing/closed Sessions and branches retain S2's
existing errors. `permissionDefaults` needs no Session. Both look up the current
index per call; an unavailable index returns `reloading`. Neither mutates policy
or grants authority.

Scheduler uses `permission` for the saving caller, the current starter before a
run, and the reused Session's `main` branch. Management `schedule.save` reads
`permissionDefaults`. Keep the accepted origin/cap replacement, policy
comparison, Settings CAS and pre-effect rechecks in
[Scheduler §3.3 and A.5](plugins/Scheduler.md#33-permission-of-a-run): a failed required
read leaves the job unchanged; a read is not atomic with CAS or input; a stale
CAS is returned without retry. Launch still applies its own current bound.
There is no dummy Session, omitted starter or creation-config fallback.

Workflow A17 continues to use the first actual launch reply's policy. These
methods do not move its capture point or add a new pre-launch policy check.
Full Access selection, CLI permission management and independent model approval
are separate product work; none is implemented by these reads.

## 3. Workflow definition files

### 3.1 Named FileStorage binding

Expose `fileStorage` from the existing FileStorage provider, through the existing
bridge `HostDispatch` and shared `ServiceCatalog` registration. The provider
owns the native operations and their resources; do not add an adapter plugin,
another root registry or a Node filesystem implementation.

The proposed foreign subset is asynchronous:

| Method | Request | Value |
| --- | --- | --- |
| `root` | `{path}`: an already-authorized canonical absolute directory | `{root}`: its existing process-local root index |
| `read` | `{root, path}`: relative file path | Bytes, received as `Uint8Array` by TypeScript |
| `list` | `{root, path}`: relative directory path | `[{name, kind}]`, where `kind` is `file`, `directory` or `other` |
| `replace` | `{root, path, bytes}` | `null`, after the existing atomic replacement and durability check |
| `createDirectory` | `{root, path}` | `null`, with FileStorage's existing non-recursive create semantics |

`root` only finds an existing authorization; it never calls `register_root`.
Use the canonical cwd returned by `settings.projects`, or the package root
chosen by the trusted host. Relative paths, reserved resources, symlinks,
generation validity and mutation locking remain FileStorage's checks. A root
index is not persisted as a definition identity. Root registration stays with
the trusted host and existing project authorization, not YAML or the model.

Replies use `{ok:true,value}` or `{ok:false,error:{code,message}}`. Codes preserve
the native distinctions `invalid_path`, `closed`, `cancelled`, `locked`,
`unsupported`, `unsynced` and `io`; a native not-found I/O error is `not_found`.
A name that cannot be represented as UTF-8 is `invalid_data`, never a lossy
alias. `unsynced` means a mutation may already be visible: do not retry it as an
unchanged write. `replace` keeps native permission behavior, with no chmod API.
No deletion, arbitrary root registration, lock API or S10 envelope is exported.

Workflow declares `fileStorage` in its process module's `inject`. FileStorage
already underlies both `settings` and `pluginFiles`, so this direct dependency
introduces no additional upstream reload source. A FileStorage reload still
retires Workflow through the existing dependencies. Optional tools and native
Session readiness retain the accepted [process binding](process-typescript.md#51-p6p7-access-through-declared-feature-dependencies).

### 3.2 Source selection and authority

| Source | Discovery and identity | Writes and enablement |
| --- | --- | --- |
| User | Source `workflow`, user file `workflow.json`; entry key is the definition name | Workflow's existing typed Settings CAS |
| Project | Authorized cwd from `settings.projects`; `.dimcode/workflows/<name>.yaml`; basename is the name | `workflow.save source:project` uses FileStorage; user-local enablement is required separately |
| Plugin | Explicit installed package roots on the Workflow process row; `workflows/<name>.yaml`; identity is package id plus basename | Read-only to Workflow; installation/enablement comes from trusted host configuration |

The proposed Settings document is
`{definitions: {[name]: Definition}, projectEnabled: {[canonicalCwd]: {[name]: boolean}}}`.
Declare `userOnly: ["/projectEnabled"]`, with no `projectFile`. Settings performs
structural validation at its existing boundary. Definition semantics stay with
Workflow at save/list/start under A.1. Owner writes preserve `projectEnabled`;
only the protected local Settings client changes it. Missing enablement means
disabled. A project file cannot enable itself, and edits to an enabled project
definition affect only later runs.

This is an explicit proposed user-source document shape, not an implicit
conversion of an existing name map. A nonconforming persisted source is invalid
and its bytes stay unchanged; no source or run-store migration is included.

Installed packages are structural row options
`packages: [{id, root}]`, where ids are unique and roots are canonical absolute
paths explicitly present in the host's FileStorage roots. This is package
placement, not a definition manifest or executable business content in loader
rows. PluginManager may populate these existing row options later; Workflow
does not depend on an unimplemented PluginManager or `PluginContents` API.
Package document `name` must match `<name>.yaml`, as for project files.

Keep the existing `project`, then `user`, then `plugin` source precedence and
the existing `source` selector. Names must be unique within a source category:
if two installed packages supply one name, list both package identities as
invalid and reject that plugin definition with `invalid_definition`; never pick
a package by directory order. Invalid selected definitions do not fall through
to another source. Lists put the name, category, package id where applicable,
and diagnostics in model-visible `content`.

Read each source at use time; the accepted run stores the definition snapshot.
Decode UTF-8/YAML and validate external structure once at the Workflow boundary.
Do not parse Settings-certified documents again. An absent project workflow
directory means no project definitions; other source failures surface as
`definition_source_failed`, not an empty catalog. An explicitly installed
package missing its workflow directory is such a failure. A missing selected
definition keeps `definition_not_found`. Project save creates `.dimcode` and
`workflows` separately if absent through `createDirectory`; it never enables the
definition or overwrites a non-directory entry. Native project-file replacement
is atomic, with the current last-writer behavior; it adds no project CAS.

## 4. Native report settlement

### 4.1 Admission belongs to the original row

Use the existing S2 `input`/`lookup` operations and Workflow's inner-v2
`attempt_open` record. No new report service or operation id is introduced.

For a call through a leased runtime-row `sessions` service, capture
`caller_row()` in `invoke`. Before starting the native input task, register its
drain effect on that row's `Ctx`. If registration fails because the row is
unloading, do not start the task. The bridge result and the effect join the same
native task; dropping the remote result does not abandon it. On ordinary
completion dispose that effect, so completed calls do not accumulate until row
unload. This is native I/O ownership, not business state or a second report log.

The task retains the actual S2 call and Session lease through append and
publication settlement. An index missing before admission returns `reloading`
without writing. Once admitted, index reload does not replace that task's result
with an early `reloading` reply; the call returns its own outcome, as admitted
`launch` already does. This deliberately changes S2 input lifetime, not its
request, reply, input event or publication contract. Native Rust callers still
own their calls; do not fabricate a row when `caller_row` is absent.

Native drain effects run before the original row's foreign cleanup. After row
retirement begins, new input admission is refused; admitted calls finish.
Normal loader replacement cannot apply that same process row's next generation
before its effects settle. A complete host-process restart also leaves no old
native writer. The new Workflow owner may rely on this guarantee only for its
own fixed process row and run store, after the acceptance below proves it.
Moving a store to another owner while the first owner runs is not a reload;
the old owner must be fully disposed before that transfer.

### 4.2 Recovery order

One configured Workflow process row owns its run store and report tasks. The
next generation starts recovery after the ordinary row lifecycle allows its
business work to start. It first performs `lookup`, as §3.4 already requires:

1. `accepted` or `ambiguous` settles the record as specified there, without
   resending.
2. For an open attempt with `unknown`, the completed original-row drain above
   proves that no previous native input task can still write. Persist
   `attempt_open:false`; then use the existing persist-open-before-input path.
3. Without that proven boundary, retain the open marker and remain lookup-only.
   A JavaScript exception, elapsed time, fresh Node process or closed FileScope
   is never the evidence.

No host epoch or row key is stored. The persisted file stays inner v2; no field,
version reuse or migration changes. The row boundary is a binding guarantee,
not a durable identity inferred from a handle. Workflow still owns record
writes and waits for their success/error before closing storage. If cleanup
cannot persist a reply, the existing open marker remains for recovery.

## 5. Implementation ownership and acceptance

| Owner | Repository-local implementation |
| --- | --- |
| Agents / S2 | Two policy reads; native `input` task lifetime and unchanged publication/lookup guarantees |
| FileStorage | Named binding of the five existing operations; no new root authority or file format |
| TypeScript assembly / SDK | Shared catalog name and data types; attach native calls to the original row using existing rutis effects |
| Workflow / Scheduler | Concrete Settings/source/package rows and business flows; actual recovery evidence |

Architecture review belongs to eric8810; Agents/storage review to yyhhyyyyyy;
binding and plugin delivery to zerob13. These are requested repository-owner
reviews, not requests for rutis to add an API.

Delivery order: accept this contract, implement the native bindings with their
Node fixtures, confirm each plugin's remaining Ready choices, then implement
the business package. The binding fixtures demonstrate reads, source authority
and native I/O ownership without implementing a Scheduler/Workflow engine.
Complete save/list/start/report behavior remains the plugin implementation's
acceptance, after Ready. #126 can proceed once either consumer is Ready and
must land before repeated-Session shipping.

Required flow evidence, with disposable homes and the real Node binding (the
business assertions are completed with the subsequent plugin implementation):

1. Narrow a branch after creation; foreign `permission` reads dispatched state,
   not config. Cover index/Permission reload, host defaults, save failure/CAS
   preservation and reuse of a Session above the job's effective policy.
2. Exercise all three source locations through a declared Settings/source
   fixture. Protected client enablement works, owner enablement fails. Complete
   Workflow save/list/start verifies project files start disabled and package-name
   collisions, invalid YAML/UTF-8, absent project directory, denied traversal,
   no S10 wrapping, and source edits preserving an accepted snapshot.
3. Hold report input before append and during publication; orderly row unload
   and index reload must not end ownership early or allow another writer.
4. Crash Node with `attempt_open:true`, holding the old native call. A replacement
   row must not recover past its original-row drain. Release it: lookup observes
   one report. Crash between the marker write and native invocation: after the
   proven boundary, unknown permits one submission. Repeat over host restart.
5. Hold a store replacement during unload, including a failed replacement of
   the reply state. Storage closes only after admitted writes settle; recovery
   retains the open marker and does not duplicate a report. Finished native-call
   effects do not grow with successive reports.

These are acceptance requirements, not tests claimed to have passed. Existing
#234/#237 flows establish process placement and dependency behavior, not the
report-specific cases. Keep P6/P7 Draft until the remaining Appendix F choices
and this evidence reach Ready. #126 remains a shipping prerequisite after Ready;
#248/#255 optional-tool work is independent of these source and authority reads.

## 6. Rule check and ablation

| Rule | Answer |
| --- | --- |
| §4.1 | Policy/file reads change no owner's state; report input goes through S2 and InputQueue. |
| §4.2 | Accepted input is never recalled; interruption uses existing withdrawal/stop semantics. |
| §4.3 | No before callback, asynchronous commit hook or publication phase. |
| §4.4 | Native services know no Scheduler job or Workflow run/report marker; they expose reads and own admitted I/O. |
| §4.5 | No new event/batch; Workflow's existing file transaction precedes each guarded effect. |
| §4.6 | Definition text enters context only in existing adopted stage inputs or tool results. |
| §4.7 | PermissionState, FileStorage and S2 are read through their owners; no consumer reconstruction of private state. |
| §4.8 | All changes close in repository-owned bindings; the requirement table below gives owners and alternatives. |
| §4.9 | No new Timeline fact, host epoch, task log or persisted root/row handle. |
| §1 | No App Server scheduler/workflow engine; its changes are access and assembly. |
| ADR 002 D1 | Run state stays process-owned; optional tools remain Session parts. |
| ADR 002 D2 | Existing providers expose bindings; no wiring/adapter plugin. |
| ADR 002 D3 | Native readiness depends on no optional package; FileStorage is already an upstream of declared Settings/storage. |
| ADR 002 D4 | Definitions/enablement remain business sources; installed package paths alone are structural options. |
| ADR 002 D5 | Named methods and data are specified above; bytes use the existing bridge binary value. |

Ablation: keep two distinct policy reads to avoid an implicit fallback from a
missing branch to host authority. The five file operations each serve a stated
source flow; do not export root registration, removal, chmod or generic leases.
Native input effects replace a separate scope service, pending-call registry,
host epoch and report queue. Keep the attempt marker and existing transactions:
they guard actual uncertain writes and are already the agreed format.

## 7. Requirements raised to mechanism owners

| Item | Problem, existing alternatives and requested change | Ordering / owner |
| --- | --- | --- |
| S2 reads | Config/history do not provide dispatched Permission; launch-only capture does not meet Scheduler save/reuse. Bind the two existing native reads in §2. | Before P6 Ready; Agents owner review, #70 |
| FileStorage binding | Private `pluginFiles` cannot read raw project/package YAML; direct Node filesystem access bypasses the agreed file boundary. Bind existing authorized-root operations in §3. | Before P7 source acceptance; FileStorage and TS binding owners, #71 |
| Native input settlement | An early index-unload reply or dropped bridge result is not a writer-termination proof. Retain admitted native input and join it on its original row using existing caller context/effects, then verify §5. | Before P7 report acceptance; Agents and TS binding owners, #71 |

No external capability request is established. If an actual acceptance flow
cannot be implemented with the locked APIs, record the reproduction, the
existing API that fails, required generic behavior, ordering effect and owner
in a Dim issue before proposing a rutis dependency. Do not turn untested
behavior into a request for a new protocol.
