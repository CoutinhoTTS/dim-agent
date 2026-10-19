# Web

Optional Session TypeScript plugin implementing the [Web contract](../../../docs/design/plugins/Web.md). It provides `toolContribution:web_search` and `toolContribution:web_fetch` through the existing C4 loader. Both declarations use `risk: "network"`; Permission and ToolExecutor retain admission, approval and terminal-fact ownership.

## Loading

Use the repository's pinned Node version and `npm ci --prefix runtime --ignore-scripts`. The runtime's production dependencies include the rutis SDK, HTML parser/Markdown converter and IP/CIDR parser.

Pass the complete [web.json](web.json) declaration through `ServerConfig.session_typescript.rows`, with `anchor` pointing to `runtime/package.json`. Its relative module name resolves to `plugins/web/index.ts` beside that anchor. The module imports `inject` and `provides` from the same declaration; the row also owns the `web` Settings source. Select `web_search` and/or `web_fetch` in `SessionConfig.tools`. This package is optional and is not installed by the default CLI profile; editable package installation belongs to PluginManager.

The required named service is `settings`. Missing dependencies retain the loader's Pending behavior. ToolBinding samples Active contributions during branch initialization; later activation or structural reload follows the existing C3/C4 contract. Configuration edits take effect on the next call without a reload.

## Configuration

Each call reads `settings.read({source: "web", scope: "user"})`. Settings validates and defaults the document. The source declares the user file `web.json`, no project file, and these fields:

| Field | Meaning |
| --- | --- |
| `searchService` | Optional `{kind, endpoint?, apiKey?}`; initial built-in adapter: `brave` |
| `allowPrivateHosts` | Exact normalized hostnames or CIDRs, default `[]`; invalid entries are ignored with a fixed diagnostic |
| `proxy` | Optional HTTP(S) proxy origin supporting CONNECT to numeric addresses; no authentication or direct fallback |

`searchService` and `allowPrivateHosts` are user-only writes. Local clients update the source through `settings.update` using the revision from `settings.read`; Web never writes configuration. User-only does not hide values from authorized local reads. Native storage protects the document with the product-home boundary described in the design.

Brave uses its [Web Search API](https://api-dashboard.search.brave.com/app/documentation/web-search/get-started), default endpoint `https://api.search.brave.com/res/v1/web/search`, GET parameters `q` and `count`, and header `X-Subscription-Token`. Configure `searchService.kind` as `brave` and `searchService.apiKey` locally. The optional endpoint uses the same API shape, including `type: "search"` and optional `web.results`. A missing `web` section means zero web results; a result without a description has an empty snippet. There is no backend registry. An absent or unusable configuration returns `search_unavailable`.

The API key is used only for the current call. Results, errors and diagnostics redact its raw and URL-encoded forms; invalid key-bearing search URLs fail without returning them. Requests carry no ambient cookies, credentials or referrer. Fetch does not send the search token.

## Tools

| Tool | Parameters | Result |
| --- | --- | --- |
| `web_search` | `query`: 1–500 Unicode scalars after trimming; `max_results`: 1–10, default 5; mutually exclusive `allowed_domains` / `blocked_domains`: up to ten hostnames | Numbered plain titles, URLs and snippets of at most 500 scalars; every result is post-filtered, including subdomains. Zero results is successful. |
| `web_fetch` | HTTP(S) `url`: at most 2048 scalars, no userinfo; `start_char`: non-negative safe integer, default 0; `max_chars`: 1000–50000, default 20000 | URL, title, media type, character range and total, followed by text. Truncation and download-limit notices are model-visible. Continue at the returned final URL. |

HTML/XHTML becomes Markdown, with absolute links, navigation and executable/control elements removed, preferring `main`, then `article`. Header charset precedes HTML meta charset and UTF-8 replacement. Plain text, Markdown, CSV, JSON and XML stay text; valid JSON is indented without losing numeric literals. Binary formats are rejected. Error responses never expose their bodies.

Each request resolves and checks every target address, then connects to one checked IP. Private, loopback, link-local, shared, multicast, reserved and equivalent IPv6 addresses are blocked unless covered by a user exception. A proxy receives `CONNECT <validated IP>:<port>` for both HTTP and HTTPS. Host and TLS identity retain the original hostname. DNS lookup, proxy establishment and TLS negotiation share the 10-second connect deadline. Search has a 20-second total deadline; fetch has 30 seconds. Responses stop at 5 MiB after decompression and close the connection before returning the prefix.

At most five redirects are followed within the normalized hostname group (lowercase, leading `www.` removed), checking addresses on each hop. Cross-host redirects return a notice and require another tool call. Fetch cache entries use the final URL, expire after 15 minutes and share a 50 MiB UTF-8 payload budget per Session; oldest entries are evicted first. Concurrent fetches are independent and the first completed conversion supplies the cached page. Reload and restart discard the cache and do not replay network operations.

The existing by-reference cancellation Promise aborts owned sockets and timers. Row cleanup cancels and drains calls before releasing the cache. Web writes no Timeline event or durable state. Non-fatal diagnostics are in `structuredContent`; pagination handles and actionable errors remain in `content`.

## Verification

```sh
node runtime/node_modules/typescript/bin/tsc -p runtime/plugins/web/tsconfig.json
node --test runtime/plugins/web/web.test.mjs
DIM_TEST_NODE="$(command -v node)" \
  DIM_TEST_JS_RUNTIME="$PWD/runtime/node_modules/@arcships/rutis-runtime" \
  cargo test --locked -p dim-app-server --test transport_flow node_web_ -- --ignored
```

The Node flows use the public rutis test loader and real loopback HTTP, TLS and CONNECT servers. They cover tool output, configuration changes, address pinning, TLS identity, proxy failure, redirects, bounded conversion, concurrent cache fills, cancellation, unload and deadlines. TLS fixtures generate temporary certificates with OpenSSL. No live Brave request or real API key is required.

The native transport flow uses real Settings, C4, Permission, ToolExecutor and SQLite in a disposable home. It covers hot configuration, denied/approved/cancelled approval, slow-response interruption, a single terminal result, cache pagination and restore without repeating completed requests. Crash settlement and cancellation before dispatch remain ToolExecutor's existing flow coverage; this test does not simulate a process crash during a Web request.
