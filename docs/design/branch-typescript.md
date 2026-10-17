# Branch TypeScript binding

Proposed contract for [C5 / #64](https://github.com/arcships/dim/issues/64), the branch portion of
[#4](https://github.com/arcships/dim/issues/4). The native operations exist; the foreign binding is
not implemented. The declaration, reference binding and lifecycle choices below require architecture
confirmation. Implementation also requires full C4 acceptance.

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

## Structural declaration and row placement

The existing `SessionTypeScriptConfig.rows` contains ordinary package rows. A row with exactly one
`branchPart:<part>` service is a branch part; `<part>` is non-empty and `bind` is its only method,
declared `async`. Its declaration determines placement, so there is no new `scope` setting:

```json
{
  "id": "goal-state",
  "name": "/absolute/goal/branch.ts",
  "provides": {"branchPart:goal": {"bind": "async"}},
  "config": {}
}
```

The module imports this same declaration's `provides` dictionary. The host keeps current C4
tool/method/event rows in the Session group and appends branch parts to the existing nested
`BRANCH_GROUP` returned by `AgentBuilder::install`. It does not create another group or tree.
The existing runtime process, anchor, structural options, `inject`, `isolate` and disablement
continue to apply. Existing Settings declarations are discovered once from the configured rows,
before placement; they are not registered again for each branch instance.

Reject mixed branchPart and Session service families, duplicate part identities in a branch,
empty identities and other bind modes at the structural boundary. Preserve C4's same-module
declaration equality and online metadata checks. A package with both Session and branch parts
uses separate entry modules. Register each `branchPart:<part>` with the existing catalog under
`BRANCH_GROUP`; its instances cannot collide across branches or Sessions.

## Original-row binding

Reuse C4's original `BoundFactory` / `BoundRow` path. For a branch part, the scoped factory obtains
the existing Session and branch instance keys from `Build`. Its declared dependencies are the
original foreign row's dependencies plus branch Timeline, Dispatcher, TimelineIdentity and
BranchLifecycle, and Session TreeCommands. No native part depends on the foreign row.

After applying the original row, register its foreign cleanup immediately and wait for its
`branchPart:<part>` projection through the existing service-change path. Invoke
`bind(binding)` once and await its null result before that row finishes loading. Failure follows
ordinary row failure/cleanup; it does not add a Session readiness phase or change Pending rules.

`binding` is a bridge record with the following named fields. Function and Promise references use
released rutis `Value::Record`, `Value::callback` and `Value::future`; they are not JSON handles.
The native functions capture this original row's `Ctx`, scoped services and cancellation token.
Structural `config` still reaches the original `apply` unchanged and is not replaced by binding data.

| Field | Value and owner |
| --- | --- |
| `identity` | `{sessionId, branchId}` from this branch's TimelineIdentity. |
| `dispatcher` | The named operations below, bound to this branch and original row context. |
| `branch` | A `BranchRef` for this branch: `{id, input}`. The input function uses `TreeCommands::get(id)` and then native `Branch::input`; it does not create a replacement facade. |
| `treeCommands` | Named operations on this Session's existing TreeCommands. |
| `timeline` | Read-only named operations on this branch's Timeline for reconstruction. |
| `activated` | `Promise<boolean>`: existing BranchLifecycle activation, false if closing or original-row cancellation wins before activation. |
| `closing` | `Promise<void>`: resolves at existing branch closing or original-row cancellation, whichever comes first. |

The original row provides `branchPart:<part>.bind`; it does not inject that service or the fields
of its own binding. Ordinary declared `ctx.use` dependencies retain the loader's existing behavior.
These owner-sensitive native functions are passed by reference rather than installed as shared
`ctx.use` services: shared `HostDispatch::invoke(method, args)` has no caller-row `Ctx` argument,
whereas native `Dispatcher::subscribe` requires the registering context. Capturing it in the
existing row factory keeps subscription removal on that row without a registry or another plugin.
This named-reference form is a C5 contract choice requiring confirmation, not an implemented
general-purpose native service lookup.

### Operations, data and errors

`Reply<T>` uses the existing named-service envelope: `{ok:true,value:T}` or
`{ok:false,error:{code,message}}`, as Settings and S2 do. A value may contain bridge references.
The notation below describes this proposed binding, not methods already exported by the runtime.

| Named operation | Mode | Request / value in `Reply` | Native operation |
| --- | --- | --- | --- |
| `dispatcher.subscribe(keys, handler)` | sync | String key array and callback reference; null value. No separate unsubscribe method. | `Dispatcher::subscribe(originalCtx, partName, keys, handler)` |
| `branch.input(data)` / `BranchRef.input(data)` | sync | Native `InputMessageData`; committed `AgentEvent` value. | Native Branch facade |
| `treeCommands.get(id)` | sync | Branch id; `BranchRef` value with `id` and input function. | `TreeCommands::get` |
| `treeCommands.list()` | sync | No arguments; native `TimelineInfo[]`. | `TreeCommands::list` |
| `treeCommands.branch(parent, at, options, signal?)` | async | Native parent id, cutoff and `BranchOptions`, optional bridge AbortSignal; `BranchRef` value. | `TreeCommands::branch` with existing admission cancellation |
| `timeline.committed()` | sync | No arguments; native committed sequence. | `Timeline::committed` |
| `timeline.history({through})` | async | Required cutoff; native `AgentEvent[]`, including the ancestor prefix. | `Timeline::history(Some(through))` |
| `timeline.local({start, through})` | async | Required inclusive local bounds; native `AgentEvent[]`. | `Timeline::local(start..=through)` |

The event callback returns void or `Promise<void>`; bridge undefined/null represents unit. Await
the actual callback result before completing native handling. Callback exceptions, transport
failure and invalid non-unit replies follow the native Dispatcher failure path, not a successful
`Reply` or a warning. `bind` is async because state reconstruction may read Timeline history; it
returns null after registration/reconstruction, never after waiting for activation.

Parse request data once at this foreign boundary and call the existing owner directly. Native
`InputMessageData`, `BranchOptions`, `TimelineInfo` and `AgentEvent` keep their serde field names;
in particular this does not use S2's raw-block or publication-receipt shapes. Sequences retain
rutis' integer representation (`number` or `bigint` in TypeScript); do not coerce them through Number.

| Error code | Source |
| --- | --- |
| `invalid_params` | Boundary request decoding or a missing/non-function callback; native operation not invoked. |
| `invalid_input` | Native `AgentError::InvalidInput`, including Dispatcher key validation. |
| `unknown_branch` | Native `AgentError::UnknownBranch`. |
| `closed` | Native Closed, or a reference whose original row has ended. |
| `cancelled` | Native Cancelled. |
| `reloading` | Native Reloading. |
| `failed` | Other native failures, with the native diagnostic message. |

Codes classify the returned failure; they do not grant retry or prove that a write/creation did
not commit. Bridge loss remains an unknown outcome. Setup failure fails the original row through
its existing loading path. A retained reference cannot act through a successor row: every exported
native function checks its original row cancellation, and cleanup releases its references.

The async creation call passes original-row and call cancellation to native admission using the
existing bridge cancellation and CancellationToken path. TimelineTree retains its own cancellation
and terminal-fact decisions. There is no retry, branch deletion or fabricated completion on loss.
No arbitrary Timeline append, dynamic event declaration, before callback or extra facade method
is exported by this C5 subset.

## Native contracts to preserve

The table describes the existing native operations behind the proposed binding. Native serde types
remain the single definitions of data; the language boundary adds no field aliases or defaults.

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

## Confirmation and implementation gates

| Gate | Required result |
| --- | --- |
| Architecture confirmation | Confirm declaration-based branch placement, original-row async bind, named function/Promise references and the operation/error subset above. This document does not make them accepted APIs. |
| C4 | Complete full #63/#168 implementation review and merge before C5 implementation. |
| Original-row cleanup | Verify existing effect ordering closes admission, settles accepted callbacks/tasks and removes subscriptions before foreign unload. Do not return success merely because a reference was cancelled. |
| Startup and recovery | Bind may register subscriptions and reconstruct local state. Background business work starts only after activated=true; closing ends it. During a row reload, its own reconstruction/live-consumption sequence uses a fixed committed cutoff and local cursor, as native owners do. No shared replay queue or lock is introduced. |
| Acceptance | Implement the confirmed binding and pass the real cross-language flows below. Documentation CI is not this acceptance. |

The native owners remain authoritative. Opening receipts retain their existing native contract;
no new opening receipt protocol is part of this subset. An unimplemented Goal, Hook or other
plugin design does not supply additional bindings or change the native ordering rules.

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

Current deliverable: the proposed binding contract and its documentation index entry. No runtime,
dependency, event format, persistence format or generated graph changes are implemented.

The proposed implementation belongs to the existing App Server language boundary: row classification,
scoped factory dependencies and reference binding in `session_typescript.rs`, and insertion into the
existing branch group in `lib.rs`. It reuses App Server's current bridge dependency and does not
add bridge dependencies or foreign business behavior to the native runtime. Business state and work
stay in the TypeScript row. Existing native types and methods remain unchanged.

| Rule | Application |
| --- | --- |
| §4 rule 1 | A part requests another owner's action through its existing input/command path; it does not mutate that owner's state. |
| §4 rule 2 | Adopted input stays with its consumer; the binding adds no revocation or transfer. |
| §4 rule 3 | No before callback is defined. |
| §4 rule 4 | Native Timeline/Dispatcher semantics remain independent of plugin business behavior. |
| §4 rule 5 | Existing batches and per-event order are preserved; no bridge batch acknowledgement is invented. |
| §4 rule 6 | Model context changes only through adopted inputs and tool results. |
| §4 rule 7 | Proposed named references call existing public services; no private state or another branch's Dispatcher. |
| §4 rule 8 | Reuse current assembly, Timeline, Dispatcher, facades and rutis. Named exposure is the existing #4/#64 task, not a new core mechanism request. |
| §4 rule 9 | No event type, notification fact or persistence format is added. |
| §1 | Business state and tasks belong to the branch part; App Server owns assembly and foreign transport. |
| ADR 002 D1 | Branch behavior follows the branch lifetime. Session and process state stay with their owners. |
| ADR 002 D2 | No forwarding, subscription aggregation or registration Capability. Bind on the original row. |
| ADR 002 D3 | Dependencies point to this branch or enclosing scopes; native core does not depend on an optional foreign part. |
| ADR 002 D4 | Structural placement uses loader rows; business settings do not change assembly. |
| ADR 002 D5 | The proposed declaration, method modes, native payloads, error data and reference cancellation are language-neutral and require confirmation before implementation. |

## Requirements raised to mechanism owners

No new mechanism request. #4/#64 already owns exposing existing runtime services to foreign parts.
Current scoped factories/catalog keys, Dispatcher subscriptions, Timeline reads, native facades,
bridge references and effects supply the binding mechanisms. The proposal adds only the C5 foreign
declaration/reference contract. It does not establish a missing core capability or authorize a
change to native event, publication, dependency-wait or lifecycle semantics.

## Source references

- [Dispatcher](../../crates/dim-agent-runtime/src/core/dispatcher.rs)
- [Branch facade](../../crates/dim-agent-runtime/src/branch.rs) and [Timeline](../../crates/dim-agent-runtime/src/core/timeline.rs)
- [Branch assembly](../../crates/dim-agent-runtime/src/assembly.rs) and [TimelineContext](../../crates/dim-agent-runtime/src/timeline_context.rs)
- [TimelineTree / TreeCommands](../../crates/dim-agent-capabilities/src/timeline_tree.rs)
- [InputQueue reconstruction](../../crates/dim-agent-capabilities/src/input_queue.rs) and [Permission reconstruction](../../crates/dim-agent-capabilities/src/permission.rs)
- [Existing C4 resolver and original-row binding](../../crates/dim-app-server/src/session_typescript.rs)
- [Time bus](../architecture/rust-agent/time-bus.md), [design rules](../architecture/rust-agent/principles.md) and [ADR 002](../adr/002-plugin-scope.md)
