# aimux source

- Upstream: https://github.com/arcships/aimux
- Release: `v0.5.0`.
- Revision: `55cd475b2e963fab32572606b95cc30ae9184568`.
- License: MIT, retained in `LICENSE`.
- Included: the four Rust library crates needed by the Agent integration. FFI, CLI, recordings, and web tools are omitted.
- The workspace member list is reduced to these crates; production source is retained verbatim except for the changes listed in `PATCHES.md`.
- Local patches are temporary until an upstream revision provides the same contracts. No change has been published upstream.
- The Agent continues to call single-attempt provider operations, not `stream_text` or host-side tool-call repair. Request tasks, cancellation, recovery and terminal events remain with their existing v4 Plugin owners.
