# Container image release

Status: Draft. Issue: to be opened with the pull request.

Scope: release packaging of the Linux executable. This document adds no plugin, part, service or
Timeline event.

## 1. Design

### 1.1 Artifact

| Item | Value |
| --- | --- |
| Repository | `ghcr.io/arcships/dim` |
| Immutable tag | `X.Y.Z-alpha.N`, the release version without `v` |
| Moving tag | `alpha`, moved after both platforms pass acceptance |
| Platforms | `linux/amd64`, `linux/arm64` in one index |
| Pin | CI uses the index digest: `ghcr.io/arcships/dim@sha256:…` |

The image is assembled from the published Release's Linux and Web archives. The executable bytes
are the same as the npm and native-archive channels.

### 1.2 Contents

| Path | Content |
| --- | --- |
| `/opt/dim/bin/dimcode` | The released executable |
| `/opt/dim/plugins/`, `default-plugins.json` | The release's default plugins and configuration, unchanged |
| `/opt/dim/web/` | The release's Web workspace |
| `/opt/dim/release-manifest.json`, `web-release-manifest.json` | Archive manifests for inspection |
| `/opt/dim/LICENSE`, `NOTICE`, `licenses/` | Distribution notices |
| `/usr/local/bin/dim`, `dimcode` | Symlinks to the executable |
| `/workspace` | Working directory, owned by `dim` |
| `/home/dim` | `HOME`; product home, plugins, sessions, runtimes |

Base image: `debian:bookworm-slim`, pinned by digest. Installed system packages: `git`,
`ca-certificates`, `tini`. User: `dim`, UID/GID 1000. `tini` is PID 1 and reaps agent background
processes.

Node and Python runtimes use the release's default configuration (`download`, `auto: false`) and are
installed into `/home/dim` when the user runs `dimcode runtime install node|python`. The image uses
the product's normal runtime acquisition.

### 1.3 Usage

The client is the container's main process. It starts the App Server on the private socket under
`/home/dim`, and the daemon ends when the container ends. One foreground client per container. The
client waits for the current turn to settle before it exits.

| Use | Command |
| --- | --- |
| TUI | `docker run --rm -it -v "$PWD:/workspace" -v dim-home:/home/dim IMAGE` |
| One-shot | `docker run --rm -v "$PWD:/workspace" -v dim-home:/home/dim IMAGE exec -p TEXT` |
| ACP | `docker run -i --rm -v "$PWD:/workspace" -v dim-home:/home/dim IMAGE acp` |
| Web (Linux) | `docker run --rm --network host -v "$PWD:/workspace" -v dim-home:/home/dim IMAGE web --no-open` |

Web listens on `127.0.0.1`, so the container shares the host network. Model credentials come from
`--env-file`/`-e`; a catalog file uses `--model-config`. Mounts: `/workspace` for the repository,
`/home/dim` for the product home. The product home is `/home/dim/.dimcode-alpha` for this alpha.
One home volume has one container writer. For a host workspace owned by another UID, run with
`--user "$(id -u):$(id -g)"` and a writable home.

### 1.4 Publication

A separate `image-publish.yml` runs after the GitHub Release is published:

1. Dispatch with `release_tag` and `publish`.
2. Check out the release tag; download `dimcode-linux-x64`, `dimcode-linux-arm64`,
   `dimcode-web` archives and `SHA256SUMS`.
3. Verify the archives against `SHA256SUMS` and their internal manifests.
4. Build and push `linux/amd64` and `linux/arm64` with `docker buildx build --provenance=mode=max
   --sbom=true`.
5. Pull the version tag on each native runner; check `dimcode --version` and the Web assets.
6. Move `alpha` to the accepted index digest.

The version tag is created once. If it exists, the workflow fails. A new attempt uses a new version.
`publish=false` builds locally and runs the amd64 acceptance.

### 1.5 Security

The container is the isolation boundary. The image runs as `dim` with ordinary container
permissions; the mounts are the workspace and the home volume. Dim's permission presets, approvals
and project-entry trust keep their normal behavior. Credentials stay in `--env-file`/`-e` or the
home volume; the image contains the runtime packages and release bytes.

## 2. Plan

### 2.1 New files

| File | Purpose |
| --- | --- |
| `scripts/stage-image.mjs` | Verify release archives and produce the Docker rootfs |
| `scripts/stage-image.test.mjs` | Flow test for verification and layout |
| `docker/Dockerfile` | Base, system packages, user, rootfs, entrypoint |
| `.github/workflows/image-publish.yml` | Build, push and accept the image |
| `scripts/ci-changes.mjs` | Map `docker/**` to the scripts check group |

### 2.2 Build

`stage-image.mjs` takes `--native ARCHIVE --web ARCHIVE --version VERSION --platform linux-x64|linux-arm64
--output DIR` and:

1. Checks file names, version, platform and the native manifest's `sourceCommit`.
2. Extracts into an empty directory and checks every file the manifest lists; rejects extra files,
   links and special files.
3. Checks the Web archive's `version` and `protocolVersion` against the release tag's
   `PROTOCOL_VERSION`.
4. Writes `DIR/rootfs-<arch>/opt/dim/{bin,plugins,web,licenses,…}` and the manifests; sets the
   executable mode.

The workflow runs it twice, producing `rootfs-amd64` and `rootfs-arm64` in one build context.

`docker/Dockerfile` takes `VERSION` and `SOURCE_SHA` build args; `BASE_IMAGE` has a
digest-pinned default (`debian:bookworm-slim@sha256:…`). It:

1. `FROM ${BASE_IMAGE}` (`debian:bookworm-slim@sha256:…`), with `ARG TARGETARCH`.
2. Creates `dim` with UID/GID 1000, installs `git`, `ca-certificates`, `tini`, and creates
   `/workspace`.
3. `COPY rootfs-${TARGETARCH}/ /`.
4. Sets `HOME=/home/dim`, `USER dim`, `WORKDIR /workspace`.
5. `ENTRYPOINT ["/usr/bin/tini","--","/opt/dim/bin/dimcode"]`.
6. Writes the OCI labels `version`, `revision`, `source` and `licenses`.

### 2.3 Publication workflow

`image-publish.yml`:

1. `workflow_dispatch` inputs `release_tag` (required) and `publish` (boolean).
2. `contents: read`, `packages: write`; environment `image-release`; repository guard
   `arcships/dim`.
3. Check out `refs/tags/$release_tag`; download and verify the Release assets; run
   `stage-image.mjs` for both platforms into one build context.
4. `publish=false`: build the amd64 image with `--load`, run `--version` and the Web asset check,
   stop.
5. `publish=true`: fail if `ghcr.io/arcships/dim:$release_tag` exists; set up Buildx and QEMU;
   build and push the multi-platform index.
6. Acceptance matrix on `ubuntu-22.04` and `ubuntu-22.04-arm`: pull the version tag, run
   `dimcode --version`, check the version and labels.
7. Move `alpha` to the accepted digest.

One-time setup: link the GHCR package to `arcships/dim`, make it public, and create the
`image-release` environment.

### 2.4 Documentation

- Add the image command to the main README install section after the first publication.
- Add the image stage to `.agents/skills/dim-release/SKILL.md`.
- Keep the link from `docs/release/README.md` and `docs/INDEX.md`.

### 2.5 Verification

| Check | Command |
| --- | --- |
| Staging script | `node --test scripts/*.test.mjs` |
| Dry-run image | `image-publish.yml` with `publish=false` |
| Published image | Acceptance matrix on both native runners |
| Documentation | `node scripts/adr-check.mjs --base <ref>` |

### 2.6 Order

1. Open the issue with this document.
2. Add `stage-image.mjs` and its test; run the script tests.
3. Add the Dockerfile; build and run locally.
4. Add `image-publish.yml`; run a dry run on a release tag.
5. Publish the first image; verify both architectures.
6. Add the README line and the release-skill stage.
