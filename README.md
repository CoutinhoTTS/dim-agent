# Dim

Dim is a Rust coding agent with a simple terminal scrollback interface. The native CLI connects to a Rust App Server that owns sessions, models, workspace tools, approvals and persistent history.

This workspace contains Agent v4 and the native CLI. It builds independently of the original TypeScript agent, OpenTUI, Desktop and web applications. Public source is licensed under [MIT](LICENSE). The Dim account is a private Rust plugin built into a separate account build of the same executable; it is not required to build or use API-key providers.

## API-key setup

The examples use Bash. Install Rust through rustup; Cargo selects Rust 1.98.1 from `rust-toolchain.toml`. Native prerequisites are described in [build setup](docs/architecture/build.md). API-key development needs no Node or pnpm. Run from the workspace root:

```sh
mkdir -p .dev
cp crates/dim-agent-aimux/examples/openai.json .dev/model.json
export OPENAI_API_KEY=your-key
```

Edit `.dev/model.json` for your provider's base URL and credential environment-variable name. Set both `defaultModel.modelId` and the corresponding model entry `id` to an ID available to your account. Leave `DIMCODE_PROVIDER`/`DIMCODE_MODEL` unset to use this JSON file's default model, or explicitly choose a provider/model from the configured catalog.

```sh
./dev --model-config "$PWD/.dev/model.json"
```

For subsequent `./dev` runs, copy `.env.example` to the ignored `.env.dev` and set `DIMCODE_MODEL_CONFIG=/absolute/path/to/model.json`. The direct Cargo path remains available without a launcher:

```sh
cargo build --locked -p dimcode
export OPENAI_API_KEY=your-key
./target/debug/dimcode \
  --home /absolute/path/to/isolated-home \
  --model-config crates/dim-agent-aimux/examples/openai.json
```

## ACP clients

Run the native stdio gateway from an ACP client such as acpx:

```sh
./target/debug/dimcode acp \
  --home /absolute/path/to/isolated-home \
  --model-config /absolute/path/to/model.json
```

The gateway connects to the existing local App Server; it owns protocol I/O,
session projection and connection recovery. Mode/model options and managed
account login depend on the daemon's installed capabilities. Client-supplied MCP
servers use disposable Session plugins alongside native tools; their configuration
is never saved. Older daemons without that interface accept and ignore the list.
Additional workspace roots are unsupported. See the
[implementation contract](docs/design/acp-gateway-implementation.md) and
[acceptance record](docs/design/acp-gateway-acceptance.md) for supported methods,
limits and verification.

## Daily development

With `DIMCODE_MODEL_CONFIG` set, run `./dev` to incrementally build both Rust executables, normally stop the isolated development daemon and start the new CLI. Session files are retained. The default development home is a short stable path under `~/.dimcode-dev`; `DIMCODE_HOME` selects a custom home.

```sh
./dev --prompt "Explain this project"       # one turn, then detach
./dev --resume SESSION_ID                   # continue the printed session
./dev restart                              # explicitly rebuild and restart
./dev stop                                 # normal shutdown, no build
./dev web                                  # serve the Web workspace until Ctrl+C (also /web in the TUI)
./dev check                                # explicit formatting/compilation checks
```

The working directory remains the caller's workspace. Pass `--cwd /absolute/path/to/project` to select another workspace. Startup does not run tests or produce release/npm distributions. See the [development contract](docs/architecture/development.md) for configuration precedence and runtime boundaries. Optional managed-provider packages document their own setup and login steps.

To try standalone binaries for every platform, as the TypeScript CLI's `build:all` did, run `node scripts/build-all.mjs` (or `--target darwin-arm64` and so on). It cross-compiles the single `dimcode` executable, App Server and Web workspace included, into `dist/dimcode-<platform>`; the file runs on its own and starts itself as its daemon (`dimcode app-server`). Linux and Windows targets need Zig and `cargo install --locked cargo-zigbuild`, plus `rustup target add` for each triple the script names.

Use `--prompt TEXT` or redirected stdin for a single noninteractive turn. Use `--resume SESSION_ID` to continue a persisted session. Run `dimcode --help` for account, model and runtime options. Help and version commands do not start a daemon.

```text
dimcode
workspace: /path/to/project
session: <session-id>

> Explain this project
Assistant:
The main entry point is ...
[tool] read README.md
[done] read README.md

[approval] exec
Allow once? [y/N] _
```

Output stays in terminal scrollback. During a turn, Ctrl+C requests a stop and waits for settlement. At idle, Ctrl+C or EOF exits the client. Approval defaults to deny; noninteractive approval requirements produce a nonzero exit code. Exiting detaches the client from the persisted session.

The default product home is `~/.dimcode`; first-time development and tests should use a separate `--home`. Existing sessions and account files are not automatically migrated. API-key model configuration and product-provider configuration are separate profiles; see the [CLI contract](crates/dim-cli/README.md) and [App Server](crates/dim-app-server/README.md).

## Workspace

| Path | Responsibility |
| --- | --- |
| `crates/dimcode` | The single `dimcode` executable: the CLI, with the App Server as its `app-server` subcommand |
| `crates/dim-cli` | Native scrollback/TUI clients, ACP stdio host and daemon startup policy |
| `crates/dim-client` | Shared protected RPC transport, product-event/Questions boundary and callback I/O |
| `crates/dim-acp` | Client-host rutis ACP gateway and conditional Lody projection |
| `crates/dim-app-server` | RPC, persistence, provider configuration, account-plugin bridge and daemon |
| `crates/dim-agent-types` | Public event and data contracts |
| `crates/dim-agent-runtime` | Agent, sessions, timelines, dispatch and lifecycle |
| `crates/dim-agent-capabilities` | Input, model, tool, permission and branch capabilities |
| `crates/dim-agent-aimux` | Native HTTP model protocols |
| `vendor/aimux` | Pinned upstream source, license and local patches |
| `tests/` | Public extensions and integration flows |
| `xtask/` | Architecture boundaries, generated graphs and measurements |
| `npm/` | `dimcode` wrapper with `dim` and `dimcode` command aliases |

See [documentation](docs/INDEX.md), [architecture](docs/architecture/README.md), [build setup](docs/architecture/build.md) and [release packaging](docs/release/README.md). The npm package name is `dimcode`; the repository is `arcships/dim`. Self-hosted CI builds draft GitHub Releases; npm publishing remains disabled. Release preparation follows the [project release skill](.agents/skills/dim-release/SKILL.md).

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -p xtask -- check
node --test scripts/*.test.mjs
```

Public source verification does not require private plugins, account credentials or network model calls. HTTP model flows use local test servers. Building account-enabled distributions additionally requires the separately supplied plugin archive, its SHA-256 and the pinned Node/rutis runtime.
