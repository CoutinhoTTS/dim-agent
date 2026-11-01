# MCP Apps host contract

Status: Draft.

Scope: Session, with one process decision (§3 D7).
Issue: [#451](https://github.com/arcships/dim/issues/451).
Rules: [design rules](../../architecture/rust-agent/principles.md), [ADR 002](../../adr/002-plugin-scope.md), [ADR 003](../../adr/003-approval-scope.md).

## 1. Scope and non-goals

An MCP server may deliver an interactive view with a tool: the tool's `_meta.ui.resourceUri`
names a `ui://` resource whose content is an HTML document
([MCP Apps, SEP-1865](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx)).
The client that shows the view is the **host**: it sandboxes the document, enforces the declared
CSP, runs the `ui/*` handshake, and forwards what the view asks for.

In Dim the MCP client — the connections, their credentials, OAuth state, tool discovery and tool
publication — lives in the daemon, owned by the Session that adopted the entry. A host therefore
cannot read a widget document or call a tool by opening its own connection: it would duplicate
daemon-owned configuration, credentials and OAuth state. This document owns what the daemon
provides a host for one rendered view:

- the document the entry declares for a tool of that Session, with its rendering metadata;
- a tool call made by a view, admitted like any other tool call;
- which of those a host may ask for, and what it never may.

It does not own:

| Surface | Owner |
| --- | --- |
| Sandboxing, CSP construction, `ui/initialize` and host context, display modes, theming, `ui/open-link`, `ui/resource-teardown`, `ui/notifications/*` | The client (Desktop, `arcships/dim-electron`) |
| MCP entry configuration, the `mcp` management namespace, the `mcp.test` probe | [MCP configuration management](mcp-management.md) |
| `session.mcp`, `mcp.auth.*`, the catalog's `auth` field, the credential store | [MCP OAuth](mcp-oauth.md#9-interfaces) |
| How a tool call is admitted (mode, rules, approval request) | [ADR 003](../../adr/003-approval-scope.md), [ADR 009](../../adr/009-permission-decides.md) |
| How an adopted input enters the model context | [Session access](../contracts/session-access.md), [Session boundary](../contracts/session-boundary.md) |
| Which clients exist and which Session methods they take | [Clients](../clients.md) |

## 2. Current behavior

`McpServer::tools` keeps a discovered tool's `name`, `description` and `inputSchema` and drops
`_meta`, so the owner discards the widget association a server declares. Nothing in the daemon
issues `resources/read`; `McpServer` knows `tools/list` and `tools/call` only. A tool result keeps
`content` and `structuredContent` (both kept in the Timeline's `ToolResult`) and drops the result's
`_meta`.

The client-facing surface stops one step short of a view:

- `session.mcp` reports per-entry state, version and rejection, and `mcp.inspect` reports the
  connections a Session holds. Neither carries tools or resources.
- `mcp.test` lists tools, but it opens a disposable connection with no Session, publishes
  nothing, and reports no resource ([MCP configuration management §4.5](mcp-management.md#45-probe-mcptest)).
- The only path that runs a tool is a model tool call: `ToolExecutor` admits it with
  `BranchSource::Tool` and writes a `tool/call` batch and its results into the branch's Timeline,
  from where they enter the model context ([Session boundary](../contracts/session-boundary.md)).
  A view's call is not a turn's call and must not become one.
- The only path that adds text to the conversation is `session.submit`, an adopted input.

So a Desktop client cannot render a widget, and cannot implement one itself without duplicating
the daemon's MCP client.

## 3. Decisions

### D1. The daemon stays the MCP client; the host contract forwards the view's requests

Rendering, sandboxing and the view's own message loop stay in the client. Everything the view asks
of its server — the document, a tool call, a resource — is a request on the Session's own
connection, which the daemon issues and the client never opens. This is the same boundary the
probe and `mcp.inspect` already respect: the daemon owns the connection, a client reads what it
needs through a method.

No second MCP client, no new connection, no new plugin, no new part: the operations use the
existing connection and the tool publication the Session already holds.

### D2. A view's operations are Session methods

Each operation acts on one Session's live connection and, for a call, on one branch's admission
decision. Both are Session- and branch-owned state, so the operations are Session methods
dispatched through `sessionId`, beside `session.mcp` and `session.mcp.refresh`
([Management methods § Dispatch and lifecycle](management-methods.md#dispatch-and-lifecycle)).
A management method cannot serve them: it is process-scoped, it cannot address a branch, and
branch `Permission` and `Approval` are not reachable from the process.

### D3. A view is identified by its Session, its entry and its tool; there is no handle

There is no widget session, no lease, no token and no daemon-side registry. Every request names
the Session it belongs to, the entry the Session adopted, and the tool; the daemon resolves them
against what the Session holds at that moment. Two consequences follow.

- Retiring a connection generation needs no invalidation: the next request uses the generation
  that is current then, and an entry that stayed the same keeps the same answer.
- The client's cache key is `session.mcp`'s per-entry `version`, which already changes exactly
  when the entry's discovered tools change. A view re-reads its document when that version
  changes or when the entry is no longer `ready`; a client that misses the change is corrected by
  the same rule, because the daemon answers from the current generation.

A runtime handle would add state to create, expire and clean up, and runtime handles are not
durable identities ([design rules §4 rule 8](../../architecture/rust-agent/principles.md)).

### D4. The mount read starts from the tool name the client already has

A `tool/call` fact carries the published tool name (`mcp_<label>_<hash>`), not the entry and not
the server's own tool name. So the mount read takes the published name, and its reply names the
entry and the server's tool, which the view's later calls need. It returns the document and
everything the host must apply to render it: the MIME type, the content, and `_meta.ui` (CSP
domains, sandbox permissions, dedicated domain, border preference). It also returns the tool's
description and input schema, which the host needs for `hostContext.toolInfo`.

The reply is a snapshot of the current generation. A tool the entry no longer declares, or one
that declares no resource, is `mcp_app_absent`; the daemon does not fall back to another tool or
another entry.

### D5. A view's tool call is admitted exactly like a model tool call

A view is untrusted content from a third-party server. Running a tool it names outside the
Session's permission decision would produce a second authority path that ignores the user's mode:
`read-only` would still write files because a view asked. So a view's call takes the same
admission as any other tool call on its branch — the branch's `Permission` decision, and the
branch's `Approval` request when the decision asks, through the existing services
([ADR 003](../../adr/003-approval-scope.md) D1's second sentence; [ADR 009](../../adr/009-permission-decides.md)).

The admission names the call's source as the host's own operation (`BranchSource::Host`), which is
the existing vocabulary for an operation a client started; no new event, field or variant is
introduced. A refusal answers `mcp_app_denied` with the reason.

The call is executed on the Session's connection, not through `ToolExecutor`: a view's call has no
turn, and its result must not enter the model context. The Timeline records the admission only
when the decision asked for an approval; nothing else is written (§7).

### D6. Only tools the server declares callable by an app can be called from an app

A tool's `visibility` decides who may call it; the default is both. The daemon therefore:

- keeps `visibility` from `_meta.ui` with the discovered tool, and refuses a view's call to a tool
  whose visibility excludes `"app"` (`mcp_app_not_callable`);
- does not publish a tool whose visibility excludes `"model"` to the Session's tool list, so the
  model never sees it. Such a tool is callable only through D5, by name.

The daemon cannot tell which view a host is forwarding for, so it enforces what it can see: the
tool exists in the entry the request names, and that entry is one of the Session's. The host's
side of the same rule is that a view's call goes to the entry whose document it mounted, because
that is what makes `"app"` mean "callable from this server's views". A tool of a different entry
is not refused by the daemon; it is a host that broke its own contract.

### D7. The daemon advertises the extension on every MCP connection

A server registers widget tools only when its client declares support, so the daemon's MCP client
declares `io.modelcontextprotocol/ui` with `text/html;profile=mcp-app` in `initialize`. The
declaration is process-wide, not per Session: a connection exists before and independently of who
views the Session, and a Session can gain a widget-capable viewer later.

The cost is that a server may now attach a `resourceUri` to tools of a Session no client renders
widgets for. Nothing else changes for that Session: a widget tool keeps meaningful text content,
which is each client's fallback, and a client that does not render views ignores the descriptor.

### D8. The view's message is an adopted input; model-context updates are not supported

`ui/message` is a user-role message to the conversation, which is exactly `session.submit`: one
adopted input, following the existing context rules. The host decides whether to send it; the
daemon records it as an ordinary input and does not record which view produced it.

`ui/update-model-context` has no conforming path and stays unsupported: the specification lets the
view replace its model context on every update, while an adopted input is handed over once and is
final ([design rules §4 rule 2](../../architecture/rust-agent/principles.md)), and the model
context holds only adopted inputs and tool results (§4 rule 6). Adding a replaceable context
channel would change the input mechanism for a view's convenience, so the host answers the request
with an error. A view that needs the model to see something sends `ui/message`.

### D9. The document a tool declares is the only resource the daemon serves

The specification lets a view issue `resources/read` for any resource of its server. The daemon
serves none of those as a resource read: the only document it fetches is the one §4.1 mounts, and
it is fetched because a tool of that entry declares it. A view asking for another URI is answered
from the document it already mounted, and anything else is a refusal the host produces — there is
no method that takes a resource URI, and so no allowlist to keep in step with a server's
`resources/list`. A server that needs more than its document can put it in the tool result's
`structuredContent`, which the host already has.

### D10. One size limit, on the document

The widget document is limited to 4 MiB (the text, or the decoded blob), because it is a document
the daemon hands to a sandbox and the host caches; a larger one answers `mcp_app_unsupported`.
Nothing else gets a new limit: a view's tool call has the same behavior as a model's tool call on
the same connection, and a limit that applied to one path only would be a second, unexplained
contract for the same connection. If abuse needs a limit, it belongs to the connection both paths
share, not to this surface.

### D11. A refusal is an error; a tool's own failure is data

The daemon answers the host's request with an RPC error only when it refuses to perform it: the
entry is gone or unusable, the tool is absent or not app-callable, the document is unsupported, or
the admission denied the call. A tool call that ran answers the tool's result, including a result
with `isError: true`, and a transport failure during the call is `mcp_unavailable` — the view is
told the call did not happen rather than receiving an empty result.

## 4. Contract

Session methods, namespace `session.mcp.app`, admitted exactly as `session.mcp` is: they are
execution-session methods, so the bare WebSocket answers `local_transport_required`, and the
owner-only local transport and the authenticated loopback web entry are served
([Session boundary A.3](../contracts/session-boundary.md#a3-entries-and-export)). Their names are
advertised in `hello.features` with the methods that require local execution, under the same
conditions. A remote paired device will need its own decision when RemoteGateway exists: a view's
document and its calls are untrusted content the daemon relays. `sessionId` is required
everywhere; `server` is the entry id `session.mcp` reports.

| Method | Request | Reply |
| --- | --- | --- |
| `session.mcp.app.widget` | `{sessionId, tool}` | `Widget` (§4.1) |
| `session.mcp.app.call` | `{sessionId, branchId, server, tool, arguments}` | `CallResult` (§4.2) |

### 4.1 Mount read (`session.mcp.app.widget`)

| Field | Type | Meaning |
| --- | --- | --- |
| `tool` | string | The published tool name from the `tool/call` fact |
| `server` | string | The entry the tool belongs to: the id `session.mcp` reports, and what `session.mcp.app.call` takes |
| `name` | string | The server's own tool name: what the view's `tools/call` sends |
| `description` | string or absent | The tool's description, as declared |
| `inputSchema` | object | The tool's input schema, for `hostContext.toolInfo` |
| `uri` | string | The declared `ui://` resource URI |
| `mimeType` | string | `text/html;profile=mcp-app` |
| `text` | string or absent | The document, when the server returned it as text |
| `blob` | string or absent | The document, base64, when the server returned it as a blob; exactly one of `text` and `blob` is present |
| `app` | object, absent when the resource declares no `_meta.ui` | `csp?: {connectDomains?, resourceDomains?, frameDomains?, baseUriDomains?}`, `permissions?: {camera?, microphone?, geolocation?, clipboardWrite?}`, `domain?: string`, `prefersBorder?: boolean`, each as declared |

```text
client (host)                     Session                       MCP entry
  | session.mcp.app.widget {tool}    |                              |
  |--------------------------------->| published tool name -> entry |
  |                                  |   and the server's tool name |
  |                                  | resources/read {uri}          |
  |                                  |----------------------------->|
  |                                  |<-- {contents:[...], _meta} ---|
  |<-- Widget {server, name, uri,    |                              |
  |            text|blob, app, ...}  |                              |
  | cache it under (server, version from session.mcp)              |
```

### 4.2 View tool call (`session.mcp.app.call`)

| Field | Type | Meaning |
| --- | --- | --- |
| `branchId` | string | The branch the view is rendered in: whose `Permission` and `Approval` decide the call, and whose client sees the approval request |
| `server` | string | The entry, from §4.1 |
| `tool` | string | The server's own tool name |
| `arguments` | object | The call's arguments, from the view |

The reply is the result the connection returned, in the shape the Timeline uses for a tool result:

| Field | Type | Meaning |
| --- | --- | --- |
| `content` | string or block array | The result's content, as `ToolResult.content` carries it |
| `structuredContent` | object or absent | The result's structured content; it is for the view, not the model |
| `isError` | boolean or absent | The server reported the call as failed; the call still happened |
| `meta` | object or absent | The result's `_meta`, which only this reply carries (§6) |

```text
client (host)                Session                       branch            MCP entry
  | session.mcp.app.call {...}  |                              |                  |
  |--------------------------->| tool callable by an app?      |                  |
  |                            |------------------------------------------->  |
  |                            | Permission decides, Approval asks if needed   |
  |                            |----------------------------->|                |
  |                            |<-- allowed / refused ---------|                |
  |                            | tools/call {name, arguments}  |                  |
  |                            |------------------------------------------------>|
  |                            |<-- {content, structuredContent, isError, _meta} |
  |<-- CallResult -------------|                              |                  |
```

The call is tracked by the connection's reply, so an interrupted request answers
`operation_interrupted`; a Session that closes, an entry that is disabled or a generation that is
replaced ends the call with `mcp_unavailable`. There is no per-call deadline: a model tool call on
the same connection has none either (D10).

### 4.3 Errors

| `appError` | When |
| --- | --- |
| `mcp_unknown_server` | `server` is not an entry of this Session's MCP selection |
| `mcp_unavailable` | The entry has no usable connection: disabled, connecting, failed, authorization required, or replaced while the call ran. The reason is `session.mcp`'s to report |
| `mcp_app_absent` | The tool is not declared by the entry's current discovery, or declares no widget resource |
| `mcp_app_not_callable` | The tool's `visibility` excludes `"app"` |
| `mcp_app_unsupported` | The declared resource is not a `ui://` `text/html;profile=mcp-app` document, carries neither text nor blob, or is larger than the limit (D10) |
| `mcp_app_denied` | The branch's permission decision or approval refused the call; the message carries the reason |
| `invalid_params` (`-32602`) | A missing or empty field, or a field of the wrong type; a tool the entry does not declare is `mcp_app_absent` |
| `local_transport_required` | The connection is the bare WebSocket |
| `session_closed` / `service_reloading` / `service_unavailable` | From the Session and the row that serves it ([management methods](management-methods.md#dispatch-and-lifecycle)) |
| `operation_interrupted` | The request was interrupted; the call may have run, so the view must not retry blindly |

## 5. What changes for a client

- A host mounts a view from a `tool/call` fact: it reads `session.mcp.app.widget` with the tool
  name it has, and caches the reply under the entry's `version` from `session.mcp`. It re-reads
  when that version changes or the entry stops being `ready`.
- It renders the document under the `app` metadata it received, sandboxed as the specification
  requires. Building the CSP, the `allow` list and the host context is the client's work and is not
  part of this contract.
- `tools/call` from the view becomes `session.mcp.app.call` with the entry and the server's tool
  name from the mount reply, and the branch the view is rendered in.
- `resources/read` from the view is answered from the mounted document; any other URI is a refusal
  the client produces itself (D9). `ui/update-model-context` is refused by the client (D8).
- `ui/message` becomes `session.submit`. Whether the user is asked to confirm is the client's
  decision; the daemon records an ordinary input.
- The Desktop client (`arcships/dim-electron#20`) replaces its local MCP Apps host's document read
  and tool call with §4 and keeps its sandbox, bridge and rendering.
- The native CLI and the Web workspace render no views. Their MCP tool results keep their text
  content, so a session they view is unaffected; the web entry is served the methods but its
  phase-1 UI mounts nothing.

## 6. What does not change

- `session.mcp`, `session.mcp.refresh`, `mcp.*`, `mcp.auth.*` and the probe keep their requests and
  replies. A view's state is read from `session.mcp`; this design adds no per-entry field there.
- No method is added to a management namespace, so no client needs a new capability check beyond
  the names in `hello.features`.
- The MCP business documents, entry sources, credential store and tool publication keep their
  formats; the tool name a Session publishes is unchanged, so no client's naming breaks.
- No Timeline event is added or changed. In particular the `tool/call` and `tool/result` facts, and
  the tool result's shape, stay as they are: a view's call is not a turn's call, and the tool
  result's `_meta` is not persisted.
- No new plugin, part, service, port or notification; the MCP Session part, its connections and the
  assembly are untouched.

## 7. Events and state

None. A view's call writes an `approval/request` only when the branch's decision asks for one,
which is the existing fact and its existing owner; nothing else about the call is a fact later
decisions use. The discovered tools a Session keeps — including the ones not published to the model
— are memory state rebuilt from `tools/list` on every connection, so a reload or a restart loses
nothing. The client owns everything else: the mounted document, the sandbox, the view's own state.

## 8. Dependencies

| Dependency | Use |
| --- | --- |
| `McpSession` ([MCP OAuth](mcp-oauth.md), [MCP configuration management](mcp-management.md)) | The Session's entries, their current connection generation and their discovered tools |
| `Permission`, `Approval` ([ADR 003](../../adr/003-approval-scope.md)) | The branch's admission of a view's call, read through the Session facade |
| `session.submit` ([Session boundary](../contracts/session-boundary.md)) | The view's message as an adopted input (D8) |
| MCP client (`rmcp`) | `resources/read` beside the existing `tools/list` and `tools/call` |

## 9. Cross-module requirements

None. Every operation is served by the owners that already hold the state: the MCP Session part for
the connection, the branch's `Permission` and `Approval` for the admission, and the existing input
path for a message. No requirement is raised to the runtime, the Timeline, the Dispatcher, the input
mechanism or rutis.

The two capabilities this design does not provide are decisions, not gaps in another owner:
`ui/update-model-context` conflicts with the finality of an adopted input (D8), and a resource read
beyond the mounted document is not served at all (D9).

## 10. Tests

Flow level, in `crates/dim-app-server/tests`, reusing the Node MCP fixtures under
`crates/dim-app-server/tests/fixtures/` (they already run the official SDK):

| File | What it protects |
| --- | --- |
| `mcp_session_flow.rs` | A widget tool's document is mounted from its published tool name and names its entry and the server's tool; a tool that declares no resource is `mcp_app_absent`; a document that is not a `ui://` `text/html;profile=mcp-app` or is larger than the limit is `mcp_app_unsupported`; a tool whose visibility is `["app"]` is absent from the Session's tool list but answers `session.mcp.app.call`, and one whose visibility is `["model"]` answers `mcp_app_not_callable`; a call naming an entry the Session does not hold answers `mcp_unknown_server`; admission is the branch's, so a `read-only` branch refuses a write tool with `mcp_app_denied` and an asking mode produces one `approval/request` whose `source` is the host operation, which `approval.respond` settles |
| `transport_flow/product_protocol.rs` | The wire contract: both methods are advertised in `hello.features`, refused with `local_transport_required` on a bare entry, and answer the error codes of §4.3; a view's call leaves no `tool/call`, `tool/start` or `tool/result` fact and does not enter the next turn's context |

The `mcp.test` probe's reply is asserted unchanged: a tool with a widget still reports its name,
description and schema, because the descriptor is not part of that reply.

## 11. Rule check

| Rule | Answer |
| --- | --- |
| §4 rule 1 (commands for another owner's state) | A view's call instructs the branch's admission exactly as a model call does (`approval/request` stays Approval's); the MCP owner writes nothing on the branch's behalf and the branch decides |
| §4 rule 2 (handover) | Not applicable: no object is handed between modules; `ui/message` uses the existing input handover |
| §4 rule 3 (`before` callbacks) | None |
| §4 rule 4 (core without business meaning) | No core change; the runtime, Timeline and Dispatcher gain no MCP Apps handling, and `BranchSource::Host` is used as it is |
| §4 rule 5 (batch) | Not applicable: no batch is written |
| §4 rule 6 (model context) | A view's tool result does not enter the model context: it is a reply to the client that asked, and no `tool/result` fact is written |
| §4 rule 7 (public events and services only) | The MCP Session part exposes the entry and its tools through its public service; the App Server reads the branch's `Permission` and `Approval` through the Session facade, as `session.permission.update` already does |
| §4 rule 8 (infrastructure unchanged) | No change to the Timeline, Dispatcher, event envelope, assembly, lifecycle or the Session and branch facades; no requirement is raised |
| §4 rule 9 (only facts later decisions use) | No new fact; only the approval a decision asked for is written |
| §1 (Capability and the App Server boundary) | No new part and no new Capability: the MCP owner keeps the connection and the discovery, the branch keeps admission, the client keeps rendering |
| ADR 002 D1 (scope follows the owner of state) | The connection and its discovery are Session state; the mode and the approval are branch state; the document and the sandbox are the client's |
| ADR 002 D2 (no wiring modules) | No new plugin, port or adapter; the two methods are added to the existing Session dispatch and the existing MCP owner |
| ADR 002 D3 (dependencies point to the core) | No dependency changes; `rmcp` and the branch services are already used |
| ADR 002 D4 (structural versus business configuration) | No configuration is added; the daemon's extension declaration (D7) is process behavior, not a stored setting |
| ADR 002 D5 (language-neutral contracts) | The requests and replies are JSON data; the resource URI, MIME type and `_meta.ui` fields keep the specification's spelling |
| ADR 003 D1 (approval covers agent decisions) | Unchanged and applied, not widened: a view's call is a tool call into an MCP server, admitted like any other; configuration-driven commands still need no approval |
| ADR 003 D3 (project entries start disabled) | Unchanged: a view of a disabled or unauthorized entry has no connection, so both methods answer `mcp_unavailable` |

## 12. Alternatives considered

| Alternative | Why not |
| --- | --- |
| A management namespace `mcp.apps` | The connection and the admission are Session and branch state; a process-scoped method cannot address a branch, and reaching into a Session from the process would be the wiring this repository removes |
| A widget handle: `app.open` returns a token, later calls carry it | State to create, replace and expire, and cleanup on three owners' lifetimes; the entry id and the tool name already identify the view, and the client's cache key is the version `session.mcp` reports |
| Let the client open its own MCP connection for the view | Duplicates daemon-owned configuration, credentials and OAuth state, and lets one entry run two connections with different authorization |
| Advertise the extension only while a widget-capable client is attached | Connections exist before and independently of the viewers; the declaration changes only what a server offers, and every widget tool keeps text content |
| Carry widget metadata in `session.mcp` | That state is subscribed to as a notification; a per-entry tool list belongs to a read on demand, and the existing `version` already tells a client when to re-read |
| Persist the widget descriptor and the result's `_meta` in the `tool/call` and `tool/result` facts | A frozen event format ([design rules §6](../../architecture/rust-agent/principles.md)) and a stored value that only presentation uses; the mount read answers from the current generation instead |
| Run a view's call through `ToolExecutor` | It needs a turn and a request batch, and its results become part of the model context; a view's call has neither |
| Let a view's call bypass `Permission` and `Approval` | A second authority path that ignores the session mode: `read-only` would write files because a view asked |
| Write a fact for every view-initiated call | It would either enter the model context as a tool result or need a new event kind; the view and its server own that record and the approval request is the only fact admission needs |
| Support `ui/update-model-context` as an adopted input | An adopted input is handed over once and is final, while the specification replaces the view's context on each update; changing the input mechanism for this would weaken the context rule |
| Expose `resources/read` by URI for a view | A view could read anything its server exposes, with no relation to what the host chose to render; the document a tool declares is the only thing a host needs, and a server that needs more puts it in the result's `structuredContent` |
| Prefetch every declared document on discovery | One request per tool for documents a client may never render; the client caches per version instead |
| Return the probe's tool list with descriptors | That reply belongs to [MCP configuration management](mcp-management.md); this design reads the descriptor in the Session, where a view lives |

## 13. Open questions

1. **A fact for view-initiated calls.** This design records only the approval a decision asked for
   (D5, §7). If the maintainer wants an audit trail in the conversation, it is a new event
   decision with its own context rule, not a field on this surface.
2. **Where admission authority ends.** [ADR 003](../../adr/003-approval-scope.md) D1 says approval
   governs actions an agent decides. D5 gives a view's call the same admission, because bypassing
   it would let a view run what the session mode refuses. A maintainer who reads D1 as excluding an
   operation a client started would have to decide the alternative explicitly: a view's call that
   no mode and no approval covers.
3. **The extension declaration per daemon profile.** D7 declares it on every connection. A
   protected profile that never renders views could stop declaring it; that would need a setting,
   which is why this design does not add one.
4. **Views in the Web workspace.** Phase 1 renders none; the methods are served to its loopback
   connection already, so a later phase is a client change, not a contract change.
