# Local development and managed provider boundary

The repository root `./dev` builds the native CLI and App Server with locked Cargo dependencies and runs them against a separate, stable development home. Use `--model-config PATH` for API-key development without the private account plugin. See the root [README](../../README.md) for complete setup and run commands.

## Entry points

| Entry | Behavior |
| --- | --- |
| `./dev --model-config PATH [CLI arguments]` | Incrementally build both Rust binaries, normally stop the development daemon, then run the new CLI with the configured provider |
| `./dev [CLI arguments]` | Use the model configuration selected in `.env.dev` or the inherited environment |
| `./dev restart [CLI arguments]` | Perform the same build and restart flow explicitly |
| `./dev stop` | Stop an existing development daemon without building or starting one |
| `./dev check` | Run explicit formatting and compilation checks; tests do not run on startup |

Caller arguments and exit status reach the native CLI. Diagnostics and build output go to stderr; stdout remains available for model output. The working directory remains the caller's workspace. On Unix the launcher replaces itself with the CLI so terminal signals are not duplicated.

Optional `.env.dev` is trusted local shell configuration and is ignored by Git. Inherited `DIM_OAUTH_SOURCE_DIR`, `DIMCODE_HOME`, `DIMCODE_MODEL_CONFIG`, `DIMCODE_PROVIDER` and `DIMCODE_MODEL` take precedence over this file. Explicit `--model-config` and `--home` arguments select the model configuration and home for that invocation. Relative model configuration paths follow the caller's working directory.

`DIMCODE_PROVIDER` and `DIMCODE_MODEL` supply a paired default only for new chat sessions; explicit caller selection and resumed sessions retain native CLI behavior. Leave both unset to use the model configuration's default. `DIMCODE_HOME` can select a development home; the default is a short path under `~/.dimcode-dev` scoped by repository. `./dev stop` uses the same home without creating it; an existing native home requires an already built CLI for authenticated shutdown.

## Managed provider boundary

The Dim account (`dimcode-api-oauth`) is a private Rust plugin in the `dim-oauth-plugin-rust` repository. That repository depends on this one by path (`../dim`) and builds the account build of `dimcode`: the same CLI and App Server with the plugin installed through `dim_app_server::cli::run_with` ([managed provider plugins](../../crates/dim-app-server/README.md#managed-provider-plugins)). This repository holds only the generic `ManagedProvider` contract; the account realm, token flows, client IDs and credentials stay out of it. The CLI picks the account directory from its entry (`dimagent` or `dimcode`) so that users never enter it; see the [CLI contract](../../crates/dim-cli/README.md#terminal-ui-commands-and-input).

With the private checkout beside this one (or `DIM_OAUTH_SOURCE_DIR` naming it), `./dev` builds its `dimcode` there and runs it; otherwise it runs the public build. The private build starts from this repository's `Cargo.lock` (its `scripts/sync-lock.sh`), so both resolve one dependency graph. No Node, package or extraction is involved. Development uses its own home, so a development login never touches the product home's `auth.json`.

## CLI and daemon lifecycle

`dimcode models PROVIDER [--refresh]` uses the catalog/refresh RPCs and prints model IDs and availability without credential information. `--provider` and `--model` select a model for a new session.

`dimcode daemon stop` connects only to an existing authenticated endpoint for the selected home. No endpoint means successful completion without creating a daemon. Shutdown uses the normal server settlement path, waits for process/endpoint closure with a bounded timeout and never force-kills a process. The developer launcher uses this command before loading a new build. Account and session files are retained across restart.

## Verification

The developer flow covers independence from Node and plugin packages, argument forwarding including spaces, isolated home permissions and missing-daemon stop. `transport_flow` injects a managed provider through the same hook; the Dim account's own flows are tested in its repository. Tests, release packaging and publication are explicit operations.
