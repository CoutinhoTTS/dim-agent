# HostPlatform process isolation

Status: proposed product contract for [F5 #76](https://github.com/arcships/dim/issues/76).
Not implemented. The [spike report](host-platform-isolation-spike.md) records the measured
mechanisms; it does not establish the policy below. Architecture review precedes implementation.

## Scope and policy

HostPlatform requires isolation by default on macOS and Linux, applying an execution-node-local
policy before a command starts. Windows currently has no supported isolation backend and retains
ordinary execution as an explicit platform exception. There is no isolation enable/disable setting.
The initial consumers are WorkspaceTools exec, including background execution, and ShellSession
PTY creation. Descendants inherit the boundary. The App Server, model connections, MCP servers
and trusted in-process/TypeScript plugins are outside this change. MCP isolation remains the
separate open question in [overview §9](overview.md#9-open-questions).

The isolation policy is:

| Resource | Contract |
| --- | --- |
| File contents | Only configured read-only and writable directories are readable. Writable directories also permit ordinary file creation, modification, removal and execution, subject to the host user's existing permissions. No implicit grant from an executable, cwd, Session, permission preset or FileStorage root. |
| Other host paths | No file-content read or write grant. Path metadata may be visible on macOS; directory names and existence are not confidential under this policy. |
| Network | No access to host or external IPv4/IPv6 networks, including host loopback. Linux may permit communication within the new network namespace. No proxy, per-command exception, network approval or automatic retry without isolation. |
| Environment | Retain only explicitly named entries from the existing ProcessSpec environment. Naming a variable grants its value, not access to a path it names. HostPlatform does not synthesize credential variables. |
| Runtime files/devices | Only the root directory itself, the Linux private namespace's `/proc`, backend-required null/random/terminal devices and the caller's existing stdio/PTY. A literal root-directory read does not grant descendant contents. |
| Existing processes | A policy edit affects the next process start. An existing exec tree or persistent shell keeps its startup policy until its owner ends it. |

This is a process filesystem/network boundary, not a sandbox for trusted plugins or a limit on
CPU, memory, output size or the semantics of authorized data. A permitted directory can contain
credentials, executable code, hard links or service sockets. Granting that directory exposes those
objects: in particular, a mounted UNIX socket can delegate host authority. Do not automatically
mount home, `/run`, host `/tmp`, SSH agents or Docker sockets. No claim is made that aliases of
an authorized writable inode elsewhere on the host remain unchanged. These limits must appear
in the configuration documentation, rather than being hidden behind the word "sandbox".

## Configuration owner and format

HostPlatform owns `host-isolation.json` under the execution node's configured application home,
read through its declared FileStorage dependency. It is not read from the project, client or
Session. Remote execution uses the remote host's own file. HostPlatformConfig gains an optional
`isolation_file` location: FileStorage root index and relative path, as with `exec-shell.json`.
The location is structural loader configuration; file contents are not loader configuration.

The new file has an explicit format version. For example, a macOS host could authorize:

```json
{
  "version": 1,
  "readOnlyPaths": ["/System", "/usr", "/bin", "/sbin", "/Library/Apple", "/private/etc"],
  "writablePaths": ["/Users/example/project", "/Users/example/dim-command-tmp"],
  "environment": ["PATH", "HOME", "LANG", "TERM", "NO_COLOR", "TMPDIR"]
}
```

The paths are host administrator choices, not a portable preset or a validated macOS profile.
The example directory and TMPDIR value must already exist and agree. No path is created or
added to the grants automatically. A Linux host lists its own executable/library/configuration
directories and toolchain caches. A command that needs an ungranted cache or dependency download
fails within the boundary; Permission approval does not widen it.

All four fields are required. Unknown fields, missing/unknown versions,
malformed data, unreadable files, non-absolute paths, nonexistent/non-directory roots and invalid
environment names are configuration errors. Names must be nonempty and contain neither NUL nor `=`.
Roots are canonicalized at this external boundary;
the same resolved root cannot occur in both lists. Writable roots must not overlap the configured
application home, which holds the policy and other host authority; a command cannot edit its own
execution policy or enabled plugin configuration. A writable descendant inside a read-only
directory is an explicit write grant; grants are additive, so a read-only descendant does not
subtract a writable ancestor's authority. Each environment name selects an exact existing key;
an absent key adds nothing. The lists may be empty.

On macOS and Linux, a missing file or a host profile with no configured policy location is a
configuration error: exec and PTY startup refuse before starting the payload. Reading does not
create a default file or grant the cwd, project or host home. The host administrator supplies
the file and its explicit grants before commands can run. Windows skips policy loading and
preserves ordinary execution until a supported Windows isolation contract exists. Existing
files and running processes are not migrated or modified.

On macOS and Linux, read and validate once for each new process, not once per Session or loader
generation. Do not cache policy, install a watcher, subscribe to Session events, reload plugins or restart running
shells when the file changes. Read at use time satisfies ADR 002 D4 without new machinery.

## Existing call flow

1. The existing consumer performs its tool or shell-input admission and constructs ProcessSpec.
   WorkspaceTools continues to use `build_process_spec` for the current shell and its command
   checks. ShellSession retains its existing persistent-shell selection.
2. Both consumers pass that spec through one HostPlatform-owned asynchronous preparation
   operation immediately before starting it. On macOS and Linux, HostPlatform reads its required
   policy, filters the existing environment and constructs the selected backend's executable/argv/cwd.
   On Windows, it returns the original spec unchanged without reading an isolation file. The
   original command and every path remain separate argv values, never interpolated into a shell command.
3. The prepared spec goes to the existing synchronous `spawn` or `spawn_pty`. Backend setup
   finishes before exec of the payload. A failed setup must not run the payload.
4. The consumer retains the existing ProcessTree/PtyProcessTree, output pumps, cancellation,
   background handoff and terminal-state responsibilities. No isolation task supervisor or
   independent process-lifetime state is added.

The preparation is an actual ProcessSpec transformation at the process-execution boundary,
not another service, registry or authorization port. Raw spawn/PTY primitives remain available
for trusted host probes and embeddings; they do not promise isolation. Internal shell/runtime
discovery stays outside the command sandbox. Consequently, this contract is guaranteed for the
listed product consumers, not for arbitrary trusted code that starts a process by another API.
Other features integrate at their own documented execution boundary, without special caller tags.

Crate-local operation: `HostPlatform::prepare_isolated_process(spec).await ->
Result<ProcessSpec, ProcessSpecError>`, visible only within dim-agent-capabilities. Only the Windows
exception returns the original spec unchanged. Both consumers already live in that crate, so
the operation does not need to be a new public service API.
No public wire method, event or language-neutral service is introduced. ProcessSpec and the
existing `{code, reason}` error data retain their single definitions.

## Platform bindings

### macOS

Use the measured `/usr/bin/sandbox-exec` argv entry point with a HostPlatform-owned generated
Seatbelt profile. Start from deny-default, allow reads only for the configured directories and
required runtime devices, writes only for writable directories/devices, and deny networking.
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

Select bubblewrap for the product policy: an empty mount namespace with configured read-only
and writable binds at their original absolute paths, a read-only namespace skeleton, a private network namespace, private PID
and IPC namespaces, a private `/proc` and minimal `/dev`. Retain the unprivileged calling user;
do not request elevated execution, capabilities, setuid mode or a host network namespace.
Use a private user namespace and drop all capabilities for the payload. Resolve the installed
`bwrap` from the host's captured PATH; no runtime download or installer.
Use its parent-death handling while retaining the existing tree handle's ability to terminate
and reap the whole command. Terminal separation belongs to the existing HostPlatform entry
points: native spawn already creates a new session with null stdin/piped output, and PTY spawn
creates a dedicated terminal. Do not add bubblewrap `--new-session` to that PTY: measured controls
show it loses the controlling terminal even though input and resizing still work. The existing
session/PTY construction preserves interactive job control without exposing a surrounding host
shell's terminal. The crate-local preparation is used only through these two entry points;
arbitrary inherited-terminal invocation has no such guarantee. No terminal proxy, custom seccomp
subsystem or additional process supervisor is proposed.

Bubblewrap provides mount/network namespaces; its arguments determine the actual boundary.
Its upstream documentation identifies mounted service sockets and terminal injection as security
considerations. Both must be checked in the real spawn and PTY flows; merely seeing a child
PID is insufficient. See [upstream policy and limits](https://github.com/containers/bubblewrap/blob/main/README.md)
and [command options](https://github.com/containers/bubblewrap/blob/main/bwrap.xml).

The measured Landlock wrapper is not the selected backend. Its ABI 4 policy does not cover UDP
or UNIX sockets, and Landlock's documented filesystem limits include metadata operations such as
chmod/chown. These differ from the proposed read-only mount boundary. See the
[kernel contract](https://docs.kernel.org/userspace-api/landlock.html). Newer ABI rights do not
retroactively expand the spike's runtime evidence.

The [feasibility probe](host-platform-isolation-spike.md#linux-bubblewrap-feasibility) passes as
UID 501 on Debian 13.7 / Linux 7.2.9 ARM64 with bubblewrap 0.12.0: workspace I/O, read-only
bytes/mode, an inaccessible private fixture, symlink rejection, a private network namespace
and basic PTY startup. A subsequent [native-handle calibration](host-platform-isolation-spike.md#linux-bubblewrap-process-and-terminal-calibration)
also verifies actual ProcessTree/PtyProcessTree settlement, PTY input/resize, interactive bash
job control and live host IPv4/IPv6 TCP/UDP plus unmounted UNIX-socket boundaries on that baseline.
These fixtures do not establish product policy loading or exec/ShellSession integration.
Product integration must verify the same boundary through its actual consumers. A container or
kernel that refuses required namespaces is unsupported
for isolation; command execution refuses without switching to a weaker Landlock policy or
ordinary execution.

### Windows

Windows retains ordinary exec and PTY execution, with the original ProcessSpec environment and
existing process-tree ownership. HostPlatform does not load an isolation file on this platform.
This is the sole platform exception to mandatory isolation; it is not evidence of Windows
isolation support. No Windows sandbox substitute or backend is included in F5.

## Errors and lifetime

Preparation uses existing ProcessSpecError data:

| Code | Meaning |
| --- | --- |
| `HOST_ISOLATION_CONFIG_INVALID` | Required macOS/Linux policy location or file is missing, or the configuration cannot be read, parsed or validated. |
| `HOST_ISOLATION_UNAVAILABLE` | macOS/Linux isolation has no usable backend, including a missing backend executable. |

Runtime setup failure, including a namespace/profile refusal, is an existing spawn failure or
backend nonzero exit with its diagnostic, according to where it occurs. It is not a successful
command and never retries the payload without isolation. Tool errors use the existing tool-result
path; ShellSession reports startup failure through its existing response. No Rpc error code,
permission category or Timeline format is added. Diagnostic text must not echo environment values
or the policy file body.

HostPlatform remains publishable when isolation configuration is invalid or unavailable, as it
does for shell preferences; only affected execution fails. File tools and unrelated Sessions
remain usable. A model argument or approval response cannot disable isolation or select the
Windows exception on another platform.
The chosen file contents and grants are one startup snapshot. Concurrent host policy/path changes
do not add a new atomicity guarantee; no lock, grant lease or filesystem virtualization is added.
Restore/fork uses the current host policy for new processes and never restores authority from
history. Detach, close and process cancellation retain their current ownership and semantics.

## Impact and acceptance

| Location | Necessary change |
| --- | --- |
| HostPlatform | Own file location, boundary parsing, mandatory macOS/Linux preparation, Windows bypass and backend argv/profile construction. Reuse FileStorage and existing process/error types. |
| WorkspaceTools exec / ShellSession | Prepare their existing ProcessSpec before their existing spawn/PTY call; preserve admission, shell checks and tree ownership. |
| App Server assembly / xtask example assembly | Supply host-local file location through HostPlatformConfig, with the existing conditional FileStorage dependency. Regenerate architecture output only when implementation changes assembly. |
| HostPlatform and integration flow configurations | Update existing HostPlatformConfig literals for the optional field; embeddings using an exhaustive literal need the same source update. |
| Owner docs and flow tests | Describe the actual supported OS baseline, configuration and refusals; verify actual shell execution through public product flows. |

No core, loader protocol, SessionConfig, event fixture, persistence database, MCP policy or CLI
management operation changes. Direct host-local file editing is sufficient for the first scope.

Reuse the spike fixtures and existing local-execution/persistent-shell flows. On both supported
OS baselines, demonstrate allowed reads/writes and shell descendants; denied outside reads,
writes and metadata changes; an escaping symlink; TCP and UDP to live host listeners; a host
UNIX socket outside grants; real PTY execution and resizing; and whole-tree cancellation with
descendants reaped. Exercise foreground and background exec, live policy edits for the next
process, retention of running-shell policy, exact environment filtering, malformed/unavailable
policy without a payload marker, missing location/file refusal without implicit grants, and
refusal of writable roots overlapping application home. Use only disposable roots/home,
credentials-free environment and local listeners. Windows exercises ordinary exec and PTY with
the original environment, without requiring or reading the isolation file. Do not add a separate
test for every private parser/helper.

Do not claim F5 complete from the spike, design approval or one backend. Product acceptance
requires mandatory isolation through exec and PTY on macOS and Linux and the Windows exception,
with the supported baselines recorded.

## Rule check

| Rule | Application |
| --- | --- |
| §1 | Extend the existing process owner. No new capability, registry, supervisor or lock. |
| §4 rule 1 | No other owner's business state is changed; admission and result publication remain with the existing consumer. |
| §4 rule 2 | Existing handoff to exec/background ownership is unchanged; preparation precedes process start. |
| §4 rule 3 | No before callback or commit-time I/O. |
| §4 rule 4 | Timeline, Dispatcher and assembly have no isolation-specific behavior. |
| §4 rule 5 | No new batch or dispatch semantics. |
| §4 rule 6 | Only existing tool results/adopted inputs enter model context. |
| §4 rule 7 | Consumers use HostPlatform and existing tree handles; configuration uses declared FileStorage. |
| §4 rule 8 | Existing contracts suffice; backend/platform limitations stay in F5. |
| §4 rule 9 | Host safety settings and backend diagnostics are not new Timeline facts. |
| ADR 002 D1 | Process owner holds host policy location; each executing consumer owns its process lifetime. |
| ADR 002 D2 | No wiring plugin or cross-scope access host. |
| ADR 002 D3 | Existing consumers depend on HostPlatform; no dependency on an optional isolation provider. |
| ADR 002 D4 | Business policy is read at each new process; loader rows retain only its structural location. |
| ADR 002 D5 | No new cross-language service; existing data/error types and native process binding are reused. |

## Requirements raised to mechanism owners

None. No new readiness phase, subscription, protocol, generic policy service or rutis feature is
required. OS-baseline coverage and product process/PTY acceptance are HostPlatform
acceptance work, not requirements on core.

## Ablation review

Retain one owner file, one preparation operation and the existing spawn/tree APIs. Separate
read/write roots express the actual filesystem grant; environment names prevent implicit host
credential inheritance. A fixed offline policy avoids a proxy, approval matrix and network
exception state. Platform selection removes an enable/disable flag; unsupported Windows keeps
its existing execution path. Read at use time removes caches, change subscriptions and reload
coordination.
No additional abstraction is required by these consumers.
