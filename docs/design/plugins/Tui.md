# Tui

Status: Implemented. Overall design: [overview §4 Plugin catalog](../overview.md#4-plugin-catalog); scope: [ADR 002](../../adr/002-plugin-scope.md) (CLI process, like [AcpGateway](AcpGateway.md)). Phase: P1. Language: Rust (the existing `dim-cli` code).

The terminal interface, the foreground `dimcode web` holder and the daemon connection they share become rutis plugins in a **client tree**: a rutis root that the `dimcode` client process owns, separate from the daemon's tree. The daemon (App Server), its loader rows and its managed plugins are untouched; the client tree depends on the daemon only through the local RPC protocol v3, exactly as the CLI does today. The plugin boundary buys two things: the client-start/daemon-start order becomes a declared dependency instead of code order, and terminal restoration and session shutdown become `Effect`s that rutis runs exactly once in reverse order on every exit path. `/web` no longer unloads the TUI: it toggles the daemon-owned web switch. Everything the user sees (`ui/`, `model/`, `projection`, `wire`) is unchanged.

This document gives the process model (§1), the startup sequences of every entry point (§3), the timing decisions (§4) and the distribution (§7). It does not move the TUI into the daemon: the daemon is a headless process shared by all clients and has no terminal, and [ADR 001](../../adr/001-harness-foundation.md) keeps the library usable without a TUI.

## 1. Structure

```text
dimcode (one executable, two roles)

  client process (per invocation)                 daemon process (one per product home)
  ┌─────────────────────────────────────────┐     ┌──────────────────────────────────────┐
  │ client tree (rutis root, compiled in)   │     │ daemon tree (rutis root + loader)    │
  │                                         │     │                                      │
  │  DaemonLink ─provides `Connection`─┐    │     │  LoaderPlugin                        │
  │     ▲ discover / spawn / hello     │    │ RPC │   ├─ process rows (Rust)             │
  │     │                              │    │ v3  │   ├─ NodeRuntime, NodeRows           │
  │  Tui ◄─────────────────────────────┤    │◄───►│   │    └─ dim-oauth (TypeScript)     │
  │   session thread + render thread   │    │sock │   └─ session / branch factories      │
  │  WebHold ◄─────────────────────────┘    │     │  WebEntry (loopback listener)        │
  │   `web.open`, waits for a signal        │     │  Transport, Rpc, Agents, ...         │
  └─────────────────────────────────────────┘     └──────────────────────────────────────┘
                                                                     ▲
  browser ◄──────────── http://127.0.0.1:<port>  (Web workspace) ────┘
```

| Part | Scope | Depends on | Provides |
| --- | --- | --- | --- |
| `DaemonLink` | client process | none (gated only by configuration: product home, `--server`, `--model-config`) | `Connection`: the slot holding the open protocol v3 `Client` after `hello` (§6); the daemon's feature list and profile travel with it |
| `Tui` | client process | `Connection` | nothing to other plugins; its result (`Leave::Exit(code)` or `Leave::Restart`) goes to the composition root |
| `WebHold` | client process | `Connection` | nothing; holds the local web entry open while `dimcode web` is loaded |
| Composition root (`run_cli`) | client process | — | parses arguments, builds the tree, loads the plugins an action needs, disposes the tree, returns the exit code |

One-shot actions (`exec`, `export`, `login`, `models`, `mcp …`, `daemon stop`) are RPC calls through `Client`; they are plain functions, not plugins, because they own no resource that outlives the call ([ADR 002 D2](../../adr/002-plugin-scope.md#d2-no-wiring-modules): no wiring modules). `help`, `version` and `--fullscreen default` build no tree.

Not part of this design: a loader in the client (no rows, no editable layer, no Node), a dylib or TypeScript TUI, and the daemon's assembly.

## 2. Data and state

| State | Owner | Durable |
| --- | --- | --- |
| Socket path `<home>/rust/daemon.sock` (Unix) or the pipe name derived from the native home (Windows), server link `<home>/rust/dimcode-server` | `DaemonLink` (computed from configuration each start) | No; the socket file is the daemon's |
| Open connection, `hello` reply (instance id, protocol version, features, daemon profile) | `DaemonLink` | No |
| Session id, projection, view snapshots, input history | `Tui` (its session thread), as `SessionHost` holds them today | Input history and terminal mode are the client's own files under the product home, unchanged |
| Web entry token, address and on/off switch | daemon `WebEntry`; `WebHold` only holds the entry and prints them; TUI `/web` toggles the switch | No |
| Exit code / restart | `Tui`, delivered to the composition root | No |

Nothing in the client tree is Timeline state. The daemon owns every Session fact; a client crash loses only its view.

## 3. Flows

### 3.1 `dimcode` (interactive): client start, daemon start, TUI

```text
client process                                      daemon process
──────────────                                      ──────────────
parse args; stdin is a terminal; action Chat/Tui
Ctx::root()  ── client tree
root.plugin(DaemonLink{options})
  apply:
    native home <home>/rust (0700, owned by user)
    try connect daemon.sock
      ├─ ok ──────────────────────────────────────► hello {profile}
      │                                            ◄ Hello {features, daemonProfile}
      └─ missing / refused:
           spawn `<home>/rust/dimcode-server app-server
                  --unix <sock> --no-ws --home <native home>
                  --product-home <home>
                  [--plugin-dir … --node … --js-runtime …]`
           (setsid; stdin/stdout null; stderr → daemon.log)
                                                    serve(): rutis root, LoaderPlugin,
                                                    reconcile assembly layer
                                                    (process rows, NodeRuntime, NodeRows →
                                                     dim-oauth) within startup_timeout;
                                                    listen on daemon.sock; chmod 0600
           poll every 50 ms, at most 10 s:
             socket present and not awaiting chmod
             → connect → hello ───────────────────► ◄ Hello
             child exited → fail (see daemon.log)
    provide Connection{Client}
    (no effect: dropping the service drops the Client, which detaches)
  ── DaemonLink Active
  bootstrap the Session:
    --resume ID: session.attach and read its workspace
    otherwise:   session.create {cwd, permissionPolicy, model?}
  session.attach
  ui.open {sessionId, kind: "tui", tty}
    ─────────────────────────────────────────────────► UiHost
                                                        UiLeafSupervisor
                                                        spawn launcher
                                                        (dimcode ui-leaf)
                                                          │
                                                          ├─ owns the TTY
                                                          └─ starts TUI child
                                                               hello / session.attach
                                                               ui.attach {uiId, token, generation}
    ◄ ui.status online (leaf ready + RPC attached)
  parent waits without reading the TTY
user quits (/exit, Ctrl+D) in the TUI child
  TUI restores the terminal and exits
                                                        leaf offline; supervisor stops it
    ◄ ui.status offline
  ui.close; parent returns to the shell; daemon keeps serving
                                                    later clients; stops on `daemon stop`
```

The normal `dimcode` command is the foreground supervisor for exactly one
daemon-owned UI leaf. It never runs the render loop itself and never
returns to the shell while the leaf owns the terminal. The TUI child still
uses the ordinary client tree and protocol v3; `ui.open` is an internal
daemon RPC used by this startup, not a separate user command.

### 3.2 `/web` from the TUI: daemon-owned switch

```text
Tui                                   SessionHost                        daemon WebEntry
───                                   ───────────                        ───────────────
user types /web
Command::ToggleWeb  ────────────────►  web.toggle {port?}  ─────────────►  enabled = !enabled
                                         ◄ {enabled, open, url?, address?}
Update::Web {enabled, open, url}
  print URL / off notice in place     (TUI keeps running)
```

The TUI does not exit and does not become a holder. While the switch or a
foreground `dimcode web` holder keeps the entry open, the TUI reports the
URL; turning the switch off leaves it open if a holder remains. A foreground
`dimcode web` still calls `web.open` and holds the entry until it exits, and
`dimcode web --off` clears the switch without disconnecting that holder.
`/web` in a daemon-owned leaf therefore has the same meaning and never
requires a terminal hand-off.

### 3.3 One-shot actions and `daemon stop`

```text
dimcode exec / export / login / models / mcp …        dimcode daemon stop
──────────────────────────────────────────────        ───────────────────
root.plugin(DaemonLink)  → Connection                  DaemonLink{start: false}:
take the Client, call the action's RPCs on it                         no socket → "Daemon is not running." exit 0
print; root.dispose(); exit                              socket → hello; daemon.shutdown;
                                                         wait for EOF (≤ 30 s); exit 0
```

`exec` without a terminal, `--prompt`, `--no-tui` and piped stdin take this path; the `Tui` plugin is never loaded.

### 3.4 Daemon side, for reference

Unchanged. `serve` builds the daemon tree, loads the loader, reconciles the assembly layer (Rust process rows, `NodeRuntime` and `NodeRows` for the account plugin when `--plugin-dir/--node/--js-runtime` are given) within `startup_timeout`, then listens. A client that connects during assembly waits in `DaemonLink`'s poll; the socket exists only after assembly.

<a id="35-stop-one-background-command-or-member"></a>

### 3.5 Background command output and member interruption

`/tasks` (`/background`) lists the Session's running commands and members from
the existing event projection. The transcript remains on main; task identities
retain their originating branch, with independent event cursors per branch.
Enter opens a command's output detail; `x` stops that command through the existing
`session.exec.stop`. Both read and stop use its actual branch.

The [command-output contract](https://github.com/arcships/dim/blob/main/docs/design/background-task-output.md)
uses WorkspaceTools' optional C1 method `session.exec.output`. This UI consumes
that read when provided; a profile without it displays an unavailable state. It never opens a
daemon path or falls back to a tool call.

```text
+-- Background tasks -------------------------------------------+
| > cargo build                                  command         |
|   Check the docs                               agent           |
| Enter output / interrupt member   x stop command   Esc close   |
+---------------------------------------------------------------+

+-- cargo build ------------------------------------------------+
| Tail preview - last 2,000 lines / 50 KiB                        |
| Compiling dim-app-server                                      |
| ...                                                           |
| x stop command   Esc back                                     |
+---------------------------------------------------------------+
```

An open command detail reads immediately, then one second after each response.
Only one read is outstanding, including a read admitted before the detail closes.
Closing, switching away, or a terminal event stops refreshes; a late reply cannot
replace another detail. Reopening permits a new attempt after an error. Loading
says `Loading output...`; an empty active preview says `No output yet`.
`exec_task_unknown` retains the last preview and shows `Live output unavailable`,
without fabricating an end. A missing method says `Live output unavailable in this
profile`. Other errors retain the preview and show their reason. Only the real
`exec/ended` supplies terminal status and the original final result. The finished
detail stays until Esc; the running list removes the task as before. Output wraps
within the existing overlay, shows the tail by default, and supports
Up/Down, PageUp/PageDown and Home/End. Narrow layouts preserve the status and key
hints; the existing minimum-size screen still applies. Output, read errors and
the command-detail title use the existing terminal-text sanitizer before styling.

For a member, `collaboration/delivered` binds the request to its actual Session.
Enter interrupts that Session's main turn through `session.stop`. Starting rows
explain that no target is available yet; rejected deliveries remove their
placeholder. Only the real collaboration result removes a delivered row. A stop
reply does not close the Session, retract adopted input or guarantee cancellation
of a particular delegation; a later turn may race it. RPC errors use the existing
notice path. Member conversation/progress, model policy and stop-all remain #263;
recently ended task retention and Workflow presentation remain #264. The controls
use the existing localization/theme paths and add no server method, event, lock
or plugin.

### 3.6 UI registry and UI leaves

The daemon is the authority over which interfaces are alive. It owns a UI
leaf's process, its attach token and its replacement generation without
touching the terminal: the launcher owns the TTY and starts the TUI as its
child, and the TUI keeps the ordinary protocol v3 business connection. One
executor, many viewers: a Session runs in the daemon only; every interface,
terminal or browser, submits input and observes events; none executes.

`UiHost` is a process `Capability` (`UiHostPlugin`). It provides the
registry and `UiLeafSupervisor`, which uses a local rutis link and the
`rutis-bridge` host/peer protocol to load a `UiLeaf` plugin in each launcher
process. The control plane is rutis only; `session.*`, `approval.*` and
question traffic stay on protocol v3 and are never exposed to the leaf
control link. `UiHost` writes no Timeline fact.

```text
daemon rutis root                              launcher: dimcode ui-leaf
  UiHostPlugin (process)                         rutis root
    ├─ registry: uiId -> {leaf, connection,       ├─ link to dim-daemon
    │            sessionId, kind, generation}      ├─ host catalog: "UiLeaf"
    └─ UiLeafSupervisor                            ├─ UiLeaf plugin
         ├─ LocalTransport spawn: ui-leaf          │    ├─ ready / heartbeat / offline
         ├─ attach token (digest only)             │    └─ owns the TTY
         └─ ui.open / ui.close / ui.replace        └─ TUI child (protocol v3)
```

The normal `dimcode` command uses the daemon's own executable as the
launcher; there is no user-facing `ui open` subcommand. `ui.open` is an
internal management RPC used by the normal startup path. The local
transport starts the launcher with a private channel and appends the
channel address. The daemon passes the exact daemon endpoint, product home,
an explicit model-config path when configured, the Session/kind/generation,
and a one-time token in the spawn environment. The launcher reads the token
at its boundary, removes it from its own environment, and passes an explicit
leaf identity to the TUI child; the child reads that identity once and
removes it before spawning tools. The leaf's rutis link declares its
token-derived capability; the daemon requires it, so a process that did not
receive the token cannot join. The daemon clears its spawner entry once the
link is ready and keeps no plaintext token after that point.

| Method (local connections, protocol v3) | Meaning |
| --- | --- |
| `ui.attach {sessionId, kind, uiId?, token?, generation?}` → `{uiId, generation}` | Registers the calling connection as one interface of `sessionId`, which it must already be attached to (`session_not_attached` otherwise). A leaf passes its `uiId`, token and generation; an ordinary interface passes none and gets a connection registration. Not exclusive: a Session has as many viewers as connections attach, terminals and browsers alike; `kind` is a label (`tui`, `web`, …). Calling again re-registers the same connection and returns its id and generation. A managed TUI that switches Session updates its leaf registration rather than creating a second identity. |
| `ui.open {sessionId, kind, tty?, cwd?, home?, replace?, expectedGeneration?}` → `{uiId, pid, generation}` | Internal management call used by normal `dimcode` startup and by a management client. The caller must already be attached to the Session. `kind` is `tui` for a managed leaf, and `tty` is required; it must be an absolute terminal path (or `console` on Windows) and is passed to the launcher; the launcher owns it and starts the TUI child. `cwd` and `home` are the caller's Session workspace and product home; the daemon uses its own configured values only when they are absent. `replace` names an old interface to close only after the new leaf is ready; `expectedGeneration` claims and refuses a lost replacement race (`ui_generation`). A failed start removes the new registration and leaves the old one untouched. |
| `ui.status {}` → `{instances: [{uiId, sessionId, kind, sinceMs, generation, state, pid?, exitCode?}], webConnections}` | Every registered interface, plus the count of browser connections on the web entry, which are not registered and have no lifecycle here. A legacy registration is `online`. A leaf is `starting`, `online`, `replacing`, `offline` or `failed`; `online` means the launcher reported ready, its TUI RPC attached and its control heartbeat is fresh (5 s). `exitCode` is the TUI child's status once it exited. |
| `ui.shutdown {uiId, reason?}` → `{}` | Sends the advisory `ui.shutdown {uiId, reason}` notification on the TUI connection. The TUI restores its terminal and exits 0 after one last frame with the reason; the Session is untouched. Leaf control carries `ready`, `heartbeat` and `offline`; daemon cleanup additionally uses the launcher's bounded signal/restore fallback when the business connection is gone. |
| `ui.replace {uiId, sessionId, kind, generation?}` → `{uiId, generation}` | The caller registers as a new connection interface of `sessionId` and the old `uiId` is closed after that registration. `generation` is the old generation the caller observed; a mismatch is `ui_generation`. The new registration stands even when the old interface is already gone (`ui_unknown` then); nothing is rolled back. A managed new leaf uses `ui.open {replace}` so it is ready before the old one closes. |
| `ui.close {uiId, reason?}` → `{}` | Sends the advisory notification, unloads the leaf's rutis plugin and disposes its link. The launcher asks its TUI child to restore the terminal, then exits; bounded force termination is the fallback. A legacy connection registration has no process and is only unregistered. |

Online and liveness are daemon facts, not the business connection alone: a
launcher reports `ready` through its rutis control link and sends a
heartbeat; the TUI child's `ui.attach` separately marks the RPC side. A leaf
whose process or control link ends is `offline`/`failed`; a dropped RPC
connection marks it offline until it reattaches. A hung TUI is still
force-closed by `ui.close` after the grace period; the launcher does not
outlive the daemon, and daemon cleanup first asks the TUI to leave so it can
restore the terminal. The daemon never writes to the terminal. A managed
leaf may use the `tui` kind only with a terminal path; a headless `ui.open`
fails `ui_tty_required`. `hello` advertises the internal `ui.leaf` feature only when the daemon's
launcher executable is available; a client of an older daemon without it
keeps the existing direct-TUI path instead of sending `ui.open`. On Unix the launcher owns the terminal device path
and the TUI child inherits it. On Windows the launcher allocates its own
console and the TUI child inherits that console.

A legacy connection registration ends with its connection's attachment to
the Session (`session.detach`, or the Session closing) or with the
connection itself; it has no process and no heartbeat besides the
connection. A managed leaf stays registered when its RPC detaches so it can
reattach without losing its process. The TUI's workspace is the Session's
`cwd`; it does not inherit the daemon's current directory. In the leaf,
`/suspend` and "Restart now" are unavailable with a notice; these are
shell/release operations and do not have a valid leaf meaning. `/web` works
by toggling the daemon-owned web switch, so the leaf stays in the terminal
and the browser connects through the same daemon. The ordinary protocol v3
web viewer remains available through `dimcode web` and through a separately
opened browser connection.

Generations are monotonic per daemon. `ui.status` exposes them, `ui.open`
returns one, and both `ui.attach` and `ui.replace` check the one the caller
observed. A replacement is not atomic across Sessions or processes: the new
registration is kept when the old close fails, and no rollback is claimed.
A Session supports multiple viewers. The local TUI is a leaf; browser
connections are registry-counted viewers, not leaves; remote devices belong
to `DeviceLease`/RemoteGateway and are never started or replaced here.

`ui.*` is advertised in `hello` to local connections only. The TUI child
calls `ui.attach` after every `session.attach` when the daemon offers it, so
an older daemon works as before. The leaf control link is a local rutis
link; it is not the business data channel and carries no Session service.

## 4. Timing and conflicts

| Case | Decision |
| --- | --- |
| Two clients start at once with no daemon | Both spawn; the second daemon fails to bind and exits; both clients connect to the survivor within the poll window. Unchanged behaviour, now inside `DaemonLink::apply`. |
| Daemon still assembling when the client polls | The socket does not exist yet; the client keeps polling until the deadline. The daemon's `startup_timeout` is independent of the client's 10 s; a slow managed-plugin start surfaces as "App Server startup timed out; see daemon.log". |
| Connection drops while the TUI runs | **Not a provider replacement.** The TUI child keeps its draft and reports the failure; the parent CLI continues waiting for the leaf. There is no automatic reconnect in the session thread. |
| `apply` of `Tui` fails after raw mode was entered | The terminal effect was registered before raw mode, so rutis runs it on the failed load; the error is printed on a restored terminal. |
| Ctrl+C in the TUI | A key event in raw mode, handled by the render thread (cancel the turn or quit), as today. The foreground `dimcode` parent does not read the TTY; SIGINT, SIGTERM or SIGHUP before or during the leaf maps to `ui.close`, and the parent returns 130. |
| Daemon exits while a client holds `Client` | RPC calls fail; the TUI child shows the failure and keeps running until the user quits; daemon cleanup asks the leaf to restore and exit. No automatic daemon restart from a running client. |
| `daemon stop` while another client is attached | The daemon shuts down after settlement (`daemon.shutdown.completed`); the advisory plus launcher restore path hands the terminal back, then the parent CLI returns. |
| `ui.open` start fails | The new registration is removed; a `replace` leaves the old one running and is never reported as an atomic rollback. |
| `ui.close` while the TUI ignores shutdown | The launcher waits for the TUI child to restore the terminal, then the supervisor force-terminates the launcher after the timeout. The Session is untouched. A TUI exit status is carried in `ui.status.exitCode`; the parent returns it after `ui.close`. |
| Daemon shutdown with leaves | `UiHostPlugin`'s effect sends the advisory to connected TUIs, then stops each launcher; the launcher restores the captured terminal and ends its TUI child. The bounded force fallback runs only when that path does not finish; no launcher or TUI process outlives the daemon. |
| Replacing the executable while a daemon runs | Unchanged: the server link is remade on the next start; `daemon stop` with the matching CLI first ([release packaging](../../release/README.md#upgrade-compatibility)). |

### Connection loss and terminal content

A failed Session worker retains the transcript, current detail and draft, but accepts no more
keyboard submissions or Session controls. Escape closes an open detail; the draft remains editable. The interface stops its running indicator, reports a disconnected
state and prints `dimcode --resume <session-id>` (or `dimcode` before a Session exists).
Ctrl+C, Ctrl+D or `/exit` leaves. Remote work is not reported completed or cancelled;
a shell command's outcome remains unknown. There is no automatic reconnect.

```text
+-- Disconnected · draft kept ---------------------------------+
| Exit and run: dimcode --resume <session-id>                  |
| > retained draft                                             |
+--------------------------------------------------------------+
```

Only one interactive `!` / `!!` command runs at once. A second command is refused with a
notice and retains its draft; the first run keeps its progress, result and stop identity.
Tool and shell text loses terminal escape/control sequences before layout; only the
renderer adds terminal styling. The write path keeps that promise for every row, model
text included: of the sequences a row carries, only SGR and an OSC 8 link to an `http`,
`https` or `mailto` target reach the terminal. Clicked links use only those schemes.

## 5. Context mapping

None. No client plugin writes to any Timeline or produces model context; user input reaches the model only through `session.submit`, as today.

## 6. Interfaces

| Interface | Form |
| --- | --- |
| `Connection` service (client tree) | Rust: `Arc<Connection>` keyed by `TypeKey::of`, provided by `DaemonLink`: a take-once slot holding the protocol v3 `Client` (`rpc`, `typed`, notifications) after `hello`. `Client` is driven through `&mut self` by one owner, so the service hands it over (`take`) rather than sharing it: the session thread of `Tui`, the holding task of `WebHold`, or the one-shot action. `take` yields `None` once taken, or when `start: false` found no daemon (`daemon stop`). The language-neutral contract is the wire protocol itself ([protocol.rs](../../../crates/dim-app-server/src/protocol.rs)); no other language runs in the client tree, so no named binding is defined ([D5](../../adr/002-plugin-scope.md#d5-contracts-are-language-neutral) as in AcpGateway). |
| `Tui` result | `Leave::Exit(i32)` or `Leave::Restart` delivered to the composition root through a one-shot channel the plugin is constructed with. |
| `WebHold` input | `port: Option<u16>`, `open_browser: bool`; its result (`Ok(0)` on a signal, the error on the daemon's end) goes to the composition root through a one-shot channel, like `Tui`'s. It holds the entry open; `/web` never loads it. |
| Daemon RPC used | Existing `hello`, `session.create/attach/submit/history`, `approval.respond`, `web.open`/`web.toggle`/`web.disable`/`web.status`, `daemon.shutdown` and the existing command/member stop paths; optional `session.exec.output` reads the command's bounded capture; `ui.open`/`ui.close`/`ui.status`/`ui.replace` for management, `ui.attach` after each `session.attach` when offered, and the `ui.shutdown` notification (§3.6). |

## 7. Distribution

Unchanged artifacts; the plugins are compiled into the client and ship with it, which is rutis's own model for application-hosted plugins ("plugins are compiled into the application and released with it").

| Artifact | Content | Change |
| --- | --- | --- |
| Six native archives (`dimcode-<platform>`) | One `dimcode` executable: CLI with the client tree, App Server as the `app-server` subcommand, and the `ui-leaf` control process; the Web workspace is the separate `dimcode-web` artifact ([web-workspace §2.4](../web-workspace.md#24-assets)) | `dimcode` gains `rutis`/`rutis-bridge` for the leaf process; `dim-cli` keeps its existing `rutis` dependency. No loader row and no Node runtime are added |
| Account build | The same executable with `runtime.bin` (Node, `@arcships/rutis-runtime`, `dim-oauth`) embedded; extracted once to `<home>/runtime/<digest>` by the client and passed to the daemon it spawns | None. The client tree never loads Node; the paths stay `DaemonLink` spawn arguments |
| Seven npm tarballs (`dimcode` wrapper + six platform packages) | Wrapper picks the platform package and runs the executable | None |
| `release-manifest.json`, `SHA256SUMS` | Provenance | None |
| Development: `./dev`, `scripts/dev.sh` in the private repository | Build the executable, stage the local plugin archive, start the daemon through the CLI | None; the CLI still spawns the daemon the same way |

No loader row is added, so there is nothing to enable, disable or persist ([D4](../../adr/002-plugin-scope.md#d4-structural-configuration-and-business-configuration-are-separate)). Normal `dimcode` startup uses the daemon's internal `ui.open` management RPC; there is no `dimcode ui open` command. The management surface adds no user-facing launcher option: the daemon's executable and its internal `ui-leaf` entry point are supplied by the CLI's existing daemon spawn. Whether the TUI or ACP front end runs is decided by the subcommand, as today. A client loader with `Builtins` rows (so that front ends could be toggled by configuration) is possible later without changing this document's plugins; it is out of scope.

## 8. Security

The client tree runs trusted in-process code only; a plugin boundary is not a sandbox ([rule 11](../../../AGENTS.md)). `DaemonLink` keeps every existing check: private native home (0700, owner), private log file, `O_NOFOLLOW`, socket permission race, server link identity. A leaf receives a one-time UI token from the daemon; the supervisor stores only its SHA-256 digest, and the rutis link requires the token-derived capability so a process that did not receive it cannot join. The launcher reads the token at its boundary, removes it from its environment before starting the TUI child, and passes an explicit leaf identity to that child. The TUI reads that identity once and removes it before spawning tools; daemon transport bookkeeping must not retain a plaintext token after the child starts. The leaf control link is local, exposes no Session or management method, and is not the business RPC channel; the TUI's `ui.attach` remains token-bound. Tokens are never logged or written to the Timeline. The sign-in link printed by `WebHold` or reported by the TUI is shown only when no browser took it, as today. Nothing here changes approval, permission or the product-home boundary.

## 9. Rule check

| Rule | Answer |
| --- | --- |
| §4 rule 1 (Capability, `Deps`, `apply`, effects) | `UiHostPlugin` is a process `Capability`; a per-leaf `UiLeafCarrier` plugin mounts the launcher link and registers one `ctx.effect` that owns the link, the control handler and the remote plugin load. The link's `Peer` is a child resource created by that apply, not an external `Deps` entry. The TUI child keeps `Tui` as a client plugin and uses `web.toggle`; `WebHold` remains only for `dimcode web`; normal `dimcode` parent uses `DaemonLink` and the internal `ui.open` call. |
| §4 rule 2 (critical effects through the Timeline) | Not applicable: no client plugin produces or consumes Timeline events; input goes through `session.submit`. |
| §4 rule 3 (handlers register tasks and return) | The launcher starts the TUI child and returns; control handlers report ready/heartbeat/offline without awaiting the business connection. |
| §4 rule 4 (state ownership, Timeline writes) | All client state is view state owned by the TUI child; the daemon owns every fact. |
| §4 rule 5 (crate restrictions, `xtask check`) | `dimcode` adds `rutis`/`rutis-bridge` for the launcher/TUI internal entry; `dim-app-server` adds `libc` on Unix for the bounded fallback; run `cargo run -p xtask -- check` and `graph`. The client tree stays outside the local-execution graph; the App Server graph records `UiHost`. |
| §4 rule 6 (tests follow requirements) | The fake-daemon CLI flow tests keep the direct-TUI fallback for older daemons; `ui_leaf.rs` covers the real normal `dimcode` startup through a leaf, `/new` keeping one UI, `/web` toggling without leaving the leaf, launcher/TUI start, token/RPC online, exit status, generation replacement, forced close, terminal restoration and daemon-shutdown cleanup. |
| §4 rule 7 (reuse mechanisms) | Reuses `client::connect_inner`, `SessionHost`, `ui::tui::run`, `open_web` unchanged in substance; the TUI child owns them, while the parent waits for the leaf. |
| §4 rule 8 (settle before removing services) | The advisory plus launcher restore path asks the TUI to leave before the launcher/link is removed; the parent waits for that leaf state. |
| §4 rule 9 (publish after commit) | Not applicable: no Timeline writes. |
| §1 (Capability and App Server boundary) | The TUI child is a separate process and a separate rutis root; it touches App Server business state only over RPC. `UiHostPlugin` (§3.6) is a process `Capability` owning launcher processes and their rutis links; Transport, Connection and Rpc gain only the `ui.*` built-ins and one notification; no Timeline fact. |
| ADR 002 D1 (scope follows the owner) | The launcher owns the terminal and its TUI child; the leaf link and remote `UiLeaf` plugin belong to the launcher. `UiHostPlugin` lives in the daemon's process scope. |
| D2 (no wiring modules) | One-shot actions stay functions; no registry, port or adapter plugin is added. Each leaf carrier owns the launcher link and its remote load as one resource; `DaemonLink` owns the connection and daemon spawn. |
| D3 (dependencies point to the core) | The TUI child depends on the daemon's public protocol; no daemon plugin knows the TUI child. A dropped business connection is not a provider replacement (§4). |
| D4 (structural vs business configuration) | No loader rows; terminal mode, workspace and input history remain client/Session state. Launcher path and endpoint are internal process wiring, not a user configuration row. |
| D5 (language-neutral contracts) | The only business contract is the wire protocol; the `Connection` service and the internal launcher argv are Rust-only bindings, listed in §6. |

## 10. Requirements raised to mechanism owners

None. The spawner entry is cleared once the link is ready, and the daemon
endpoint reaches the child explicitly.

## Appendix A. Implementation notes

- `run_cli` keeps the `tokio` runtime it builds; the rutis root lives on it. The TUI's blocking loop moves from the main thread to the `dim-tui` thread spawned in `Tui::apply` (as `rutis-agent`'s `TuiPlugin` does); the main thread blocks on the root until `Leave` resolves.
- `SessionHost` becomes the session thread inside `Tui`; `SessionPort` stays the interface between `ui` and the session thread, unchanged.
- `DaemonLink` takes `Options` by value and a `start: bool` (false only for `daemon stop`).
- Terminal restoration is registered as the first effect, before entering raw mode, so a failed load still restores the terminal.
- The new module sits at the top of `dim-cli`, beside `lib.rs`: it uses `client`, `session_host` and `ui::tui`, which the `xtask` layer rules allow (`session_host` must not use `ui`, and `ui` must not use the line client; neither is touched).
- Actual change: one module `crates/dim-cli/src/plugins.rs` (`Connection`, `DaemonLink`, `Tui`, and the `WebHold` used by `dimcode web`), `run_cli` rewritten to build and dispose the root, `SessionHost::start` taking the connection instead of connecting, and `dim-cli/Cargo.toml` gaining `rutis.workspace = true`. `/web` is a `web.toggle` call in `SessionHost` and no longer unloads `Tui`. Under `ui/` only the restore surface and the Web report are touched; no loader row or daemon plugin is added.
