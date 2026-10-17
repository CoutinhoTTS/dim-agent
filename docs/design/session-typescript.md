# Session TypeScript parts

Status: tool/method bindings implemented for [C4 #63](https://github.com/arcships/dim/issues/63).
Native Session broadcast uses the rutis 0.11 leaf-event route tracked by
[#168](https://github.com/arcships/dim/issues/168); its binding review, implementation and full
C4 acceptance remain pending. The [spike](session-typescript-spike.md)
establishes transport feasibility; its private service names and combined fixture are not this API.

## Scope and ownership

A TypeScript Session part supplies named tools and Session methods and observes its Session's
broadcast. One Node process serves all such parts; loader instances isolate their services.
The Session owns the part's state and lifetime. Native execution remains available when a part
is absent, Pending, failed or reloading. Instance labels are namespacing, not a sandbox.

The existing owners implement their language bindings:

| Owner | Responsibility |
| --- | --- |
| App Server assembly | Validate structural input, register names derived from declarations, append ordinary rows inside `SESSION_GROUP`, select the existing runtime resolver. |
| ToolRegistry | Read a named foreign declaration at its existing first-read sample and retain that provider generation in the fixed table. |
| C1 `SessionMethods` | Bind a foreign handler to the existing generation-owned admission, progress, interruption and drain. |
| Session broadcast | Encode its own records for the native rutis event path. |
| TypeScript part | Own its business state, work, effects, errors and cancellation settlement. |

There is no registration plugin, event-forwarding service, second bus, optional dependency,
new Session facade method or readiness phase. Timeline, Dispatcher and persisted event formats
remain unchanged. This work does not expose arbitrary runtime services through the Session facade.

## Host input and declaration discovery

The initial usable entry is the public App Server embedding API:
`serve(builder, ServerConfig { session_typescript: Some(...), .. })`. It runs an actual App Server and its
normal Rpc path. The [embedding example](../../crates/dim-app-server/examples/session_typescript.rs)
uses a protected Unix connection and a disposable storage home. General CLI installation,
marketplaces and editable plugin intent remain PluginManager work.

The input contains the existing `LocalRuntime::node` arguments (`node`, `jsRuntime`, `anchor`,
absolute paths) and `rows`, an array of ordinary loader row objects. One Session TypeScript
runtime is shared across the rows and Sessions. Its runtime name differs from an account runtime;
no private account manifest or account service is required by this entry.

Each row retains loader's `id`, `name` (module entry), `config`, `disabled`, `inject` and `isolate`
semantics. The C4 row metadata is `provides`, with **exactly the existing
`rutis_bridge::runtime::RowSchema.provides` shape**, rather than a parallel tool/namespace list,
and, only for a package that owns Settings sources, `settings` (below):

```json
{
  "id": "web",
  "name": "/absolute/web/index.ts",
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
key; the namespace keeps its dots. The catalog registers each full name as a shared service in
`SESSION_GROUP`. Tools and known namespaces are derived from these names and passed to the
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

At this external configuration boundary reject malformed identities, unsupported method modes,
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

ToolRegistry samples the ordinary native contribution and neutral service identity at its
existing `LazyLock` initialization. Assembly permits only one configured owner per tool key;
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

The A.5 sample stays at branch initialization. Absent/Pending providers omit only their tool
and produce the existing attach diagnostic. A malformed foreign declaration is rejected at
that boundary, omits only that provider's tool, and records an explicit invalid-declaration
diagnostic; the registry never caches an invalid definition or an empty successful substitute.
This preserves the existing infallible fixed-table read instead of extending a core interface.
A provider active after the sample waits for the
next registry/Session reload, even if it starts before publication. ToolBinding still adopts
the fixed table through Prompt/BranchOpening before accepting input, and each request keeps
its selected handlers. A new generation is not substituted into an old request.

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
Errors are the existing neutral `{code, appError, message}` data, decoded once into C1's wire
error. Provider-defined application codes must be preserved rather than replaced with a success
value or leaked runtime exception; C1's Rust error storage must own the decoded string. The
fulfilled call returns either the value reply or that error object. A thrown transport/runtime
exception is a binding failure, not a provider-defined application error, and exposes no stack
or runtime internals to the client.

The binding composes the existing `RuntimeResolver`'s resolved factory in the **same row apply
context**. It first applies the runtime row and immediately registers that row's cleanup with
`ctx.effect`; it then obtains its own foreign service and calls `SessionMethods::provide` with
one handler for the generation. Cleanup order is therefore C1 admission/drain first, Node row
unload afterwards. No extra row, child capability or per-call `MethodProvider` is introduced.
Reading a service provided by this same apply context uses rutis' existing self access rule;
external required reads remain declared dependencies.

Rpc continues to select the longest known namespace, authorize the Session and acquire/restore
it through Agents. Listed but inactive is `service_reloading`; an unlisted namespace is
`unknown_method`. `session.methods` and actual calls read the native C1 service through the
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
ToolRegistry's first read. The row context owns the subscription and cancellation ends the wait.
The foreign cleanup is registered immediately after loading, so a failed declaration or cancelled
binding still releases that row. This wait does not make another optional row required or change
Session publication.

## Native Session broadcast

The selected transport is rutis 0.11 leaf events, as required by
[#168](https://github.com/arcships/dim/issues/168). The existing shared
`LocalRuntime` declares a fixed event name with `emits`; the host uses
`Process::emit`, and TypeScript leaf parts subscribe with `ctx.on`.
This route needs no per-row link, peer identity, token or configuration replacement.
The dependency upgrade makes these APIs available; the C4 binding is not implemented.

The short binding contract must settle these owner-level details before implementation:

| Boundary | Required behavior |
| --- | --- |
| Event name and data | One fixed name; payload `{sessionId, branchId, event}`. `event` is the existing `AgentEvent` serialization, without filtering or conversion. |
| Session isolation | All rows share the runtime process's event bus. The binding checks `sessionId` before invoking that row's business listener; this is routing for trusted plugins, not a security sandbox. |
| One delivery | Emit each native Session record once into the shared process. Several rows of one Session must not each re-emit the same record to every row. The projection's existing owner and lifetime must be explicit. |
| Activation | Register during apply; business delivery waits for the existing `SessionLifecycle::activated()`, not row Active. Eligibility is checked when the native listener runs: a captured pre-activation emission whose callback runs after activation may be delivered. No replay for emissions without a registered listener. |
| Order | Preserve native channel order and each event's branch sequence; no total order between parallel branches. The branch Dispatcher must not wait for Node I/O. |
| Unload | End subscriptions with their original generation, settle accepted delivery and reject stale callbacks. No work is rebound to a replacement row. |

Native `SessionBroadcastEvent` remains the source. Host `Fence` records are
observation machinery and never business data. Its `Closed` record is distinct
from an `AgentEvent`; its neutral end representation and the single Session-owned
projection still require binding review. The event payload above does not silently
encode closure as a fabricated Timeline event. C6 retains ownership of its
`{source, kind, data}` notices and Rpc forwarding contract
([C6 #49/#85](https://github.com/arcships/dim/issues/49)).

The tool and method bindings do not depend on broadcast. Full C4 acceptance
requires its actual isolation, activation, order and unload flows. No delivery
registry, durable cursor, acknowledgment protocol, retry loop or core lifecycle
API is added by the dependency upgrade.

## Lifetime and failure

Loader owns generations and rows. Session closure stops branches before unloading Session
parts. Each TS part immediately registers its acquired resources with its own `effect`, stops
new work on disposal, releases I/O, settles accepted work and rejects retained service objects.
A retained remote object must reject calls after the part's effect begins unloading.
The native C1 effect drains while the Node part is still alive. Tool cancellation follows its
existing turn owner and the original row's context cancellation; no work transfers to another
generation.

The tool/method subset permits synchronous service/effect registration in TS apply, and business
work driven by accepted calls after Session publication. It does not yet expose a neutral
Session activation service for arbitrary autonomous TS background tasks. Such tasks must not be
started from TS apply on the assumption that row Active means Session published. Native broadcast
delivery's activation path remains part of the binding review above; no new startup phase
or API is proposed to hide this distinction.

Session assembly uses the registered required-part membership implemented by [#137](https://github.com/arcships/dim/pull/137).
Session assembly settles rows already Loading, including their tool/method projection waits.
Host-appended optional rows stay in the original disposal group and diagnostics and do not enter
the Session's required Active wait: a row genuinely Pending on a missing dependency does not
delay native Session use. Their hard dependencies, key validation and failure reporting do
not weaken. Disabled parts have no row instance. An optional failure changes only its features;
there is no automatic retry of an operation that may have produced external side effects.

Restore rebuilds rows and samples providers through the same code as create; history does not
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
| ToolRegistry and its page | Native/foreign owner selection and declaration decoding at its existing first read. Existing ToolBinding, Prompt and ToolExecutor flow is reused. |
| C1 `methods` module | Neutral DTO/error decoding and generation-owned foreign handler construction. No Rpc dispatch replacement. |
| Session broadcast | Pending: owner serialization, native projection and activation delivery after their contracts are settled; no hard dependency on optional transport. C6 owns notice data. |
| Dependencies | Existing pinned `rutis-bridge` at the language-boundary owners; no new library or rutis change. Regenerate graphs through `xtask graph`, then run `xtask check`. |
| Documentation | Owner pages, App Server README and Session-boundary Appendix F updated to the actually implemented subset. |
| Tests and fixture | One real App Server/Node flow using a disposable home and local Rpc; focused malformed-boundary cases in that flow. |

The executable Node/Rpc flows cover two Sessions sharing a Node PID; an Active tool declared on the immediate
first model request after create and restore and executed through ToolExecutor; a pending optional
part that does not block native use; a later Active tool omitted until reload; disabled rows;
actual C1 Rpc call/listing, progress, `report` and `drain`; stale provider rejection; continued
native/other-Session use after one part closes. Native Session broadcast isolation and unload
remain acceptance requirements for the pending broadcast binding.
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
Full C4 acceptance, broadcast delivery and cross-platform evidence remain pending.

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
new liveness API. The additional native peer remains a reviewed transport decision, not an
assumed requirement.

## Rule check

| Rule | Application |
| --- | --- |
| §4 rule 1 | No binding writes another owner's state. A business method uses its owner's existing service or command event; the binding does not produce its result facts. |
| §4 rule 2 | ToolExecutor and C1 retain accepted generation-owned work; no rebinding or revocation of a handed-over call. |
| §4 rule 3 | No before callback; boundary checks cover shapes/identity, not feature decisions. |
| §4 rule 4 | Core has no tool/plugin-specific branch. Neutral bindings live with their contract owners. |
| §4 rule 5 | No new Timeline batch. Business owners retain their existing atomic writes and dispatch order. |
| §4 rule 6 | Model-visible declarations use Prompt adoption; model-visible content uses adopted inputs and ToolResult only. |
| §4 rule 7 | Named services, C1, public Session methods and native broadcast only; no private state reads. |
| §4 rule 8 | Existing resolver, catalog, factory, scope, event and effect APIs; no infrastructure API, optional-dependency semantics or rutis requirement. |
| §4 rule 9 | Declarations and transient notifications stay out of Timeline; C6 notices retain their own owner. |
| §1 | Each business part is a Session Capability/foreign row. App Server validates and assembles; acquired resources are immediately owned by effects. |
| ADR 002 D1 | Session state belongs to that part; Node and its loader resources are shared process resources. Branch facts remain branch-owned. |
| ADR 002 D2 | No wiring capability, registration host, event-forwarding service or method aggregation plugin. |
| ADR 002 D3 | Core never hard-depends on optional parts; first-read tool lookup and per-call C1 lookup are unchanged. Required dependencies remain required. |
| ADR 002 D4 | Only structural rows enter loader input; business Settings changes do not reload parts. |
| ADR 002 D5 | Canonical named service/key, data, errors and cancel action; Rust/native details are bindings. Existing unsupported C1 features are explicit. |

## Requirements raised to mechanism owners

None. #137 supplies the required assembly membership rule. Static declaration discovery,
owner-level DTO bindings and native event composition are C4's implementation choices within
existing mechanisms, not grounds for a new core or rutis capability. The unresolved broadcast
handoff is reviewed in this C4 design, with C6 retaining its notice-record responsibility.
