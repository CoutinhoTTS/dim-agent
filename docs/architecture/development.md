# Local development and managed provider boundary

The repository root `./dev` builds the native CLI and App Server with locked Cargo dependencies and runs ordinary commands against a separate, stable development home. Migration follows its [distribution routing](../design/cli-commands.md#migration-interaction). Use `--model-config PATH` for API-key development without the private account plugin. See the root [README](../../README.md) for complete setup and run commands.

## Entry points

| Entry | Behavior |
| --- | --- |
| `./dev --model-config PATH [CLI arguments]` | Incrementally build both Rust binaries, normally stop the development daemon, then run the new CLI with the configured provider |
| `./dev [CLI arguments]` | Use the model configuration selected in `.env.dev` or the inherited environment |
| `./dev migration` | Import Global `.dim/v2` into `.dim` and CN `.dimcode/v2` into `.dimcode` separately, showing progress and results; do not inject the chat development home or restart a daemon |
| `./dev restart [CLI arguments]` | Perform the same build and restart flow explicitly |
| `./dev stop` | Stop an existing development daemon without building or starting one |
| `./dev check` | Run explicit formatting and compilation checks; tests do not run on startup |

Caller arguments and exit status reach the native CLI. Diagnostics and build output go to stderr; stdout remains available for model output. The working directory remains the caller's workspace. On Unix the launcher replaces itself with the CLI so terminal signals are not duplicated.

Optional `.env.dev` is trusted local shell configuration and is ignored by Git. Inherited `DIM_OAUTH_CLIENT_ID_CN`, `DIM_OAUTH_CLIENT_ID_GLOBAL`, `DIMCODE_HOME`, `DIMCODE_MODEL_CONFIG`, `DIMCODE_PROVIDER`, `DIMCODE_MODEL` and `DIMCODE_DEFAULT_PLUGINS` take precedence over this file. Explicit `--model-config` and `--home` arguments select the model configuration and home for that invocation. Relative model configuration paths follow the caller's working directory.

`DIMCODE_DEFAULT_PLUGINS` can name a default plugin configuration file for a run. Otherwise every command that can start the daemon (`./dev check`, `./dev stop`, `./dev help`, `./dev daemon stop`, migration, `--help` and `--version` cannot) stages the default plugins into the checkout's ignored `staging/dev-plugins` and passes that copy, so the plugin code resolves the runtime's single copy of `@deepseek-ai/cordis` instead of one installed in the checkout. Staging copies every listed plugin's sources on every run, runs `npm ci` only for a plugin whose package files changed since the previous stage, and removes a plugin that is no longer listed; each plugin is built beside the live copy, whose dependencies it copies when they are reused, and the live copy is untouched until the replacement is complete and swapped in, so a failure names the plugin, prints one line in the launcher, and the run continues without the default plugins. A recorded directory is removed only when no current plugin uses it, and a directory that resolves outside the stage is refused before anything changes. Without Node or npm the run also continues without them.

`DIMCODE_PROVIDER` and `DIMCODE_MODEL` supply a paired default only for new chat sessions; explicit caller selection and resumed sessions retain native CLI behavior. Leave both unset to use the model configuration's default. `DIMCODE_HOME` can select a development home; the default is a short path under `~/.dimcode-dev` scoped by repository. `./dev stop` uses the same home without creating it; an existing native home requires an already built CLI for authenticated shutdown.

## Managed provider boundary

The Dim account (`dimcode-api-oauth`) is the Rust plugin in [`crates/dim-oauth`](../../crates/dim-oauth/README.md). `dimcode` installs it through `dim_app_server::cli::run_with` ([managed provider plugins](../../crates/dim-app-server/README.md#managed-provider-plugins)) when it is built as the account build. The CLI picks the account directory from its entry (`dimagent` or `dimcode`) so that users never enter it; see the [CLI contract](../../crates/dim-cli/README.md#terminal-ui-commands-and-input).

The account's OAuth client IDs never enter the repository. `dimcode` reads `DIM_OAUTH_CLIENT_ID_CN` and `DIM_OAUTH_CLIENT_ID_GLOBAL` at compile time: with both set it is the account build, with neither the API-key build, and with only one the build fails. Locally they come from `.env.dev` or the environment, so `./dev` builds the account build when they are set; release CI reads them from the repository secrets of the same names. No Node, package or extraction is involved. Development uses its own home, so a development login never touches the product home's `auth.json`.

## CLI and daemon lifecycle

`dimcode models PROVIDER [--refresh]` uses the catalog/refresh RPCs and prints model IDs and availability without credential information. `--provider` and `--model` select a model for a new session.

`dimcode daemon stop` connects only to an existing authenticated endpoint for the selected home. No endpoint means successful completion without creating a daemon. Shutdown uses the normal server settlement path, waits for process/endpoint closure with a bounded timeout and never force-kills a process. The developer launcher uses this command before loading a new build. Account and session files are retained across restart.

## Verification

The developer flow covers staging the default plugins from the checkout and updating them incrementally, running without Node or npm, argument forwarding including spaces, isolated home permissions and missing-daemon stop. `transport_flow` injects a managed provider through the same hook; the Dim account's own flows are tested in `crates/dim-oauth/tests`. Tests, release packaging and publication are explicit operations.
