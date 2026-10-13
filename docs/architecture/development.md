# Local development and managed provider packages

The repository root `./dev` builds the native CLI and App Server with locked Cargo dependencies and runs them against a separate, stable development home. Rust source remains independent of private providers. An explicit API-key model configuration does not require the private repository.

## Entry points

| Entry | Behavior |
| --- | --- |
| `./dev setup` | Install missing locked JS dependencies, build and package the local provider, then verify and extract its runtime archive |
| `./dev [CLI arguments]` | Prepare the local provider when selected, incrementally build both Rust binaries, normally stop the development daemon, then run the new CLI |
| `./dev restart [CLI arguments]` | Perform the same preparation and restart flow explicitly |
| `./dev stop` | Stop an existing development daemon without building or starting one |
| `./dev check` | Run explicit formatting and compilation checks; tests do not run on startup |

Caller arguments and exit status reach the native CLI. Diagnostics and build output go to stderr; stdout remains available for model output. The working directory remains the caller's workspace. On Unix the launcher replaces itself with the CLI so terminal signals are not duplicated.

Optional `.env.dev` is trusted local shell configuration and is ignored by Git. The committed `.env.example` contains no internal addresses or credentials. Inherited `DIM_OAUTH_SOURCE_DIR`, `DIMCODE_HOME`, `DIMCODE_MODEL_CONFIG`, `DIMCODE_PROVIDER` and `DIMCODE_MODEL` take precedence over this file. `DIM_OAUTH_SOURCE_DIR` selects the local private source repository and defaults to the sibling `dim-oauth-plugin` directory. Relative source paths resolve from this repository; model configuration paths follow the caller's working directory. `DIMCODE_MODEL_CONFIG` or explicit `--model-config` selects API-key development instead. `DIMCODE_PROVIDER` and `DIMCODE_MODEL` supply a paired default only for new chat sessions; explicit caller selection and resumed sessions retain native CLI behavior. `DIMCODE_HOME` can select a development home; the default is a short path under `~/.dimcode-dev` scoped by repository. `./dev stop` uses the same home without creating it; an existing native home requires an already built CLI for authenticated shutdown.

## Local package boundary

The first implementation accepts only a package built from local OAuth source. It does not download provider artifacts, select remote versions or update them automatically. Build uses the private repository's frozen lockfile and existing `pack:runtime` command. The private source is never copied into the public repository.

The verified runtime is extracted under the ignored public `.dev` directory. Archive SHA-256, size, paths, entry types, exact runtime file inventory, manifest schema, host API, rutis version, package version and entry digest use the same boundary validation as release staging. The accepted files remain `package.json`, `dim-plugin.json` and `dist/index.js`. A failed validation does not replace the last verified package and never launches it as a fallback for the failed preparation.

The host supplies the pinned Node executable and locked rutis runtime. Dependency installation is reused only when the package manifests, lockfiles and selected Node version match and installed dependencies are present. Local source is built and packaged before loading. Development does not create native release archives or npm distributions.

## CLI and daemon lifecycle

`dimcode models PROVIDER [--refresh]` uses the existing catalog/refresh RPCs and prints model IDs and availability without credential information. `--provider` and `--model` select a model for a new session. These controls complete the login, catalog and chat path without hand-written RPC clients.

`dimcode daemon stop` connects only to an existing authenticated endpoint for the selected home. No endpoint means successful completion without creating a daemon. Shutdown uses the normal server settlement path, waits for process/endpoint closure with a bounded timeout and never force-kills a process. The developer launcher uses this command before loading a new server or plugin build. Account and session files are retained across restart.

```text
$ ./dev login <provider> --directory <directory> --device
Verification URL: ...
Signed in.

$ ./dev models <provider> --refresh
<model-id>    <display-name>    available

$ ./dev --provider <provider> --model <model-id>
dimcode
workspace: /path/to/project
session: <session-id>
> Explain this project
Assistant:
...
```

The private repository's `scripts/dev.sh` delegates to this entry, supplies its own source directory and preserves caller arguments. It does not maintain a second build/runtime installation implementation.

## Verification

Verify local source packaging through the actual archive boundary, argument forwarding including spaces, isolated home permissions, persistent state through normal restart, explicit API-key independence, catalog refresh and missing-daemon stop. Reuse existing release rejection tests for the shared archive validator. CI schema and publication controls are verified separately from an actual release; no release tag or registry write is part of local implementation verification.
