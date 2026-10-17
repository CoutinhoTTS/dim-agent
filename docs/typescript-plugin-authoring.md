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
| Offer client methods or model tools | `sessionMethods:<namespace>`, `toolContribution:<tool>`, Session | Implemented by C4 through ordinary structural rows and the existing App Server embedding entry. |
| Observe this Session or act on a branch | Session broadcast / branch Dispatcher | Native Session broadcast is tracked by #168; branch bindings remain C5 work. |

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

## Session tools and client methods

A Session part provides `toolContribution:<tool>` or `sessionMethods:<namespace>`.
The host configures ordinary rows through `ServerConfig.session_typescript`;
general CLI installation remains PluginManager work. Give each row the same
`provides` dictionary the module exports. The host derives tool names and method
namespaces from that dictionary; keep it as one packaging source rather than
maintaining separate registration lists. The public
[embedding example](../crates/dim-app-server/examples/session_typescript.rs)
shows the row input and actual Session calls.

```ts
import { definePlugin, type MethodKind } from '@arcships/rutis';
import type { ToolContribution, SessionMethodsContribution } from 'dimcode/plugin';

export const provides = {
  'toolContribution:example_echo': { declaration: 'sync', execute: 'async' },
  'sessionMethods:session.example': { describe: 'async', call: 'async' },
} satisfies Record<string, Record<string, MethodKind>>;

export default definePlugin({
  name: 'SessionExample',
  provides,
  apply(ctx) {
    const tool: ToolContribution = {
      declaration() {
        return {
          name: 'example_echo', description: 'Echo supplied text',
          source: 'extension', risk: 'session',
          inputSchema: {
            type: 'object', properties: { text: { type: 'string' } },
            required: ['text'], additionalProperties: false,
          },
        };
      },
      async execute(call) {
        if (typeof call.arguments.text !== 'string') {
          return { content: 'text must be a string', isError: true };
        }
        return { content: call.arguments.text };
      },
    };
    ctx.provide('toolContribution:example_echo', tool);

    const methods: SessionMethodsContribution = {
      async describe() {
        return [{ name: 'session.example.echo', mutating: false,
          interruption: 'report', progress: true }];
      },
      async call(call, _cancelled, progress) {
        progress({ phase: 'started' });
        return { kind: 'value', value: call.params };
      },
    };
    ctx.provide('sessionMethods:session.example', methods);
  },
});
```

Copy the exported declaration into the existing host row during your package's
build. The App Server does not discover this export or install a plugin by itself.
Online module metadata must match the configured row snapshot. Disabled, Pending
and malformed rows have the outcomes in the
[binding contract](design/session-typescript.md#host-input-and-declaration-discovery).
ToolRegistry samples active providers once at branch initialization; a provider
that becomes active later is available after the next registry/Session reload.

`declaration` is synchronous and must not start asynchronous work. Its name matches
the tool key and its Permission risk is nonempty. `execute` receives the neutral
`ToolCall`: `sessionId`, `branchId`, `turnId`, `callId`, `arguments` and `config`.
This is distinct from the native `{id, name, input}` call. Validate model-supplied
arguments at the tool's external-input boundary. `config` is the creation-time
Session snapshot; provider-owned nested configuration remains JSON in the SDK.
The native serializer omits `interactive` when true, so an absent field means
interactive and only `false` is written. It is not the latest adopted Prompt/model
configuration after a configuration command.

The second `execute` argument is a cancellation Promise. It settles when the call
ends, is cancelled or its row unloads. Cooperative work releases its resources
before settling. Ordinary provider failures return a valid `ToolResult` with
`isError: true`; thrown exceptions are binding failures. Put every handle or
reference the model needs into `content`; `structuredContent` is for persistence,
UI and internal consumers. The SDK preserves the native result fields without
another conversion layer.

`describe` and `call` are asynchronous. Each method declares its full name,
`mutating`, `interruption` and `progress`. `call` receives an opaque runtime
`callId`, the method and original params containing `sessionId` and optional
`branchId`; it also receives cancellation and a per-call progress callback.
Return `{kind: 'value', value}` or `{code, appError, message}`. An omitted value
becomes JSON null. Handoff and connection-bound notifications are not provided by
this binding. On unload, C1 stops admission and settles accepted calls before the
JavaScript row unloads: `report` replies with interruption, while `drain` preserves
the provider's reply. No business task in this example runs during `apply`.

Native Session broadcast remains [#168](https://github.com/arcships/dim/issues/168)
work, waiting for a rutis release with leaf events and the binding contract.
The tool/method binding does not expose `ctx.on`, a Session activation method,
branch Dispatcher access or a replacement polling mechanism.

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
