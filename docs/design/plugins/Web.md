# Web

Status: Draft. Overall design: [overview §5.5](../overview.md#55-web). Phase: P2. Language: TypeScript first ([overview §11.3](../overview.md#113-assignment)).

Web is a process plugin that gives the model two tools, `web_search` and `web_fetch`, through the [process-level tool contribution](../contracts/process-tool-contribution.md) contract. Search goes to a configured search service; fetch downloads one public web page and returns bounded text. Web writes no Timeline events: every call and result is already a tool call fact.

## 1. User flows

1. The user configures a search service and a credential reference through Settings. Web publishes a new tool set version that includes `web_search`; each Session's branches adopt it before their next request.
2. The model calls `web_search`. Approval admits the call by the `network` risk category (ask under workspace-write, deny under read-only, allow under FullAccess). Web queries the service and returns a numbered result list as `ToolResult.content`.
3. The model calls `web_fetch` with a URL. Web validates the URL and each resolved address, downloads at most the byte limit, converts the page to text, and returns a header plus a bounded slice of the body. If the body is longer, the model calls again with `start_char` to read the next slice.
4. The user interrupts the turn while a fetch runs. The request is aborted, the handler returns a cancelled error result, and ToolExecutor settles the batch.
5. The credential is removed from Settings. Web publishes a set without `web_search`; calls bound to the previous version that run after the removal return `search_unavailable`.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| `web_search` and `web_fetch` definitions, parameters, results and limits | Admission decisions (Approval, Permission own the `network` category) |
| Search adapters behind a Web-defined backend contract | Browser automation, JavaScript rendering, logged-in pages |
| HTTP fetching, redirect and address policy, content conversion | Sending data to sites (only `GET`; no forms, cookies or auth headers) |
| Reading its own configuration section and resolving credential references | Storage of secrets (host secret store) |
| Per-process fetch cache for pagination | Any Session or branch state |

## 3. Scope and dependencies

Web is a single process part. It has no Session or branch part: everything it needs per call arrives through `ProcessToolSession` and `ToolExecutionContext`.

**Contracts depended on**

| Contract | Owner | Use | Status |
| --- | --- | --- | --- |
| `ProcessToolSource` / `ProcessToolProvider` / `ProcessToolHandler` | ToolRegistry ([contract](../contracts/process-tool-contribution.md)) | Offer the two tools; receive calls with a cancellation token | Proposed (P0) |
| Typed plugin configuration read and change notification | Settings ([overview §7.2](../overview.md#72-app-server-process-plugins), #3) | Read the `web` section of the loader editable layer and observe changes without a plugin reload | Proposed |
| Secret reference resolution | Host secret store | Resolve `secretRef` values at call time | Proposed generic form; the existing `ResolveProviderSecret` is keyed by Provider id (see §14) |
| `ToolDefinition.risk = "network"` | Permission (`Risk::of` in [permission.rs](../../../crates/dim-agent-capabilities/src/permission.rs)) | Admission | Existing |
| Diagnostics channel | Trace | Request logs without content or secrets | Existing |

**Contracts defined here** (owner: Web): the tool names, parameters and result formats in §6; the `WebSearchBackend` extension point; the `web` configuration section. Web does not depend on any specific search provider plugin. A search backend contributed by another plugin (for example one backed by an account Provider, which the feature list calls "account permissions") implements `WebSearchBackend` under a name; Web injects them as `When<ProcessContributions<WebSearchBackend>>` (A1). Without contributions, only Web's built-in HTTP adapters exist.

| Part | Scope | Injects | Provides |
| --- | --- | --- | --- |
| `Web` | Process (control node by default) | Settings configuration read, secret resolution, `When<ProcessContributions<WebSearchBackend>>` | `ProcessToolSource` registered as `web` |

**Node.** Web performs network access from the node that runs its loader row. By default that is the control node, independent of a Session's resource node, because the network is not a Session-bound host resource in [overview §3.1](../overview.md#31-multiple-nodes) (A2). An operator may load Web as a `peer:<node>/web` row so egress leaves from another node; then every Session uses that node. Web never uses FileStorage or HostPlatform.

## 4. Events

None. Each call is recorded by the existing `step/tool-call`, `step/tool-start`, `step/tool-completed` and `step/tool-result`; admission by `approval/*`. Search results and fetched text are in the tool result. Configuration changes are tool set versions recorded by the branch's `context/configured`. Nothing Web knows affects later behavior except through these facts.

## 5. State

| State | Where | Rebuild |
| --- | --- | --- |
| Current configuration snapshot | Web memory, from Settings | Re-read on `apply` and on each Settings change notification |
| Current tool set and version | Web memory | Recomputed from configuration; version = generation id + counter, never reused |
| Fetch cache (A9) | Web memory, bounded | Not persisted; empty after reload or restart |
| In-flight requests | Web memory, one abort handle per call | Not rebuilt; aborted on unload |

Web owns no Timeline state and no `StateCell`.

## 6. Interfaces

### 6.1 Tool set

One set, key `web`. `web_fetch` is always present while Web is loaded. `web_search` is present only when a search service is configured and its adapter or backend is available (A3). Both tools: `risk: "network"`, `requires: { interactive: false, roles: None }` (offered to every Session, including non-interactive ones).

### 6.2 `web_search`

| Parameter | Type | Rule |
| --- | --- | --- |
| `query` | string | Required; 1 to 500 characters after trimming (A4) |
| `max_results` | integer | Optional; 1 to 10; default 5 (A4) |
| `allowed_domains` | string[] | Optional; at most 10 hostnames; results restricted to these hosts and their subdomains |
| `blocked_domains` | string[] | Optional; at most 10; mutually exclusive with `allowed_domains` |

Domain filters are passed to the service when it supports them and applied by Web to the returned list in every case. Parameters are validated by the handler (the external boundary); a violation returns an error result without contacting the service.

`ToolResult.content` (text):

```
Search results for "<query>" (<n> results)

[1] <title>
<url>
<snippet>

[2] ...
```

Titles and snippets are reduced to plain text, one line each, snippet at most 500 characters. Zero results yield `No results for "<query>".` (not an error). The service name is not shown to the model. `structured_content` carries `{ results: [{ title, url, snippet, published_at? }] }` for clients; it is not model text. Total time limit 20 seconds (A5).

### 6.3 `web_fetch`

| Parameter | Type | Rule |
| --- | --- | --- |
| `url` | string | Required; absolute `http` or `https`; at most 2048 characters; no userinfo (`user:pass@`); fragment removed (A6) |
| `start_char` | integer | Optional; default 0; offset into the converted text |
| `max_chars` | integer | Optional; 1000 to 50000; default 20000 (A7) |

`ToolResult.content` (text):

```
URL: <final url>
Title: <title, if any>
Content-Type: <media type>
Characters: <start>-<end> of <total>
[Truncated: call web_fetch again with start_char=<end> to continue.]
[Download limit reached: the page exceeds <limit> and later content is missing.]

<converted text slice>
```

The bracketed lines appear only when they apply. `structured_content`: `{ url, final_url, status, content_type, title?, start, end, total, truncated, download_limited }`.

Fetch rules:

| Item | Rule |
| --- | --- |
| Method and headers | `GET` only; fixed `User-Agent: dim-web/<version>`; `Accept` preferring HTML and text; no cookies, no stored credentials, no referrer (A8) |
| Limits | Connect 10 s; whole call 30 s; at most 5 MiB of decoded body read, then the connection is closed and `download_limited` set (A5) |
| Status | 2xx: converted; 3xx: redirect rules; 4xx/5xx: error result `http_status` with code and reason phrase, no body |
| Redirects | Up to 5 hops. Every hop target is revalidated against the URL and address rules. A redirect to a different host (after lowercasing and removing a leading `www.`) is not followed: the result is a non-error notice `Redirected to <url>; call web_fetch with that URL to follow it.` so the new host is admitted as a separate call (A10) |
| `text/html`, `application/xhtml+xml` | Charset from header, then `<meta>`, else UTF-8 with replacement. `script`, `style`, `noscript`, `template`, `svg`, `iframe`, navigation and form controls removed; main content preferred when detectable; converted to Markdown with absolute links; whitespace normalized |
| `text/plain`, `text/markdown`, `text/csv`, `application/json`, `application/xml`, `text/xml`, `*+json`, `*+xml` | Decoded text returned as is (JSON pretty-printed if valid) |
| Other types (images, PDF, archives, binaries) | Error result `unsupported_content_type` naming the type; nothing returned (A11) |
| Empty conversion | Non-error result with header and `(no text content)` |
| Slicing | Converted text is sliced by Unicode scalar values `[start_char, start_char + max_chars)`; `start_char >= total` returns an empty slice with the header |

### 6.4 Address policy (SSRF)

| Rule | Content |
| --- | --- |
| Resolution | Web resolves the hostname itself, validates every resolved address, and connects only to a validated address (the connection is pinned to it), so DNS rebinding between check and connect is impossible |
| Blocked by default | Loopback (`127.0.0.0/8`, `::1`), unspecified, RFC 1918 private, link-local (`169.254.0.0/16`, `fe80::/10`, which includes cloud metadata endpoints), CGNAT `100.64.0.0/10`, unique local `fc00::/7`, multicast, broadcast, reserved ranges, IPv4-mapped and IPv4-compatible IPv6 forms of all of these, and hostnames `localhost` and `*.localhost` |
| Literal IPs | Allowed only when public by the same rules |
| Ports | Any port on a public address (A12) |
| Exceptions | `web.fetch.allow_private_hosts`: exact hostnames or CIDRs the user permits; empty by default. Written only through Settings by the user, never by a tool |
| Proxy | When `web.proxy` is configured, Web still validates the resolution it performs locally and sends the request through the proxy; the proxy's own resolution is outside Web's control (stated in §11) |

### 6.5 `WebSearchBackend` (extension point defined by Web)

```rust
/// Process service; key TypeKey::keyed_dynamic::<WebSearchBackend>(name). Defined by Web.
pub trait WebSearch: Send + Sync + 'static {
    fn available(&self) -> watch::Receiver<bool>;
    fn search(&self, request: SearchRequest, cancel: CancellationToken)
        -> HostFuture<'_, Vec<SearchHit>>;
}
pub struct SearchRequest { pub query: String, pub max_results: u8,
    pub allowed_domains: Vec<String>, pub blocked_domains: Vec<String> }
pub struct SearchHit { pub title: String, pub url: String, pub snippet: String,
    pub published_at: Option<String> }
```

A backend knows nothing about tools, Sessions or Web's configuration; Web knows nothing about the backend's account or provider.

### 6.6 Configuration (`web` section, loader editable layer)

| Key | Type | Default |
| --- | --- | --- |
| `web.search.service` | `{ kind: "<built-in adapter id>", endpoint?: string, secretRef?: string }` or `{ backend: "<contributed backend name>" }` or absent | absent (no `web_search`) |
| `web.fetch.allow_private_hosts` | string[] | `[]` |
| `web.proxy` | string (URL) | absent |

The set of built-in adapter ids is an implementation choice listed in the plugin's README (A13). Credentials appear only as `secretRef`. There are no Web management methods: the section is read and written through Settings' typed operations and the CLI's `/settings` aggregate entry.

No Session methods, no management methods, no CLI subcommand.

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Publish tools | Web's `apply` reads configuration, builds the set, registers its `changed()` sender and the Settings subscription with `ctx.effect`. ToolRegistry adapts and refreshes; branches adopt through ToolBinding into `context/configured` (`Timeline::append_batch_registered` by Prompt's configuration publisher). Web writes nothing |
| Configuration change | Settings notification → Web rebuilds the set with a new version and signals `changed()`. Not a plugin reload |
| Call | ToolExecutor consumes `step/tool-call` on the Dispatcher, registers the batch task and returns; per call appends `step/tool-start` (`Timeline::append`), Approval admits, the handler (Web) runs on the executor's task, `step/tool-completed`, then one `step/tool-result` for the batch |
| Dispatcher | Web has no Dispatcher subscription; the handler never runs on the Dispatcher |
| State and recheck | No `StateCell`. The handler decides only from its parameters and Web's own configuration snapshot at call start; it consumes no Timeline facts |
| Terminal facts | Written by ToolExecutor (the initiator of the call). Web returns exactly one `ToolResult` per call |
| Web reload | rutis reloads injecting ToolRegistries and downstream branches per the [reload protocol](../../architecture/rust-agent/single-tree.md); in-flight batches are settled as reload by ToolExecutor; Web's disposer aborts all requests first. The new generation publishes a set with a new version; branches re-adopt |
| Process restart | Committed calls without results are settled by ToolExecutor as not surviving recovery; Web never repeats a request |

## 8. Timing and conflicts

Web makes no Timeline decision; all rows are decided by the owners named in the contract.

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Turn `stop/interrupt` before the batch is dispatched | interrupt < `step/tool-call` consumption | Not executed | ToolExecutor |
| Turn `stop/interrupt` during approval | `approval/request` < interrupt | `approval/cancelled`; no request sent | Approval |
| Turn `stop/interrupt` during the request | `step/tool-start` < interrupt | Token cancelled; Web aborts the socket and returns error `cancelled`; a response already fully converted before the abort is returned as is | ToolExecutor, Web |
| Branch-scoped interrupt or branch close | branch interrupt | Same as turn interrupt; Web keeps no work after returning | ToolExecutor |
| Batch with another long call | Web returned, other call running | Result held until the batch's `step/tool-result` | ToolExecutor |
| Concurrent Steer/Queue/Append | Any | No effect on the running call; Steer is consumed after the batch result | Loop |
| Compaction after the result | `step/tool-result` < compaction | Fetched text survives only through the summary; the model can fetch again | ContextPreparation |
| Child branch cut | cut after `step/tool-result` | Child inherits the result; Web holds no per-branch state | — |
| Credential removed while a call bound to the old version runs | set V2 published during call | Call continues with credentials resolved at its start | Web |
| Old-version `web_search` called after removal | call > V2 publication, request bound to V1 | Error result `search_unavailable` | Web |
| Web reload or process restart mid-call | no result yet | Settled by ToolExecutor; not repeated | ToolExecutor |
| Session unload and restore | `branch/resume` | Tools re-adopted; cache unaffected | ToolRegistry, ToolBinding |
| Resource node offline | — | No effect: Web does not use resource services. If Web itself runs as a peer row and that node is offline, Web is Pending and injecting ToolRegistries are Pending | rutis lifecycle |
| Two branches fetch the same URL concurrently | — | Two independent requests; cache filled by whichever finishes first | Web |
| Client detach | — | No effect | — |

## 9. Context mapping

| What the model sees | Form | Position |
| --- | --- | --- |
| `web_search`, `web_fetch` name, description, input schema | Tool declarations | Adopted `context/configured.tools`; replaced when a new set version is adopted |
| Search results, fetched text, errors and redirect notices | `ToolResult.content` (text only) | Tool result position of the call's batch |

Declarations are configuration and survive compaction as the latest adopted set; results survive only through the compaction summary. Child branches inherit declarations and results in their ancestor prefix. Non-interactive Sessions see the same tools. The model never sees `structured_content`, the search service or backend name, configuration, credential references, the private-host exceptions, request logs or diagnostics. Fetched page text is untrusted third-party content placed in a tool result; Web adds no instruction framing around it (A14).

## 10. Errors and recovery

Every error is an error `ToolResult` (`is_error: true`) whose content is `web_fetch failed: <code>: <message>` or `web_search failed: ...`. No error affects other calls or the Session.

| Code | Meaning |
| --- | --- |
| `invalid_input` | Parameter violates §6 |
| `blocked_address` | URL host or any redirect hop resolves to a blocked address; the message names the rule, not the address list |
| `dns_failed`, `connect_failed`, `tls_failed` | Network errors |
| `timeout` | Connect or whole-call limit reached |
| `http_status` | Non-2xx final status |
| `too_many_redirects` | More than 5 hops on the same host |
| `unsupported_content_type` | Type not in §6.3 |
| `search_unavailable` | No service configured, backend unavailable, or credential missing |
| `search_failed` | Service returned an error or invalid data; the service's message is shortened and stripped of secrets |
| `cancelled` | Token cancelled |

Recovery: Web holds no durable work; reload and restart lose only the cache.

## 11. Security

- Admission: both tools declare `network`; every call passes Approval and Permission. FullAccess allows them; read-only denies; workspace-write asks.
- SSRF: §6.4. Cross-host redirects require a new call and therefore a new admission (A10).
- No ambient authority: no cookies, no stored site credentials, no client certificates; requests carry nothing from the Session except the URL the model wrote.
- Credentials: resolved per call from `secretRef`; never in definitions, results, `structured_content`, diagnostics or errors.
- Exfiltration: a model can encode data in a URL; this is the inherent risk of the `network` category and is controlled by admission, not by Web.
- Prompt injection: fetched content is untrusted; it reaches the model only as a tool result.
- Proxy: with `web.proxy`, the proxy resolves names itself; private-address protection then depends on the proxy (documented to the user).
- Isolation: Web runs in the App Server process (or its peer row), not under a Session's isolation configuration.

## 12. Acceptance

Flow tests through public interfaces (tool calls in a test Session, local test HTTP server reachable only through a test `allow_private_hosts` entry, a fake search backend):

1. Without a search service, only `web_fetch` is declared; configuring one publishes `web_search` in a new version adopted before the next request; removing it withdraws it, and an old bound call returns `search_unavailable`.
2. `web_search` result format, domain filters, zero results, service error.
3. `web_fetch` of HTML returns the header and Markdown; `start_char` pagination reaches the end; `max_chars` bounds; download limit sets the notice.
4. Plain text, JSON, unsupported type, 404, empty page.
5. Same-host redirects followed up to 5; sixth fails; cross-host redirect returns the notice.
6. Loopback, private, link-local, IPv4-mapped IPv6, `localhost`, and a hostname resolving to a private address are blocked without allowlist; DNS rebinding test resolves public then private and is not connected.
7. Permission: read-only denies, workspace-write asks, FullAccess allows.
8. Conflict rows: interrupt before dispatch, during approval and during a slow response (socket closed, one result); reload and restart settlement without repeated requests.
9. Non-interactive Session declares both tools.
10. Diagnostics and results never contain the resolved credential.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Search backends from other plugins plug in through a Web-defined `WebSearchBackend` contract | Feature list ("configuration or account permissions"); overview §3 constraint 10 | Account-backed search must be a built-in adapter of Web |
| A2 | Network access happens on the node running Web (control node by default), not on the Session's resource node | Overview §3.1 lists network services as process resources; judgment | Web would need a Session-bound network service on each resource node |
| A3 | `web_search` is withdrawn from the set when no service is usable | Judgment | Tool always declared, failing at call time |
| A4 | Query length 500, `max_results` 1 to 10 default 5 | Judgment | Limits change |
| A5 | Timeouts 10 s connect, 30 s fetch, 20 s search; 5 MiB download limit | Edge-case reference; judgment | Limits change |
| A6 | Only `http`/`https`, URL at most 2048 chars, no userinfo | Judgment | Wider input surface |
| A7 | `max_chars` 1000 to 50000, default 20000, with `start_char` pagination | Judgment | Different bounding of tool results |
| A8 | `GET` only, fixed user agent, no cookies or credentials | Judgment | Authenticated fetching would need a separate design |
| A9 | In-memory cache of converted text for 15 minutes, at most 50 MiB, keyed by final URL, used so pagination does not refetch | Judgment | Each page slice refetches; results may differ between slices |
| A10 | Cross-host redirects are reported, not followed | Edge-case reference; judgment | Redirects silently reach hosts never admitted |
| A11 | PDFs and images are not converted in this version | Judgment | Add extraction or image content blocks |
| A12 | Any port on public addresses is allowed | Judgment | Restrict to 80/443 |
| A13 | The built-in search adapter list is an implementation choice | Overview §5.5 names no service | Overview or feature list fixes a service |
| A14 | No wrapper or warning text is added around fetched content | Judgment | Model-visible framing to define |

## 14. Open questions

| Question | Description |
| --- | --- |
| Settings change without reload | #3 makes plugin configuration loader-row configuration; a row config change may reload Web, which by the [tool contribution contract](../contracts/process-tool-contribution.md) reloads every injecting ToolRegistry and fails running turns. Web needs a typed configuration read with change notification that is not a reload |
| Generic secret resolution | The existing `ResolveProviderSecret` takes a Provider id; Web and Notifications need a host secret resolution not tied to Provider |
| Cross-language migration | Overview §5.5 names Web as the first #4 migration target; since Web needs no capability port, the TS implementation uses only process services |
