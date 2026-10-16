# TypeScript plugin authoring

Dim loads trusted JavaScript/TypeScript plugins through rutis. Use `@arcships/rutis`
0.10.0 for the plugin shape and type-only imports from `dimcode/plugin` for Dim's
implemented named services. These declarations add no runtime wrapper or service.
The source declaration is [npm/plugin.d.ts](../npm/plugin.d.ts).
The pinned rutis 0.10 declarations have an optional/required `Plugin.inject`
inheritance conflict under TypeScript 6.0.3. Consumer examples use `skipLibCheck`;
Dim's service declarations are checked separately with strict library checking.
This does not disable checking a plugin's calls against the service interfaces.

## Available services

| Need | Service and scope | Availability |
| --- | --- | --- |
| Persist a process plugin's own data | `pluginFiles`, process | Implemented; use its native transactions and lifecycle scope. |
| Start or drive another Session | `sessions`, process | Implemented by Agents; all eight methods are typed. |
| Read/update business configuration | Settings business sources | Rust sources exist; S8b's named TypeScript contract is not implemented on main. |
| Offer client methods or model tools | `sessionMethods`, `toolContribution`, Session | Rust bindings exist; C4's TypeScript binding is not implemented on main. |
| Observe this Session or act on a branch | Session broadcast / branch Dispatcher | TypeScript bindings are pending C4/C5; do not replace them with polling or a process registry. |

See [Session TypeScript bindings](design/session-typescript.md) and
[Settings](design/appserver/settings-service.md) for their contracts and delivery
status. Do not depend on an API merely because a design describes it. A process
plugin uses `sessions` for other Sessions; it does not use that service to imitate
a Session part or hold a Session's private state.

## Plugin shape and lifetime

`definePlugin` declares `inject`, `provides`, structural configuration schema and
`apply`. Declare every dependency in `inject`; obtain it with `ctx.use`. Each
provided method has the same `sync` or `async` kind in `provides` and its implementation.
`ctx.provide` makes the named object available for this plugin's lifetime.
`ctx.effect` registers cleanup; register it immediately after acquiring a resource.

During `apply`, acquire owned resources and register services, subscriptions and
cleanup. Business input consumption and background work start after publication,
as required by [the architecture rules](architecture/rust-agent/principles.md).
An injected dependency becoming available is not Session publication. Do not
start a timer or task from `apply` and assume that the Session is ready. The
current rutis TypeScript `Context` has `use`, `provide` and `effect`; it has no
`activated()` method. C4 owns the Session binding rather than a plugin inventing
a second readiness mechanism.

State shared by one Session belongs to its Session part; a branch's work and
cancellation belong to that branch. A process part holds only state/resources
shared across Sessions or outliving them. Read other plugins only through their
declared public services/events. See [ADR 002](adr/002-plugin-scope.md), including
the distinction between loader structural configuration and business sources.

## Process storage

Use `pluginFiles` for durable process-owned state rather than constructing a path
under the application home or importing native SQLite internals. The host chooses
the storage root. Paths passed to a scope are relative; namespace them by your
plugin's stable identity.

```ts
import { definePlugin } from '@arcships/rutis';
import type { PluginFiles } from 'dimcode/plugin';

export default definePlugin({
  name: 'Notebook',
  inject: ['pluginFiles'],
  provides: { notebook: { read: 'async', replace: 'async' } },
  async apply(ctx) {
    const files = await ctx.use<PluginFiles>('pluginFiles').open();
    ctx.effect(() => files.close());
    ctx.provide('notebook', {
      async read() {
        return files.withLock('notebook/state.json', file => file.read());
      },
      async replace(text: string) {
        return files.withLock('notebook/state.json', file => file.replace(text));
      },
    });
  },
});
```

`withLock` holds the native file lease through its complete async callback. The
outer `FileResult` is the transaction result; a callback returning a `FileResult`
produces a nested result. Check each result rather than treating failure as empty
state. `read()` returns `null` only when the file is absent. `replace` atomically
writes and flushes the host's v1 envelope around your opaque text; keep your own
payload schema/version inside that text. Invalid, unversioned or unknown storage
records return `invalid_data` and preserve the original bytes.

Do not retain a lease after its callback. `backup` uses a fresh alphanumeric/hyphen
suffix and never overwrites a backup; a retained partial backup is not proof of a
successful copy. Quiesce stops new acquisition; close joins admitted work. An
optional cancellation Promise cancels acquisition when it settles. Settle it on
ordinary completion too, so it cannot hold runtime drain open. rutis 0.10 does
not encode an `AbortSignal` as this argument.

Native storage locking and durability do not make remote side effects atomic.
After an uncertain external outcome, use that system's reconciliation contract
before retrying. Full semantics and fixed storage error codes are in
[trusted plugin storage](../crates/dim-app-server/README.md#trusted-plugin-storage).

## Driving Sessions

Declare `inject: ['sessions']` and use `ctx.use<Sessions>('sessions')`. Agents
provides the process directory service and looks up the current index on each
call. `launch` creates a non-interactive Session with permissions capped by the
starter's current policy, or the host's defaults when no starter is supplied.
The caller does not own or close the Session by dropping a handle.

| Method | Use and result |
| --- | --- |
| `launch` | Create a Session; reply includes its stable id and effective policy. |
| `input` | Write marked model-facing blocks and optional user content; reply confirms publication. |
| `lookup` | Resolve an uncertain input by `submissionId`; writes are not deduplicated. |
| `history` | Page local events through a fixed published prefix; carry `through` across pages. |
| `withdraw` | Withdraw a held input; `notHeld` means nothing was deleted. |
| `stop` | Interrupt the branch and wait for settlement. |
| `idle` | Wait for the first idle point; pending approval can keep it waiting. |
| `delete` | Close and delete the Session through its existing owner. |

Methods return `SessionsResult<T>`. Branch calls take both `sessionId` and
`branchId`; there is no implicit current branch. Input must include the caller's
source marker as its first raw block and a nonempty `submissionId`, conventionally
`<source>:<uuid>`. That id is a correlation key, not idempotency. After a lost reply,
`reloading` or `publication_unknown`, look up the write before submitting again.
`retryable` alone is not authority to repeat a side effect. A lost launch reply
can leave an empty Session: launch has no deduplication key.

`history` returns at most a requested page before extending to a batch boundary;
`limit` is 1–256. Use either `through` or `atLeast`, not both. Event sequence and
timestamp fields are JSON numbers; event payloads belong to their declaring
owners. The SDK types their envelope and JSON data without asserting that one
event payload describes every domain. For complete publication, paging, failure
and cleanup semantics, use the [sessions contract](design/appserver/sessions-service.md).

## Packaging and trust

Keep the `dimcode/plugin` import type-only. Use a plugin's own build dependencies
to resolve it and bundle the rutis SDK/business dependencies, or include them in
that plugin's package. Module resolution must not rely on the host repository's
development installation. Pin the rutis SDK to the version the host supports.

`pluginFiles` is a trusted-plugin convention, not a sandbox: Node code can still
use `fs` directly. Dependency declarations do not isolate trusted in-process
code. OS isolation of model commands does not sandbox plugin packages, hooks or
MCP startup. Credentials and other private business data must stay out of model
content, terminal RPC, history and diagnostics. See [ADR 003](adr/003-approval-scope.md).
