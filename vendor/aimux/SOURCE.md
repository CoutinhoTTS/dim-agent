# aimux source

- Upstream: https://github.com/arcships/aimux
- Release: `v0.6.0`.
- Revision: `fa29a30ad25e362acc349bea6ce4b025bdd8be5a`.
- License: MIT, retained in `LICENSE`.
- Included: the four Rust library crates needed by the Agent integration. FFI, CLI, recordings, and web tools are omitted.
- Three support files are copied verbatim because upstream test modules include them with `#[path]`, so the vendored tests need them beside the code: `fixtures/aisdk/pinned_versions.rs`, `aimux-providers/tests/common/mock_fetch.rs` and `aimux-providers/tests/common/replay.rs`.
- The workspace member list is reduced to these crates; production source is retained verbatim except for the changes listed in `PATCHES.md`.
- Local patches are temporary until an upstream revision provides the same contracts. `PATCHES.md` names each patch's upstream issue or request.
- The Agent continues to call single-attempt provider operations, not `stream_text` or host-side tool-call repair. Request tasks, cancellation, recovery and terminal events remain with their existing v4 Plugin owners.
