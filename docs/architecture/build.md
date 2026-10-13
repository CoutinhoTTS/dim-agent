# Build environment

Run commands at the workspace root. `rust-toolchain.toml` pins Rust 1.98.1, rustfmt and Clippy. Do not copy target directories between worktrees.

```sh
rustup show
unset RUSTC_WRAPPER
export CARGO_INCREMENTAL=0
cargo build --workspace --locked
```

A normal Cargo installation is sufficient; compiler caching is optional. Each worktree keeps its own target directory. Existing source caches can be reused without sharing build outputs.

Node is only needed for packaging, packaging checks and JS account-plugin development. `.node-version` pins the Node version bundled into account-enabled releases. The `runtime/package-lock.json` fixes the rutis Node runtime and its transitive dependencies; `npm ci --ignore-scripts --prefix runtime` installs them for local bridge testing. The private plugin bundles its SDK and requires no runtime dependency installation in its own directory.

Release runners build natively for macOS arm64/x64, Linux arm64/x64 and Windows x64. They require Rust, a C/C++ toolchain and CMake, the pinned official Node distribution including npm and its LICENSE, tar, and Bash (Git Bash on Windows). Each runner's OS/libc baseline must be recorded and verified before declaring that platform supported. Cross-compilation alone is not installation acceptance.

Public Node provider and MCP interoperability fixtures use development dependencies declared in `runtime/package.json`. Install them with `npm ci --prefix runtime --ignore-scripts`; distribution staging uses `--omit=dev` and does not ship those SDKs.
