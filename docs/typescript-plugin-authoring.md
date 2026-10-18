# TypeScript plugin authoring

Dim loads trusted JavaScript/TypeScript plugins through rutis. Match `@arcships/rutis`
to the host's pinned runtime: main uses 0.12.1; older hosts may still use 0.11.0 or 0.10.0.
Use type-only imports from `dimcode/plugin` for Dim's implemented named services.
These declarations add no runtime wrapper or service.
The source declaration is [npm/plugin.d.ts](../npm/plugin.d.ts). C5 service types are generated
from the Rust exports into [npm/branch-services.d.ts](../npm/branch-services.d.ts) and re-exported
through the same entry point.
The pinned rutis 0.12 declarations pass `tsc --strict` without `skipLibCheck`.
This does not disable checking a plugin's calls against the service interfaces.

## Available services

| Need | Service and scope | Availability |
| --- | --- | --- |
| Persist a process plugin's own data | `pluginFiles`, process | Implemented; use its native transactions and lifecycle scope. |
| Start or drive another Session | `sessions`, process | Implemented by Agents; all eight methods are typed. |
| Read or set a Session's title | `sessionMetadata`, process | Implemented by SessionStore: `read` and the revision-checked `setTitle` ([title metadata](design/appserver/session-title-metadata.md)). SessionTitle is the title's one writer; other plugins only read. |
| Read/update business configuration | `settings`, process | Owner read/write and Settings event delivery implemented; declare sources in the C4 row's `settings` member. |
| Resolve a credential reference | `credentials`, process | Host dispatch implemented; no product plugin provides a secret store. Inject only when the embedding host supplies one. |
| Offer client methods or model tools | `sessionMethods:<namespace>`, `toolContribution:<tool>`, Session | Implemented by C4 through ordinary structural rows and the existing App Server embedding entry. |
| Observe this Session | `sessionEvents:<part>` setup and `session.event`, Session | Implemented by C4 on the original row; filter by its bound Session id. |
| Consume branch events in order or act on branches | `dispatcher`, `branchLifecycle`, `timelineIdentity`, branch; `treeCommands`, Session | Implemented by C5 through rutis-generated exports; use `BranchServices` from the SDK. |

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
cleanup. Background I/O starts after publication, as required by
[the architecture rules](architecture/rust-agent/principles.md). Branch Dispatcher handlers
can receive opening facts before publication; they follow the ordered-consumption contract below.
An injected dependency becoming available is not Session publication. Do not
start a timer or task from `apply` and assume that the Session is ready. The
rutis TypeScript `Context` has `use`, `provide` and `effect`, and since rutis 0.11
leaf event APIs such as `ctx.on`. Neither supplies Dim Session activation;
there is no `activated()` method. C4 owns the Session binding rather than a plugin inventing
a second readiness mechanism. C5 branch parts can inject the existing `branchLifecycle`
service for background work's `activated()` and `closing()` waits.

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

## Business configuration and credentials

Declare `inject: ['settings']` and obtain `ctx.use<Settings>('settings')`.
Settings owns business documents; loader rows hold structural configuration.
Declare each source in the `settings` member of its C4 row, next to `provides`,
through `ServerConfig.session_typescript.rows`. The package keeps one JSON source
declaration and hands that same file to the host; the TypeScript module does not
repeat it. There is no separate package manifest or `ServerConfig.settings_sources`.

For example, this row declares a user-scoped Web source:

```json
{
  "id": "web",
  "name": "/absolute/path/to/web.ts",
  "provides": {
    "toolContribution:web_fetch": { "declaration": "sync", "execute": "async" }
  },
  "settings": [{
    "source": "web",
    "userFile": "web.json",
    "schema": {
      "type": "object",
      "properties": {
        "maxResults": { "type": "integer", "minimum": 1, "default": 5 }
      },
      "additionalProperties": false
    }
  }]
}
```

Each entry requires `source`, `userFile` and `schema`; optional `projectFile`
adds project scope and `userOnly` lists protected member JSON Pointers. The row's
`id` supplies the owner; do not put `owner` in the entry. Unknown fields, malformed
entries, duplicate source claims and claims on Rust sources fail assembly. Rust
sources such as `mcp` are not served by this named API. See the
[source declaration contract](design/appserver/settings-service.md#31-source-declaration)
and the working [owner fixture](../runtime/fixtures/settings-owner.ts), its
[provides declaration](../runtime/fixtures/settings-owner.json) and
[Settings declaration](../runtime/fixtures/settings-owner.settings.json).

`read` and `update` are asynchronous and return `SettingsResult<SettingsSnapshot>`.
`projects({})` is synchronous and returns the same envelope around
`{projects: [{root, cwd}]}`. Scope is `'user'` or `{project: root}`; root numbers
belong to this process and must not be persisted as project identities.

For a row declaring the Web source above:

```ts
import type { Settings } from 'dimcode/plugin';

declare const settings: Settings;
const current = await settings.read({ source: 'web', scope: 'user' });
if (!current.ok) throw new Error(current.error.code);
const updated = await settings.update({
  source: 'web', scope: 'user', expectedRevision: current.value.revision,
  document: { ...current.value.document, maxResults: 10 },
});
if (!updated.ok) throw new Error(updated.error.code);
```

An update compares the opaque revision and validates the complete document after
defaults. Owner writes cannot change user-only fields; local client
`settings.update` is the separate path allowed to change them. `conflict` requires
reading the current snapshot before deciding what to write. `unsynced` means the
write is already installed and announced; read before retrying. Dropping a call
cancels its wait, but does not undo a committed write. Native Settings events are
emitted after commit. In the Session TypeScript runtime, register
`ctx.on('settings.changed', listener)` for `{source, scope, revision}` and
`ctx.on('settings.project_authorized', listener)` for `{root, cwd}`. Register
listeners during `apply`, filter changes by source and read the current document
when needed; event payloads contain no document. Reading at use time remains valid
for owners that do not need notifications. See the
[event contract](design/appserver/settings-service.md#45-events); no separate
subscription service or polling loop is needed.

When the embedding host supplies a secret store, declare `credentials` as a
dependency and use `ctx.use<Credentials>('credentials').resolve({owner, reference})`.
For a TypeScript source, `owner` is its declaring row's `id` (`web` above);
the Rust Provider uses `provider`. `reference` is the opaque
nonempty string from a `*Ref` field. Check `CredentialsResult`: success is
`{ok: true, value: {value: string}}`; failure has `code` and `message`, without
`retryable`. Resolve per use and keep values out of Settings, tool results,
terminal replies, history and diagnostics. No product store is provided by Dim;
these types do not make one available. See the
[Settings service contract](design/appserver/settings-service.md#4-interfaces).

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
the tool key and its Permission risk is nonempty. Permission recognises
`read_filesystem`, `write_filesystem`, `execute_process`, `network` and `session`.
Any other string (including `read`) is unclassified: without a tool-specific policy
override it is denied under read-only and requires approval under workspace-write. Use the exact category for the tool's
actual effects. `execute` receives the neutral
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

## Session observation

A Session row adds `sessionEvents:<part>: {bind: 'sync'}` to the same `provides`
dictionary used for tools/methods. C4 invokes `bind` once on that original row,
after its projection is visible and before it becomes Active. It supplies the
Session id, creation-time configuration and original-generation closing Promise.
Register subscriptions/effects synchronously and return JSON null; returning a
Promise, a non-null value or throwing fails the optional row. An event-only row
needs no tool or client method.

```ts
import { definePlugin, type MethodKind } from '@arcships/rutis';
import type { SessionEventRecord, SessionEventsContribution } from 'dimcode/plugin';

export const provides = {
  'sessionEvents:example': { bind: 'sync' },
} satisfies Record<string, Record<string, MethodKind>>;

export default definePlugin({
  name: 'SessionObserver',
  provides,
  apply(ctx) {
    const observer: SessionEventsContribution = {
      bind(binding, closing) {
        let closed = false;
        closing.then(() => { closed = true; });
        ctx.effect(() => { closed = true; });
        ctx.on('session.event', (record: SessionEventRecord) => {
          if (closed || record.sessionId !== binding.sessionId) return;
          console.info(record.branchId, record.event.type, record.event.name);
        });
        return null;
      },
    };
    ctx.provide('sessionEvents:example', observer);
  },
});
```

`SessionEventRecord` is `{sessionId, branchId, event}` using the existing
`AgentEvent` envelope. `ctx.on` is runtime-wide, so compare the bound Session id
before business code. The immutable `config` uses the same serialized form as
ToolCall: absent `interactive` means true; eligibility that needs an interactive
leader checks `config.interactive !== false && config.collaborationRole === 'leader'`.
It does not report later Prompt/model adoption.

The example only logs transient observations. Handlers that start async work
register it with their original owner and return; on closing, stop admission,
cancel/release resources and join accepted work in a row effect. `ctx.on` removes
the listener on unload. Closing settles for native Session close or original-row
cancellation; captured handlers must not start work afterward or follow a replacement.

C4 forwards native Event records after publication, preserving per-Session native
channel order without imposing order between Sessions. It does not replay history
or deliver native Fence/Closed frames, C6 title notices or Settings changes as
`session.event`. Process Settings events keep their own names. Already-admitted
delivery may finish during close; a newly bound listener can receive a delayed
observation but receives no replay of completed emissions. A slow leaf does not
block native Session delivery; pending observations can accumulate in its transport.
The [binding contract](design/session-typescript.md#native-session-broadcast) and
[real fixture](../runtime/fixtures/session-observer.ts) describe these boundaries.
Observation supplies no branch write/context-input path or Session activation
method. Use C5's Dispatcher for critical ordered branch consumption.

Since rutis 0.11, integer values beyond ±(2^53−1) cross into TypeScript as `bigint`,
including integers nested in model-supplied tool arguments or event data. Ordinary
Dim sequence and millisecond timestamp values remain within the safe-number range.
The SDK's `JsonValue` describes JSON-only values and does not model this bridge
`bigint` case; a declared `JsonValue` is not proof that incoming tool or event data can be
JSON-encoded unchanged.
`JSON.stringify` throws on `bigint`; do not stringify an unchecked argument object
or coerce every integer to `number`. At the tool's external-input boundary, validate
against its actual schema and explicitly choose a lossless representation when
sending such a value to a JSON-only API. Preserve the native value when passing
it to a compatible owner.

## Ordered branch consumption

C5 exports the existing Rust services through rutis 0.12.1. Import their generated types from
`dimcode/plugin`; no per-service adapter or branch setup call is needed. A row that injects
`dispatcher`, `branchLifecycle` or `timelineIdentity` runs in each branch of its Session.
Its services are branch-scoped. Keep Session tools, client methods and observers in separate rows.

| Service | Contract |
| --- | --- |
| `dispatcher` | `listen(name, keys, handler)` registers on the calling row's context. Its throw/rejection follows the native branch failure path. `committed/local/history` read committed history for reconstruction. |
| `treeCommands` | Session directory `list/info`, validated `input` by branch id and `create`. These are the Rust data methods, not the process `sessions` request envelopes. |
| `branchLifecycle` | Background work waits for `activated()` and releases its resources when `closing()` resolves. Never wait for activation inside a Dispatcher callback. |
| `timelineIdentity` | `get()` returns the original Session and branch ids. |

This part reconstructs its own input count, then follows live input. The native subscription
belongs to its row, so no second event channel or manual unsubscription is needed.

```ts
import { definePlugin } from '@arcships/rutis'
import type { BranchEvent, BranchServices } from 'dimcode/plugin'

const provides = { exampleCounter: { get: 'sync' } } as const

export default definePlugin({
  name: 'ExampleCounter',
  provides,
  inject: ['dispatcher'],
  async apply(ctx) {
    const dispatcher = ctx.use<BranchServices['dispatcher']>('dispatcher')
    let count = 0
    let through = 0n
    const fold = (event: BranchEvent) => {
      if (event.type === 'input' && event.name === 'message') count++
      through = BigInt(event.sequence)
    }
    let finishRebuild!: () => void
    const rebuilt = new Promise<void>(resolve => { finishRebuild = resolve })
    await dispatcher.listen('example-counter', ['input/message'], async event => {
      await rebuilt
      if (BigInt(event.sequence) > through) fold(event)
    })
    const cutoff = await dispatcher.committed()
    for (const event of await dispatcher.local(0, cutoff)) fold(event)
    through = BigInt(cutoff)
    ctx.provide('exampleCounter', { get: () => ({ count, through }) })
    finishRebuild()
  },
})
```

Configure the same declaration on its structural row:

```json
{
  "id": "example-counter",
  "name": "/absolute/counter/index.ts",
  "provides": { "exampleCounter": { "get": "sync" } },
  "inject": ["dispatcher"],
  "config": {}
}
```

Subscribe before sampling the committed cutoff. Rebuild from `local` through that fixed cutoff;
`history` also includes the ancestor prefix and is a different input. The handler waits only for
this initial fold, skips events already folded and resumes ordered consumption. A rebuild repeats
no external side effect. Publish a derived service after reconstruction. This example's state
belongs to the branch row and is rebuilt on each generation.

Callbacks can run on opening facts before publication. Register long work in the part's owned
tasks and return; do not wait for dispatch, activation or a child creation whose progress depends
on this callback returning. `treeCommands.input` commits validated input to the Timeline tail;
its reply does not prove dispatch or adoption. The native `create` cancels admission when its
future is dropped; its generated three-argument TypeScript signature supplies no explicit caller
AbortSignal or cancellation Promise. Do not infer one from the C4 tool cancellation contract.

`BranchEvent` reuses the existing event envelope and retains `number | bigint` sequences and
owner-defined payloads. Keep sequence values lossless. Errors are thrown `AgentErrorThrown` values
with native variant codes; the code alone does not authorize retrying a write. The generated types
also retain native serde field names such as `operation_id` and `branch_point`, rather than the
process `sessions` service's envelopes.

Other branch parts can inject a provided branch service. Addressing that service from a Session
part by branch id remains the open Goal contract; this guide supplies no lookup facade. See the
[C5 contract](design/branch-typescript.md) and [real Node part](../runtime/fixtures/branch-part.ts).
The export list and signatures come from [build.rs](../crates/dim-app-server/build.rs); regenerate
with `cargo check --locked -p dim-app-server`. CI verifies that the committed SDK declarations
match the generator and compiles this example from the actual packed archive.

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
