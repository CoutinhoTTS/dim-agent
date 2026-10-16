# HostPlatform isolation spike

Scope: [F5 #76](https://github.com/arcships/dim/issues/76), [PR #112](https://github.com/arcships/dim/pull/112).
Status: measured macOS transport/lifetime spike. Product isolation and Linux acceptance remain open.

The existing `ProcessSpec` can launch `sandbox-exec` with the original executable and argv as its
arguments. Both `HostPlatform::spawn` and `spawn_pty` accept this spec unchanged; their existing tree
handles still own termination. This experiment adds no product API, configuration, dependency,
protocol or lifecycle phase. The profile and wrapping function live only in the example.

## Reproduce

```sh
cargo run --locked -p dim-agent-capabilities --example host_platform_isolation_spike
```

The example creates and removes its own temporary tree. It does not read a product home or use
credentials. On other platforms it returns an explicit unsupported-probe error.

Measured on 2026-10-16: macOS 27.0.1 (26A434), ARM64, Rust 1.98.1, dev profile. Each operation has
a five-second completion bound; the long-running process and PTY announce readiness before
termination. Startup samples are 20 alternating plain/isolated executions of `/usr/bin/true`,
including spawn, output drainage and exit. No warmup or parallel load.

| Spawn to exit | Median | p95 | Range |
| --- | ---: | ---: | ---: |
| Existing plain spawn | 1.346 ms | 1.668 ms | 1.184–2.033 ms |
| Existing spawn with Seatbelt argv | 6.812 ms | 7.848 ms | 6.383–7.850 ms |

Median uses the middle pair; p95 uses the nearest-rank sample. One process-tree termination took
21.453 ms. These are local observations, not latency guarantees or application throughput.
The example emits every raw sample as JSON.

## Exercised boundaries

| Flow | Observed result |
| --- | --- |
| Workspace path containing spaces and a quote | Read and write succeed; paths use `-D` parameters and separate argv elements |
| Shell descendant reads private fixture | Kernel rejects the read; no private bytes reach stdout |
| Workspace symlink points into private fixture | Read and write are rejected; outside file is not created |
| Direct outside write | Rejected; outside file is not created |
| TCP to a live loopback listener | Plain control succeeds; sandboxed `nc` reports `Operation not permitted` |
| Invalid profile | Wrapper exits unsuccessfully; payload marker is never created |
| Shell and its sleeping child | Existing `ProcessTree::terminate` completes; child PID no longer exists |
| PTY process | Existing PTY starts, sends readiness bytes and terminates through its existing handle |

The fixture profile permits host reads except its private directory, permits writes only under
its workspace and to `/dev/null`, and leaves networking denied. It demonstrates enforcement of
these rules; it does not define the product's filesystem or network policy. In particular, the
profile does not hide the rest of the user's readable files. PTY checks cover startup and
termination, not every file/network restriction through an interactive shell.

An exploratory profile allowing reads only under common system directories aborted `/bin/sh`
with `SIGABRT`. Its cause is unknown; no production workaround or broader product permission is
inferred from that result. The checked-in fixture states its broader read allowance explicitly.

## Existing implementation and integration points

| Existing owner | Relevant behavior |
| --- | --- |
| `host_platform.rs` / `HostPlatform::spawn` | Exact child environment, cwd, argv and process-group ownership; an executable wrapper fits its existing input |
| `HostPlatform::spawn_pty` | Same spec enters the portable PTY implementation; the same wrapper starts successfully |
| `workspace_tools/exec.rs` | Foreground/background execution both use `HostPlatform::spawn` |
| `shell_session.rs` | Persistent shell startup uses `HostPlatform::spawn_pty` |
| `HostPlatform::build_process_spec` (S7) | Reads the current shell and constructs argv/environment; isolation need not duplicate shell parsing |

No event subscription is needed to launch or terminate this process: each caller owns its tree
and already owns cancellation. Tool permission and approval stay at their existing boundaries.
An enabled local isolation policy belongs to HostPlatform, not Timeline history or a client's
request. This spike does not install that policy or change which calls execute with isolation.

## Platform evidence and remaining work

- **macOS:** the local `sandbox-exec(1)` manual marks the command deprecated; the installed Apple
  profiles describe their rules as a private interface. The mechanism worked on the measured
  host, so support on another macOS release requires running this probe there. No replacement
  API is assumed by this report.
- **Linux:** no Linux runtime was available for this measurement. The installed Docker client
  could not connect to its OrbStack daemon. No Linux behavior or performance is claimed.
  [Landlock's kernel documentation](https://docs.kernel.org/6.12/userspace-api/landlock.html)
  describes inherited restrictions and ABI-dependent filesystem/network coverage.
  [Bubblewrap's own documentation](https://github.com/containers/bubblewrap#sandbox-security)
  states that its arguments define the security policy and documents namespace and socket
  exposure considerations. Either needs an actual Linux probe before choosing the backend.
- **Windows:** isolation is unavailable in this spike; the example refuses to run.

Next, run the Linux mechanism through the existing process host, then specify the host-local
filesystem/network policy and its unavailable behavior using the existing HostPlatform contract.
Implement the selected policy at process startup only after that decision. F5 remains open until
real isolated exec on both macOS and Linux passes its flow acceptance.

Unmeasured boundaries include inherited file descriptors, hard links, filesystem mutation during
startup and non-TCP sockets. Their effect on the selected policy needs evaluation; this spike
adds no core mechanism or protocol to address hypothetical outcomes.

## Rule check

| Rule | Application |
| --- | --- |
| §4 rules 1–3 | No business commands, handovers or before callbacks change |
| §4 rule 4 | Existing process mechanics receive argv; no plugin-specific core behavior |
| §4 rules 5–6 | Timeline batches, dispatch and model-context entry are unchanged |
| §4 rule 7 | Uses the public HostPlatform and tree handles, not another plugin's private state |
| §4 rule 8 | No infrastructure change or requirement; existing APIs suffice for the experiment |
| §4 rule 9 | Measurements stay in JSON output, outside the Timeline |
| §1 | No new plugin, service, task supervisor or lock |
| ADR 002 D1–D2 | Process lifetime remains with its caller; no wiring plugin |
| ADR 002 D3 | No new dependency or readiness condition |
| ADR 002 D4 | No business settings or loader configuration is added |
| ADR 002 D5 | Existing executable/argv/cwd/environment data is used directly |

## Requirements raised to mechanism owners

None. The measured mechanism uses current process and PTY entry points. Product policy and Linux
acceptance belong to HostPlatform's F5 work, without a core or rutis extension.

## Ablation review

The example keeps only fixture setup/cleanup, argv construction, bounded process collection and
the measured flow assertions. Each retained helper serves repeated experiment operations. The
plain network control establishes that a refusal is not an unavailable endpoint, and the invalid
profile marker establishes that the payload did not execute. No product adapter is extracted.
