# Session TypeScript parts

Status: proposed tool/method binding design for [C4 #63](https://github.com/arcships/dim/issues/63),
with native broadcast choices requiring owner alignment. Production implementation and full C4
acceptance are pending. The [spike](session-typescript-spike.md)
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
normal Rpc path; an example must demonstrate a protected local connection with a disposable home.
This configuration field is proposed here and does not exist yet. General CLI installation,
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

## Native Session broadcast

Use rutis' existing node `Events` / Cordis `Link` path. Do not invoke a `notice` service to
deliver broadcasts. Subscriptions are registered during row apply. Business consumption must
wait for the existing `SessionLifecycle::activated()`, not merely the row becoming Active:
row publication and Session publication are different. A subscriber does not delay Session
publication. It does not replay missed events; the owning service's query supplies current state
when its feature requires it. Order is the existing channel's order, not a total order between
parallel branches; each event retains its branch sequence.

Session broadcast owns the single serialization of its public record at the language boundary:
an event carries `{branchId, event}` with the existing `AgentEvent` serde value, and the Session
end is distinct from an event. Host `Fence` records stay observation machinery and are not
business data. C4 does not define C6's `{source, kind, data}` notice or put UI notices into
Timeline. C6 can extend its own record and reuse this binding; final notice/Rpc forwarding
semantics remain [C6 #49/#85](https://github.com/arcships/dim/issues/49).

The native `EventsPlugin` consumes `NodeEvent` JSON arguments, whereas the current broadcaster
emits non-serializable `SessionBroadcastEvent`. A codec alone does not connect these two keys.
The broadcast owner must define the sole neutral payload encoding; the native event projection
must then consume it without making the broadcaster hard-depend on optional TypeScript transport.
Neither a second emit nor saying both types use rutis proves that ownership/dispatch contract.
This design does not select an App Server listener plugin or a forwarding service as that bridge.
The exact owner-level native projection remains to be settled before broadcast implementation.
The branch Dispatcher must not wait for Node I/O. No delivery registry, durable cursor,
acknowledgment protocol or retry loop is added.

### Transport choice requiring review

The shared raw row runtime starts with `Mount::default()`: its event emission whitelist is fixed
at startup, and it does not offer `events.forward`. A direct `Process::emit` or an `EventsPlugin`
aimed at that endpoint cannot subscribe to a Session channel allocated later. This is a limit
of that composition, not a missing rutis event feature or a request to change rutis.

The measured existing route mounts a full native peer in the same Node process for each foreign
row, binds that peer's `Events` input to only `session/<this Session id>`, and owns the Rust peer
and event effects with the original row. Native delivery, dynamic names, independent unload and
continued delivery to another row are proven. A retained remote object must reject calls after
the part's effect begins unloading; a stale handle is not evidence of a live generation.

This route is a **candidate requiring review**, with these costs and implementation conditions:

- An additional loopback transport listener/link and a Node I/O worker per foreign row, in
  addition to the shared raw runtime channel. No RSS, maximum Session count or throughput claim
  follows from the measured transport success.
- Connection settings, peer identities and channel names must be derived by assembly from the
  Session instance, passed as structural runtime data, and cleaned up by that same row. They
  cannot be user-maintained Session-id lists or a mutable central subscription manager.
- Establish the native link and event subscription as part of that row's apply before publishing
  its services; dispose subscriptions and settle the row's work on unload. Its real required
  dependencies retain their normal audit and reload behavior.
- The exact structural handoff to the native Cordis row and public end-record encoding must be
  agreed with the broadcast owner before implementation. They are not inferred from the spike's
  `notice` DTO, fixed fixture tokens or private `c4*` services.
- The same-row factory can declare and obtain the existing scoped `SessionLifecycle` in native
  apply, and row-owned delivery work can await `activated()`. Its actual native projection and
  cancellation path must be demonstrated before claiming TS event consumption satisfies that
  lifecycle. The small `definePlugin` context currently offers no named lifecycle binding.

The tool and method bindings above do not depend on selecting a new event transport. C4's full
acceptance does depend on native broadcast, so it cannot be reported complete while this choice
or C6 integration requirements remain unresolved. No speculative core/transport capability is
proposed as a workaround.

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

| Area | Necessary change after contract approval |
| --- | --- |
| App Server `ServerConfig` and assembly | Public embedding input, row boundary validation, names/catalog derived from `provides`, optional Session rows and native resolver factory composition. |
| ToolRegistry and its page | Native/foreign owner selection and declaration decoding at its existing first read. Existing ToolBinding, Prompt and ToolExecutor flow is reused. |
| C1 `methods` module | Neutral DTO/error decoding and generation-owned foreign handler construction. No Rpc dispatch replacement. |
| Session broadcast | Owner serialization, native projection and activation delivery after their contracts are settled; no hard dependency on optional transport. C6 owns notice data. |
| Dependencies | Existing pinned `rutis-bridge` at the language-boundary owners; no new library or rutis change. Regenerate graphs through `xtask graph`, then run `xtask check`. |
| Documentation | Owner pages, App Server README and Session-boundary Appendix F updated to the actually implemented subset. |
| Tests and fixture | One real App Server/Node flow using a disposable home and local Rpc; focused malformed-boundary cases in that flow. |

The flow must cover two Sessions sharing a Node PID; an Active tool declared on the immediate
first model request after create and restore and executed through ToolExecutor; a pending optional
part that does not block native use; a later Active tool omitted until reload; disabled rows;
actual C1 Rpc call/listing, progress, `report` and `drain`; stale provider rejection; native
Session broadcast isolation and unload; continued native/other-Session use after one part closes.
Gate each observed transition on an actual service/event/fence, not a sleep. Boundary cases cover
declaration mismatch, invalid tool/method reply, name collisions and cancellation without leaking
a Promise lease. Reuse C3/C1 tests for their unchanged native semantics.

No automated test is warranted for this documentation-only change. The existing spike and local
same-row/native-event experiments are evidence for their stated boundaries, not results for the
product implementation. Full product acceptance and cross-platform evidence remain pending.

## Ablation

`provides` is retained solely because the existing assembly needs names before Node is online;
all names and service modes are derived from that one existing dictionary shape. A second
manifest, separate name arrays, registration manager and per-call provider are unnecessary.
Same-row factory composition is retained because C1 must drain before unloading its foreign
handler. The tool declaration stays at first registry read, so no deferred contribution wrapper
or eager tool registration is needed. Promise cancellation preserves the existing `drain` reply.
The additional native peer remains a reviewed transport decision, not an assumed requirement.

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
