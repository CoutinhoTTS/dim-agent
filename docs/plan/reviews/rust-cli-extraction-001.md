# Rust CLI extraction verification

## Result

The public Rust workspace, native scrollback client, private TypeScript OAuth plugin and disabled release pipeline are implemented. The repositories build independently. macOS arm64 native packaging and local npm installation are verified. Other native platforms and actual publication require their configured runners and explicit activation.

Source input is Agent v4 commit `e79aa2257f77d2ace1041c768160f3514e839733`. Product baseline is `0.1.0`; Rust is 1.98.1, bundled Node is 24.21.0, rutis is 0.10.0 and the private host API is 1. All development commits and merge requests stay on GitLab.

## Evidence

| Boundary | Evidence |
| --- | --- |
| Public workspace | Locked independent build; one workspace run passed 453 Rust tests. Affected provider/storage/CLI flows were checked against the final contracts. No dependency on the original monorepo or private source is present in Cargo metadata. |
| Architecture | `xtask graph` and boundary checks pass from the standalone root; the CLI is included in source-derived module analysis. |
| Style/compiler | Final `cargo fmt --all -- --check` and workspace Clippy with `-D warnings` pass. |
| Public providers | Static provider, catalog, Codex/Grok and transport flows pass. Configured API-key calls use eight production workspace tools and restore the same persisted session. |
| Native CLI | Scrollback, Unicode streaming reconciliation, headless approval failure, terminal default-deny, Ctrl+C settlement, detach/resume and incompatible-daemon preservation pass. Real mock/configured profile conflicts preserve the process and endpoint. The npm launcher's real PTY flow delivers one interrupt and returns to a prompt. |
| Private plugin | Eight HTTP/store/plugin/archive flows pass; credential rotation across processes, stale-grant compare-and-delete, cancellation/close, recovery, backup and schema rejection are covered. The local development entry preserves arguments, spaces and private home permissions. |
| Public JS bridge | Actual Node/rutis with a neutral public plugin: normalized catalog, model HTTP 401, one credential recovery, both leases released, no secrets in history/error, Node crash confined to managed providers, API-key requests remain usable, graceful shutdown. |
| Private cross-runtime | Actual Rust App Server + rutis + private plugin: browser login, catalog normalization, model selection, authenticated HTTP stream persisted to history, logout and normal shutdown. Only external authorization/model HTTP servers are fixtures. |
| Storage | v11 documented full-record restoration passes. Previous v10 and unknown future formats are rejected; original database bytes remain unchanged. |
| MCP interoperability | Public dev-only SDK fixtures resolve from this repository's runtime manifest. Two stdio and one legacy SSE opt-in flows pass. Production packages omit their development dependencies. |
| Packaging | Eight package/trust-boundary checks pass, including exact tarballs, offline npm installation, digest/content changes, private archive links/schema/hash rejection, coherent platform provenance, publication receipt mismatch and npm retry integrity. |
| macOS installation | Real CLI/server/private plugin, bundled Node, native archive and platform npm archive verified; offline npm installation, version, providers/status and graceful daemon shutdown pass in isolated homes. |
| Source boundary | No Dim private provider IDs/auth fields/endpoints/classification code, old frontend dependency or legacy executable path remains in public production source/resources. Frozen TypeScript fixtures are wire-only data contracts. Public export excludes internal plans and Git history. |
| Publication | Every GitHub template remains `.yml.disabled`; YAML structure checked. No workflow dispatch, tag publication, npm publish or GitHub synchronization occurred. |

The private runtime archive is `dim-oauth-plugin-0.1.0.tar.gz`, SHA-256 `6e5689388616aa25cadcebe864d9566321717173435474987b2b99e4bb095210`. It contains exactly `package.json`, `dim-plugin.json` and `dist/index.js`; private source, source maps, tests and account files are excluded. Its SDK is bundled, while the product supplies the locked runtime. `NOTICE` excludes the private artifact from the public MIT grant.

Final receipt checks bind GitHub Release archives to the exact build consumed by npm publication, including package digests. Retries skip only already published archives with matching registry SHA-512 integrity; registry errors and different content fail before writes.

The development-only MCP SDK is pinned to 1.31.0, the fixed version identified by the [upstream advisory](https://github.com/modelcontextprotocol/typescript-sdk/security/advisories/GHSA-6qxp-vccf-f47h). Dependency installation reports no known vulnerabilities. It is not shipped in product runtime payloads.

## Independent acceptance and simplification

Provider/CLI implementation and independent acceptance used different executors. The private-plugin executor also checked actual public package contents and installation. The CLI executor reviewed provider contracts and ran the npm wrapper through a real terminal flow. The parent integrated and verified public boundaries, storage, packaging and the private implementation.

Removed unnecessary layers include the old TypeScript CLI/core, OpenTUI, the private Rust OAuth implementation, provider-specific relay classification, retired TypeScript fixture generator, compiler-cache installer, redundant CLI event wrappers, duplicate lease expiry sweeps and protocol string conversion. Existing rutis owns plugin loading; no second event bus, plugin framework, account writer or private App Server was introduced. Instance identity, scoped bindings, lease generations, persistence locks/CAS and archive validation remain because they protect real boundaries.

## Not verified or enabled

- Production Dim authorization or cloud model calls with real credentials.
- Native Linux arm64/x64, macOS x64 and Windows execution/installation; the Windows CLI was compile-checked only.
- GitHub self-hosted runner labels, internal artifact HTTP authentication and real npm OIDC publication. Workflow templates and local archive boundaries are checked, but no job is enabled or triggered.
- The opt-in live Context7 network test, which requires an external API key.

Old databases have no migration path in this extraction. Use an isolated home; unsupported files are preserved. Select a new unused npm product version and configure environments/publishers before any future activation.

The public source archive was unpacked outside either source checkout and built with the locked dependencies. Its configured API-key CLI flow also exercised production workspace tools and persisted-session restoration. The release-profile macOS platform archive passed an actual offline npm installation and account/plugin discovery; its 305 staged files match the manifest and no development SDK is packaged.

Unix home paths must fit the platform's socket-path limit after canonicalization. macOS test homes should use a short path such as `/tmp/dim-dev`; a 105-byte canonical socket path is rejected explicitly and logged without changing account data.
