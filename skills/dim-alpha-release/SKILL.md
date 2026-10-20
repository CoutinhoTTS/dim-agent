---
name: dim-alpha-release
description: Publish the Rust dimcode CLI as an npm alpha (X.Y.Z-alpha.N on the `alpha` dist-tag). Use for "release/publish an alpha", choosing the next alpha version, bumping it, tagging, checking the draft Release and publishing its npm packages locally. Stable and beta releases follow dim-release.
---

# Dim alpha release

The Rust CLI ships under the npm name `dimcode`, the same name the TypeScript CLI uses. Each channel has its own dist-tag and upgrades only within itself:

| Version | dist-tag | Install |
| --- | --- | --- |
| `X.Y.Z` | `latest` | `npm i -g dimcode` |
| `X.Y.Z-beta.N` | `beta` | `npm i -g dimcode@beta` (TypeScript) |
| `X.Y.Z-alpha.N` | `alpha` | `npm i -g dimcode@alpha` (Rust) |

`scripts/publish-npm.mjs` derives the dist-tag from the version and rejects other prerelease identifiers. [dim-release](../../.agents/skills/dim-release/SKILL.md) owns the shared rules (version files, PR, tag, draft inspection, recovery); this skill covers what is specific to alphas. The human-readable walkthrough is [docs/release/alpha.html](../../docs/release/alpha.html); keep both in step.

## Constraint: stay below the newest beta

Installed TypeScript betas offer an upgrade to the highest prerelease of any identifier. An alpha above the newest beta would pull beta users onto the Rust CLI. `publish-npm.mjs` refuses such an alpha before any registry write. Never weaken that check to get a release out; choose a lower version instead and tell the user why.

## Choose the version

Use the version the user gave. Otherwise propose one and confirm it before changing files:

```sh
npm view dimcode versions --json | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).filter(v=>/-alpha\.\d+$/.test(v)).join("\n")||"(none)"))'
npm view dimcode dist-tags.beta
```

- No alpha yet: the Cargo workspace version with `-alpha.0` (the baseline is `0.1.0-alpha.0`).
- Otherwise: the highest alpha with `N + 1`.
- The result must be below `dist-tags.beta` (same `X.Y.Z` is fine, since `alpha` sorts before `beta`).
- Check the tag is unused: `git ls-remote --tags origin "v$VERSION"` and `gh release view "v$VERSION"` must both find nothing.

## Prepare and merge

Branch `codex/release-v$VERSION` from fresh `origin/main`. Set every version field from the repository root:

```sh
VERSION=0.1.0-alpha.0 node --input-type=module -e '
import { readFileSync, writeFileSync } from "node:fs";
const v = process.env.VERSION;
const old = readFileSync("npm/package.json", "utf8").match(/"version": "([^"]+)"/)[1];
const edit = (f, fn) => writeFileSync(f, fn(readFileSync(f, "utf8")));
const json = (f, fn) => edit(f, s => { const d = JSON.parse(s); fn(d); return JSON.stringify(d, null, 2) + "\n"; });
edit("Cargo.toml", s => s.replace(/(\[workspace\.package\]\s+version = ")[^"]+/, `$1${v}`));
edit("npm/package.json", s => s.replaceAll(`"${old}"`, `"${v}"`));
json("runtime/package.json", d => { d.version = v; });
json("runtime/package-lock.json", d => { d.version = v; d.packages[""].version = v; });
'
cargo metadata --format-version 1 > /dev/null
```

The diff must touch exactly `Cargo.toml`, `Cargo.lock` (workspace package versions only), `npm/package.json`, `runtime/package.json` and `runtime/package-lock.json`. Commit `chore: prepare v$VERSION`, push and open a PR only within the user's request; merge after its CI passes.

## Build the draft

Tag the merged commit on `origin/main` and push the tag only when the user asked for the release; the push starts `release.yml` on self-hosted runners:

```sh
git fetch origin
git tag -a "v$VERSION" origin/main -m "v$VERSION"
git push origin "v$VERSION"
gh run watch "$(gh run list --workflow release.yml -L 1 --json databaseId --jq '.[0].databaseId')" --exit-status
```

Run the watch in the background. Then confirm the draft is a prerelease with fifteen attachments (six native archives, seven npm tarballs, `release-manifest.json`, `SHA256SUMS`):

```sh
gh release view "v$VERSION" --json isDraft,isPrerelease,assets --jq '{isDraft, isPrerelease, count: (.assets|length), names: [.assets[].name]}'
```

Publish the GitHub Release (`gh release edit "v$VERSION" --draft=false`) only when the user asks; npm publication requires a non-draft Release.

## Publish to npm

`.github/workflows/npm-publish.yml.disabled` stays disabled, so npm publication runs locally from the Release's original tarballs. Never rebuild or repack them.

```sh
TAG="v$VERSION"
rm -rf staging/npm && mkdir -p staging/npm
gh api "repos/arcships/dim/releases/tags/$TAG" > staging/github-release.json
SHA=$(gh api "repos/arcships/dim/commits/refs%2Ftags%2F$TAG" --jq .sha)
gh release download "$TAG" --repo arcships/dim --dir staging/npm --pattern '*.tgz' --pattern release-manifest.json
node scripts/publish-npm.mjs staging/npm "$VERSION" "$SHA"
```

That last command only validates; run it yourself and report the result. The registry write is the user's: npm login and per-package OTP prompts need their credentials. Hand them this command to run in their terminal instead of running it:

```sh
node scripts/publish-npm.mjs staging/npm "$VERSION" "$SHA" --publish staging/github-release.json
```

Afterwards verify that `latest` and `beta` are unchanged and `alpha` is the new version:

```sh
npm view dimcode dist-tags
npm view dimcode-windows-arm64 dist-tags
```

## Recovery

- Partial npm publish: rerun the same `--publish` command. Packages already published with identical bytes are skipped; anything else stops without overwriting.
- "must stay below the newest dimcode beta": pick a lower alpha and restart from Prepare. The tag and Release for the rejected version stay as they are unless the user asks to remove them.
- Build failures, incomplete drafts and tag rules: follow dim-release Recovery. Never move a pushed tag.
