# Release packaging

All GitHub workflow templates are stored as `.yml.disabled`. They are not registered workflows and cannot run. Current development commits and merge requests stay on the internal GitLab repositories; GitHub synchronization and workflow activation require an explicit separate instruction.

The public source uses MIT. An account-enabled distribution can contain a separately supplied private runtime plugin; `NOTICE` excludes that artifact from the public source license. This project does not select or publish a license for that private plugin.

## Local packaging

One product version is shared by `Cargo.toml`, `npm/package.json`, its exact optional dependencies, native version output and the release tag. Keep them synchronized before building. Current `0.1.0` is a development baseline, not a claim that a new npm version is available for publication; check existing `dimcode` versions before selecting a release.

```sh
cargo build --release --locked -p dim-cli -p dim-app-server
node scripts/stage-release.mjs target/release staging/local 0.1.0 FULL_SOURCE_COMMIT
node scripts/pack-release.mjs staging/local release/local
```

The first command stages an API-key-only distribution. Account-enabled staging adds `--with-plugin` and requires `DIM_OAUTH_PACKAGE_SHA256` plus exactly one of `DIM_OAUTH_PACKAGE_URL` (build-time internal download) or `DIM_OAUTH_PACKAGE_ARCHIVE` (local archive). An optional `DIM_OAUTH_DOWNLOAD_TOKEN` is a Bearer download credential. Addresses and credentials are never added to manifests. Redirects fail; archive digest, exact file list, path containment, file types, entry digest and interface versions are checked before loading code.

An accepted plugin archive contains only `package.json`, `dim-plugin.json` and `dist/index.js`. The manifest uses schema 1, host API 1 and rutis 0.10.0. The private SDK is bundled in `dist/index.js`; source maps, tests and account files are excluded.

Account-enabled staging copies the pinned Node executable and license, installs the locked rutis runtime with install scripts disabled, and copies the verified plugin. The installed layout is fixed:

```text
bin/dimcode[.exe]
bin/dim-app-server[.exe]
runtime/node/bin/node          # node.exe on Windows
runtime/rutis/node_modules/@arcships/rutis-runtime/
plugins/dim-oauth/{package.json,dim-plugin.json,dist/index.js}
LICENSE
NOTICE                        # present when the private plugin is included
release-manifest.json
```

The CLI discovers runtime paths relative to its installed binary. Local plugin development uses explicit `--plugin-dir`, `--node` and `--js-runtime` paths; no private source copy into this repository is needed.

`pack-release.mjs` rejects any changed staged file, then creates a native tar.gz (Windows zip) and a platform npm tgz from the same payload. It records SHA-256 for both. `collect-release.mjs` requires `release-<platform>` directories for all five targets, checks identical source/version/toolchain/plugin provenance and artifact digests, creates the thin npm wrapper, then validates all six npm archives without publishing.

```sh
node scripts/collect-release.mjs staging/platforms staging/complete
node scripts/publish-npm.mjs staging/complete/npm VERSION FULL_SOURCE_COMMIT
node scripts/check-native.mjs staging/complete/native VERSION FULL_SOURCE_COMMIT
```

The wrapper resolves the installed platform optional dependency and forwards arguments, terminal streams and exit status. Installation runs no lifecycle scripts or executable downloads. Package checks include a real offline npm install of local tarballs.

## Disabled GitHub workflows

| Template | Runtime and purpose |
| --- | --- |
| `ci.yml.disabled` | Ephemeral isolated self-hosted runner; public source checks with no private network/credentials |
| `release.yml.disabled` | Manual tag build on native self-hosted runners; internal plugin download, staging and artifact upload |
| `npm-publish.yml.disabled` | Manual public `ubuntu-latest` job; verifies the build and uses OIDC for npm |
| `github-release.yml.disabled` | Manual public runner; checks successful build/npm provenance and uploads native archives |

Runner labels are `dim-isolated-ci` and `dim-release-<platform>`. Build jobs use the `release-build` Environment variables `DIM_OAUTH_PACKAGE_URL` and `DIM_OAUTH_PACKAGE_SHA256`, plus the optional secret `DIM_OAUTH_DOWNLOAD_TOKEN`. Public fork checks must only use the isolated runner. Build runners must not be available to arbitrary untrusted pull requests.

After separate activation, dispatch publishing **from the release tag**, for example using `gh workflow run ... --ref vVERSION`; a branch dispatch is rejected by the tag gate. npm needs one trusted publisher for `dimcode` and each of the five platform packages, bound to `arcships/dim`, `npm-publish.yml` and `npm-release`. No npm token is injected into self-hosted build jobs. Configure first publication/package ownership before activation. [npm trusted publishing documentation](https://docs.npmjs.com/trusted-publishers/).

Publication validates every package before the first registry write. A retry compares an already published version's SHA-512 integrity with the original local archive and skips an exact match. A different archive, authentication error or network error fails; only an explicit registry E404 permits publication. All platform packages precede `dimcode`; prereleases use `next`, stable releases use `latest`.

`npm-packages` contains six tgz files and a release manifest; `native-packages` contains five native archives, the matching manifest and `SHA256SUMS`. Successful npm publication uploads a receipt containing its build run ID and package digests. GitHub Release verification requires that exact build ID and matching npm package digests, preventing different plugin builds at the same public commit from being mixed. GitHub Releases are created only after successful npm publication; existing releases are rejected rather than overwritten. All templates remain disabled in this repository.

## Public source export

Internal extraction plans and review records are not part of the public source bundle. `scripts/export-public-source.sh OUTPUT.tar.gz` exports a whitelist of tracked public source and documentation from HEAD, without `.git`, old monorepo history, `docs/plan`, private plugins, caches or staging directories. This creates a local archive only and does not push to any remote. Verify the export before any later GitHub synchronization.
