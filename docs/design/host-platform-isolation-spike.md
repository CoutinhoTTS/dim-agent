# HostPlatform isolation spike

Scope: [F5 #76](https://github.com/arcships/dim/issues/76), [PR #112](https://github.com/arcships/dim/pull/112).
Status: measured macOS and Linux transport/lifetime spike. Product isolation remains open.

The existing `ProcessSpec` can launch `sandbox-exec` or a single-threaded Landlock wrapper with the
original executable and argv as arguments. Both `HostPlatform::spawn` and `spawn_pty` accept this
spec unchanged; their existing tree handles still own termination. This experiment adds no
product API, configuration, dependency, protocol or lifecycle phase. Policies and wrappers live
only in the examples.

## Reproduce

```sh
# macOS
cargo run --locked -p dim-agent-capabilities --example host_platform_isolation_spike
# Linux, as an unprivileged user
cargo run --locked -p dim-agent-capabilities --example host_platform_landlock_spike
```

Each example creates and removes its own temporary tree, without reading a product home or using
credentials. A platform that does not match its example returns an explicit unsupported error.
The Linux example requires Landlock ABI 4 or newer and refuses to run without that support.

## macOS measurement

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
Both examples emit every raw sample as JSON. Measurements include spawn, output drainage and exit;
they do not establish a cross-platform backend comparison.

## Linux measurement

Measured on 2026-10-16: Linux `7.2.9-orbstack-00391-g2b298d05fbd4`, aarch64, Alpine 3.23.0,
Rust 1.98.1 musl, dev profile, UID 1000. The kernel reported Landlock ABI 10. The fixture requests
only ABI 4 filesystem rights through `TRUNCATE`, plus TCP bind/connect restrictions. The wrapper
applies `PR_SET_NO_NEW_PRIVS` and the ruleset before `exec`, before creating any runtime threads;
the collector remains unrestricted and owns the existing process handles.

The environment was an ordinary Docker container with its default seccomp profile, without
privileged mode or added capabilities. Its cached base was
`node@sha256:9632533eda8061fc1e9960cfb3f8762781c07a00ee7317f5dc0e13c05e15166f`;
the Node executable is not used by this probe. Build tools and pinned Rust were installed in that
disposable container. `tini -s` reaped orphaned descendants, as a host init normally does.
After building, the measured invocation was:

```sh
docker exec --user 1000:1000 CONTAINER tini -s -- \
  /target/debug/examples/host_platform_landlock_spike
```

| Spawn to exit | Median | p95 | Range |
| --- | ---: | ---: | ---: |
| Existing plain spawn | 0.196 ms | 0.249 ms | 0.168–0.509 ms |
| Existing spawn with Landlock wrapper | 0.251 ms | 0.506 ms | 0.211–0.560 ms |

There are 20 alternating samples per case, without warmup or parallel load. Process-tree
termination, including observed grandchild removal, took 21.807 ms. Every operation has the
same five-second bound as the macOS probe. This proves the exercised mechanisms in this Linux
VM/container environment; other kernels, distributions and bare-metal deployment remain unmeasured.

## Exercised boundaries

Both macOS and Linux examples assert these boundaries:

| Flow | Observed result |
| --- | --- |
| Workspace path containing spaces and a quote | Read and write succeed; paths remain separate argv elements (Seatbelt policy paths use `-D`) |
| Shell descendant reads private fixture | Kernel rejects the read; no private bytes reach stdout |
| Workspace symlink points into private fixture | Read and write are rejected; outside file is not created |
| Direct outside write | Rejected; outside file is not created |
| TCP to a live loopback listener | Plain control succeeds; restricted connection reports a permission error |
| Invalid policy | Invalid Seatbelt profile or missing Landlock rule path fails before the payload marker can be created |
| Shell and its sleeping child | Existing `ProcessTree::terminate` completes; child PID no longer exists |
| PTY process | Existing PTY starts, sends readiness bytes and terminates through its existing handle |

The macOS fixture profile permits host reads except its private directory, permits writes only under
its workspace and to `/dev/null`, and leaves networking denied. It demonstrates enforcement of
these rules; it does not define the product's filesystem or network policy. In particular, the
profile does not hide the rest of the user's readable files. PTY checks cover startup and
termination, not every file/network restriction through an interactive shell.

The Linux fixture grants read/execute on `/usr`, `/bin`, available library directories and its
own executable; the workspace gets the handled filesystem rights, and `/dev/null` permits read/write.
All handled TCP bind/connect operations are denied. UDP, UNIX sockets and ioctl rights are not
restricted by this ABI 4 fixture. Its libc calls and kernel ABI structs remain local to the example;
no isolation backend is installed in HostPlatform.

An exploratory Seatbelt profile allowing reads only under common system directories aborted `/bin/sh`
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
- **Linux:** the Landlock fixture passed all seven flow groups as UID 1000 on the kernel above.
  [Landlock's kernel documentation](https://docs.kernel.org/userspace-api/landlock.html)
  defines the inherited restriction model and ABI-dependent coverage. The probe uses its
  existing syscall contract without a new crate. Bubblewrap is not measured or selected.
- **Windows:** isolation is unavailable in this spike; both examples refuse to run.

The proposed [product policy](host-platform-isolation.md) specifies host-local filesystem,
environment and network boundaries and unavailable behavior. It requires design review before
implementation. F5 remains open until product isolated exec on both macOS and Linux passes its
acceptance; this spike does not establish the narrower proposed policy or select its backend.

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

None. The measured mechanisms use current process and PTY entry points. Product policy and
production exec acceptance belong to HostPlatform's F5 work, without a core or rutis extension.

## Ablation review

The examples keep only fixture setup/cleanup, argv construction, bounded process collection and
the measured flow assertions. Each retained helper serves repeated experiment operations. The
plain network control establishes that a refusal is not an unavailable endpoint, and the invalid
policy marker establishes that the payload did not execute. The Linux wrapper runs restriction
before threads and exec; its small ABI boundary avoids adding a library or product adapter for
the experiment. No additional production abstraction is needed by these measured flows.
