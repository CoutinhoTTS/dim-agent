# Local development and managed provider boundary

The repository root `./dev` builds the native CLI and App Server with locked Cargo dependencies and runs them against a separate, stable development home. Use `--model-config PATH` for API-key development without an account plugin, Node or JS dependency preparation. See the root [README](../../README.md) for complete setup and run commands.

## Entry points

| Entry | Behavior |
| --- | --- |
| `./dev --model-config PATH [CLI arguments]` | Incrementally build both Rust binaries, normally stop the development daemon, then run the new CLI with the configured provider |
| `./dev [CLI arguments]` | Use the model configuration selected in `.env.dev` or the inherited environment |
| `./dev restart [CLI arguments]` | Perform the same build and restart flow explicitly |
| `./dev stop` | Stop an existing development daemon without building or starting one |
| `./dev check` | Run explicit formatting and compilation checks; tests do not run on startup |

Caller arguments and exit status reach the native CLI. Diagnostics and build output go to stderr; stdout remains available for model output. The working directory remains the caller's workspace. On Unix the launcher replaces itself with the CLI so terminal signals are not duplicated.

Optional `.env.dev` is trusted local shell configuration and is ignored by Git. Inherited `DIMCODE_HOME`, `DIMCODE_MODEL_CONFIG`, `DIMCODE_PROVIDER` and `DIMCODE_MODEL` take precedence over this file. Explicit `--model-config` and `--home` arguments select the model configuration and home for that invocation. Relative model configuration paths follow the caller's working directory.

`DIMCODE_PROVIDER` and `DIMCODE_MODEL` supply a paired default only for new chat sessions; explicit caller selection and resumed sessions retain native CLI behavior. Leave both unset to use the model configuration's default. `DIMCODE_HOME` can select a development home; the default is a short path under `~/.dimcode-dev` scoped by repository. `./dev stop` uses the same home without creating it; an existing native home requires an already built CLI for authenticated shutdown.

## Managed provider boundary

Optional managed-provider packages own their source location, dependency preparation, setup and login instructions. The native host accepts a separately supplied runtime package and owns its generic service connection, credential leases and lifecycle; provider-specific authorization and catalog rules stay in the package. The one exception is the Dim account directory, which the CLI picks from its entry (`dimagent` or `dimcode`) so that users never enter it; see the [CLI contract](../../crates/dim-cli/README.md#terminal-ui-commands-and-input).

Runtime archive validation covers SHA-256, size, paths, entry types, exact file inventory, manifest schema, host API, rutis version, package version and entry digest. The runtime files are `package.json`, `dim-plugin.json` and `dist/index.js`. A failed preparation does not replace the last verified package and never launches it as a fallback.

The host supplies the pinned Node executable and locked rutis runtime. See [release packaging](../release/README.md) and the [App Server contract](../../crates/dim-app-server/README.md) for package loading and service boundaries.

## CLI and daemon lifecycle

`dimcode models PROVIDER [--refresh]` uses the catalog/refresh RPCs and prints model IDs and availability without credential information. `--provider` and `--model` select a model for a new session.

`dimcode daemon stop` connects only to an existing authenticated endpoint for the selected home. No endpoint means successful completion without creating a daemon. Shutdown uses the normal server settlement path, waits for process/endpoint closure with a bounded timeout and never force-kills a process. The developer launcher uses this command before loading a new server or plugin build. Account and session files are retained across restart.

## Verification

The developer flow covers API-key independence, argument forwarding including spaces, isolated home permissions and missing-daemon stop. Shared package checks cover immutable archive bytes and failed preparation without starting Rust or replacing a verified package. Managed-provider packages own provider-specific setup/login integration checks. Tests, release packaging and publication are explicit operations.
