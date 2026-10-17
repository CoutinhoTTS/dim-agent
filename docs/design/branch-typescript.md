# Branch TypeScript binding

Contract preparation for [C5 / #64](https://github.com/arcships/dim/issues/64), the branch portion of
[#4](https://github.com/arcships/dim/issues/4). The native behavior below exists; its foreign binding
is not implemented or Ready. Implementation requires full C4 acceptance and confirmation of the
binding decisions in this document.

## Scope and ownership

A TypeScript branch part consumes its own branch's committed events through the existing
Dispatcher and submits validated input through `Branch::input`. It reaches branch handles through
its Session's existing `TreeCommands`. It is an ordinary loader part inside the existing branch
instance, with the same owner for subscriptions, derived state, tasks and cleanup.

This is distinct from [C4 Session observation](session-typescript.md): a Session notification can
be queued independently of native notification delivery, while a Dispatcher callback participates
in the branch's ordered consumption. No Session notification queue is used for critical branch work.

| Concern | Existing owner or mechanism |
| --- | --- |
| Branch instance and typed keys | `BranchScope`, nested in its `SessionScope` |
| Ordered consumption and handler failures | Branch `Dispatcher` |
| Committed history for state reconstruction | Branch `Timeline` |
| External input validation and commitment | `Branch::input(InputMessageData)` |
| Branch handles, directory and creation | Session `TreeCommands` / `TimelineTree` |
| Activation and closing | Existing `BranchLifecycle` |
| Opening work that must finish before publication | Existing `BranchOpening` receipts |
| Foreign calls and references | Pinned rutis loader/bridge and the original row's effects |

The part must declare every dependency, including load-order dependencies. Missing providers retain
the existing Pending behavior. A consumer cannot obtain a sibling branch's Dispatcher or mutable
state; communication with another branch uses its validated input and does not wait for its work.

## Native contracts to preserve

The table describes native operations, not an already available TypeScript API. Wire names,
declarations and binding placement remain explicit decisions below. Native serde types remain the
single definitions of data; the language boundary does not introduce renamed fields or defaults.

| Operation | Existing contract |
| --- | --- |
| `Dispatcher::subscribe(ctx, name, keys, handler)` | Registers on this branch's instance channel. Keys accept an exact event key, `*`, or `domain/*`. The registering `ctx` must be in the branch subtree and owns listener removal. |
| Dispatcher callback | Receives the original committed `AgentEvent`. The Dispatcher awaits its completion before advancing this branch's dispatch sequence. Handler errors and panics fail the Dispatcher; business outcomes remain the capability owner's facts. |
| `Branch::input(data)` | Validates `InputMessageData` and returns a committed `AgentEvent`. It does not promise publication, dispatch, adoption or model consumption. |
| `TreeCommands::get(id)` | Returns the Session's existing owned branch handle or its existing error. It does not create or restore a branch. |
| `TreeCommands::list()` | Returns native `TimelineInfo` records. A directory record is not a publication or activation acknowledgement. |
| `TreeCommands::branch(parent, at, options, admission)` | Uses the existing branch creation and cancellation path. `BranchOptions` and the returned `Branch` retain their native contracts. |
| `Timeline::history` / `local` | Capabilities can reconstruct state through a fixed committed cutoff. Public `Branch::history` / `local` instead expose published history; those boundaries are not interchangeable. |

`TreeCommands::get` can find a context after TimelineTree stores it and before publication.
It is not an initialization guarantee: the Dispatcher is started while the context is being built,
before TimelineTree stores that context. An early lookup can therefore fail. The binding must not
hide that outcome with retries, a replacement branch or another readiness phase.

## Subscription, opening and recovery

1. Register the foreign callback on the original branch part's native `Ctx`, not a process or
   Session subscriber. A row-only reload must remove the old listener without changing another
   branch's listeners. A shared `HostDispatch` lease alone does not establish that ownership.
2. Preserve the native serial await and failure path. Do not return early into a C4 transport FIFO
   or turn a handler failure into a warning followed by later critical events.
3. An event handler registers work with its owning part and returns. Network and tool waits belong
   to that work. Background work started during initialization waits for existing activation and
   ends on existing closing; the row registers cleanup as soon as it acquires a resource.
4. Opening events are dispatched before publication/Active. A callback processing those events
   must not await the activation or dispatch fence whose completion depends on that callback.
   Existing `BranchOpening` remains the owner of required opening receipts.
5. A live subscription does not replay durable history. A stateful part reconstructs its derived
   state from the existing Timeline, serializes that reconstruction with live consumption and
   avoids applying the same local sequence twice. Historical folding does not repeat external
   side effects. An in-memory cursor or bridge handle is not a durable identity.
6. Input written from a callback is committed to the Timeline tail and consumed later. The callback
   cannot wait for its own subsequent dispatch. It must not treat the input receipt as completion
   of the requested action or substitute S2's after-publication receipt contract.

These are native flow constraints. The opening/activation cycle is a control-flow inference from
the existing source, not a reproduced C5 runtime failure.

## Decisions required before Ready

| Decision | Required result using existing mechanisms |
| --- | --- |
| Structural row placement | Define how ordinary rows enter the existing branch group, preserve current C4 Session rows and share the existing language runtime. Do not add another assembly tree or per-row process. |
| Named declarations and native dependency mapping | Define the language-neutral names, method modes, request/reply forms and native error mapping for the C5 subset. Required native dependencies must be known before the original row loads; do not create a self-dependency on a service first published during that row's own apply. |
| Original-row subscription ownership | Show how a named subscribe call retains the original row's `Ctx` and callback lifetime. `Dispatcher::subscribe` requires that context; the released bridge's shared-service lease is not proof of caller-row cleanup. |
| Startup, reconstruction and closing | Specify the exact use of existing history, activation, closing, effects and opening receipts, including a row reload while a callback or owned task is active. Do not add a replay queue, lifecycle manager or hot attachment rule. |
| Branch input and creation calls | Bind the native facade/TreeCommands operations directly, with existing cancellation. State when a branch handle is obtainable and retain committed-versus-published acknowledgement semantics. |

No wire example is presented as callable until these choices are complete. The native owners remain
authoritative; an unimplemented Goal, Hook or other plugin design does not supply missing bindings.

## Verification required for delivery

Use real Node parts with the existing App Server/loader, isolated storage and a disposable home.
Extend the nearest public flow tests rather than testing private binding helpers.

- A branch part consumes native events in order, registers its own work, and writes validated input
  through the native Branch facade. Assert the resulting durable input and observed model flow.
- Two branches and two Sessions share the runtime while retaining independent subscriptions,
  state and task ownership; cross-branch communication remains input-only and asynchronous.
- Row reload removes the old subscription and settles owned work. Closing leaves no callback or
  task waiting for an activation/fence that can no longer occur. Callback errors follow the native
  Dispatcher failure path; later events do not continue as healthy processing.
- Opening, restore and reconstruction preserve the committed/published boundaries and do not
  repeat a side effect or lose/duplicate a live event at the reconstruction cutoff.
- Existing C4 tools, C1 methods, Session broadcasts, first-read sampling and Pending behavior remain
  covered by their existing flows. Run graph/check for actual assembly/dependency changes.

## Impact and rule check

Current deliverable: this contract preparation and its documentation index entry. No runtime,
protocol, dependency, event format, persistence format or generated graph changes.

| Rule | Application |
| --- | --- |
| §4 rule 1 | A part requests another owner's action through its existing input/command path; it does not mutate that owner's state. |
| §4 rule 2 | Adopted input stays with its consumer; the binding adds no revocation or transfer. |
| §4 rule 3 | No before callback is defined. |
| §4 rule 4 | Native Timeline/Dispatcher semantics remain independent of plugin business behavior. |
| §4 rule 5 | Existing batches and per-event order are preserved; no bridge batch acknowledgement is invented. |
| §4 rule 6 | Model context changes only through adopted inputs and tool results. |
| §4 rule 7 | Use public named bindings of the existing services, not private state or another branch's services. |
| §4 rule 8 | Reuse current assembly, Timeline, Dispatcher, facades and rutis. Named exposure is the existing #4/#64 task, not a new core mechanism request. |
| §4 rule 9 | No event type, notification fact or persistence format is added. |
| §1 | Business state and tasks belong to the branch part; App Server owns assembly and foreign transport. |
| ADR 002 D1 | Branch behavior follows the branch lifetime. Session and process state stay with their owners. |
| ADR 002 D2 | No forwarding, subscription aggregation or registration Capability. Bind on the original row. |
| ADR 002 D3 | Dependencies point to this branch or enclosing scopes; native core does not depend on an optional foreign part. |
| ADR 002 D4 | Structural placement uses loader rows; business settings do not change assembly. |
| ADR 002 D5 | Native methods, data, errors and cancellation need language-neutral declarations before implementation. |

## Requirements raised to mechanism owners

No new mechanism request. #4/#64 already owns exposing existing runtime services to foreign parts.
Current scoped factories/catalog keys, Dispatcher subscriptions, Timeline reads, native facades,
bridge references and effects are the mechanisms to evaluate for that binding. The unresolved
items above are binding decisions within C5; they do not establish a missing core capability or
authorize a change to native event, publication, dependency-wait or lifecycle semantics.

## Source references

- [Dispatcher](../../crates/dim-agent-runtime/src/core/dispatcher.rs)
- [Branch facade](../../crates/dim-agent-runtime/src/branch.rs) and [Timeline](../../crates/dim-agent-runtime/src/core/timeline.rs)
- [Branch assembly](../../crates/dim-agent-runtime/src/assembly.rs) and [TimelineContext](../../crates/dim-agent-runtime/src/timeline_context.rs)
- [TimelineTree / TreeCommands](../../crates/dim-agent-capabilities/src/timeline_tree.rs)
- [InputQueue reconstruction](../../crates/dim-agent-capabilities/src/input_queue.rs) and [Permission reconstruction](../../crates/dim-agent-capabilities/src/permission.rs)
- [Existing C4 resolver and original-row binding](../../crates/dim-app-server/src/session_typescript.rs)
- [Time bus](../architecture/rust-agent/time-bus.md), [design rules](../architecture/rust-agent/principles.md) and [ADR 002](../adr/002-plugin-scope.md)
