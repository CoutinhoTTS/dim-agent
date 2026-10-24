# Session TypeScript parts

Status: tool/method and leaf broadcast bindings implemented for [C4 #63](https://github.com/arcships/dim/issues/63)
and [#168](https://github.com/arcships/dim/issues/168), following the approved [#174 contract](https://github.com/arcships/dim/pull/174).
The runtime uses rutis 0.16.0.
The verification section distinguishes executable acceptance from platform limits.
The [spike](session-typescript-spike.md) establishes transport feasibility;
its private service names and combined fixture are not this API.

The [process-row and management binding](process-typescript.md) uses the same
embedding input and runtime. Every row declares its group
([How Dim uses rutis §2](rutis-usage.md#2-parts-are-loader-rows)): Session parts are rows in
`session`, branch parts rows in `branch` ([branch binding](branch-typescript.md)), and process
owners rows without a group.

## Scope and ownership

A TypeScript Session part supplies named tools and Session methods and observes its Session's
broadcast. One Node process serves all such parts; loader instances isolate their services.
The Session owns the part's state and lifetime. Native execution remains available when a part
is absent, Pending, failed or reloading. Instance labels are namespacing, not a sandbox.

The existing owners implement their language bindings:

| Owner | Responsibility |
| --- | --- |
| App Server assembly | Validate structural input, register names derived from declarations, append ordinary rows inside `SESSION_GROUP`, select the existing runtime resolver. |
| ToolRegistry | Read a named foreign declaration when it attaches a provider generation and retain that generation in its versioned contributed set (A.5, #241). |
| C1 `SessionMethods` | Bind a foreign handler to the existing generation-owned admission, progress, interruption and drain. |
| Session broadcast | Encode its own records for the native rutis event path. |
| TypeScript part | Own its business state, work, effects, errors and cancellation settlement. |

There is no registration plugin, event-forwarding service, second bus,
new Session facade method or readiness phase. Core never requires a TypeScript row. Timeline, Dispatcher and persisted event formats
remain unchanged. This work does not expose arbitrary runtime services through the Session facade.

## Host input and declaration discovery

The initial usable entry is the public App Server embedding API:
`serve(builder, ServerConfig { session_typescript: Some(...), .. })`. It runs an actual App Server and its
normal Rpc path. The [embedding example](../../crates/dim-app-server/examples/session_typescript.rs)
uses a protected Unix connection and a disposable storage home. General CLI installation,
marketplaces and editable plugin intent remain PluginManager work.

The input contains the existing `LocalRuntime::node` arguments (`node`, `jsRuntime`, `anchor`,
absolute paths), an optional Python runtime (`python`: an interpreter with the `rutis` package and
the project directory its `py:<module>` rows are imported from) and `rows`, an array of ordinary
loader row objects. One Node runtime, and one Python runtime if configured, is shared across the
rows and Sessions. Its runtime name differs from an account runtime;
no private account manifest or account service is required by this entry. A part listens to the
Settings events on the bus with `ctx.on` ([settings-service §4.4](appserver/settings-service.md#44-events)).

Each row retains loader's `id`, `name` (module entry), `config`, `disabled`, `inject` and `isolate`
semantics. It adds `group`: `"session"` for a Session part, `"branch"` for a branch part, absent for a
process owner. The host places the row in that group and does not pass `group` to the loader, whose
own `group` field marks a row that is a group. The row metadata is `provides`, with **exactly the
existing `rutis_bridge::runtime::RowSchema.provides` shape**, rather than a parallel tool/namespace
list, and, only for a package that owns Settings sources, `settings` (below):

```json
{
  "id": "web",
  "name": "/absolute/web/index.ts",
  "group": "session",
  "config": {},
  "provides": {
    "toolContribution:web_fetch": {"declaration": "sync", "execute": "async"},
    "sessionMethods:session.web": {"describe": "async", "call": "async"}
  }
}
```

The empty configuration above means this particular part has no structural options. Business
settings belong to the owning plugin's Settings source, not to this object.

A package that owns Settings sources declares them in a second structural member, `settings`, an
array of the entries defined in [settings-service §3.1](appserver/settings-service.md#31-source-declaration).
The host passes them to Settings with the row id as owner and does not hand `settings` to the
loader. A source is process state, so it belongs to one row: a second row declaring the same
source fails assembly, as two packages would. Rows without `settings` are unchanged.

`toolContribution:<tool>` and `sessionMethods:<namespace>` encode the existing neutral service
identity and key as one bridge service name. The colon separates the family from the complete
key; the namespace keeps its dots. The catalog registers every name a row provides, if it does not
hold it yet, as a shared service in the row's group, so each instance has its own copy
([How Dim uses rutis §3.5](rutis-usage.md#35-how-each-runtime-resolves-a-name)). The tools and
namespaces of `session` rows are derived from these names and passed to the
existing `AgentBuilder::session_tool` / `session_methods` during assembly. Authors do not make
these Rust calls and do not separately maintain arrays of tool names and namespaces.

`provides` and `settings` are structural input, not a new package manifest or persistence format.
No filename, package schema, version field, content discovery or integrity scheme is introduced.
A package may import one JSON declaration into its TypeScript `provides` and give that same
declaration to the host; the two sides must not be separately hand-maintained. Its `settings`
entries are likewise one JSON file the package ships and the host is given; the TypeScript code
does not repeat them. A future PluginManager can produce these rows from its package boundary. Its proposed schema 2 is not assumed to be available, and the
private managed-provider schema 1 is not accepted here.

The static declaration is necessary because `RuntimeResolver` can describe a module only after
Node starts, while the host installs AgentBuilder's factories and Rpc's namespace list before
the loader starts. C4 does not move that list into a mutable registration service or bootstrap
a second runtime. On the existing online resolution, compare the Node description's `provides`
with the row snapshot before loading that row. A mismatch is a row resolution error; it does
not alter the configured names. The pinned SDK's initial offline placeholder has no `provides`;
only its existing runtime-source/unavailable-schema marker permits that provisional resolution.
The SDK later uses `resolve_again` to obtain the online declaration before publishing its
`RuntimeRows` catalog. An online description must match the snapshot, including when its actual
`provides` is empty; missing online declarations are not accepted as placeholders. The same
resolved factory retains the original schema, dependency declarations, scoped factory, metadata
and configuration validation.

After process reconciliation releases its loader operation, App Server's existing startup
readiness waits for the configured runtimes' `RuntimeRows` catalogs as well as their process
rows. It uses the existing startup deadline and failure cleanup. This orders Session creation
after declaration discovery; it does not wait for genuinely Pending optional Session rows.

Rows naming the same module must use the same snapshot; distinct row ids alone do not make
conflicting structural declarations valid.

At this external configuration boundary reject a group other than `session` or `branch`, malformed identities, unsupported method modes,
duplicate tools, duplicate namespaces, host-tool collisions and the C1 reserved-name/namespace
collisions. Disabled rows are not instantiated and contribute no callable methods. Their declared
tool identities remain known to ToolRegistry, so a stored `SessionConfig.tools` can omit a disabled
provider rather than fail as an unknown host tool. New model requests never declare that missing
tool. Enabled rows alone determine C1's known namespace list: a disabled namespace is
`unknown_method`, not an indefinitely reloading service. A declaration changing structurally
requires rebuilding the host assembly; business settings do not change it. Missing dependencies
of an enabled row keep their existing Pending/reload behavior.

## Tools

The service and data contract is [Session boundary A.5](contracts/session-boundary.md#a5-tool-contribution).
`declaration()` is synchronous; `execute(call, cancelled)` is asynchronous. The second argument
is the existing bridge's by-reference cancellation Promise, an implementation of A.5's cancel
action, not a new durable token, method or cancellation registry. It resolves on turn cancellation,
when the owning row unloads, and when the native call ends. The provider must release external
resources before settling.

The tool owner performs the only conversion between the neutral call and native execution:

| Neutral field | Source |
| --- | --- |
| `sessionId` | This Session's identity. |
| `branchId`, `turnId` | Existing `ToolExecutionContext`. |
| `callId`, `arguments` | Native `ToolCall.id`, `ToolCall.input`. |
| `config` | This Session's immutable `SessionConfig` serialized with its existing serde representation. |

Do not pass the native `{id, name, input}` object and silently treat it as the neutral shape.
The selected service identity already fixes the tool name. Declaration and result decoding use
the existing `ToolDefinition` and `ToolResult` serde types. The declaration's `name` must equal
the selected tool key, and a contributed tool supplies its required Permission risk. No second
schema parser or normalization pass is added after this boundary.

`ToolResult.content` is model-readable and uses the existing native JSON contract (text or typed
content blocks); `structuredContent`, blob references and background fields retain their existing
meaning. The shorthand block array in A.5 is not a restriction that removes native result fields.
A normal provider failure returns a valid `ToolResult` with `isError: true`. A bridge failure,
malformed result or unloaded provider becomes the existing host/tool failure and diagnostic;
it is never turned into an empty successful result. Cancellation uses ToolExecutor's existing
interruption result rather than inventing a provider error fact.

ToolRegistry reads the ordinary native contribution and neutral service identity when it
attaches a provider generation (A.5). Assembly permits only one configured owner per tool key;
no other provider is tried after that owner's declaration or execution fails. The foreign
service is looked up directly by its Session instance and bridge name. Its synchronous
`declaration()` is decoded there, and the handler retains that exact object together with the
original row context's existing `CancellationToken`, published under the same tool service name
and Session instance as a typed native key. Cancellation rejects later executions through this
retained handler and ends its in-flight native wait; it does not select a replacement generation
or force an uncooperative JavaScript task to stop. The TypeScript owner still settles its work
through its effects. There is no eager declaration during row apply, polling, asynchronous
attachment or changed `ToolContribution`
storage type. Synchronous declaration must not perform external asynchronous business work;
like a synchronous Rust provider it can delay its caller if it does not return.

Each configured foreign tool is a registry-owned versioned set (A.5, #241). Absent/Pending
providers omit only their tool and produce the existing attach diagnostic; once a row's
generation is active, the next request offers its tool without a Session reload. A malformed
foreign declaration is rejected when that generation is attached, omits only that provider's
tool, and records an explicit invalid-declaration diagnostic; the registry never caches an
invalid definition or an empty successful substitute, and does not retry the same generation.
A retired generation's set is withdrawn and the next active generation gets a new version.
ToolBinding adopts changes through Prompt/BranchOpening as for other sources, and each request
keeps its selected handlers. A new generation is not substituted into an old request.

## Session methods

`sessionMethods:<namespace>` uses C1's neutral `describe` and `call` service. C4 implements the
currently supported C1 subset; it does not implement connection-bound notifications, handoff,
RemoteGateway origin or a new process `callChannel` service.

`describe()` returns `MethodDecl[]`: `name`, `mutating`, `interruption` (`report` or `drain`) and
`progress`. Existing C1 validation applies. A declaration requesting unsupported notification
or handoff behavior fails the row rather than accepting work it cannot honor. Declarations
are decoded once for that provider generation, before its native `SessionMethods` is published.
Cancellation of the original row context also ends a pending `describe()` wait, so unload does
not depend on that declaration Promise settling.

`call` receives the existing neutral `{callId, method, params}` data plus the bridge cancellation
Promise and per-call progress callback. `params` includes the original `sessionId` and optional
`branchId`. `callId` is an opaque runtime correlation identity for this accepted call, not a
durable business identity or the client's JSON-RPC id. No unimplemented `origin` fields are
invented. A reply is `{kind: "value", value}`; an omitted `value` (including JavaScript `undefined`
omitted during JSON serialization) becomes JSON `null`. `{kind: "handedOff"}` is unsupported in
this subset.
Errors are data, `{code, message}`, decoded once into C1's wire error
([plugin contract](plugin-contract.md#9-requirements-raised-to-mechanism-owners) R3): the code
becomes `appError`, with JSON-RPC code -32602 for `invalid_params` and -32000 otherwise. A provider
may instead rethrow a service error it received; one with a `code` maps the same way, and a native
`AgentError` maps as it does for built-in methods. Provider-defined application codes must be
preserved rather than replaced with a success value or leaked runtime exception; C1's Rust error
storage must own the decoded string. A thrown exception without a code is a binding failure, not a
provider-defined application error, and exposes no stack or runtime internals to the client.

The binding composes the existing `RuntimeResolver`'s resolved factory in the **same row apply
context**. It first applies the runtime row and immediately registers that row's cleanup with
`ctx.effect`; it then obtains its own foreign service and calls `SessionMethods::provide` with
one handler for the generation. Cleanup order is therefore C1 admission/drain first, Node row
unload afterwards. No extra row, child capability or per-call `MethodProvider` is introduced.
Reading a service provided by this same apply context uses rutis' existing self access rule;
external required reads remain declared dependencies.

Rpc continues to select the longest known namespace, authorize the Session and acquire/restore
it through Agents. Listed but inactive answers from the part's row copy in that Session:
`service_reloading` while it is not started, Pending, Loading or between generations,
`service_unavailable` while it is Failed, Unresolved or Stopped
([management methods](appserver/management-methods.md#dispatch-and-lifecycle)); an unlisted
namespace is `unknown_method`. `session.methods` and actual calls read the native C1 service through the
existing Session facade. No arbitrary `HostDispatch` is made a public Session service.

C1 owns call admission, progress closure, interruption and waiting for accepted work:

| Situation | Result |
| --- | --- |
| Provider or admitting Rpc generation unloads | Its cancellation Promise resolves. `report` replies `operation_interrupted` and settles the work; `drain` returns the provider's own reply. Both settle before provider unload finishes. |
| Client disconnects | Existing C1 behavior: work continues; that client's progress/reply are discarded. |
| Provider reloads | Retained native provider rejects new calls. No call crosses to the new generation. |
| Progress after reply / undeclared progress | Existing `Progress` drops it; no second queue. |

The by-reference Promise is deliberate: dropping a remote future can discard its eventual reply,
which cannot implement `drain`. The native call ends its Promise lease on normal completion too.
TypeScript code cooperatively settles after cancellation; no timeout or forced-success reply
is invented for an uncooperative plugin.

`rows.load` can reply before the native projection publishes its exported services. The original
row subscribes to the existing `ServiceChanged` event before loading and waits for all its own
declared tool and method-service projections before its apply returns and it becomes Active.
C1 binds the method namespaces during that apply; tool declarations remain unread until
ToolRegistry attaches the row's generation. The row context owns the subscription and cancellation ends the wait.
The foreign cleanup is registered immediately after loading, so a failed declaration or cancelled
binding still releases that row. This wait does not make another optional row required or change
Session publication.

## Native Session broadcast

This is the #168 implementation of the approved #174 contract.
Observation grants no branch write or context-input path; critical ordered
consumption retains the existing Dispatcher/C5 boundary.

### The Session channel

SessionBroadcast emits a Session's records on its channel `session/{sessionId}`
([Session broadcast](../architecture/rust-agent/event-driven-flows.md#6-session-广播)). rutis has one bus per root
([rutis usage §5](rutis-usage.md#5-the-event-bus)), so a listener in the Node runtime receives the channel's records
directly. The host relays nothing and declares no event names; there is no forwarding Capability, queue or public
subscription registry.

A record's serde form is the SDK type `SessionBroadcast` ([npm/plugin.d.ts](../../npm/plugin.d.ts)):

| `kind` | Fields | An observer |
| --- | --- | --- |
| `event` | `branchId`, `event` (the unchanged `AgentEvent` serialization) | Consumes it |
| `notice` | `notice: {source, kind, data}` ([Session broadcast notices](appserver/session-notices.md)) | Skips it; C6 and Rpc own notices |
| `fence` | `recipient`, `token` | Skips it; a fence belongs to its recipient |
| `closed` | — | Skips it; closing ends the row's work (below) |

- A branch event reaches the listener after its branch dispatches it. A row listens from its `apply`, so it
  receives the Session's records from then on, Loading included.
- Records of one Session start in emit order. An `async` listener does not hold back the next record, and a slow
  one delays neither native delivery, Rpc notifications nor fences. Nothing orders Sessions against each other.
- Nothing is replayed: a listener registered later sees only later records, including after a row reload or a
  restore.
- The host does not filter records by Session closing or unloading; a row stops admitting work when
  `sessionLifecycle.closing()` settles or the row unloads. Unloading a row waits for its running listeners up to the host's shutdown deadline.
- A throwing listener's failure goes to the rutis error sink. It does not stop later records and writes no Timeline
  fact.

A row that needs strict order consumes facts through `dispatcher` ([branch binding](branch-typescript.md)).

### Session identity, configuration and lifecycle

A Session row reads its Session from the Session services every language has
([plugin contract](plugin-contract.md#9-requirements-raised-to-mechanism-owners) R2), injected by name:

| Service | Contract |
| --- | --- |
| `sessionIdentity` | `get()` returns `{sessionId, restored}`. |
| `sessionConfig` | `get()` returns the creation-time SessionConfig in the serde form ToolCall uses. |
| `sessionLifecycle` | `activated()`, `closing()` and `hold(released)`, the phases of the Rust `SessionLifecycle`; a hold lasts until `released` completes or the caller's row unloads. |

There is no setup call and no `sessionEvents:<part>` entry: the row registers
``ctx.on(`session/${sessionId}`, listener)`` in its `apply` with the id it read. The channel name is
the routing, not a sandbox: trusted plugins can listen to any channel or emit events. Do not infer
identity from the first event or a runtime handle.

`sessionConfig` keeps the creation-time form's rule: absent `interactive` means true; only false is
serialized. SessionTitle tests `config.interactive !== false && config.collaborationRole === 'leader'`,
never truthiness of the omitted field. No consumer fills or rewrites the snapshot. It does not report
later Prompt/model adoption.

Event handlers register owned tasks and return. When `closing()` settles or the row unloads, stop
admitting work, cancel/release resources and join accepted work through existing row effects.
`ctx.on` removes the listener on unload; captured handlers do not start work after their generation
ends. No callback or work is rebound.

### Acceptance

`fence` and `closed` records belong to their owners. Foreign lifetime ends through closing and row
effects, with no additional end frame or fabricated `AgentEvent`. C6 retains notice/Rpc ownership:
its `{source, kind, data}` notice records ([Session broadcast notices](appserver/session-notices.md))
reach a listening row as `notice` records, which an observer skips. SessionTitle observes `input/applied`;
its stored title notice reaches clients through C6/Rpc, not this TS binding.

Settings events are outside #168: a row listens to `settings/{source}` and
`settings/projects` itself ([settings-service §4.4](appserver/settings-service.md#44-events)).
The Session channel does not carry them.

Real Node/App Server flows cover: two rows in one Session each receive one copy; another Session
receives only its records; order/content match native history. A leaf listener held pending does
not hold back the following native records or an attached client's fence acknowledgement; once
released, delivery is ordered and single-copy. Closing while a listener is held starts no new work
after closing. Delivery from `apply`, late listeners without replay, and interactive true/false plus
leader/member eligibility using the serialized creation snapshot are checked. Owned work held
through close and row-only reload settles before effects finish, stale listeners stop, and
native/other-Session use continues. A Session row that names a branch service fails at composition,
naming the branch group, without affecting native or other rows. Throwing listeners, Pending and
disabled rows, tools that follow their row's activation (#241) and C1 drain keep their behavior.

## Lifetime and failure

Loader owns generations and rows. Session closure stops branches before unloading Session
parts. Each TS part immediately registers its acquired resources with its own `effect`, stops
new work on disposal, releases I/O, settles accepted work and rejects retained service objects.
A retained remote object must reject calls after the part's effect begins unloading.
The native C1 effect drains while the Node part is still alive. Tool cancellation follows its
existing turn owner and the original row's context cancellation; no work transfers to another
generation.

Service and effect registration happen in TS apply, and business work is driven by accepted
calls, Session channel records, or background tasks that wait for `sessionLifecycle.activated()`
and end at `closing()`. Row Active alone never permits business work from TS apply.

Session assembly uses the registered required-part membership implemented by [#137](https://github.com/arcships/dim/pull/137).
Session assembly settles rows already Loading, including their tool/method projection waits.
Host-appended optional rows stay in the original disposal group and diagnostics and do not enter
the Session's required Active wait: a row genuinely Pending on a missing dependency does not
delay native Session use. Their hard dependencies, key validation and failure reporting do
not weaken. Disabled parts have no row instance. An optional failure changes only its features;
there is no automatic retry of an operation that may have produced external side effects.

Restore rebuilds rows and reads and follows providers through the same code as create; history does not
install or enable code. Structural reload can fail in-flight calls as ADR 002 permits. Runtime
process loss is an infrastructure failure; no special runtime-crash recovery mechanism is added.
Completely removing a configured tool identity differs from disabling its row: the existing
Session tool validation rejects an unknown saved name. C4 does not silently edit persisted
Session configuration or introduce a history-driven reinstallation path. Retain the disabled
structural declaration when existing Sessions need to remain loadable; package-uninstallation
policy belongs to PluginManager rather than an undocumented fallback in ToolRegistry.

## Impact and verification

| Area | Implementation and remaining boundary |
| --- | --- |
| App Server `ServerConfig` and assembly | Public embedding input, row boundary validation, names/catalog derived from `provides`, optional Session rows and native resolver factory composition. |
| ToolRegistry and its page | Native/foreign owner selection and declaration decoding when a generation is attached. Existing ToolBinding, Prompt and ToolExecutor flow is reused. |
| C1 `methods` module | Neutral DTO/error decoding and generation-owned foreign handler construction. No Rpc dispatch replacement. |
| Session broadcast | Rows listen to `session/{sessionId}` themselves, with the id from `sessionIdentity`. C6 retains notices. |
| Dependencies | Existing pinned `rutis-bridge` at the language-boundary owners; no new library or rutis change. Regenerate graphs through `xtask graph`, then run `xtask check`. |
| Documentation | Owner pages, App Server README and Session-boundary Appendix F updated to the actually implemented subset. |
| Tests and fixture | One real App Server/Node flow using a disposable home and local Rpc; focused malformed-boundary cases in that flow. |

The executable Node/Rpc flows cover two Sessions sharing a Node PID; an Active tool declared on the immediate
first model request after create and restore and executed through ToolExecutor; a pending optional
part that does not block native use; a later Active tool offered from the next request without a reload; disabled rows;
actual C1 Rpc call/listing, progress, `report` and `drain`; stale provider rejection; continued
native/other-Session use after one part closes. Native Session broadcast flows additionally cover single-copy delivery to two rows,
Session/branch routing and native content/order, delivery from `apply`, client
notifications/fences while a leaf stalls, and continued delivery to another Session.
Row-only reload ends owned work without replay, idle unload ends the observer's generation
and a restore's rows observe again, and close/shutdown settle held listeners.
An Agents index restart leaves the observer and tool/method rows' generations unchanged;
subsequent records retain single-copy delivery and native order.
Creation snapshots cover interactive/non-interactive leaders and members; a misplaced row and
throwing listeners keep native and other-row execution available. These flow checks use real services and fences.
Gate each observed transition on an actual service/event/fence, not a sleep. Boundary cases cover
declaration mismatch, invalid tool/method reply, name collisions and cancellation without leaking
a Promise lease. Reuse C3/C1 tests for their unchanged native semantics.
Native-store fork creates fresh Session-scoped providers from the copied configuration, and
delete drains the fork's admitted method before unloading it while its source remains usable.

The existing CI Node step executes the product Node/Rpc flows alongside the managed-provider
flow. Ordinary Session assembly covers the immediate first request after create and restore,
including a tool-only module with no method namespace or test-only required dependency.
Separate Pending and disabled rows verify native continuity. Row-only reload exercises in-flight
tool cancellation and rejection through retained handlers, and unload during a stalled `describe`
exercises binding cancellation. Invalid tool/method declarations, malformed replies, omitted
method values, online snapshot mismatch and conflicting module snapshots remain boundary cases
in those flows. The protected embedding example prints durable history; submit/idle success
alone does not certify that a TypeScript tool executed.

The spike and local native-event experiments remain evidence only for their stated boundaries.
The executable Node/App Server acceptance is calibrated on macOS/Unix. Native Windows
Node delivery is unverified; the CLI Windows checks do not establish C4 support there.

## Ablation

`provides` is retained solely because the existing assembly needs names before Node is online;
all names and service modes are derived from that one existing dictionary shape. A second
manifest, separate name arrays, registration manager and per-call provider are unnecessary.
Same-row factory composition is retained because C1 must drain before unloading its foreign
handler. The tool declaration stays at first registry read, so no deferred contribution wrapper
or eager tool registration is needed. Promise cancellation preserves the existing `drain` reply.
Startup uses the existing process readiness loop and SDK catalog publication; the row's own
service-publication wait covers tools and methods without moving declaration reads. Retaining
the original row's token ends stale native tool waits without a tracker, per-call subscription or
new liveness API. Rows listen to the Session channel themselves, so there is no relay, per-row
transport or duplicate emission; identity, configuration and closing come from the Session
services every language reads, so there is no setup method.

## Rule check

| Rule | Application |
| --- | --- |
| §4 rule 1 | No binding writes another owner's state. A business method uses its owner's existing service or command event; the binding does not produce its result facts. |
| §4 rule 2 | ToolExecutor and C1 retain accepted generation-owned work; no rebinding or revocation of a handed-over call. |
| §4 rule 3 | No before callback; boundary checks cover shapes/identity, not feature decisions. |
| §4 rule 4 | Core has no tool/plugin-specific branch. Neutral bindings live with their contract owners. |
| §4 rule 5 | No new Timeline batch. Business owners retain their existing atomic writes and dispatch order. |
| §4 rule 6 | Model-visible declarations use Prompt adoption; model-visible content uses adopted inputs and ToolResult only. |
| §4 rule 7 | Named services, C1 and the native Session channel; no private state reads. |
| §4 rule 8 | Existing resolver, catalog, factory, scope, event and effect APIs; no infrastructure API, optional-dependency semantics or rutis requirement. |
| §4 rule 9 | Declarations and transient notifications stay out of Timeline; C6 notices retain their own owner. |
| §1 | Each business part is a Session foreign row; existing NodeRows owns shared runtime I/O. `sessionLifecycle.closing()` stops owned work. |
| ADR 002 D1 | Session state belongs to that part; Node and its loader resources are shared process resources. Branch facts remain branch-owned. |
| ADR 002 D2 | No new wiring capability or forwarding owner: existing NodeRows and original-row binding own their resources. No registration host or method aggregation plugin. |
| ADR 002 D3 | Core never hard-depends on optional parts; ToolRegistry reads tool providers by name and follows their activation (#241), and per-call C1 lookup is unchanged. Required dependencies remain required. |
| ADR 002 D4 | Only structural rows enter loader input; business Settings changes do not reload parts. |
| ADR 002 D5 | Canonical named service/key, data, errors and cancel action; Rust/native details are bindings. Existing unsupported C1 features are explicit. |

## Requirements raised to mechanism owners

None. #137 supplies the required assembly membership rule. Static declaration discovery,
owner-level bindings and native event composition are C4 choices within existing mechanisms.
The Session services are the plugin contract's R2; the native Session channel provides delivery
without another readiness mechanism. C6 retains notices.
