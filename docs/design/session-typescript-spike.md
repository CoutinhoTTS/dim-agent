# C4 Session TypeScript spike

Scope: issue [#63](https://github.com/arcships/dim/issues/63), PR [#99](https://github.com/arcships/dim/pull/99). This is a transport and lifecycle experiment, not the product implementation of C4.

Status: measured spike. The production binding design and C4 acceptance remain open.

The probe uses the pinned rutis 0.10 runtime and loader, actual Dim Sessions, instance-scoped named services, the existing Session broadcast and a TypeScript `definePlugin` part. All experimental bindings stay in the example and fixture. No production capability, core lifecycle, dependency audit or persistence format changes.

## Reproduce

```sh
npm ci --prefix runtime --ignore-scripts
env -u RUSTC_WRAPPER CARGO_INCREMENTAL=0 cargo run --locked -p dim-app-server --example session_typescript_spike
```

The example uses the in-memory store and mock model/host, with no product home, credentials or network model calls. It returns JSON timings and fails if its flow assertions fail. Measurements include bridge serialization and dispatch; they are local observations rather than performance promises.

Source: [Rust host](../../crates/dim-app-server/examples/session_typescript_spike.rs), [TypeScript part](../../runtime/spikes/session-part.ts).

## Results

Measured on 2026-10-15: macOS ARM64, Node v24.21.0, Rust host in the dev profile, four Tokio workers. Each case makes 20 warm-up calls and then 500 sequential calls over the real local runtime channel. Percentiles use nearest rank. Payload size is the ASCII string content, excluding JSON and channel framing. Timings include argument copying, serialization, dispatch, the JS reply and result decoding; they are not just socket latency.

| Payload | Method | p50 | p95 | p99 |
| --- | --- | --- | --- | --- |
| 64 B | sync | 68.916 µs | 82.708 µs | 97.583 µs |
| 64 B | async | 109.625 µs | 122.125 µs | 147.000 µs |
| 4 KiB | sync | 152.000 µs | 163.584 µs | 171.250 µs |
| 4 KiB | async | 201.709 µs | 218.458 µs | 234.834 µs |
| 64 KiB | sync | 1.506 ms | 1.596 ms | 1.638 ms |
| 64 KiB | async | 1.552 ms | 1.643 ms | 1.704 ms |

Top-level loader reconciliation took 124.059 ms; creating the first Session took 8.634 ms; closing a Session with a hanging TS call took 2.366 ms. These are single observations, not startup percentiles. The Agent anchor declares the existing `RuntimeRows` dependency: row metadata refresh must finish before Sessions are created. No startup polling or new readiness phase is used.

| Flow | Evidence |
| --- | --- |
| Two real Sessions, one Node process | Both parts report the same process ID and their own Session IDs. |
| Instance service isolation | Each part reads its Session's native `c4Session` service through loader-generated isolate labels. |
| Tool data crosses the channel | Rust sends a native `ToolCall`; the returned declaration and result deserialize as `ToolDefinition` and `ToolResult`. |
| Actual Session broadcast, isolated | The first Session's native echo turn reaches its TS observer; the second receives none of that turn's `input/applied` records. Existing broadcast fences establish completion without sleeps. |
| Explicit cancellation | A hanging call observes the exported cancellation Promise and completes once. |
| Unload settlement | Session close runs the TS effect, completes its hanging call and records exactly one cleanup. |
| Stale handler | A retained service object rejects calls after its part unloads. This is enforced by the fixture's generation guard, not by assuming the old remote object disappeared. |
| Other Session continues | The second part remains callable and its native model flow still completes after the first Session closes. |

The native call owns a standard cancellation-token drop guard. It settles the exported Promise when the call returns, including completion caused by unload, so an unused Promise cannot hold runtime drain open. The experiment uses no hand-written mutex, extra event bus or cancellation manager.

Not measured: concurrent throughput, process RSS, runtime-crash recovery, Windows/Linux, or an end-to-end model request using the TS tool through ToolExecutor. TS executes through the pinned runtime's `tsx`; a separate TypeScript static compiler was not installed. Native boundary decoding and the executable flow assertions check the exercised data paths.

## Boundary under investigation

- `ServiceCatalog::register_shared_instance` maps a named service to each Session instance and gives its Node copy an isolate label. `register_instance` alone only names a typed Rust service; it does not create a cross-language binding.
- Existing runtime rows export `HostDispatch`, not native `ToolContribution`. The tool DTO experiment calls the exported object directly and deserializes its declaration/result at the Rust boundary. It does not prove the ToolRegistry / ToolExecutor path.
- `definePlugin` exposes only `use`, `provide` and `effect`; it has no event subscription method. The probe serializes the actual Session channel into an experimental named service call. This is a fixture for measuring transport, not an accepted broadcast protocol or an additional product event bus.
- Method DTO transport is distinct from C1 Session RPC dispatch. C1 #83 remains partial pending the contract decision in #120; the probe must not introduce a replacement `sessionMethods` service.
- Session assembly waits for installed required rows, including runtime dependencies. The experiment does not add optional dependency edges or skip Pending rows.

## Integration ownership

Production bindings belong to the existing contract owners: ToolRegistry / tool contribution, Session broadcast, and C1 SessionMethods. The example's combined probe must not become a wiring plugin. C3 #82 supplies fixed-tool sampling, E2 #122 owns tool-change event replacement, C1 #83/#120 owns Session method lookup/dispatch, and C6 #85 owns broadcast records. Those tasks are not implemented here.

## Minimum implementation sequence

1. **Finalize the neutral bindings before product code.** Define tool service identity, declaration/error decoding, cancellation and Session broadcast subscription/cleanup against A.5 and the existing broadcast. The probe's `c4*` names, `notice` DTO and extra Promise argument are experimental, not public contracts. Do not expose the combined probe as a production adapter.
2. **Use existing loader assembly.** Register shared services inside `SESSION_GROUP`; load TS rows inside the Session subtree using the existing runtime resolver. Keep factories limited to configuration and instance identity. Share the Node process and use instance service labels; those labels provide namespace isolation, not a security sandbox. Do not reuse the private managed-provider configuration as a general capability loader.
3. **Connect the tool at its owner.** Implement the language binding at the tool-contribution boundary, reuse C3's one-time fixed-table sampling and ToolBinding request snapshots, and validate the real model declaration → ToolExecutor → TS result flow on the immediate first request after create and restore. The binding must be available before that sample; no attachment polling or tool-specific branches in Timeline, Dispatcher or Session readiness.
4. **Integrate the existing method and broadcast owners.** Once C1 #83/#120 is decided and implemented, expose that same SessionMethods contract to TS. Align broadcast data with C6 #85 and subscribe to the existing Session channel; do not create a second event source or duplicate RPC dispatch. Final subscription delivery/order and cancellation semantics require contract review; the transport measurements do not decide them.
5. **Verify product acceptance.** A TS part must serve one real tool and one real Session RPC method, receive only its Session's broadcast, settle cancellation and unload, and preserve native execution when its optional provider becomes unavailable. Disabled parts must not be installed. Installed required Pending rows keep their current waiting semantics; the roadmap's stronger optional-startup wording needs explicit reconciliation before claiming it is implemented.

No evidence from this spike requires changing rutis, adding core lifecycle gates, weakening dependency auditing or skipping Pending rows. C4 can proceed with a contract review based on the proven transport, while #63 remains open for the production flow.

## Verification and ablation

The executable probe, example-target Clippy with warnings denied, `xtask check`, formatting and whitespace checks pass. Production source, dependencies, generated architecture output, wire formats and frozen event fixtures are unchanged. Full workspace tests are left to PR CI because no product implementation changed.

The retained harness pieces each serve an exercised boundary: Node/rows wrappers install existing runtime plugins as loader rows; the anchor waits for the existing rows gate; the native fixture supplies per-Session identity and transports its actual broadcast; fences replace timing sleeps; cancellation uses the existing token guard. The native helper service and event encoding remain probe-only. No additional abstractions are needed by the measured flows.

## Rule check

| Rule | Application |
| --- | --- |
| §4 rule 1 | No other capability's state is written; native Session input uses the public submit path. |
| §4 rule 2 | Exported objects keep their generation; old handlers must reject calls after unload. |
| §4 rule 3 | No before callback. |
| §4 rule 4 | No business-specific core handling. |
| §4 rule 5 | No new Timeline facts or batch behavior. |
| §4 rule 6 | The TS tool is called by the measurement harness; no unadopted content is inserted into model context. |
| §4 rule 7 | Public services, Session input and Session broadcast only. |
| §4 rule 8 | Existing loader, runtime, scope and shutdown contracts are measured unchanged. |
| §4 rule 9 | Measurements and lifecycle observations stay outside the Timeline. |
| §1 | The loader owns each TS row; `effect` owns cleanup and outstanding work. |
| ADR 002 D1 | TS state belongs to one Session; the Node process is shared. |
| ADR 002 D2 | No production wiring module; experimental bindings remain in the probe. |
| ADR 002 D3 | Required dependencies retain normal reload/Pending behavior; optional observation uses runtime lookup. |
| ADR 002 D4 | No business settings or loader configuration API is added. |
| ADR 002 D5 | Probe methods use serializable DTOs and Promise cancellation; product contracts require their owners' bindings. |

## Requirements raised to mechanism owners

None. The spike investigates existing mechanisms and records integration gaps; it does not infer a need to extend rutis or core lifecycle.
