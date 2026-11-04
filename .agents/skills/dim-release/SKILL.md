---
name: dim-release
description: Prepare and deliver Dim releases in this repository. Use for product version changes, release branches or PRs, version tags, self-hosted alpha builds, draft GitHub Releases, guarded npm OIDC publication from a published alpha Release, or recovery of an incomplete release.
---

# Dim release

Alpha releases (`X.Y.Z-alpha.N`, the Rust CLI on the `alpha` dist-tag) follow [dim-alpha-release](../../../skills/dim-alpha-release/SKILL.md), which builds on the rules here.

Read [release packaging](../../../docs/release/README.md) for runner prerequisites, platform artifacts and plugin configuration. The workflow is `.github/workflows/release.yml`; CI is `.github/workflows/ci.yml`.

GitHub release automation accepts only alpha versions. Stable and beta release automation is not configured.

## Prepare

- Inspect the working tree, current branch, `origin/main`, tags and existing GitHub Releases before changing release metadata. Fetch `origin/main` and tags. Preserve unrelated local work.
- Select the version requested by the user; if none was supplied, ask for it. Use `vX.Y.Z`, `vX.Y.Z-beta.N` or `vX.Y.Z-alpha.N`; npm publication rejects other prerelease identifiers. `0.1.0` is a development baseline, not evidence of an unused npm version.
- Prepare on `codex/release-vVERSION` from `origin/main`, or continue the corresponding existing branch. Use a normal PR into `main`; this repository has no separate release integration branch or special merge command.
- Set the same version in `Cargo.toml` under `[workspace.package]`, `npm/package.json` and every exact optional dependency, `runtime/package.json`, and both root version fields in `runtime/package-lock.json` (`version` and `packages[""].version`). Refresh the workspace package versions in `Cargo.lock` with `cargo metadata --format-version 1 > /dev/null`; review the lockfile for unrelated dependency changes.
- Summarize user-visible changes from commits since the previous tag for the PR and draft release notes. Do not add unrelated features or release infrastructure.

## Verify and merge

- Run the applicable checks from `AGENTS.md`, reusing unchanged evidence. Metadata-only changes need version consistency and locked Cargo metadata checks. Run `node --test scripts/*.test.mjs` when changing packaging scripts. The PR check must succeed before tagging.
- Commit in English following repository conventions. When no stronger convention exists, use `chore: prepare vVERSION`.
- Push the branch and create or update its PR when the user has requested that delivery. Merge only within the user's authorization. Never force-push `main`.

## Build the draft

- Create an annotated `vVERSION` tag on the verified release commit already reachable from `origin/main`. The tag, Cargo version, wrapper, optional dependencies and runtime metadata must agree.
- Pushing the tag triggers release CI. Treat a tag push as a release action and perform it only within the user's request. For an existing tag, use `gh workflow run release.yml --ref main -f tag=vVERSION`; do not dispatch while the tag-triggered run is active.
- Watch the run with `gh run watch RUN_ID --exit-status`. All jobs use self-hosted runners. Cross-built macOS x64, Linux ARM64 and Windows ARM64 artifacts receive architecture checks; these checks do not demonstrate target-system runtime acceptance.
- CI collects seven native archives (including Web) and eight npm tarballs, validates their version/source/toolchain/plugin provenance and SHA-256, and attaches all fifteen original archives, `release-manifest.json` and `SHA256SUMS` to a prerelease draft GitHub Release. This workflow never writes to npm.
- Inspect the draft with `gh release view vVERSION --json url,isDraft,tagName,assets`. Confirm all seventeen attachment names and sizes, check native archive digests against `SHA256SUMS` and npm tarball digests against the manifest, and review generated notes. Publish the GitHub draft only when requested; npm publication requires it to be non-draft first.
- Actions artifacts expire after one day. All original archives persist as GitHub Release attachments, so later npm publication does not depend on Actions retention.

## Publish npm packages

- `.github/workflows/npm-publish.yml` is registered, but publication requires `publish=true` and repository variable `NPM_ALPHA_PUBLISH_ENABLED=true`. Keep the variable unset or `false` until the eight alpha packages have trusted publishers and publication is explicitly authorized. Follow [the npm setup instructions](../../../docs/release/README.md#alpha-npm-publication).
- `gh workflow run npm-publish.yml --ref main -f release_tag=vVERSION` performs verification only, using the self-hosted Linux runner. It never schedules a GitHub-hosted runner. Do not dispatch builds or workflows merely to test their configuration when the user has ruled out those runs; use local static and offline checks.
- For an explicitly requested npm publication, use `gh workflow run npm-publish.yml --ref main -f release_tag=vVERSION -f publish=true`. The tag must identify an already published, non-draft alpha GitHub Release. There is no automatic npm trigger.
- The self-hosted job downloads and validates the original npm tarballs and manifest. The GitHub-hosted Ubuntu job receives those same archives, rechecks published status and tag identity, and validates exact packages, version, source SHA and digests before OIDC publication. It performs no build or test suite. Never rebuild, repack or substitute packages during publication, and never give npm write credentials to self-hosted jobs.
- The six platform packages and `dimcode-web` publish before `dimcode-alpha`, all on the `alpha` dist-tag. The workflow never publishes `dimcode` or stable/beta versions. An alpha must stay below the newest `dimcode` beta on the registry, because installed TypeScript betas upgrade to the highest prerelease of any identifier; publication checks this before any write. An existing version is skipped only when its registry integrity exactly matches the original archive. Different bytes, registry authentication errors and network failures stop publication.
- The publication receipt records the Release ID, tag and package digests as a one-day Actions artifact. It does not modify the published GitHub Release.

## Recovery

- Fix build failures on the release preparation branch and use a new version tag if the source changes. Never move an existing release tag or overwrite published assets without an explicit request.
- A failed build creates no draft. Rerun failed jobs while successful artifacts are still available; after one day rerun all jobs or dispatch a complete build for the unchanged tag.
- An upload failure may leave an incomplete draft. The workflow rejects an existing release instead of replacing attachments. Inspect it, then delete only that incomplete draft when authorized before retrying. Keep its tag.
- Missing runner labels or cross toolchains must be fixed on the self-hosted fleet; never fall back to GitHub-hosted release runners. Plugin provenance mismatches require the same immutable archive URL and SHA-256 across all platforms before rebuilding.
- If an older published Release lacks its original npm tarballs or manifest, stop npm publication. Do not rebuild packages to fill the gap; prepare a new release containing the complete original artifacts.
- Do not set `NPM_ALPHA_PUBLISH_ENABLED=true` or dispatch with `publish=true` during ordinary verification. Enabling publication requires package ownership and trusted publishers for all eight alpha packages, and an explicit publication request.
