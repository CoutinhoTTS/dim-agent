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
# Linux bubblewrap, through actual process/PTY handles
cargo run --locked -p dim-agent-capabilities --example host_platform_bubblewrap_spike
# Compare controlling-terminal behavior, without repeating the boundary flows
cargo run --locked -p dim-agent-capabilities --example host_platform_bubblewrap_spike -- --terminal-controls
```

Each example creates and removes its own temporary tree, without reading a product home or using
credentials. A platform that does not match its example returns an explicit unsupported error.
The Landlock example requires ABI 4 or newer. The bubblewrap example requires unprivileged user
namespaces and installed `/usr/bin/bwrap` on the Debian baseline below. Neither downgrades when
its mechanism is unavailable.

## macOS measurement

Measured on 2026-10-16: macOS 27.0.1 (26A434), ARM64, Rust 1.98.1, dev profile. Each operation has
a five-second completion bound; the long-running process and PTY announce readiness before
termination. Startup samples are 20 alternating plain/isolated executions of `/usr/bin/true`,
including spawn, output drainage and exit. No warmup or parallel load. These timings were
captured for #112's original broad-read profile; they are not measurements of the current
narrow-read fixture described below.

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

The original measured macOS profile in #112 permitted host reads except its private directory.
The current macOS fixture instead grants content reads only under explicit system directories,
its workspace and required devices, plus the literal root directory `/`. It grants metadata reads
globally, writes only under its workspace and to `/dev/null`, and leaves networking denied. There
is no fixture-specific private-path deny rule: default denial protects paths outside the grants.
The fixture's PATH-only environment is explicit and contains no credentials.

Measured narrow-read enforcement on 2026-10-16, on the same macOS 27.0.1 ARM64 baseline, passes
the existing file/symlink, invalid-policy, process-tree and PTY flows. It additionally rejects
chmod on the outside fixture without changing its mode, and TCP/UDP to live IPv4 and IPv6 host
loopback listeners. The UDP plain control must deliver a datagram, not merely return success.
PTY checks cover startup and termination, not every file/network restriction through an
interactive shell. This remains a fixture, not enabled product isolation.

The Linux fixture grants read/execute on `/usr`, `/bin`, available library directories and its
own executable; the workspace gets the handled filesystem rights, and `/dev/null` permits read/write.
All handled TCP bind/connect operations are denied. UDP, UNIX sockets and ioctl rights are not
restricted by this ABI 4 fixture. Its libc calls and kernel ABI structs remain local to the example;
no isolation backend is installed in HostPlatform.

In controlled narrow-read probes, `/bin/sh` exited 134 without `(literal "/")` and printed its
payload with that single additional literal grant. A clean environment and an already granted
cwd did not remove the failure. The failing syscall has not been identified. The checked-in
fixture retains that literal root-directory read; it does not grant reads of the root's descendants.
No broad host/private-directory read allowance is needed by these exercised flows.

## Linux bubblewrap feasibility

Measured on 2026-10-16 in a disposable, isolated OrbStack Linux machine: Debian 13.7 (trixie),
Linux `7.2.9-orbstack-00391-g2b298d05fbd4`, aarch64, bubblewrap 0.12.0. The probe ran as UID 501;
root was used only to install packages. The machine shared no macOS files or SSH agent. This is
ordinary unprivileged namespace execution, not a privileged container or a change to host policy.

The command below shows the measured mount/namespace construction. Run it as an ordinary user
on that Linux baseline; all fixture data is disposable. The shell array preserves quoted paths.

```bash
set -euo pipefail
probe_root=$(mktemp -d /tmp/dim-f5-bwrap.XXXXXX)
trap 'rm -rf -- "$probe_root"' EXIT
workspace="$probe_root/workspace with ' quotes"
readonly="$probe_root/readonly"
private="$probe_root/private"
mkdir -p "$workspace" "$readonly" "$private"
printf workspace-input > "$workspace/input"
printf readonly-input > "$readonly/input"
printf private-sentinel > "$private/sentinel"
chmod 600 "$readonly/input"
ln -s "$private" "$workspace/escape"
sandbox=(bwrap --unshare-user --unshare-pid --unshare-net --unshare-ipc --unshare-uts
  --die-with-parent --new-session --cap-drop ALL
  --ro-bind /usr /usr --ro-bind /etc /etc
  --symlink usr/bin /bin --symlink usr/sbin /sbin --symlink usr/lib /lib
  --proc /proc --dev /dev --ro-bind "$readonly" "$readonly"
  --bind "$workspace" "$workspace" --remount-ro / --chdir "$workspace"
  --clearenv --setenv PATH /usr/bin:/bin)
"${sandbox[@]}" /bin/sh -c 'cat input; printf written > output' </dev/null
```

The measured assertions were:

| Flow | Observed result |
| --- | --- |
| Workspace path with spaces/quote | Reads `workspace-input`, writes `written` through exact argv |
| Explicit read-only fixture | Reads succeed; append and chmod fail; original bytes and mode 0600 remain |
| Unmounted private fixture | Read/create fail, no private bytes are returned and no file is created |
| Workspace symlink to private fixture | Read fails |
| Network namespace | Child `/proc/self/ns/net` identity differs from the parent's |
| PTY setup | `script -q -e -c` runs the same argv with `--new-session` and prints `pty-ready` |

The PTY collector has null stdin so it cannot consume the probe script. These preliminary
mechanism checks did not use Linux HostPlatform's tree/PTY handles, test cancellation/reaping,
or attempt packets to host TCP/UDP/UNIX listeners. A distinct network namespace is not reported
as a packet-level acceptance test. Those flows remain required for product integration. There
is no bubblewrap latency comparison. The temporary machine and its fixtures are removed after
calibration; the product backend and configuration remain unimplemented.

## Linux bubblewrap process and terminal calibration

Measured on 2026-10-16 in a second disposable isolated machine with the same Debian 13.7,
Linux 7.2.9 ARM64 and bubblewrap 0.12.0 baseline, Rust 1.98.1 and UID 501. The
[bubblewrap example](../../crates/dim-agent-capabilities/examples/host_platform_bubblewrap_spike.rs)
uses actual `HostPlatform::spawn`, `ProcessTree`, `spawn_pty` and `PtyProcessTree` handles.
It grants the fixture's workspace, a read-only helper directory, `/usr`, `/etc` and private
runtime mounts. Its environment contains only PATH. No host share, SSH agent, product home
or credential is used. Each completion has a five-second bound; packet non-arrival is checked
for 300 ms after the isolated sender exits.

| Flow | Observed result |
| --- | --- |
| User and network namespace | Payload retains UID 501; its network namespace differs from the collector's |
| Host IPv4/IPv6 TCP | Plain control sends bytes to live listeners; isolated connection fails and no connection arrives |
| Host IPv4/IPv6 UDP | Plain control delivers the exact datagram; isolated payload sends no datagram to either listener |
| Host UNIX socket outside grants | Plain control exchanges bytes; the unmounted path is inaccessible inside the namespace |
| ProcessTree cancellation | Termination completes; all recorded monitor, namespace-init and shell/sleep descendant PIDs disappear |
| PTY input and resize | Payload reads terminal input, reports the changed size 41 × 132 and terminates with all recorded descendants gone |
| Interactive bash job control | A stopped job resumes through `fg`, receives terminal input and finishes; subsequent background work is removed on PTY termination |
| Invalid bind source | Setup fails before the payload marker is created |

The terminal control compares plain PTY, bubblewrap with `--new-session`, and bubblewrap using
the existing HostPlatform session. All have a tty on stdin, but `--new-session` loses the
controlling terminal: `tcgetpgrp(0)` returns -1 and `/dev/tty` cannot be opened. Plain PTY and the
existing-session variant retain both. Therefore startup output and `stty size` alone are not
acceptance of interactive job control.

The measured lifetime/interactive construction omits `--new-session`. Existing native spawn
already calls `setsid` with null stdin and piped output; existing PTY spawn creates a new dedicated
terminal. This composition exposes no surrounding host shell's terminal to the payload.
Bubblewrap's parent-death handling and private PID namespace retain whole-tree settlement,
including jobs that create their own process group. It does not add a terminal proxy or process
supervisor. This justification is specific to these two HostPlatform entry points; it does not
make arbitrary wrapper invocation safe with an inherited host terminal. See bubblewrap's
[TIOCSTI warning](https://github.com/containers/bubblewrap/blob/main/README.md#limitations).
The measured kernel has `dev.tty.legacy_tiocsti=1`; the example does not change that setting.

These are native mechanism flows, not WorkspaceTools foreground/background execution, ShellInput
user commands or ShellSession integration with the proposed policy file. Environment selection,
policy edits and
actual consumer admission/result flows remain product acceptance work. No bubblewrap latency,
external-network routing, abstract UNIX-socket or other-kernel guarantee is measured. The machine
is deleted after collecting results; the checked-in example reproduces the flows.

## Linux bubblewrap normal exit and background compatibility

Measured on 2026-10-16 in a disposable isolated Debian 13.7 / Linux 7.2.9 ARM64 machine,
bubblewrap 0.12.0, Rust 1.98.1 and UID 501. Run the existing example with `--background-exit`.
The raw control and two bubblewrap constructions use actual `HostPlatform::spawn`, root `wait`
and handle drop. A background shell appends to a workspace heartbeat with stdout/stderr redirected;
the command root waits for a fixture release file, then exits normally. The collector records
only its descendants before releasing that root, observes later heartbeat growth, and invokes a
fresh command with `kill -0` on the original `$!`. That operation observes identity without
signalling another process. Cleanup terminates and verifies disappearance of the recorded PIDs.

| Construction | Root exit | Heartbeat grows after wait/drop | Later command addresses original `$!` |
| --- | --- | --- | --- |
| Ordinary spawn | 0 | Yes | Yes |
| Existing private namespaces with `--die-with-parent` | 0 | No | No |
| Same arguments except no `--die-with-parent` | 0 | Yes | No |

All root waits complete within five seconds. For the ordinary control, the printed child PID
equals its host PID. Both namespace cases print PID 3, with distinct host PIDs; a later fresh
namespace cannot address the surviving child by that printed PID. The namespace-only variant
is an ablation, not a proposed weakening of the selected boundary. The ordinary control asserts
continued execution and successful PID observation; the isolated cases report their actual result
rather than asserting compatibility that has not been achieved. Linux example Clippy with
`-D warnings` passes.

This establishes a concrete incompatibility with [ShellInput A.2](plugins/ShellInput.md#a2-execution):
the candidate arguments stop redirected background work on normal exit, and merely removing
parent-death handling does not preserve the documented later-command stop method. It does not
exercise actual ShellInput/WorkspaceTools policy loading, Session/App Server shutdown, another
kernel or the proposed product environment filtering. No product argument, process lifetime,
protocol, background registry or supervisor is changed. Resolve the authoritative compatibility
contract before implementation. The machine has no host share or SSH agent and is deleted after
collecting the results.

## Existing implementation and integration points

| Existing owner | Relevant behavior |
| --- | --- |
| `host_platform.rs` / `HostPlatform::spawn` | Exact child environment, cwd, argv and process-group ownership; an executable wrapper fits its existing input |
| `HostPlatform::spawn_pty` | Same spec enters the portable PTY implementation; the same wrapper starts successfully |
| `workspace_tools/exec.rs` | Foreground/background execution both use `HostPlatform::spawn` |
| `shell_input.rs` | User-started `shell.run` uses the shared command builder and spawn, with cancellable preparation before final run admission |
| `shell_session.rs` | Persistent shell startup uses `HostPlatform::spawn_pty` |
| `HostPlatform::build_process_spec` (S7) | Reads the current shell and constructs argv/environment; isolation need not duplicate shell parsing |

No event subscription is needed to launch or terminate this process: each caller owns its tree
and already owns cancellation. Tool permission and approval stay at their existing boundaries.
A local isolation policy belongs to HostPlatform, not Timeline history or a client's
request. This spike does not install that policy or change which calls execute with isolation.

## Platform evidence and remaining work

- **macOS:** the local `sandbox-exec(1)` manual marks the command deprecated; the installed Apple
  profiles describe their rules as a private interface. The mechanism worked on the measured
  host, so support on another macOS release requires running this probe there. No replacement
  API is assumed by this report.
- **Linux:** the Landlock fixture passed all seven flow groups as UID 1000 on the kernel above.
  [Landlock's kernel documentation](https://docs.kernel.org/userspace-api/landlock.html)
  defines the inherited restriction model and ABI-dependent coverage. The probe uses its
  existing syscall contract without a new crate. The bubblewrap feasibility section measures a
  separate candidate for the proposed product policy; it does not change the Landlock fixture's
  rights or evidence.
- **Windows:** isolation is unavailable in this spike; both examples refuse to run. This fixture
  behavior is separate from the proposed product contract's explicit Windows exception, which
  retains ordinary execution while Windows isolation is unsupported.

The proposed [product policy](host-platform-isolation.md) requires isolation by default on macOS
and Linux, with host-local filesystem/environment grants and an offline boundary. Missing policy
or unavailable isolation refuses execution on those platforms; unsupported Windows retains
ordinary execution without loading the policy. It requires design review before
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
the experiment. The bubblewrap fixture reuses one bounded PTY reader for its readiness, resize
and foreground-job observations; removing repeated read tasks preserves all seven native flow
groups. Process ancestry collection is necessary because PID-namespace numbers are not host PIDs;
termination must verify that the actual monitored descendants disappear. No additional production
abstraction is needed by these measured flows.
