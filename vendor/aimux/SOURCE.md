# aimux source

- Upstream: https://github.com/arcships/aimux
- Release: `v0.6.1`.
- Revision: `164d7f4bea6d6b726f5f9eef9bda492190766558`.
- License: MIT, retained in `LICENSE`.
- Included: the four Rust library crates needed by the Agent integration, their in-crate test modules, and the files those modules include: `fixtures/`, `contract-tests/`, `aimux-core/tests/fixtures/`, `aimux-providers/tests/fixtures/` and `scripts/aisdk-fixtures/package-lock.json`. FFI, CLI, web tools and the tag's standalone integration test binaries (`*/tests/*_test.rs`, and `aimux-providers/tests/capabilities_export.rs`, which does not match that pattern) are omitted.
- Three support files are copied verbatim because upstream test modules include them with `#[path]`, so the vendored tests need them beside the code: `fixtures/aisdk/pinned_versions.rs`, `aimux-providers/tests/common/mock_fetch.rs` and `aimux-providers/tests/common/replay.rs`.
- The workspace member list is reduced to these crates; production source is retained verbatim except for the changes listed in `PATCHES.md`.
- Local patches are temporary until an upstream revision provides the same contracts. `PATCHES.md` names each patch's upstream issue or request.
- The Agent continues to call single-attempt provider operations, not `stream_text` or host-side tool-call repair. Request tasks, cancellation, recovery and terminal events remain with their existing v4 Plugin owners.
- The crates' own library tests run in a scratch copy, since this tree is excluded from the root workspace: copy `vendor/aimux` and the root `Cargo.lock` into a new directory, run `cargo metadata --format-version 1` there once so cargo adds the seven dependency entries the root lock lacks, then run `cargo test --locked -p aimux-core --lib` and the same for `aimux-provider-utils`, `aimux-providers` and `aimux-stream` from that directory, with `CARGO_TARGET_DIR` outside it and the provider environment variables (`ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY`) unset.
