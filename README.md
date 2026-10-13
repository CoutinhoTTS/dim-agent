# Dim

Dim is a Rust coding agent with a simple terminal scrollback interface. The native CLI connects to a Rust App Server that owns sessions, models, workspace tools, approvals and persistent history.

This workspace contains Agent v4 and the native CLI. It builds independently of the original TypeScript agent, OpenTUI, Desktop and web applications. Public source is licensed under [MIT](LICENSE). An optional private account plugin is supplied separately and is not required to build or use API-key providers.

## Build and run

Run commands at the repository root. The Rust toolchain is pinned in `rust-toolchain.toml`.

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
