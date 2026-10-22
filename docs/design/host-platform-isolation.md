# HostPlatform process isolation

Status: implemented for [F5 #76](https://github.com/arcships/dim/issues/76), delivered
by merged [#189](https://github.com/arcships/dim/pull/189). The macOS developer-tool baseline
and the per-process temporary directory are calibrated for [#309](https://github.com/arcships/dim/issues/309).
Requirements come from [overview Appendix B.1](overview.md#b1-session-and-branch-capabilities),
the [feature list](../feature-list.md), and #76's maintainer decision: configurable isolation
for model-initiated execution. The [spike report](host-platform-isolation-spike.md) supplies
mechanism evidence, not product requirements.

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
| Files | Platform system executable/library/configuration directories are read-only; the Session workspace is writable. Persistent shells also have their host-owned current branch output directory writable by default. On macOS each isolated process tree also has its own host-owned [temporary directory](#temporary-directory) writable by default. Explicit read/write lists replace those defaults and may be empty. |
| Workspace | The immutable Session `cwd`, supplied by the consumer. A model's per-command cwd never expands the grant. |
| Network | Disabled by default; `network: true` enables host/external networking. No per-command exception or approval matrix. |
| Environment | An explicit name list selects values already in ProcessSpec. Defaults are PATH, HOME, LANG, LC_ALL, LC_CTYPE, TERM, NO_COLOR, PAGER, GIT_PAGER and GH_PAGER. When the default temporary directory applies, TMPDIR names it and replaces any selected value. No credential variables are synthesized. |
| Runtime | Backend-required devices and the caller's existing stdio/PTY; Linux has its private /proc and namespace skeleton. Host /tmp, the user's own TMPDIR and the home receive no implicit grant; macOS temporary files use the per-process directory above. |
| Running processes | Policy is a startup snapshot. Edits affect the next process; running exec and persistent shells retain their original boundary. |

A missing policy file uses these defaults; it does not prevent all commands from starting.
The platform's default system roots are calibrated as part of its supported baseline: macOS
uses /System, /usr, /bin, /sbin, /Library/Apple and /private/etc (including its /etc alias),
plus /var/select (the root-owned selection links that /bin/sh and the /usr/bin developer-tool
shims read, including its /private/var/select source) and the developer directory that its
`developer_dir` link selects; Linux uses its installed system
executable, loader/library and /etc roots, including the needed absolute-path aliases. Optional
architecture-specific system directories, and the selected developer directory, are included only
where installed. These are platform
runtime grants, not an implicit read grant to the user's home. Additional toolchains (Homebrew,
rustup, nvm and similar), caches or downloads may require local configuration. The default must pass a
real system-shell/workspace read/write flow on each supported baseline; on macOS that flow
includes the selected developer tools (git, python3) and a temporary file.

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
Names are nonempty and contain neither NUL nor '='. A declared or resolved read-only root cannot equal or
be inside a writable root in the same path space: neither backend carves read-only exceptions out of a
writable parent. A writable child inside a read-only root is supported.
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
   `prepare_isolated_process(spec, workspace: &Path, output_dir: Option<&Path>).await -> Result<(ProcessSpec, Option<TemporaryDirectory>), ProcessSpecError>`.
   Only ShellSession supplies its immutable, host-owned current branch output directory.
   It reads policy, selects the existing environment names and constructs backend argv/profile.
   The second value is the process's [temporary directory](#temporary-directory), when one applies.
   Explicitly disabled policy and unsupported Windows return the original spec and none. No public
   service, protocol or ProcessSpec field is added.
3. The consumer calls existing synchronous spawn or spawn_pty and attaches the temporary directory
   to the returned tree. Backend setup precedes payload
   execution; a failed setup runs no payload. Arguments and paths stay separate argv values.
4. Existing process/PTY trees, pumps, cancellation and terminal facts remain with their current
   owners. The background handoff moves the already-running ExecProcess; it does not start another
   namespace, change policy or require another supervisor.

Raw HostPlatform spawn/PTY remain available for ShellInput and trusted host uses. Product isolation
is promised only for the listed model consumers. Shell/runtime discovery stays outside the command
sandbox. ShellInput Q4 keeps its user commands outside F5; their normal-exit/disconnect behavior is
unchanged.

## Platform bindings

### macOS

Use the measured `/usr/bin/sandbox-exec` argv entry point with a HostPlatform-owned generated
Seatbelt profile. Start from deny-default, allow reads only for the configured directories and
required runtime devices, writes only for writable directories/devices, and deny networking when
network is disabled. Explicitly enabled network permits IP operations and the system DNS
resolver socket at `/private/var/run/mDNSResponder`, with metadata access to its `/var` alias.
Other UNIX-socket connections remain limited to the granted file trees, with local socket binding
limited to writable trees.
Use profile parameters for host paths; never embed an unescaped path into policy source. The
profile is internal implementation data, not a second user-configurable policy language.
Each granted root, at its canonical source and its requested destination, also permits metadata
lookups on its path ancestors, as Apple's system profile does for firmlink components. This allows
`stat` of those directories, not listing or reading them. `realpath(3)` examines every path
component: without it, Python and Clang in the developer directory cannot locate themselves, and
the same holds for executables in configured roots.
Inherited PTYs permit only terminal-mode, geometry and foreground-group ioctl commands on tty
devices. This permits terminal detection, resize and job control without granting device opens
or terminal-input injection.

The original spike's broad `file-read*` allowance is not an implementation of this contract.
The current narrow-read fixture passes on macOS 27.0.1 with a literal `/` read in addition to
its named read roots. Controlled probes fail before payload output without that literal; the
specific failing syscall has not been identified. Keep the literal directory grant, not a
recursive root read. Outside reads/writes/chmod, symlink escapes and host IPv4/IPv6 TCP/UDP
are rejected; process-tree and PTY lifetime checks pass. Actual model exec and configured
persistent-shell flows cover local policy, environment selection, terminal resize, task stop and
Session close. Strict-default persistent background execution still awaits the output decision
below. Network-enabled native socket flows verify IP traffic and the UNIX-socket path boundary
against live listeners, with explicit file grants as the positive UNIX-socket control.
Do not compensate for another baseline's failure with broader reads, networking or an
unisolated fallback. Report an incompatible platform explicitly.

The /usr/bin developer shims (`git`, `python3`, `clang`, `make`) read `/var/select/developer_dir`
through the /var alias and then run the tools in the selected directory. The selected directory
is resolved from that link by the ordinary root resolution at each policy load, so the grant follows
`xcode-select` rather than a fixed install location. With the Command Line Tools selected,
macOS 27.0.1 needs neither `/private/var/db/xcode_select_link` nor `/Library/Preferences`. An
Xcode.app selection resolves to its `Contents/Developer` directory; it has not been run on this
baseline, and if Xcode's license check reads other files, those need a local grant. Without
TMPDIR, the shims fall back to the ungranted /tmp for their lookup cache and print errors on every
call. Git exits when the user's global configuration (`~/.gitconfig` or `~/.config/git/config`)
exists but cannot be read. The default policy grants no home reads, so such a file needs a local
grant of the directory that contains it.

The local `sandbox-exec(1)` manual marks the interface deprecated. Support is limited to actual
OS versions that pass the isolation acceptance flows; macOS 27.0.1 fixture results are
evidence for their exercised profiles only.


### Linux

Use bubblewrap with an empty mount namespace and configured read-only/writable binds at their
original destinations, private PID/IPC namespaces, a private /proc and minimal /dev. Retain the
unprivileged user, use a private user namespace and drop payload capabilities. Resolve installed
bwrap through the captured host PATH; the host must trust that PATH and the selected executable,
including its writable ancestors. No executable provenance or integrity check is promised.
Do not download or elevate a backend. With network disabled,
unshare the network namespace; with network enabled, retain the host network. Backend failure
while enabled refuses execution, rather than selecting a weaker or unisolated path.

`network: true` does not grant resolver files or service sockets outside the configured roots.
The default mounts exclude `/run`; Linux resolver setups whose `/etc/resolv.conf` points into
`/run/systemd/resolve`, or whose NSS module requires a socket there, are not supported by the
default policy. The exercised Debian baseline uses a plain resolver file; systemd-resolved
hostname resolution has not been verified. No resolver mount or broader default read grant is added.

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

ShellSession reads a background job PID once at its existing PTY marker boundary. HostPlatform
uses the owned PTY's existing foreground-group handle to identify the shell's Linux PID namespace
and OS session, then resolves that runtime identity through the kernel's `/proc` fields. The
private task stores a typed host PID or its actual I/O error; stop and shell-exit consumers do not
parse strings or resolve identities again. The existing taskId and persisted records are unchanged.
The native stop operation waits for the group to release, escalating after the existing bounded
grace, before ShellSession records a cancelled terminal fact. The public synchronous signal helper
retains its signature. No PID mapping table, shared lock, namespace registry or supervisor is held.

The separate [normal-exit calibration](host-platform-isolation-spike.md#linux-bubblewrap-normal-exit-and-background-compatibility)
shows redirected `!cmd &` descendants do not satisfy ShellInput's outliving/PID contract under the
candidate private-PID/parent-death arguments. That is valid evidence for excluding user ShellInput
from isolation; it is not proof that a held BackgroundExec tree dies when its tool call returns.
There is no cross-command namespace/PID handoff, detached-namespace registry or new stop protocol.

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

## Persistent shell output

Persistent ShellSession adds only its host-owned current branch output directory,
`ShellSessionConfig.output_dir` (normally `HostPlatform.home/.dim/exec/<session>/<branch>`),
to the default writable roots alongside the Session workspace. The shell owner creates this
directory before preparation; the model's cwd and command cannot select or expand the grant.
An explicit `writablePaths` list fully replaces both defaults. In particular, `[]` grants no
directory writes, and a workspace-only list does not authorize the shell output directory.

Existing output paths have different owners. WorkspaceTools' `ExecProcess` pumps child output
into its host-owned output sink; an occupying ShellSession command appends its captured PTY
text from `OccupyingTask::commit`. Explicit POSIX background jobs instead open their log inside
`posix_background`; the shared PTY carries the PID/completion markers, and the host reads the
log for previews. The current PTY capture has no per-background-job output framing. Reusing it
for concurrent jobs would require changing ShellSession's output transport and attribution;
the existing host pump alone does not remove the background wrapper's file open.

Existing stdio/PTY remain runtime channels rather than directory grants. Persistent background
redirection fails when local policy omits write access to its output directory; the payload must
not run after that failure. Ordinary WorkspaceTools receives no output-directory grant because
its host captures output. No Core API, protocol, ProcessSpec field, lifecycle phase or supervisor
is added. The startup snapshot applies to these default roots as to explicit policy. A sibling
branch's output directory and the rest of the home receive no implicit access. Unreadable login
startup files outside the grants can produce shell diagnostics; preparation does not silently
read the user's home or discard that output.

## Temporary directory

On macOS, when isolation is enabled and `writablePaths` is omitted, preparation creates one empty
directory for the process tree, adds it to the default writable roots and sets TMPDIR to it. It is
created with mode 0700 and a fresh `dim-isolated-<uuid>` name under the App Server's system
temporary directory (normally the per-user `/var/folders/…/T`). The parent directory, its other
entries and /tmp stay ungranted. Each WorkspaceTools exec process receives its own directory;
a persistent ShellSession shell receives one for its lifetime.

The process tree handle owns the directory. The consumer attaches it to the ProcessTree or
PtyProcessTree it spawns, and dropping that handle removes the directory on a blocking worker.
A background handoff moves the handle, so BackgroundExec keeps the directory until it settles the
task. A failed spawn removes it at once. Descendants that outlive their reaped command root lose
the directory. If the App Server dies, the directory stays in the system temporary area for the
OS's ordinary cleanup; there is no startup sweep, registry or recovery.

The directory is a default writable root, like the Session workspace and the shell output
directory: an explicit `writablePaths` list replaces it. Then no directory is created, TMPDIR is not
set, and a TMPDIR named in `environment` passes through unchanged and needs its own grant. Disabled
policy and Windows keep ordinary execution. Failure to create the directory refuses the payload
with `isolation_unavailable`.

Linux is unchanged in this revision. Its namespace has no /tmp and receives no TMPDIR; binding the
same directory there needs a Linux acceptance run first.

## Impact and acceptance

| Location | Necessary change |
| --- | --- |
| HostPlatform | Local policy parsing/defaults and backend transformation, including the macOS developer-tool roots, ancestor metadata and the per-process temporary directory owned by the tree handle; reuse FileStorage, ProcessSpec and error types. |
| WorkspaceTools / ShellSession | Pass immutable Session workspace to preparation before existing spawn/PTY and attach the returned temporary directory to the spawned tree; ShellSession also supplies its owned output directory for default writes. Preserve admission and tree ownership. |
| BackgroundExec | Preserve its existing live-process handoff, taskId control and terminal facts; no new process service. |
| ShellInput | None; user commands continue ordinary spawn. |
| Product assembly | Supply optional local file location through existing HostPlatformConfig/conditional FileStorage; default policy works without a file. |
| Owner docs and flows | Supported baselines, real model exec/background/PTY acceptance and explicit disabled/Windows paths. |

Verify actual model foreground/background exec and persistent shells on macOS/Linux: default
system-shell plus workspace read/write without a file (on macOS also git, python3 and a TMPDIR
file, with /tmp writes and home reads still denied), configured stricter/wider roots, symlink
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
local setting rather than per-command exceptions. The policy is decoded once at its local-file
boundary; configured roots retain their canonical source and requested absolute destination.
Explicit policy fully replaces default roots; no separate runtime-directory exception is needed.
The developer directory comes from the existing link resolution, not a probe or fixed install path.
The temporary directory lives and dies with the tree handle that already owns the process; no
sweeper, registry or policy field is added.
