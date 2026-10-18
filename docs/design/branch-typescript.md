# Branch TypeScript binding

Status: implemented for [C5 / #64](https://github.com/arcships/dim/issues/64), the branch portion of
[#4](https://github.com/arcships/dim/issues/4), on rutis 0.12.1 `build::Exports`
([arcships/rutis#440](https://github.com/arcships/rutis/issues/440)). Goal (#69) is its consumer;
reading a branch-owned business service from a Session part is decided with Goal (§Open).

## Scope and ownership

A TypeScript branch part consumes its own branch's committed events through the existing
Dispatcher, rebuilds from the branch's committed history, and submits validated input through
`Branch::input`. It uses the existing Rust services by name, as a Rust part injects them: there is
no binding layer, branch handle, registry or setup service of Dim's own. App Server lists the
services to export in `crates/dim-app-server/build.rs`; rutis generates their dispatch and
TypeScript declarations from the Rust definitions.

The generated declarations are published in `npm/branch-services.d.ts` and re-exported as
`BranchServices` and the service types by `dimcode/plugin`. The SDK's `BranchEvent` reuses the
existing event envelope with native bridge sequences and owner-defined data. The
[authoring guide](../typescript-plugin-authoring.md#ordered-branch-consumption) shows the row,
subscription and rebuild. Regenerate with `cargo check --locked -p dim-app-server`; CI checks
the generated file before compiling the packed SDK.

This is distinct from [C4 Session observation](session-typescript.md): a Session notification can
be queued independently of native delivery, while a Dispatcher callback is part of the branch's
ordered consumption.

| Concern | Existing owner |
| --- | --- |
| Branch instance and typed keys | `BranchScope`, nested in its `SessionScope` |
| Ordered consumption, handler failures, committed reads | Branch `Dispatcher` (over its `Timeline`) |
| Input validation and commitment | `Branch::input`, reached by branch id through Session `TreeCommands` |
| Branch directory and creation | Session `TreeCommands` / `TimelineTree` |
| Activation and closing | `BranchLifecycle` |
| Branch identity | `TimelineIdentity` |

## Exported services

`Exports::service_in` serves each one inside every instance of its group; a row gets the nearest
enclosing instance. Its methods are the type's public `&self` methods that can cross languages, all
`async` for the caller. The generator leaves the others out with a build warning.

| Name | Group | Rust type | Methods used by parts |
| --- | --- | --- | --- |
| `treeCommands` | `session` | `TreeCommands` | `list()`, `info(id)`, `input(id, input)`, `create(parent, at, options)` |
| `dispatcher` | `branch` | `Dispatcher` | `listen(name, keys, handler)`, `committed()`, `local(start, through)`, `history(through)` |
| `branchLifecycle` | `branch` | `BranchLifecycle` | `activated()`, `closing()` |
| `timelineIdentity` | `branch` | `TimelineIdentity` | `get()` → `{sessionId, branchId}` |

The data methods were added for this purpose; the existing ones that return handles or take Rust
closures stay as they are for Rust callers:

- `TreeCommands::info`, `input` and `create` address branches by id and return data:
  `TimelineInfo`, the committed `AgentEvent`, and the new branch's id. `create` cancels admission
  when the call is dropped, as `branch`'s token does.
- `Dispatcher::listen` is `subscribe` in data form. Its `&Ctx` parameter is the calling row's
  context, so the subscription belongs to that row and ends when it unloads or restarts. The
  handler receives the committed event by value; its throw or rejection is the subscriber's handler
  failure and follows the native Dispatcher failure path.
- `Dispatcher::committed`, `local` and `history` are the Timeline's committed reads for the
  subscription's rebuild (§Subscription). The `Timeline` type itself is not exported, so its
  writes (`append`, `before`), `close` and snapshots are not reachable from other languages.
- `TimelineIdentity::get` returns the identity as data.

Native serde forms stay authoritative (`InputMessageData`, `BranchOptions`, `TimelineInfo`,
`AgentEvent`). `AgentEvent`'s TypeScript is declared in `build.rs` with the `npm/plugin.d.ts` shape,
because its sequence fields use custom serde. Sequences cross as `number | bigint`; do not coerce
them through `Number`.

Errors are thrown in the caller's language as `AgentError` with its variant as `code`
(`InvalidInput`, `UnknownBranch`, `Closed`, `Cancelled`, `Reloading`, …). A code classifies the
failure; it neither grants a retry nor proves that a write did not commit.

## Placement

A row that injects a branch service (`dispatcher`, `branchLifecycle` or `timelineIdentity`) is a
branch part: App Server appends it to the existing nested `BRANCH_GROUP`, so it runs once in every
branch. Its declared dependencies decide its placement; there is no scope setting or setup service.
Other rows stay Session rows (C4).

```json
{
  "id": "goal-state",
  "name": "/absolute/goal/branch.ts",
  "provides": {},
  "inject": ["dispatcher", "timelineIdentity", "treeCommands"],
  "config": {}
}
```

A branch row may provide branch services (registered in its branch instance) but not the Session
families `toolContribution:`, `sessionMethods:` or `sessionEvents:`; that fails at the configuration
boundary. A package with Session and branch parts uses separate entry modules. The C4 rules on ids,
same-module snapshots, online metadata, disablement and Pending apply unchanged.

## Subscription, rebuild and recovery

1. Subscribe first with `dispatcher.listen`, then read `committed()` and `local(0, cutoff)`, and
   fold that history. Live events at or below the cutoff are already folded; the handler waits for
   the fold before handling later ones. This is InputQueue's native pattern.
2. Folding history repeats no side effect; decisions act on live events. A restored or reloaded
   generation rebuilds the same way and does not repeat a write its predecessor committed.
3. A handler registers its work with its part and returns. Network and tool waits belong to that
   work. Background work waits for `branchLifecycle.activated()` and ends on `closing()`.
4. Opening events are dispatched before publication. A handler must not await an activation or
   dispatch that depends on its own completion.
5. Input written from a handler is committed to the Timeline tail and consumed later; its reply
   proves commitment, not dispatch or adoption.
6. One branch never receives another branch's Dispatcher. Cross-branch communication is input by
   id through `treeCommands`, asynchronous.

## Open

- **A Session part reading a branch-owned service.** Goal's `session.goal.get` reads the branch
  part's fold. A branch part's `ctx.provide` reaches rutis inside its branch instance; how a Session
  part addresses one branch's instance by id is decided with Goal's implementation. No Session
  registry or caller-side fold is added here.
- **Generator limitations worked around.** rutis' generator resolves a generic alias's item type
  where the alias is defined, so the exported methods spell out `std::result::Result<_, AgentError>`
  instead of the runtime's `Result<_>` ([arcships/rutis#463](https://github.com/arcships/rutis/issues/463)); and, as documented, it
  does not read `#[serde(with)]`, so `AgentEvent` is declared with `Exports::declared`.

## Verification

`crates/dim-app-server/tests/transport_flow/branch_typescript.rs` runs a real Node branch part
(`runtime/fixtures/branch-part.ts`) on persistent storage:

- In two Sessions sharing the runtime, each `main` part reads its own identity, follows
  `input/message`, `turn/start` and `turn/end` in commit order, and writes one marked input to its
  branch through `treeCommands.input`. Later turn ends do not write again.
- A child in the first Session uses its own Dispatcher, identity and local fold. It writes its own
  marker despite inheriting its parent's prefix, without writing another marker to the parent.
- After a restart, each restored Session's part rebuilds from committed history through a nonzero
  cutoff, including the child branch. It does not write the first marker again and acts on its next
  live turn end.

Existing C4 tools, methods, observation and Settings flows keep their own `node_` flows.

## Rule check

| Rule | Application |
| --- | --- |
| §4 rule 1 | A part requests another owner's action through validated input; it mutates no other owner's state. |
| §4 rule 2 | Adopted input stays with its consumer; nothing is revoked or transferred. |
| §4 rule 3 | No before callback is exported. |
| §4 rule 4 | Native Timeline/Dispatcher semantics are unchanged; the data methods delegate to them. |
| §4 rule 5 | Batches and per-event order are the Dispatcher's; no bridge acknowledgement is added. |
| §4 rule 6 | Model context changes only through adopted inputs and tool results. |
| §4 rule 7 | Parts use existing public services by name; no private state or other branch's Dispatcher. |
| §4 rule 8 | No new mechanism in Dim; the generic capability (rutis#440) and the alias fix are rutis's. |
| §4 rule 9 | No event type, notice or persistence format is added. |
| §1 | Business state and tasks belong to the branch part; App Server owns assembly and the export list. |
| ADR 002 D1–D5 | Branch behavior follows the branch lifetime; no wiring Capability; dependencies point to the branch or enclosing scopes; placement is structural; contracts are the native serde forms. |

## Source references

- [Dispatcher](../../crates/dim-agent-runtime/src/core/dispatcher.rs), [TimelineIdentity and BranchLifecycle](../../crates/dim-agent-runtime/src/config.rs)
- [TimelineTree / TreeCommands](../../crates/dim-agent-capabilities/src/timeline_tree.rs)
- [InputQueue rebuild](../../crates/dim-agent-capabilities/src/input_queue.rs)
- [Export list](../../crates/dim-app-server/build.rs) and [row placement](../../crates/dim-app-server/src/session_typescript.rs)
- [Goal consumer contracts](plugins/Goal.md)
