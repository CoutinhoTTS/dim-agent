# Session TypeScript parts

Status: tool/method bindings implemented for [C4 #63](https://github.com/arcships/dim/issues/63).
Native broadcast choices require owner alignment; broadcast implementation and full C4 acceptance
remain pending. The [spike](session-typescript-spike.md)
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
| Session broadcast | Supply the existing typed records and their existing `AgentEvent` serde value at the language boundary. |
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
semantics. The only C4 row metadata is `provides`, with **exactly the existing
`rutis_bridge::runtime::RowSchema.provides` shape**, rather than a parallel tool/namespace list:

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

`toolContribution:<tool>` and `sessionMethods:<namespace>` encode the existing neutral service
identity and key as one bridge service name. The colon separates the family from the complete
key; the namespace keeps its dots. The catalog registers each full name as a shared service in
`SESSION_GROUP`. Tools and known namespaces are derived from these names and passed to the
existing `AgentBuilder::session_tool` / `session_methods` during assembly. Authors do not make
these Rust calls and do not separately maintain arrays of tool names and namespaces.

This is structural input, not a new package manifest or persistence format. No filename, package
schema, version field, content discovery or integrity scheme is introduced. A package may import
one JSON declaration into its TypeScript `provides` and give that same declaration to the host;
the two sides must not be separately hand-maintained. A future PluginManager can produce these
rows from its package boundary. Its proposed schema 2 is not assumed to be available, and the
private managed-provider schema 1 is not accepted here.

The static declaration is necessary because `RuntimeResolver` can describe a module only after
Node starts, while the host installs AgentBuilder's factories and Rpc's namespace list before
the loader starts. C4 does not move that list into a mutable registration service or bootstrap
a second runtime. On the existing online resolution, compare the Node description's `provides`
with the row snapshot before loading that row. A mismatch is a row resolution error; it does
not alter the configured names. The same resolved factory retains the original schema,
dependency declarations, scoped factory, metadata and configuration validation.

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
action, not a new durable token, method or cancellation registry. It resolves on turn cancellation
and when the native call ends. The provider must release external resources before settling.

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
`declaration()` is decoded there, and the handler retains that exact object. There is no eager
declaration during row apply, polling, asynchronous attachment or changed `ToolContribution`
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

`call` receives the existing neutral `{callId, method, params}` data plus the bridge cancellation
Promise and per-call progress callback. `params` includes the original `sessionId` and optional
`branchId`. `callId` is an opaque runtime correlation identity for this accepted call, not a
durable business identity or the client's JSON-RPC id. No unimplemented `origin` fields are
invented. A reply is `{kind: "value", value}`; `{kind: "handedOff"}` is unsupported in this subset.
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
row subscribes to the existing `ServiceChanged` event before loading, reads only its declared
method-service keys, and waits for their publication before binding C1. Its context owns the
subscription; context cancellation ends the wait. The foreign cleanup is registered immediately
after loading, so a failed declaration or cancelled binding still releases that row. This wait
does not make another optional row required or change Session publication.

## Native Session broadcast

**Proposed, not implemented.** This binding receives the current Session's existing Event facts.
The original Config convention, activation behavior and transport integration require confirmation
before implementation. Tool/C1 bindings above are independent of that approval.

### Source and payload

The original row subscribes to `session_channel(SessionIdentity.session_id)`. Its
`SessionBroadcastEvent::Event { branch_id, event: Arc<AgentEvent> }` already supplies the complete
data. At the Rust-to-Node boundary, build `{branchId, event}` with the existing
[`AgentEvent` / `EventKind` Serialize implementation](../../crates/dim-agent-types/src/events.rs);
workspace serde already supports `Arc`. The SDK `NodeEvent.args` is `[{branchId, event}]`, so the
Cordis handler receives one record. The source channel and row's Link select its Session.

Preserve every Event kind and its native fields, sequence and safe-integer contract. Do not
substitute Rpc's filtered `ProductEvent`, rewrite controls/model/prompt facts, or convert to the
legacy v1 union. Existing [wire](../../crates/dim-agent-types/tests/wire_contract.rs),
[control](../../crates/dim-agent-types/tests/control_contract.rs) and
[branch](../../crates/dim-agent-types/tests/branch_contract.rs) fixtures stay unchanged. Serialization
errors follow the listener's existing error path. No new owner codec, record type or core API is
needed to serialize an already defined type at this external boundary.

`Fence` remains host observation machinery. A row inside the Session ends through closing and
disposal and is not guaranteed to receive `Closed`; an end-record observer must outlive the
Session ([event-driven flows §6](../architecture/rust-agent/event-driven-flows.md#6-session-广播)).
Do not translate this into Rpc `session.closed`, whose owner also waits for trace/resource
settlement. C6's Notice payload and arbitrary TS publication are separate contracts.

### Original-module Config

The proposed opt-in is `config.nativeEvents: null` on an ordinary loader row. Absence requests no
binding or transport resources. This is a new structural Config convention to confirm, not an
existing RowSchema/metadata declaration or an inference from `provides`.

```json
{
  "id": "session-observer",
  "name": "/absolute/session-observer/index.ts",
  "config": {"nativeEvents": null},
  "provides": {}
}
```

The original module explicitly admits this optional field in its Config schema: `null` for the
request and the following Link object for execution. App Server accepts only an absent or null
external marker; it derives the object during that original row's apply and replaces only the
marker before the original factory runs. The original factory/schema checks and Cordis
`plugin.Config` validate the realized config. Other members and original inject/provides
contracts retain their owners; unknown fields are not stripped or admitted implicitly.

| Realized `nativeEvents` field | Existing SDK contract | Producer / consumer |
| --- | --- | --- |
| `peer` | `Link.peer`, expected remote endpoint | Shared host listener identity; consumed by Link and Events.peer |
| `id` | `Link.id`, this Node endpoint | A fresh SDK PeerId per apply, using the existing UUID facility; also this row's native event name |
| `dial` | `Link.dial` | Host's actual loopback listener endpoint, consumed by Link |
| `token` | `Link.token` | Fresh credential accepted by this row's named SDK identity; consumed by the existing token handshake |

These are structural runtime inputs, never Settings, Timeline data, logs or replies. Loader/fiber
identity remains distinct from the transport peer: reapply uses a fresh peer id even when the row
identity stays the same. Queued events retain their old key and cannot select a new generation's
listener through a reused name. No separate event-name field, generation counter or peer-id list
is added.

The module starts native Cordis Link and Events in its own row, with local isolation of
`rutisPeer.<peer>` / `rutisLink.<peer>` services. `Events.in: [nativeEvents.id]` and
`ctx.on(nativeEvents.id, handler)` use the same unique name. Service isolation alone does not
filter the shared Cordis event bus. The small `definePlugin` context has no event API; this binding
uses a native Cordis module.

The pinned RuntimeResolver JsFactory permits deferred `inner.build`: its inject keys match the
resulting row, and its native config/instance validators add nothing beyond the defaults. BoundRow
can retain that factory/config and realize it in apply. Node still validates the original Config
in `rows.load`. This does not authorize deferring meaningful validators of arbitrary factories.

Initial apply and full reapply are covered by this construction; volatile updates are not.
Pinned loader `JsRow::start` forwards `VolatileUpdate.config` directly to `Process::update_row`;
Node `updateRow` commits volatile references or restarts its Cordis row, bypassing BoundRow apply.
A raw null marker would overwrite the realized Link. Current App Server embedding uses
`reconcile(..., None)` without an editable layer. That is the current entry's boundary, not proof
of mutable-row support. This proposal preserves that update path and records the limitation;
it adds no interceptor or second config authority. PluginManager remains responsible for editable
intent; mutable opted-in rows are outside this proposal's acceptance claim.

### Existing transport handoff and owners

```text
SessionBroadcastEvent on session/<Session id>
  -> original row: synchronous phase/cancellation check and existing serde
  -> emit(node_event(Link.id), NodeEvent { args: [{branchId, event}] })
  -> existing SDK EventsPlugin on this row's unique key, same rutis bus
  -> peer events.forward -> Cordis Events.in: [Link.id]
```

`NodeEvent` and `node_event(name)` are existing SDK transport definitions. The Session callback
projects and emits synchronously; SDK's per-key ordered chain owns network/handler waits. This
obeys the Session listener's no-I/O-wait contract in event-driven flows §6 without a product
forwarding service, wiring row, custom queue/worker or second bus. Forwarding the canonical Session
name independently through several peers duplicates delivery in shared Cordis; each link instead
uses its unique id in both directions of this one-way path.

The existing SessionNodeRuntime owns one shared loopback listener. Each original row owns its
named StaticIdentity, listening peer/link, SDK EventsPlugin and source subscription, registering
cleanup immediately. Each apply uses `StaticIdentity::new(host_peer).accept_token(token, row_peer)`;
its LinkConfig names that identity and the shared listener. The host endpoint is common, while
credentials and peer registration are row-owned.

Pinned `rutis-bridge` 0.10.0 supports this through `identity.rs` named identities, `link.rs` named
identity lookup and per-link Registration, and `registration.rs::Registrations` credential checks
against each live registration's identity and generation. A central credential registry or fixed
row whitelist is unnecessary. The synthetic probe used a fixed three-token identity and did not
verify this dynamic composition inside actual BoundRow.

The existing raw loader endpoint starts with Mount::default and does not serve events.forward;
the native peer path targets the shared Cordis context instead. Measured transport cost is one
shared listener, one native peer/link and one Node I/O worker per receiving row, alongside the
existing loader channel. There is no RSS, throughput or maximum Session-count guarantee. The
PeerId-alias probe established two rows receiving one copy each, second-source isolation and
independent unload. It awaited peer calls directly; it did not verify the NodeEvent handoff,
real Session activation, Config realization or dynamic identities. Those are product acceptance.

### Apply and disposal

1. Obtain declared Session identity/lifecycle and shared runtime dependencies. Provide the row's
   named identity and start its listening peer/link with immediate cleanup registration. Do not
   wait for a connection before Node can dial it.
2. Build/apply the original JS factory with realized Config and register its cleanup immediately.
   Its native module starts Link and Events.
3. Wait cancellably for this row's peer Ready and events feature, then install SDK EventsPlugin
   with outbound `[Link.id]` and no inbound names. Await its existing fiber registration and
   register the typed Session listener before apply returns. Another optional row stays independent.
4. The source callback uses its captured row context/token and synchronously checks Session phase
   Active before emitting. Loading/closing/cancelled callbacks return without delivery. It does
   not await activated() inside the Session chain. Failed Sessions never activate; facts before
   subscription or activation are not buffered or replayed. This startup-delivery behavior requires
   confirmation; autonomous startup work is outside this binding.

The wrapper retains the original inject keys and declares every external SDK dependency; its
own named identity uses existing self access. Peer/readiness waits observe row cancellation and
Session closing. The callback's supplied Ctx belongs to the emitter and is not its cleanup owner.

Pinned rutis `bus.rs::emit` snapshots listeners synchronously and chains dispatch per exact key.
Session emit order therefore reaches each row's NodeEvent key in order, where SDK EventsPlugin
waits for each forward and settlement. Branch sequence stays unchanged; other keys do not await
that chain, though rows share Node's event loop and transport resources. Local acceptance is not
remote completion.

On disposal, stop new source handoffs, unregister the source and SDK subscription, then explicitly
close the captured peer connection with existing `Connection::close` before releasing foreign
resources. This synchronous close interrupts blocked transport, closes the call table and finishes
outstanding waits; it does not depend on an aborted Link task having been polled. Existing C1
drain still precedes foreign unload. The original module settles its own accepted work through
its effects; closing a link cannot revoke an already consumed fact or forcibly stop a TS handler.

Exact-key unsubscribe does not join captured snapshots. SDK EventsPlugin's Forward holds the old
Arc<Peer>, ignores callback Ctx and does not select row cancellation; a captured old forward must
fail against its closed connection, not select a new generation. LinkPlugin task abort alone is
not a completed close, and Connection::drain is not an outgoing-notification join. Actual BoundRow
unload/reload must verify connection closure, module effect settlement and stale-delivery rejection.
No extra tracker, lock, retry loop, cursor or lifecycle phase is prescribed.

### Review and acceptance

Confirm the Config convention, current immutable-entry limitation, startup-delivery behavior and
transport/ownership integration before implementation. The complete Event payload retains the
existing source contract. Product verification then uses protected serve/Rpc, the original module
schema, two receiving rows in one Session and another Session in the same Node process. It must
cover activation/failed initialization, absent/disabled opt-in, one copy per delivered emission,
ordering, dynamic identities, in-flight unload, same-row reload, stale peers, and continued native
tool/C1 execution. Synthetic transport success does not fulfill this acceptance.

[S8b §4.5](appserver/settings-service.md#45-events) process settings/<source> and settings/projects
are different typed sources, owned by SettingsChanged and ProjectAuthorized. Their approved
serialization is not implemented, and Link does not declare source selection. They remain S8b
owner work; until binding exists, TS owners read at use time as its design specifies. This Session
binding adds no generic source map or Settings watch channel. Full C4 stays incomplete until its
Session fact reception passes the actual flow; C6 Notice remains separate.

## Lifetime and failure

Loader owns generations and rows. Session closure stops branches before unloading Session
parts. Each TS part immediately registers its acquired resources with its own `effect`, stops
new work on disposal, releases I/O, settles accepted work and rejects retained service objects.
The native C1 effect drains while the Node part is still alive. Tool cancellation follows its
existing turn owner; no work transfers to another generation.

The tool/method subset permits synchronous service/effect registration in TS apply, and business
work driven by accepted calls after Session publication. It does not yet expose a neutral
Session activation service for arbitrary autonomous TS background tasks. Such tasks must not be
started from TS apply on the assumption that row Active means Session published. Native broadcast
delivery's activation path is one of the explicit remaining contracts above; no new startup phase
or API is proposed to hide this distinction.

Session assembly uses the registered required-part membership implemented by [#137](https://github.com/arcships/dim/pull/137).
Host-appended optional rows stay in the original disposal group and diagnostics and do not enter
the Session's required wait. Their hard dependencies, key validation and failure reporting do
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
| Session broadcast | Pending: Config/SDK handoff and lifecycle acceptance with the existing AgentEvent serde payload; no hard dependency on optional transport. C6 owns notice data. |
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
flow. The Active-at-first-read fixture explicitly hard-depends on the TS method provider; it is
a test host configuration, not a product readiness guarantee. Separate ordinary assembly covers
Pending and disabled rows without delaying native Session use. Invalid tool/method declarations,
malformed replies, online snapshot mismatch and conflicting module snapshots are boundary cases
in those flows. The protected embedding example may observe Pending and prints durable history;
submit/idle success alone does not certify that a TypeScript tool executed.

The spike and local native-event experiments remain evidence only for their stated boundaries.
Full C4 acceptance, broadcast delivery and cross-platform evidence remain pending.

## Ablation

`provides` is retained solely because the existing assembly needs names before Node is online;
all names and service modes are derived from that one existing dictionary shape. A second
manifest, separate name arrays, registration manager and per-call provider are unnecessary.
Same-row factory composition is retained because C1 must drain before unloading its foreign
handler. The tool declaration stays at first registry read, so no deferred contribution wrapper
or eager tool registration is needed. Promise cancellation preserves the existing `drain` reply.
The existing SDK NodeEvent handoff is retained to keep native I/O out of the canonical Session
listener while reusing per-key ordering; removing it would violate the existing listener contract.
The additional native peer remains a proposed transport choice requiring confirmation.

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
