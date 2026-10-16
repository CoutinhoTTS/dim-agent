# HostPlatform process isolation

Status: proposed product contract for [F5 #76](https://github.com/arcships/dim/issues/76).
Not implemented. Requirements come from [overview Appendix B.1](overview.md#b1-session-and-branch-capabilities),
the [feature list](../feature-list.md), and #76's maintainer decision: configurable isolation
for model-initiated execution. The [spike report](host-platform-isolation-spike.md) supplies
mechanism evidence, not product requirements. Implementation waits for design review.

## Scope and policy

Isolation covers WorkspaceTools exec, its BackgroundExec handoff, and persistent ShellSession
creation. User-typed `!` / `!!` commands belong to ShellInput and use ordinary spawn; they are
outside F5 ([ADR 003 D1](../adr/003-approval-scope.md)). App Server, model connections, MCP servers
and trusted plugins are outside this process boundary. MCP isolation remains a separate
[open question](overview.md#9-open-questions).

On macOS/Linux isolation is enabled by default and can be configured or disabled locally.
Unsupported Windows preserves ordinary execution. When enabled, missing backend support or
failed setup refuses the payload; it never silently retries without isolation.

| Resource | Default and configuration |
| --- | --- |
| Files | Platform system executable/library/configuration directories are read-only; the Session workspace is writable. Explicit read/write lists replace those defaults and may be empty. |
| Workspace | The immutable Session `cwd`, supplied by the consumer. A model's per-command cwd never expands the grant. |
| Network | Disabled by default; `network: true` enables host/external networking. No per-command exception or approval matrix. |
| Environment | An explicit name list selects values already in ProcessSpec. Defaults are PATH, HOME, LANG, LC_ALL, LC_CTYPE, TERM, NO_COLOR, PAGER, GIT_PAGER and GH_PAGER. No credential variables are synthesized. |
| Runtime | Backend-required devices and the caller's existing stdio/PTY; Linux has its private /proc and namespace skeleton. No implicit host /tmp or home mount. |
| Running processes | Policy is a startup snapshot. Edits affect the next process; running exec and persistent shells retain their original boundary. |

A missing policy file uses these defaults; it does not prevent all commands from starting.
The platform's default system roots are calibrated as part of its supported baseline: macOS
uses /System, /usr, /bin, /sbin, /Library/Apple and /private/etc; Linux uses its installed system
executable, loader/library and /etc roots, including the needed absolute-path aliases. Optional
architecture-specific system directories are included only where installed. These are platform
runtime grants, not an implicit read grant to the user's home. Additional toolchains, caches,
temporary directories or downloads may require local configuration. The default must pass a
real system-shell/workspace read/write flow on each supported baseline.

This isolates process file/network access, not trusted plugins, all tools, CPU, memory or output
size. A granted directory may contain credentials, executable code, hard links or service sockets;
that data and delegated authority are exposed. A workspace containing the application home also
contains host configuration: a writable workspace grant does not promise to protect files inside
it. Do not claim safety for credentials or UNIX sockets inside an allowed tree. With isolation
disabled, commands have the existing host user's ordinary access; disabling is an explicit local
policy choice. These limits belong in user configuration documentation.

## Configuration owner and format

HostPlatform owns `host-isolation.json` under the execution node's application home, through
its declared FileStorage dependency. It is not project, client or Session configuration; remote
execution reads the remote node's file. HostPlatformConfig supplies an optional file location,
as with exec-shell.json. The location is structural; the file's policy is business data.

The file is a new, versioned format. Version 1 accepts `enabled`, `network`, `readOnlyPaths`,
`writablePaths` and `environment`. Each omitted policy field uses the default in the table;
an explicitly empty list grants none of that resource. Only `version` is required when a file
exists. Examples:

```json
{"version": 1, "enabled": false}
```

An explicit Linux policy can keep isolation enabled, allow downloads and name a writable workspace:

```json
{
  "version": 1,
  "enabled": true,
  "network": true,
  "readOnlyPaths": ["/usr", "/bin", "/sbin", "/lib", "/etc"],
  "writablePaths": ["/workspace/project"],
  "environment": ["PATH", "HOME", "LANG", "TERM", "NO_COLOR"]
}
```

The path list is node-specific; it is not a portable Linux runtime promise. Preserve executable
aliases at their requested absolute destinations while resolving/validating their host roots
once at this external boundary. Unknown fields, missing/unknown versions, malformed/unreadable
files, non-absolute or nonexistent/non-directory roots and invalid environment names are errors.
Names are nonempty and contain neither NUL nor '='. A resolved root cannot be in both lists.
Writable roots also permit reads; grants are additive. Naming an environment value grants no
filesystem access to the path it contains. No file is written merely because defaults are used.

Read at each model process start; no cache, watcher, Session subscription, loader reload or
restart of an existing shell. No configured location, or a missing file at that location, uses
the defined default. An existing invalid file fails explicitly. `enabled: false` returns the
ordinary ProcessSpec without requiring a sandbox backend. Windows skips this unsupported policy
and retains ordinary execution until it has an accepted backend contract.

## Existing call flow

1. WorkspaceTools builds its existing ProcessSpec. ShellSession selects its existing persistent
   shell. Their owning Session configuration supplies the workspace separately from the model's
   requested working directory. ShellInput never enters this preparation.
2. A crate-local HostPlatform operation prepares that spec once before spawn:
   `prepare_isolated_process(spec, workspace: &Path).await -> Result<ProcessSpec, ProcessSpecError>`.
   It reads policy, selects the existing environment names and constructs backend argv/profile.
   Explicitly disabled policy and unsupported Windows return the original spec. No public service,
   protocol or ProcessSpec field is added.
3. The consumer calls existing synchronous spawn or spawn_pty. Backend setup precedes payload
   execution; a failed setup runs no payload. Arguments and paths stay separate argv values.
4. Existing process/PTY trees, pumps, cancellation and terminal facts remain with their current
   owners. The background handoff moves the already-running ExecProcess; it does not start another
   namespace, change policy or require another supervisor.

Raw HostPlatform spawn/PTY remain available for ShellInput and trusted host uses. Product isolation
is promised only for the listed model consumers. Shell/runtime discovery stays outside the command
sandbox. The accepted ShellInput Q4 transparency assumption is immaterial to this scope: its user
commands do not use F5, and their normal-exit/disconnect behavior is unchanged.

## Platform bindings

### macOS

Use the measured `/usr/bin/sandbox-exec` argv entry point with a HostPlatform-owned generated
Seatbelt profile. Start from deny-default, allow reads only for the configured directories and
required runtime devices, writes only for writable directories/devices, and deny networking when
network is disabled. Explicitly enabled network permits the normal network operations.
Use profile parameters for host paths; never embed an unescaped path into policy source. The
profile is internal implementation data, not a second user-configurable policy language.

The original spike's broad `file-read*` allowance is not an implementation of this contract.
The current narrow-read fixture passes on macOS 27.0.1 with a literal `/` read in addition to
its named read roots. Controlled probes fail before payload output without that literal; the
specific failing syscall has not been identified. Keep the literal directory grant, not a
recursive root read. Outside reads/writes/chmod, symlink escapes and host IPv4/IPv6 TCP/UDP
are rejected; process-tree and PTY lifetime checks pass. Product configuration, environment
selection and the actual exec/PTY consumer flows remain to be implemented and verified.
Do not compensate for another baseline's failure with broader reads, networking or an
unisolated fallback. Report an incompatible platform explicitly.

The local `sandbox-exec(1)` manual marks the interface deprecated. Support is limited to actual
OS versions that pass the isolation acceptance flows; macOS 27.0.1 fixture results are
evidence for their exercised profiles only.


### Linux

Use bubblewrap with an empty mount namespace and configured read-only/writable binds at their
original destinations, private PID/IPC namespaces, a private /proc and minimal /dev. Retain the
unprivileged user, use a private user namespace and drop payload capabilities. Resolve installed
bwrap through the captured host PATH; do not download or elevate a backend. With network disabled,
unshare the network namespace; with network enabled, retain the host network. Backend failure
while enabled refuses execution, rather than selecting a weaker or unisolated path.

Retain the existing ProcessTree/PtyProcessTree and bubblewrap parent-death handling. Native spawn
already establishes a session and piped output; PTY spawn owns its terminal. Do not add
`--new-session` to the PTY path: measured input/resize still worked but controlling-terminal/job
control did not. No terminal proxy, custom seccomp system or process supervisor is added.
See the upstream [policy limits](https://github.com/containers/bubblewrap/blob/main/README.md) and
[options](https://github.com/containers/bubblewrap/blob/main/bwrap.xml). The existing
[native-handle calibration](host-platform-isolation-spike.md#linux-bubblewrap-process-and-terminal-calibration)
checks tree settlement, PTY/job control and live IPv4/IPv6 TCP/UDP and UNIX-socket boundaries on
Debian 13.7 / Linux 7.2.9 ARM64, UID 501, bubblewrap 0.12.0. It does not verify product policy.

Landlock is not selected for this boundary: the measured ABI 4 wrapper does not cover UDP/UNIX
sockets or all read-only metadata operations. See its [kernel contract](https://docs.kernel.org/userspace-api/landlock.html).
Those measured limits are not claims about every newer ABI.

### Background lifetime

The existing [BackgroundExec](../architecture/rust-agent/plugins/BackgroundExec.md) owns model
background tasks. `exec::hand_off` passes the same live ExecProcess to `BackgroundExec::start`;
its existing task retains the tree until completion. Returning a ToolResult or stopping a turn
therefore does not drop that process. There is no cross-command namespace PID handoff.

| Transition | Existing owner contract |
| --- | --- |
| Foreground becomes background | Move the live process/tree and its startup policy; retain the same namespace. |
| Turn stops | BackgroundExec keeps the task running and retains its completion/wake ownership. |
| Explicit task stop or branch/Session unload | Existing task cancellation terminates/releases the owned tree before terminal settlement. |
| Command completes normally | Record exec/ended once and publish the existing bounded wake; no orphan process is reclassified as a new task. |
| App Server dies | Backend parent-death handling terminates isolated descendants; no work is recovered from history. |

Shell backgrounding inside a command does not create a BackgroundExec task: normal root exit
completes that command, with the backend's namespace cleanup. Long-lived model work uses the
existing runInBackground handoff and keeps its command root alive.
Task control uses the existing taskId and host-owned tree, not `kill $!` from a fresh namespace.
ShellSession likewise retains its original PTY/tree for the shell's lifetime. Product acceptance
must verify transfer, later task stop and Session shutdown with the actual isolated model paths.

The separate [normal-exit calibration](host-platform-isolation-spike.md#linux-bubblewrap-normal-exit-and-background-compatibility)
shows redirected `!cmd &` descendants do not satisfy ShellInput's outliving/PID contract under the
candidate private-PID/parent-death arguments. That is valid evidence for excluding user ShellInput
from isolation; it is not proof that a held BackgroundExec tree dies when its tool call returns.
No PID translation, detached-namespace registry or new stop protocol is proposed.

### Windows

Windows has no supported isolation backend. It keeps the original exec/PTY, environment and tree
ownership and does not load this file. Do not present that path as isolated or simulate a sandbox
with path checks or Permission rules.

## Failure and persistence

Policy parsing/setup failure runs no payload. Enabled isolation has no implicit fallback; explicit
disablement is ordinary execution. Approval never widens filesystem/network policy. Existing
consumers report their existing typed error/result; no new approval category or Timeline fact.
The policy is a startup snapshot. Concurrent path/policy edits add no atomicity guarantee, grant
lease or filesystem virtualization. Restore/fork uses current local policy for each new process.

## Impact and acceptance

| Location | Necessary change |
| --- | --- |
| HostPlatform | Local policy parsing/defaults and backend transformation; reuse FileStorage, ProcessSpec and error types. |
| WorkspaceTools / ShellSession | Pass immutable Session workspace to preparation before existing spawn/PTY; preserve admission and tree ownership. |
| BackgroundExec | Preserve its existing live-process handoff, taskId control and terminal facts; no new process service. |
| ShellInput | None; user commands continue ordinary spawn. |
| Product assembly | Supply optional local file location through existing HostPlatformConfig/conditional FileStorage; default policy works without a file. |
| Owner docs and flows | Supported baselines, real model exec/background/PTY acceptance and explicit disabled/Windows paths. |

Verify actual model foreground/background exec and persistent shells on macOS/Linux: default
system-shell plus workspace read/write without a file, configured stricter/wider roots, symlink
escape, outside file/metadata denial, TCP/UDP denial with network disabled and success against live
listeners when enabled, ungranted UNIX sockets, PTY input/resize/job control, tree stop/unload and
background task survival across a turn stop. Verify later taskId stop, one terminal/wake, Session
shutdown and no process replay. Policy edits affect only new processes; malformed existing files
refuse payload startup; explicit disablement executes normally without requiring a backend.
A model-supplied cwd outside the workspace must not enlarge default writes. Reuse ShellInput's
existing ordinary `!`/`!!` flows as the exclusion control. Windows uses ordinary exec/PTY.

Use disposable homes, roots and local listeners. Mechanism probes do not establish defaults,
product policy loading or the real consumers. F5 remains open until actual isolated model exec
and PTY acceptance pass on both supported platforms. CLI startup timing is a separate fix, [#158](https://github.com/arcships/dim/pull/158).

## Rule check

| Rule | Application |
| --- | --- |
| §1 | Extend the existing process owner; no new capability, registry, supervisor or lock. |
| §4 rule 1 | Consumer admission/results retain their existing owners. |
| §4 rule 2 | Background transfer retains the live tree and its startup boundary. |
| §4 rule 3 | No before callback or commit-time I/O. |
| §4 rule 4 | Core and assembly contain no isolation business branch. |
| §4 rule 5 | No new batch or dispatch semantics. |
| §4 rule 6 | Only existing tool results and adopted inputs enter model context. |
| §4 rule 7 | Public HostPlatform/tree interfaces and declared FileStorage only. |
| §4 rule 8 | Existing mechanisms suffice; platform limits stay in F5. |
| §4 rule 9 | Host policy and diagnostics are not Timeline facts. |
| ADR 002 D1 | Host owns local policy; consumers own their executing process lifetime. |
| ADR 002 D2 | No wiring or cross-scope policy service. |
| ADR 002 D3 | No optional isolation-provider dependency. |
| ADR 002 D4 | Read business policy per process; loader retains only its structural location. |
| ADR 002 D5 | Existing data/error/process contracts; no new language-neutral service. |

## Requirements raised to mechanism owners

None. Defaults, configuration and backend acceptance belong to HostPlatform. No new readiness,
subscription, protocol, supervisor or rutis feature is required.

## Ablation

One local file and its explicit defaults replace a policy service or watcher. One preparation
operation reuses existing spawn/PTY and native errors. BackgroundExec already holds the process;
no namespace/PID manager is needed. User ShellInput stays outside this boundary. Network is one
local setting rather than per-command exceptions. No isolation product code is introduced by
this design/probe PR.
