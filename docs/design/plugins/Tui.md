# Tui

Status: Implemented. Overall design: [overview §4 Plugin catalog](../overview.md#4-plugin-catalog); scope: [ADR 002](../../adr/002-plugin-scope.md) (CLI process, like [AcpGateway](AcpGateway.md)). Phase: P1. Language: Rust (the existing `dim-cli` code).

The terminal interface, the foreground `dimcode web` holder and the daemon connection they share become rutis plugins in a **client tree**: a rutis root that the `dimcode` client process owns, separate from the daemon's tree. The daemon (App Server), its loader rows and its managed plugins are untouched; the client tree depends on the daemon only through the local RPC protocol v3, exactly as the CLI does today. The plugin boundary buys three things: the client-start/daemon-start order becomes a declared dependency instead of code order, terminal restoration and session shutdown become `Effect`s that rutis runs exactly once in reverse order on every exit path, and the `/web` hand-off becomes an unload of one plugin and a load of another. Everything the user sees (`ui/`, `model/`, `projection`, `wire`) is unchanged.

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
| `Tui` | client process | `Connection` | nothing to other plugins; its result (`Leave::Exit(code)` or `Leave::Web(handoff)`) goes to the composition root |
| `WebHold` | client process | `Connection` | nothing; holds the local web entry open while it is loaded |
| Composition root (`run_cli`) | client process | — | parses arguments, builds the tree, loads the plugins an action needs, disposes the tree, returns the exit code |

One-shot actions (`exec`, `export`, `login`, `models`, `mcp …`, `daemon stop`) are RPC calls through `Client`; they are plain functions, not plugins, because they own no resource that outlives the call ([ADR 002 D2](../../adr/002-plugin-scope.md#d2-no-wiring-modules): no wiring modules). `help`, `version` and `--fullscreen default` build no tree.

Not part of this design: a loader in the client (no rows, no editable layer, no Node), a dylib or TypeScript TUI, and the daemon's assembly.

## 2. Data and state

| State | Owner | Durable |
| --- | --- | --- |
| Socket path `<home>/rust/daemon.sock` (Unix) or the pipe name derived from the native home (Windows), server link `<home>/rust/dimcode-server` | `DaemonLink` (computed from configuration each start) | No; the socket file is the daemon's |
| Open connection, `hello` reply (instance id, protocol version, features, daemon profile) | `DaemonLink` | No |
| Session id, projection, view snapshots, input history | `Tui` (its session thread), as `SessionHost` holds them today | Input history and terminal mode are the client's own files under the product home, unchanged |
| Web entry token and address | daemon `WebEntry`; `WebHold` only prints them | No |
| Exit code / hand-off | `Tui`, delivered to the composition root | No |

Nothing in the client tree is Timeline state. The daemon owns every Session fact; a client crash loses only its view.

## 3. Flows

### 3.1 `dimcode` (interactive): client start, daemon start, TUI

```text
client process                                      daemon process
──────────────                                      ──────────────
parse args; stdin is a terminal; action Chat
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
root.plugin(Tui{env})                                (gated: starts once Connection is provided)
  apply:
    effect: restore terminal        ── registered first: runs on every exit path
    enter raw mode / alternate screen
    spawn "dim-session" thread: session.create or session.attach, projection, snapshots
    spawn "dim-tui" thread: render loop, key handling
    effect: shut the session thread down (drain, join)
  ── Tui Active
user quits (/exit, Ctrl+D)
  Tui resolves Leave::Exit(code) to the root
root.dispose():  Tui effects (session thread, then terminal), then DaemonLink effect (detach)
exit(code)                                          daemon keeps running (setsid), serves
                                                    later clients; stops on `daemon stop`
```

The daemon never starts before a client asks for it, and a client never talks to it before `hello` succeeds; both facts are now the single dependency `Connection`. The 10 s wait, the chmod race check and the `daemon.log` hint are `DaemonLink`'s `apply` and stay as implemented in `client::connect_inner`.

### 3.2 `/web` from the TUI: hand-off

```text
Tui                                   root                                   WebHold
───                                   ────                                   ───────
user types /web
resolve Leave::Web(handoff)  ───────► unload Tui
                                        effects: render thread joined (it shut the
                                        session thread down), terminal restored
                                      unload DaemonLink: its Connection slot is
                                        empty, the session thread's Client is gone
                                      root.plugin(DaemonLink)  → hello on a new connection
                                      root.plugin(WebHold{handoff, open_browser})
                                                                             apply:
                                                                               take the Client from Connection
                                                                               listen for Ctrl+C / SIGTERM
                                                                               web.open {port?}  ───► daemon WebEntry
                                                                               print URL / sign-in link / session id
                                                                               open the browser if asked
                                                                               task: hold the Client until a signal
                                                                                 or the daemon's EOF; report through
                                                                                 a one-shot to the root
                                                                               effect: end the task (dropping the
                                                                                 Client closes the entry)
                                      wait for the one-shot
                                      root.dispose()  → WebHold, then DaemonLink
exit(0) on a signal; exit(1) "The App Server stopped" on EOF
```

The interface's connection ends with the interface: the session thread owns the `Client` it took from the slot and drops it when it stops, which detaches the client as before. The holder therefore links the daemon again (the daemon is running, so this is one `hello`) instead of inheriting a connection, which keeps the hand-off's observable behaviour: the session keeps running in the daemon, and the browser attaches to it.

`dimcode web [--port] [--no-open]` is the same flow without a `Tui` load.

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

### 3.6 UI registry: `UiHost`

The daemon is the authority over which interfaces are alive, without becoming their parent process or touching a terminal: an interface process keeps its TTY, and the daemon records, lists, closes and replaces interfaces over the connection each one already has. One executor, many viewers: a Session runs in the daemon only; every interface, terminal or browser, submits input and observes events, none executes. `UiHost` is App Server process code held by the `ConnectionHub`, like `WebEntry`; it is not a Capability and writes nothing to the Timeline.

```text
daemon                                   dimcode (client tree unchanged)
  ConnectionHub                            Tui
    └─ UiHost: uiId -> {connection,          ├─ hello, session.attach
                        sessionId, kind}     ├─ ui.attach {sessionId, kind: "tui"} -> {uiId}
         ui.attach / ui.status /             └─ notice ui.shutdown {uiId, reason}
         ui.shutdown / ui.replace                 -> restore the terminal, exit 0
```

| Method (local connections, protocol v3) | Meaning |
| --- | --- |
| `ui.attach {sessionId, kind}` → `{uiId}` | Registers the calling connection as one interface of `sessionId`, which it must already be attached to (`session_not_attached` otherwise). Not exclusive: a Session has as many viewers as connections attach, terminals and browsers alike; `kind` is a label (`tui`, `web`, …). Calling again re-registers the same connection and returns its `uiId`. |
| `ui.status {}` → `{instances: [{uiId, sessionId, kind, sinceMs}], webConnections}` | Every registered interface, plus the count of browser connections on the web entry, which are not registered and have no lifecycle here. |
| `ui.shutdown {uiId, reason?}` → `{}` | Sends the `ui.shutdown {uiId, reason}` notification on that interface's connection. The interface leaves (the TUI restores its terminal and exits 0 after one last frame that shows the reason); the Session is untouched. `ui_unknown` when no such interface is registered. |
| `ui.replace {uiId, sessionId, kind}` → `{uiId}` | The caller registers as a new interface of `sessionId` and the old `uiId` is asked to shut down. The new registration stands even when the old interface is already gone (`ui_unknown` then); nothing is rolled back. |

Online means registered and connected: a registration ends with its connection's attachment to the Session (`session.detach`, or the Session closing) or with the connection itself, so there is no heartbeat besides the connection. The TUI detaches from a Session it leaves before attaching to the next. An interface that does not obey `ui.shutdown` stays registered until its connection ends; the daemon never kills a process. `ui.*` is advertised in `hello` to local connections; the TUI calls `ui.attach` after every `session.attach` only when the daemon offers it, so an older daemon works as before.

## 4. Timing and conflicts

| Case | Decision |
| --- | --- |
| Two clients start at once with no daemon | Both spawn; the second daemon fails to bind and exits; both clients connect to the survivor within the poll window. Unchanged behaviour, now inside `DaemonLink::apply`. |
| Daemon still assembling when the client polls | The socket does not exist yet; the client keeps polling until the deadline. The daemon's `startup_timeout` is independent of the client's 10 s; a slow managed-plugin start surfaces as "App Server startup timed out; see daemon.log". |
| Connection drops while the TUI runs | **Not a provider replacement.** `DaemonLink` keeps `Connection` registered; the session thread owns the dropped `Client`, reports the failure and keeps the interface up, as today (there is no automatic reconnect in the session thread today either). Letting rutis evict `Tui` would tear the terminal down for a transient fault; the same reasoning as [ADR 002 D3](../../adr/002-plugin-scope.md#d3-dependencies-follow-scope-and-point-from-extensions-to-the-core) (optional providers are looked up per call, not depended on). |
| `apply` of `Tui` fails after raw mode was entered | The terminal effect was registered before raw mode, so rutis runs it on the failed load; the error is printed on a restored terminal. |
| Ctrl+C in the TUI | A key event in raw mode, handled by the render thread (cancel the turn or quit), as today. Ctrl+C in `WebHold` and one-shot actions is a signal that ends the wait and disposes the tree. |
| Daemon exits while a client holds `Client` | RPC calls fail; `Tui` shows the failure and keeps running until the user quits; `WebHold` exits. No automatic daemon restart from a running client. |
| `daemon stop` while another client is attached | The daemon shuts down after settlement (`daemon.shutdown.completed`); the other client's connection ends as above. |
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
| `Tui` result | `Leave::Exit(i32)` or `Leave::Web(WebHandoff)` delivered to the composition root through a one-shot channel the plugin is constructed with. |
| `WebHold` input | `port: Option<u16>`, `open_browser: bool`, `handoff: Option<Box<WebHandoff>>`; its result (`Ok(0)` on a signal, the error on the daemon's end) goes to the composition root through a one-shot channel, like `Tui`'s. |
| Daemon RPC used | Existing `hello`, `session.create/attach/submit/history`, `approval.respond`, `web.open`, `daemon.shutdown` and the existing command/member stop paths; optional `session.exec.output` reads the command's bounded capture; `ui.attach` after each `session.attach` when offered, and the `ui.shutdown` notification (§3.6). |

## 7. Distribution

Unchanged artifacts; the plugins are compiled into the client and ship with it, which is rutis's own model for application-hosted plugins ("plugins are compiled into the application and released with it").

| Artifact | Content | Change |
| --- | --- | --- |
| Six native archives (`dimcode-<platform>`) | One `dimcode` executable: CLI with the client tree, App Server as the `app-server` subcommand; the Web workspace is the separate `dimcode-web` artifact ([web-workspace §2.4](../web-workspace.md#24-assets)) | None. `dim-cli` gains the `rutis` crate (workspace dependency, already built for the daemon); binary size change is the rutis core only, no loader, no bridge |
| Account build | The same executable with `runtime.bin` (Node, `@arcships/rutis-runtime`, `dim-oauth`) embedded; extracted once to `<home>/runtime/<digest>` by the client and passed to the daemon it spawns | None. The client tree never loads Node; the paths stay `DaemonLink` spawn arguments |
| Seven npm tarballs (`dimcode` wrapper + six platform packages) | Wrapper picks the platform package and runs the executable | None |
| `release-manifest.json`, `SHA256SUMS` | Provenance | None |
| Development: `./dev`, `scripts/dev.sh` in the private repository | Build the executable, stage the local plugin archive, start the daemon through the CLI | None; the CLI still spawns the daemon the same way |

No new configuration surface: there are no loader rows in the client, so nothing to enable, disable or persist ([D4](../../adr/002-plugin-scope.md#d4-structural-configuration-and-business-configuration-are-separate)). Whether the TUI or ACP front end runs is decided by the subcommand, as today. A client loader with `Builtins` rows (so that front ends could be toggled by configuration) is possible later without changing this document's plugins; it is out of scope.

## 8. Security

The client tree runs trusted in-process code only; a plugin boundary is not a sandbox ([rule 11](../../../AGENTS.md)). `DaemonLink` keeps every existing check: private native home (0700, owner), private log file, `O_NOFOLLOW`, socket permission race, server link identity. The sign-in link printed by `WebHold` is shown only when no browser took it, as today. Nothing here changes approval, permission or the product-home boundary.

## 9. Rule check

| Rule | Answer |
| --- | --- |
| §4 rule 1 (Capability, `Deps`, `apply`, effects) | `Tui` and `WebHold` declare `Connection` in `Deps`; `apply` registers the terminal and session-thread effects immediately with `ctx.effect` before acquiring them. They are rutis `TypedPlugin`s, not runtime `Capability`s: the client tree has no Session or branch factories. |
| §4 rule 2 (critical effects through the Timeline) | Not applicable: no client plugin produces or consumes Timeline events; input goes through `session.submit`. |
| §4 rule 3 (handlers register tasks and return) | `apply` spawns the session and render threads and returns; no handler awaits the daemon. |
| §4 rule 4 (state ownership, Timeline writes) | All client state is view state owned by `Tui`; the daemon owns every fact. |
| §4 rule 5 (crate restrictions, `xtask check`) | `dim-cli` adds `rutis`; run `cargo run -p xtask -- check` after the dependency change. The client tree is outside the local-execution graph, so `graph` output is unchanged. |
| §4 rule 6 (tests follow requirements) | The CLI flow tests (`crates/dimcode/tests`) keep asserting the same user flows: start, chat, `/web`, `exec`, `daemon stop`. |
| §4 rule 7 (reuse mechanisms) | Reuses `client::connect_inner`, `SessionHost`, `ui::tui::run`, `open_web` unchanged in substance; only their ownership moves into `apply`/effects. |
| §4 rule 8 (settle before removing services) | `Tui`'s effect drains and joins the session thread before `DaemonLink` drops the connection (reverse order of registration). |
| §4 rule 9 (publish after commit) | Not applicable: no Timeline writes. |
| §1 (Capability and App Server boundary) | The client tree is a separate process and a separate rutis root; it touches the App Server only over RPC. `UiHost` (§3.6) is App Server process code under the `ConnectionHub`, like `WebEntry`: Transport, Connection and Rpc gain only the `ui.*` built-ins and one notification; no Capability, no Timeline fact. |
| ADR 002 D1 (scope follows the owner) | The terminal and the connection are owned by the client invocation, so the plugins are client-process scope, like AcpGateway. |
| D2 (no wiring modules) | One-shot actions stay functions; no registry, port or adapter plugin is added. `DaemonLink` owns a resource (the connection and the spawn), it does not only connect other plugins. |
| D3 (dependencies point to the core) | Client plugins depend on the daemon's public protocol; no daemon plugin knows the client tree exists. A dropped connection is not a provider replacement (§4). |
| D4 (structural vs business configuration) | No loader rows; terminal mode and input history remain the client's business files. |
| D5 (language-neutral contracts) | The only contract is the wire protocol; the `Connection` service is a Rust-only binding inside one process, listed in §6. |

## 10. Requirements raised to mechanism owners

None. rutis 0.12's `TypedPlugin`, `Effect::Disposer`/`AsyncDisposer` and dependency gating cover every need above.

## Appendix A. Implementation notes

- `run_cli` keeps the `tokio` runtime it builds; the rutis root lives on it. The TUI's blocking loop moves from the main thread to the `dim-tui` thread spawned in `Tui::apply` (as `rutis-agent`'s `TuiPlugin` does); the main thread blocks on the root until `Leave` resolves.
- `SessionHost` becomes the session thread inside `Tui`; `SessionPort` stays the interface between `ui` and the session thread, unchanged.
- `DaemonLink` takes `Options` by value and a `start: bool` (false only for `daemon stop`).
- Terminal restoration is registered as the first effect, before entering raw mode, so a failed load still restores the terminal.
- The new module sits at the top of `dim-cli`, beside `lib.rs`: it uses `client`, `session_host` and `ui::tui`, which the `xtask` layer rules allow (`session_host` must not use `ui`, and `ui` must not use the line client; neither is touched).
- Actual change: one new module `crates/dim-cli/src/plugins.rs` of 276 lines (`Connection`, `DaemonLink`, `Tui`, `WebHold` and the hold's signal wait), `run_cli` rewritten to build and dispose the root (`open_web` moved into `WebHold`), `SessionHost::start` taking the connection instead of connecting, `dim-cli/Cargo.toml` gaining `rutis.workspace = true`. Under `ui/` only two lines of surface: `Env` derives `Clone` for the render thread, and `terminal::restore_if_held` exposes the existing restore for the `Tui` effect. No change under `model/`, `projection.rs`, `wire.rs` or in the daemon.
