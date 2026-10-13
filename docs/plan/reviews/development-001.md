# Local development verification

## Result

The public development launcher, local private-source package preparation, shared archive boundary and delegated private entry pass independent acceptance on macOS arm64. Local loading uses source-built packages only. No private release CI, remote artifact downloader, GitHub synchronization, release tag or npm publication is part of this change.

The public source baseline is `7ed00aa515a3c30473eb54b1a2d93889e9ffd45a`; the private source baseline is `05f83f7ce45fffab8fca6a75289f477c91be75a6`. The implementation follows [the development contract](../../architecture/development.md).

## Evidence

| Boundary | Verification |
| --- | --- |
| Actual local source setup | Two `./dev setup` executions used the sibling private repository, pinned Node 24.21.0 and pnpm 12.3.4. Both built `pack:runtime`, verified the archive and extracted exactly `package.json`, `dim-plugin.json` and `dist/index.js`. The extracted entry digest matches the private source build. No Cargo invocation is required for setup. |
| Dependency reuse | The first setup ran frozen pnpm installation and locked npm installation. The second setup log contains only plugin build/pack and verification; neither dependency installation repeats. Evidence: `/tmp/dim-dev-setup-first-20261013.log` and `/tmp/dim-dev-setup-second-20261013.log`. |
| API-key independence | The developer flow passes with a deliberately failing Node executable and no private source. It preserves caller working directory, model path and prompt arguments with spaces, redirected input and native exit status 23. Build diagnostics stay on stderr; stdout remains native output. |
| Environment and home | Inherited development settings override `.env.dev`. The flow checks paired model defaults, omission on resume, isolated home permissions and a cold `stop` that creates no home and invokes no build. macOS `/bin/bash` 3.2 also runs `./dev help` with all five inherited settings absent. |
| Immutable archive verification | A flow replaces the source archive with a different valid package when tar is invoked. Listing, type validation and extraction consume the same SHA-256-verified Buffer, so the original verified entry is extracted and its provenance remains correct. |
| Preparation failure | The local-source failure flow and an independent temporary two-repository check supplied an actual tar/checksum with unsupported host API 99. The launcher exited 1, preserved the previous verified package byte-for-byte, did not create a product home, and did not invoke Cargo or the native CLI. External pnpm/npm calls were successful fixtures; archive reading and validation were real. |
| Shared release boundary | Existing hash, schema and link rejection evidence remains applicable to the extracted validator. Its exact file inventory, compressed archive size limit, path and entry-type checks, schema/host/rutis compatibility, package/manifest version match and entry digest checks are retained. |
| Private delegation | Two private `test/dev-flow.test.mjs` flows pass: caller environment, custom home, spaces and empty arguments, workspace and exit status survive delegation; default sibling discovery leaves home selection to the public launcher. |
| Native CLI controls | The CLI flow suite passes 13 active flows. Actual App Server mock/configured profile-conflict checks preserve the original daemon, then explicit CLI stop succeeds despite changed invalid model configuration. Formatting, Clippy, package builds and Windows MSVC compilation pass. Root independently reviewed the shared authenticated connection, typed catalog projection and normal shutdown path. |
| Complete developer flow | Private opt-in `test/development.test.mjs` passes against the actual public checkout in 16 seconds. Both repository launchers build and verify the real private package, device login succeeds against a local external HTTP fixture, model refresh returns safe IDs, two model HTTP turns run on the same persisted session across normal daemon restarts, account identity remains available and explicit stop removes the endpoint. Fixture credentials are absent from stdout. Evidence: `/tmp/dim-dev-integration-20261013.log`. |
| Source export | The tracked-source whitelist includes executable `dev` and `.env.example`; `.env.dev`, `.dev`, private source and generated private packages remain excluded. Export still requires a clean committed tree. |

The public `node --test scripts/development.test.mjs` flows pass 3/3, and the private `node --test test/dev-flow.test.mjs` delegation flows pass 2/2. The independent reviewer executed the API-key/immutable-archive flows and the separate preparation failure check; the root executor supplied the final 3/3 result and shared release rejection result. Shell syntax and whitespace checks pass. Native catalog refresh, selected chat and authenticated daemon shutdown have separate CLI acceptance; this review covers the launcher and package boundaries.

The private wrapper received independent review from the CLI developer. The public launcher/shared package boundary received independent review from the private-entry developer, and the root independently reviewed native CLI changes. Integration uses actual implementations and an external HTTP fixture, without a live provider account.

## Simplification and safety

The private launcher delegates to the public entry instead of maintaining a second pnpm/npm/Cargo startup implementation. Existing `pack:runtime`, pinned runtime dependencies and the release archive validator supply the package boundary. API-key mode bypasses the private repository and JavaScript preparation entirely. Startup performs incremental compilation without running tests or producing native release/npm distributions.

Failed preparation never replaces or launches the previous package as a fallback. Verified extraction is staged before replacement. Restart uses the daemon's normal settlement path and retains account/session files; no process is force-killed. A cold stop does no build, while an existing native home requires an already built CLI for authenticated shutdown.

## Limits

Production Dim authorization and cloud model calls with real credentials were not exercised. Actual Windows/Linux development-shell execution and terminal signal behavior were not repeated for this launcher; Unix execution uses `exec` to preserve native signal ownership. No publication workflow was enabled or dispatched. The selected Node and pnpm versions must match their pins, and custom Unix homes must fit the platform's canonical socket-path limit.
