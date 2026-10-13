# Dim

Dim is a Rust coding agent with a simple terminal scrollback interface. The native CLI connects to a Rust App Server that owns sessions, models, workspace tools, approvals and persistent history.

This workspace contains Agent v4 and the native CLI. It builds independently of the original TypeScript agent, OpenTUI, Desktop and web applications. Public source is licensed under [MIT](LICENSE). An optional private account plugin is supplied separately and is not required to build or use API-key providers.

## Local OAuth setup

The examples use Bash. Install Rust through rustup; Cargo selects Rust 1.98.1 from `rust-toolchain.toml`. Local OAuth development also requires Node 24.21.0 (see `.node-version`), npm, pnpm 12.3.4 and tar. If pnpm is missing, install the pinned version with `npm install --global pnpm@12.3.4`. Native build prerequisites are described in [build setup](docs/architecture/build.md).

Keep the private source checkout beside this repository:

```text
workspace/
├── dimrust-cli/
└── dim-oauth-plugin/
```

Run this once from `dimrust-cli`:

```sh
cp .env.example .env.dev
./dev setup
```

For another source location, set `DIM_OAUTH_SOURCE_DIR=/absolute/path/to/dim-oauth-plugin` in `.env.dev`. Setup installs frozen JS dependencies when needed, builds the local plugin, packages it and verifies/extracts its three runtime files. It does not compile Rust, start a daemon or download a remote plugin. The first run compiles both Rust executables; later runs compile incrementally.

Log in, refresh the directory, then use a model ID from its output:

```sh
./dev login dimcode-api-oauth --directory https://dimagent.cn --device
./dev models dimcode-api-oauth --refresh
./dev --provider dimcode-api-oauth --model MODEL_ID
```

Open the printed verification URL and enter its device code. Replace `MODEL_ID` with an available ID printed by `models`. For a custom account directory, replace the login URL. The private plugin supplies this provider; public API-key users do not need its source.

## Daily development

Set the selected model in the ignored `.env.dev` file:

```sh
DIMCODE_PROVIDER=dimcode-api-oauth
DIMCODE_MODEL=MODEL_ID
```

Then start with `./dev`. It rebuilds and verifies the local source package, reuses matching dependency installations, incrementally builds Rust, normally stops only the isolated development daemon and loads the new server/plugin. Login and session files are retained. The default development home is a short stable path under `~/.dimcode-dev`; `DIMCODE_HOME` selects a custom home. No production home is migrated.

```sh
./dev --prompt "Explain this project"       # one turn, then detach
./dev --resume SESSION_ID                   # continue the printed session
./dev restart                              # explicitly rebuild and restart
./dev stop                                 # normal shutdown, no build
./dev check                                # explicit formatting/compilation checks
```

The working directory remains the caller's workspace. To work on another project, run `/absolute/path/to/dimrust-cli/dev --cwd /absolute/path/to/project`. Startup does not run tests or produce release/npm distributions. From the private checkout, `./scripts/dev.sh` delegates to the same entry. See the [development contract](docs/architecture/development.md) for configuration precedence and runtime boundaries.

## API-key setup

API-key development needs Rust only; Node, pnpm and the private checkout are optional. Prepare a local model configuration:

```sh
mkdir -p .dev
cp crates/dim-agent-aimux/examples/openai.json .dev/model.json
export OPENAI_API_KEY=your-key
```

Edit `.dev/model.json` for your provider's base URL and credential environment-variable name. Set both `defaultModel.modelId` and the corresponding model entry `id` to an ID available to your account. Remove OAuth `DIMCODE_PROVIDER`/`DIMCODE_MODEL` defaults from `.env.dev` to use this JSON file's default model, or explicitly choose a provider/model from the configured catalog.

```sh
./dev --model-config "$PWD/.dev/model.json"
```

For subsequent `./dev` runs, set `DIMCODE_MODEL_CONFIG=/absolute/path/to/model.json` in `.env.dev`. The launcher bypasses the private source and JS setup in this mode. Clear that setting when returning to OAuth development. The direct Cargo path remains available without a launcher:

```sh
cargo build --locked -p dim-cli -p dim-app-server
export OPENAI_API_KEY=your-key
./target/debug/dimcode \
  --home /absolute/path/to/isolated-home \
  --model-config crates/dim-agent-aimux/examples/openai.json
```

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
| `crates/dim-cli` | Native scrollback client and local RPC transport |
| `crates/dim-app-server` | RPC, persistence, provider configuration, account-plugin bridge and daemon |
| `crates/dim-agent-types` | Public event and data contracts |
| `crates/dim-agent-runtime` | Agent, sessions, timelines, dispatch and lifecycle |
| `crates/dim-agent-capabilities` | Input, model, tool, permission and branch capabilities |
| `crates/dim-agent-aimux` | Native HTTP model protocols |
| `vendor/aimux` | Pinned upstream source, license and local patches |
| `tests/` | Public extensions and integration flows |
| `xtask/` | Architecture boundaries, generated graphs and measurements |
| `npm/` | `dimcode` wrapper with `dim` and `dimcode` command aliases |

See [documentation](docs/INDEX.md), [architecture](docs/architecture/README.md), [build setup](docs/architecture/build.md) and [release packaging](docs/release/README.md). The npm package name is `dimcode`; public repository metadata targets `arcships/dim`. Publishing remains disabled.

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -p xtask -- check
node --test scripts/*.test.mjs
```

Public source verification does not require private plugins, account credentials or network model calls. HTTP model flows use local test servers. Building account-enabled distributions additionally requires the separately supplied plugin archive, its SHA-256 and the pinned Node/rutis runtime.
