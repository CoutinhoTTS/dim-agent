# Release packaging

Dim uses self-hosted runners for PR checks and the complete GitHub release workflow. npm publication stays disabled in `.github/workflows/npm-publish.yml.disabled`. The [project release skill](../../.agents/skills/dim-release/SKILL.md) covers version preparation, PRs, tags, draft inspection and recovery. [Rust alpha release](alpha.html) walks through publishing a `-alpha.N` version end to end (Chinese).

The public source uses MIT. An account-enabled distribution can contain a separately supplied private runtime plugin; `NOTICE` excludes that artifact from the public source license. This project does not select or publish a license for that private plugin.

## Upgrade compatibility

Distributions using host API 2 require managed plugins that implement `quiesce`; API 1 packages are rejected. Before replacing an older CLI and daemon, stop that daemon with its matching CLI. The new `dimcode daemon stop` refuses daemons without `daemon.shutdown.completed` and leaves them running. Include both requirements in the release notes for this upgrade. Forced shutdown does not certify storage or plugin cleanup.

## Runner setup

Workflows use GitHub's [default self-hosted OS and architecture labels](https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/use-in-a-workflow). Register runners with these labels and grant this repository access to their runner group:

| Job or target | Rust target | Runner labels | Build mode |
| --- | --- | --- | --- |
| PR checks | Host default | `self-hosted`, `macOS`, `ARM64` | Native workspace build and tests |
| PR Windows CLI checks | Host default | `self-hosted`, `windows`, `x64` | Native CLI tests |
| macOS ARM64 | `aarch64-apple-darwin` | `self-hosted`, `macOS`, `ARM64` | Native |
| macOS Intel | `x86_64-apple-darwin` | `self-hosted`, `macOS`, `ARM64` | Cross compile with Apple Clang |
| Linux x64 | `x86_64-unknown-linux-gnu` | `self-hosted`, `linux`, `x64` | Native |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | `self-hosted`, `linux`, `x64` | Cross compile with AArch64 GCC/G++ |
| Windows x64 | `x86_64-pc-windows-msvc` | `self-hosted`, `windows`, `x64` | Native MSVC |
| Windows ARM64 | `aarch64-pc-windows-msvc` | `self-hosted`, `windows`, `x64` | MSVC ARM64 libraries and clang-cl |
| Release preflight and draft assembly | None | `self-hosted`, `linux`, `x64` | Version/provenance checks and GitHub upload |

All runners need a current GitHub Actions runner, Git, rustup, a C/C++ toolchain, tar and Bash. Actions installs Node from `.node-version`; Rust is pinned by `rust-toolchain.toml`, and release jobs install their Rust target. PR runners must be isolated from release credentials and private networks. Apply GitHub's approval policy before running untrusted PRs on self-hosted machines.

- macOS: install Xcode Command Line Tools and accept the Xcode license. Apple Clang and its SDK build both architectures.
- Linux: provision a GNU/Linux baseline suitable for the intended users. On Debian/Ubuntu install `build-essential gcc-aarch64-linux-gnu g++-aarch64-linux-gnu`. The ARM64 job selects that cross compiler, archiver and linker; it does not require emulation. Install `gh` for release assembly.
- Windows: install Git for Windows (Bash and tar), PowerShell 7, Visual Studio Build Tools with the x64 and ARM64 MSVC tools and Windows SDK, LLVM/clang-cl, and NASM for x64 assembly. Put `clang-cl` and `nasm` on the runner's PATH. The workflow configures `amd64` or `amd64_arm64` and selects the target MSVC linker by full path to avoid Git Bash's unrelated `link.exe`. [Windows native dependency requirements](https://aws.github.io/aws-lc-rs/requirements/windows.html).

Cross-built executables are inspected for their ELF, Mach-O or PE target architecture. Staging runs `dimcode --version` on native builds. Cross-target version execution and target-system runtime acceptance require the corresponding system; a successful cross build alone is not that acceptance. Record and verify OS/libc baselines before declaring installation support.

## GitHub workflows

`ci.yml` runs on pull requests, pushes to `main`, and manual dispatch. It builds the workspace, checks formatting and Clippy, runs workspace tests and boundary checks, runs the packaging/development script tests, and exercises the ignored actual CLI profile and public Node-backed flows. It receives no private plugin credentials. Pushes to `main` and manual runs do all of it; a pull request runs only the groups its changed paths need, as decided by `scripts/ci-changes.mjs` (documentation-only changes skip every group, and unrecognized paths or an incomplete file listing run all of them). `ci-ok` passes when every selected job passed, so it is the check to require. CI builds without debug info (`CARGO_PROFILE_DEV_DEBUG=0`).

`release.yml` runs when a `v*.*.*` tag is pushed, or when manually dispatched with an existing tag:

```sh
gh workflow run release.yml --ref main -f tag=vVERSION
```

Preflight checks the exact tag commit is reachable from `origin/main`, and verifies the tag, Cargo workspace, npm wrapper/optional dependencies and runtime metadata have the same version. Every build checks out the resolved commit SHA. Six target jobs stage and pack their payloads. Assembly requires every target, matching toolchain/plugin provenance and valid artifact digests before creating a draft GitHub Release. Prerelease tags mark the draft as a prerelease. Release notes are generated from GitHub history and should be reviewed before publishing.

The draft receives seven native archives (six platforms and the Web workspace), eight original npm tarballs (six platform packages, `dimcode-web` and the wrapper), `release-manifest.json` and `SHA256SUMS`: seventeen attachments in total. `SHA256SUMS` covers the native archives; the manifest records digests for both native and npm archives. npm tarballs are not published to a registry by this workflow. Existing releases, including drafts, are rejected to avoid replacing attachments. If an upload leaves an incomplete draft, inspect it and remove that draft only when authorized before retrying; keep its tag.

All Actions artifacts, including the disabled npm publication receipt, use `retention-days: 1`. GitHub Release attachments persist independently of Actions retention. A retry after artifacts expire must rebuild all targets.

## Local packaging

One product version is shared by `Cargo.toml`, workspace versions in `Cargo.lock`, `npm/package.json`, its exact optional dependencies, runtime package/lock metadata, native version output and the release tag. Keep them synchronized before building. `0.1.0` is a development baseline; check existing npm versions before enabling registry publication.

```sh
cargo build --release --locked -p dimcode
node scripts/stage-release.mjs target/release staging/local 0.1.0 FULL_SOURCE_COMMIT
node scripts/pack-release.mjs staging/local release/local
node scripts/stage-web.mjs release/web 0.1.0 FULL_SOURCE_COMMIT
```

The executable carries no frontend. `stage-web.mjs` builds `web/` once (`npm ci`, `npm run build`), records `web/dimcode-web.json` (`version`, `protocolVersion`, `sourceCommit`) beside it and packs the tree as `dimcode-web-VERSION.tar.gz` and the npm package `dimcode-web`, which the wrapper lists as an optional dependency and names to the executable through `DIMCODE_WEB_ASSETS`. A native-archive user unpacks `dimcode-web-VERSION.tar.gz` so that `web/index.html` sits beside the executable's `bin/` or in `<product home>/web`. A build that must carry the frontend uses `cargo build --features embedded-web`, which needs npm on the build host.

For a cross build, set `DIM_BUILD_PLATFORM` to the intended platform, build with the matching Rust `--target`, and stage from `target/TARGET/release`. The runner must use the same OS as the target. The six platform names are listed in the workflow matrix.

These commands stage an API-key-only distribution. An account build embeds its runtime in the executable, so the runtime is staged first and the build reads it:

```sh
node scripts/stage-runtime.mjs staging/runtime
DIMCODE_EMBEDDED_RUNTIME="$PWD/staging/runtime/runtime.bin" cargo build --release --locked -p dimcode
node scripts/stage-release.mjs target/release staging/local 0.1.0 FULL_SOURCE_COMMIT --runtime staging/runtime
```

`stage-runtime.mjs` requires `DIM_OAUTH_PACKAGE_SHA256` plus exactly one of `DIM_OAUTH_PACKAGE_URL` (build-time download) or `DIM_OAUTH_PACKAGE_ARCHIVE` (local archive). An optional `DIM_OAUTH_DOWNLOAD_TOKEN` is a Bearer download credential. Addresses and credentials are never added to manifests. Redirects fail; archive digest, exact file list, path containment, file types, entry digest and interface versions are checked before loading code.

In CI, configure the `release-build` Environment with `DIM_OAUTH_PACKAGE_URL` and `DIM_OAUTH_PACKAGE_SHA256`, plus optional secret `DIM_OAUTH_DOWNLOAD_TOKEN`, to include the same immutable plugin archive on all targets. If neither variable is set, CI builds API-key-only payloads. Partial or invalid configuration fails instead of silently omitting the plugin.

An accepted plugin archive contains only `package.json`, `dim-plugin.json` and `dist/index.js`. The manifest uses schema 1, host API 2 and rutis 0.10.0. Host API 2 requires `quiesce` and supports native file transactions; API 1 archives are rejected. The private SDK is bundled in `dist/index.js`; source maps, tests and account files are excluded.

The bundled host runtime is rutis 0.12.1, which also accepts plugin API 1 exports from the 0.10.0 SDK. The managed-provider manifest and host API contract above are unchanged; its SDK version is distinct from the host runtime version recorded in the release inventory.

For the host platform `stage-runtime.mjs` copies the pinned host Node and its license; for a cross target (`DIM_BUILD_PLATFORM`) it downloads the pinned official Node distribution for the **target** OS/CPU and verifies its archive against the official `SHASUMS256.txt`. Both install the locked rutis runtime with [npm target OS/CPU overrides](https://docs.npmjs.com/cli/v11/using-npm/config/#cpu) and install scripts disabled, then copy the verified plugin. Staging never bundles the host Node into a cross-target payload. It writes the tree as `runtime.bin` (gzip over `DIMRT1\n` and, per file in path order, a u32 BE path length, the UTF-8 path, a flags byte where 1 marks an executable, a u64 BE size and the bytes) and records its digest, file digests, toolchain and plugin provenance in `runtime.json`. `build.rs` in `dim-cli` embeds the archive and its SHA-256; `stage-release.mjs --runtime` rejects an executable that does not contain that digest.

On first use the CLI extracts the archive to `<product home>/runtime/<first 16 digest characters>/`, rejecting any path outside it, marks it complete last and removes the runtimes of earlier versions. Each platform package and native archive holds only the executable:

```text
bin/dimcode[.exe]              # the CLI; runs its daemon as `dimcode app-server`
LICENSE
NOTICE                        # present when the private plugin is embedded
release-manifest.json         # `runtime` lists the embedded archive's digest and files
```

The extracted runtime keeps the layout the App Server is started with:

```text
<product home>/runtime/<digest>/
  runtime/node/bin/node        # runtime/node/node.exe on Windows
  runtime/rutis/node_modules/@arcships/rutis-runtime/
  plugins/dim-oauth/{package.json,dim-plugin.json,dist/index.js}
  .complete
```

Local plugin development uses explicit `--plugin-dir`, `--node` and `--js-runtime` paths; no private source copy into this repository is needed.

`pack-release.mjs` rejects changed staged files, then creates a native tar.gz (Windows zip) and a platform npm tgz from the same payload. It records SHA-256 for both. `collect-release.mjs` requires `release-<platform>` directories for all six targets and `release-web` from `stage-web.mjs`, checks identical source/version/toolchain/plugin provenance and artifact digests, creates the thin npm wrapper, then validates all seven npm archives without publishing.

```sh
node scripts/collect-release.mjs staging/platforms staging/complete
node scripts/publish-npm.mjs staging/complete/npm VERSION FULL_SOURCE_COMMIT
node scripts/check-native.mjs staging/complete/native VERSION FULL_SOURCE_COMMIT
```

The wrapper resolves the installed platform optional dependency and forwards arguments, terminal streams and exit status. Installation runs no lifecycle scripts or executable downloads. Package checks include a real offline npm install of local tarballs.

## Disabled npm publication

`.github/workflows/npm-publish.yml.disabled` is not registered and cannot run. Do not activate it until package ownership and npm trusted publishing are configured and activation is explicitly requested. It is the only workflow intended to use a GitHub-hosted `ubuntu-latest` runner. No npm token is injected into self-hosted build jobs.

Future activation needs one trusted publisher for `dimcode`, `dimcode-web` and each of the six platform packages, bound to `arcships/dim`, `npm-publish.yml` and `npm-release`. [npm trusted publishing documentation](https://docs.npmjs.com/trusted-publishers/).

Release construction and npm publication are separate operations. Review and publish the GitHub draft first. The npm workflow has only `workflow_dispatch`; it takes the tag of an existing **non-draft** GitHub Release and can run from `main` regardless of that release's age:

```sh
gh workflow run npm-publish.yml --ref main -f release_tag=vVERSION
```

This command becomes available only after explicit workflow activation. Publication downloads the original eight `.tgz` attachments and manifest from the specified Release, resolves the tag's commit, and verifies published status, exact package names, versions, source commit and digests before any registry write. It rechecks Release status and tag identity immediately before publishing. It does not rebuild packages or consume expiring Actions artifacts. [Release attachment download](https://cli.github.com/manual/gh_release_download).

Publication validates every package before the first registry write. A retry compares an already published version's SHA-512 integrity with the original local archive and skips an exact match. A different archive, authentication error or network error fails; only an explicit registry E404 permits publication. All platform packages precede `dimcode`; versions are `X.Y.Z`, `X.Y.Z-beta.N` or `X.Y.Z-alpha.N` and publish to `latest`, `beta` or `alpha` respectively, so each channel upgrades only within itself. An alpha must stay below the newest `dimcode` beta on the registry, because installed TypeScript betas upgrade to the highest prerelease of any identifier; publication checks this before any write.

`npm-packages` contains eight tgz files and a release manifest; `native-packages` contains seven native archives, the matching manifest and `SHA256SUMS`. The GitHub Release retains all fifteen original archives and the shared metadata independently of Actions retention. Future npm publication uploads an Actions receipt containing the Release ID, tag and package digests; that receipt also expires after one day.

## Public source export

`scripts/export-public-source.sh OUTPUT.tar.gz` exports a whitelist of tracked public source, project skills and documentation from HEAD, without `.git`, repository history, private plugins, caches or staging directories. This creates a local archive only and does not push to any remote.

Native archives and platform npm packages include the vendored terminal libraries’ Apache-2.0 license and notices under `licenses/grok-build/`, referenced by the root `NOTICE`. The extra Swift syntax asset is excluded pending license verification.
