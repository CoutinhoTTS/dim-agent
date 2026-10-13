# Development contract

Dim is the MIT-licensed harness mainline. Read [architecture](docs/architecture/README.md),
[foundation decisions](docs/adr/001-harness-foundation.md), and the owning module before editing.
Accepted design is not evidence of implementation; keep implementation status accurate.

## Existing runtime rules

- Capabilities use rutis `TypedPlugin` and runtime `Capability`. Declare dependencies
  through `Deps`, including lifecycle gates; use scoped keys and `When` for conditional dependencies.
- Factories capture configuration and stable identities, not live I/O. Each generation
  acquires its resources in `apply` and registers cleanup with `ctx.effect`.
- Capabilities own their tasks, cancellation and terminal outcomes. Dispatcher handlers
  register work and return; do not wait for network or tool completion in a handler.
- Branch background work waits for activation and observes closing. Settle accepted work
  and required terminal facts before removing its services. Detaching a client is not closing a Session.
- Timeline owns durable business facts. Derived state must be reconstructible; runtime
  handles are not durable identities. Publish events only after their atomic batch commits.
- Runtime context injection uses adopted inputs containing final content. Domain-specific
  history rendering must not become a second route into model context.
- Validate external data at ingress and storage read boundaries. Internal consumers use
  the established types; fix the producer instead of adding alias/default repair layers.
- Preserve the public/private boundary: no company account flows, relay policy, credentials
  or private implementation material in public history, logs, documentation or fixtures.
- A trusted in-process plugin is not sandboxed by dependency declarations. Authorization
  and isolation require explicit host execution boundaries.

## Changes and verification

- Follow [build setup](docs/architecture/build.md). Do not hand-edit generated graphs.
- Keep decisions, implemented contracts and contributor instructions distinct. Do not require
  contributors to use planned APIs before those APIs exist.
- Review correctness, security, compatibility and unnecessary abstraction before committing.
  Use removal/ablation experiments where they distinguish necessary behavior; restore the
  final implementation and rerun the affected tests. Do not manufacture redundant abstractions.
- Commit complete, reviewable units on their declared parent. Update their documentation and
  observable-behavior tests together. Never commit secrets or scratch experiments.
- Use disposable homes/databases for validation. Incompatible persistence changes require an
  explicit format decision; do not silently reuse a version for incompatible committed formats.
- At an integrated delivery boundary run:

  ```sh
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets --locked -- -D warnings
  cargo test --workspace --locked
  cargo run --locked -p xtask -- graph
  cargo run --locked -p xtask -- check
  node --test scripts/*.test.mjs
  ```

  Inspect generated changes, commit required outputs, then confirm regeneration is stable.
  During implementation, run targeted checks rather than repeating every full gate per edit.
