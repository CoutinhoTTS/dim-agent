# Agent tool modes: normal, codemode and shell

Status: Draft. Scope: process (mode availability), Session (the mode and the tool surface),
branch (the mode a turn pins).
Issue: [#2135](https://github.com/arcships/dim/issues/2135). Delivery status belongs in the issue.

Today the model has one way to act: one tool call per decision, with every tool declared before the
first request. This design gives the product three mutually exclusive working styles — one call, one
program, one shell session — so a person can choose how the agent acts, not just which tools it
sees.

A mode is therefore a rule over the Session's tool set: it decides the unit of action and, from
that, which declarations a request carries. It enables and disables no plugin: a Session whose tool
set lacks the mode's entry tool runs `normal`, and every tool that is registered stays registered.

## 1. Why a tool-mode choice

### 1.1 One way to act

The model has one way to act today: each decision calls one tool, and every tool is declared before
the first request. Some work does not have that shape. A task may need the model to find out which
tools exist, call several of them in a loop, branch on their results, and collect one answer. As
single calls that is many model round trips, and every intermediate result has to re-enter the
context.

Two capabilities in this repository already offer another shape: Code Mode runs one bounded program
per call ([CodeMode](plugins/CodeMode.md)), and the persistent shell runs commands in one shell
session that keeps its working directory, environment and jobs (`ShellSession`). A third, the
ordinary tool set, offers the opposite: one precise call at a time, each one its own decision. The
product exposes only the third. `codemode` is installed beside the direct tools, so the model keeps
choosing the direct tools — as it does in this very session — and the persistent shell exists only
in the minimal-execution profile.

### 1.2 What the modes change: the unit of action

A mode is the unit of action the model is given, not a subset of tools. Each already exists in the
repository; the shell is the one without a product form.

| Mode | Unit of action | What one step can do | Product form today |
| --- | --- | --- | --- |
| `normal` | one tool call | one precise action, its own approval, one result | the product |
| `codemode` | one program | discover tools, call them, branch, loop, aggregate; child results stay out of the model context | the `codemode` tool beside the direct tools |
| `shell` | one shell session | compose commands; working directory, environment and jobs persist across calls | the minimal-execution profile only |

In `codemode` and `shell` only the entry and the tools §2 lists are declared. That absence is a
property of the mode — the model does its work through one surface — not the goal of the change.
Fewer declarations per request, fewer round trips and less context churn follow from it; the tools
declared today carry about 12 kB of description and about 20 kB with schemas, but saving that is a
result, not the requirement.

### 1.3 The one real risk: how well the model uses the mode

The modes change the unit of action, and whether that is better depends on the model:

- **Writing the program.** `codemode` pays off only if the model writes a correct program: it reads
  the tool set (`tools.list`, `tools.describe`), handles `ToolCallError`, bounds its own loops and
  aggregates the results. A model trained mostly on direct tool calls may write one-call programs,
  guess an argument shape, or fail on syntax, and then the mode is slower than the tools it
  replaced.
- **Writing the pipeline.** `shell` pays off only if the model is fluent in the platform's shell
  dialect: quoting and escaping, the difference between an interactive and a non-interactive
  command, when to background and when to wait. A command that waits for input occupies the shell.
- **Fitting the task.** A task that needs one edit gains nothing from a program or a shell session.
  The default `normal` and the ability to switch keep that cheap, but the mode is a product bet on
  how people work.

The mechanism cannot make a model better at this. What it provides is the material to judge and to
fail visibly: each child call is a Timeline fact, a program has hard bounds (time, memory, calls,
output), and a failure returns its partial result. Whether a model is good enough is measured with
the evaluation harness, not argued here, and the design claims no success rate.

### 1.4 The modes are exclusive

A mode exists only if the model faces one unit of action. With both `codemode` and the direct tools
the model uses the direct tools, as it does today, and `codemode` is just another declaration. With
both `execute` and the file tools the model mixes two write paths (shell redirection and
`write`/`edit`), and "the command is the state" stops holding. Exclusivity is what makes each mode
mean something, not a simplification.

### 1.5 Alternatives considered

| Alternative | Why not |
| --- | --- |
| Keep `codemode` as one more tool | The model keeps choosing the direct tools, so the orchestrator is never the way it works |
| A tool picker in settings | Changes which tools exist, not the unit of action; still one call per decision |
| A mode that loads or unloads plugins | Structural configuration changes loader rows and reloads them, which contradicts "independent of plugin enablement" |
| One daemon profile per mode | A different process and home, no shared sessions, and no way to switch with a setting |

## 2. Product behavior

`codemode` and `shell` declare what the table lists: their entry tool and the extras their mode
needs. `codemode` adds `skill`, whose instructions the model reads directly; `shell` has no
program, so it adds `ask_user` and `recall`. `normal` declares the work tools.

| Mode | Declared | A program may call | Hidden |
| --- | --- | --- | --- |
| `normal` (default) | every tool except `codemode` and `execute` | — | `codemode`, `execute` |
| `codemode` | `codemode`, `skill` | every tool except `codemode` | — |
| `shell` | `execute`, `ask_user`, `recall` | — | every other tool |

- `codemode` loses no reach: its program calls every tool except `codemode`, so images, the web,
  MCP, lines, agents, schedules, settings and the shell are all available inside a program.
  `ask_user` and `recall` are orchestration inputs a program can process and use — it may ask the
  operator several times and act on each answer, or search the history and decide from it — so they
  are calls, not declarations.
- `skill` is declared because the model needs the instructions directly; a program may call it too,
  but loading them through a program would only return them as data.
- `shell` is the shell, plus `ask_user` and `recall`, because it has no program to carry them. It
  cannot load a skill, read an image, call an MCP tool, open a line, start a member, schedule or
  manage settings and plugins. Those are the mode's boundary: a task that needs one runs in another
  mode.
- What each entry tool must provide to carry its mode is §7.

### Goals in shell mode

`shell` declares no `goal_update`, so a Session whose mode **in force** is `shell` cannot complete
or block a goal. Everything below keys on the mode in force — the `effective` value
`session.tool_mode.get` reports — never on the chosen mode.

- **Update.** `session.tool_mode.update` calls Goal's settle operation with the pending mode before
  the record write and waits for it. Goal settles only when that mode would be effective `shell` —
  its entry tool is in the callable set — because a chosen `shell` without `execute` runs `normal`.
- **Registration.** When the entry tool registers or withdraws and makes `shell` effective without
  an update, `ToolRegistry` publishes the effective mode; Goal settles on that signal. A held
  continuation released before Goal settles can reach one turn under `shell`, and its `goal_update`
  call is refused as an unknown tool. Closing that window needs a pre-change signal from the
  registry; §12 records it.
- **Settle.** The Goal Session part fans out to every branch with an active goal: it writes
  `paused` with reason `mode`, a pause input text carrying that reason (Goal state alone does not
  enter the model context), and withdraws the continuation it queued when the InputQueue has not
  applied it. An applied continuation belongs to a turn that started earlier, which pinned the old
  mode and finishes under it.
- **Order.** The update runs: Goal settles, the record is written, the live mode is replaced. A
  failure before the record write leaves the mode pair unchanged and a committed Goal pause in
  place, and the operator resumes it; a Goal failure stops the update before the mode changes.
- **Guarantee.** A turn that starts after the pause is committed carries the pause text in its
  history, and its requests declare no `goal_update`.
- While `shell` is in force, a new continuation pauses the goal instead of continuing.
- `session.goal.set` and `session.goal.resume` are refused while `shell` is in force, with a named
  error; `pause`, `edit`, `clear` and `get` keep working.
- Leaving `shell` does not resume the goal: the operator resumes it explicitly, as Goal already
  requires for a paused goal.
- A completed or blocked goal stays as it is.

Goal's owner states this lifecycle and its acceptance cases in [Goal](plugins/Goal.md); this design
sets the requirement and writes no Goal fact itself.

Rules that hold in every mode:

- Approval and Permission are unchanged. Each call is admitted under the called tool's own risk,
  including a Code Mode child call; approving `codemode` approves no child.
- Tool sources are unchanged. MCP servers, extension tools and other registered tools still
  contribute to the Session's tool set; the mode decides what the model sees and, in `codemode`,
  what a program may call. It does not enable or disable a source.
- Apart from the mode control and the Goal control in §8, client surfaces are unchanged. Mention
  search (`session.files`), command-output paging, skills, approvals and settings work in every
  mode.

## 3. Who owns the mode

The mode belongs to a Session, and the operator owns it. A mode is a capability choice — `shell`
replaces the work tools with one shell, `codemode` hides them behind one program — so nobody else
may change it behind the operator's back.

| Situation | Rule | Why |
| --- | --- | --- |
| A new Session from a client | starts in the mode the request names, or `normal` when it names none; the client sends the mode it last used | the operator should not re-choose for every Session |
| A running Session | the operator can switch it at any time; the next turn declares the new mode, a turn already started keeps its pin (an in-process budget recovery reuses it; a process restart settles the in-flight request), and a call already admitted finishes | otherwise a change of style means abandoning the conversation |
| A branch (line) | has no mode of its own; every line of a Session runs the Session's mode | a line is the same agent; a line in another mode would hand the agent tools the operator excluded |
| A member Session | starts in its creator Session's current mode, and the operator can switch it like any Session | the member is the same agent doing delegated work; the model must not decide how the agent acts |
| The model | never chooses or changes a mode | it is the operator's choice of capability |

A switch does not rewrite the history the model already has: the next turn carries the new
declarations.

## 4. The tool surface rule

The Session's tools are one set with one owner. `ToolRegistry` already owns the mount list and the
Session's tool set ([ToolRegistry](../architecture/rust-agent/plugins/ToolRegistry.md)), so it also
owns this projection. Three terms:

- **callable**: what the Session mounts: host tools, registered tools and each source's sets, after
  `SessionConfig.tools` and `excluded_tools`. The branch adopts and pins this set, and it does not
  change when the mode does.
- **visible**: what the request declares and what a top-level model call may name.
- **delegated**: what a Code Mode child call may name. The request binds their union; neither is an
  admission set by itself.

| Mode | visible (top-level) | delegated (child) | entry tool absent |
| --- | --- | --- | --- |
| `normal` | every tool except `codemode` and `execute` | none | — |
| `codemode` | `codemode`, `skill` | every tool except `codemode` | the Session runs `normal` |
| `shell` | `execute`, `ask_user`, `recall` | none | the Session runs `normal` |

- The sets follow §2 and name no other tool: a new tool is declared in `normal`, callable from a
  `codemode` program and hidden in `shell`, so it needs no edit here.
- Admission is by call origin: a top-level call is admitted only inside `visible`, a child call only
  inside `delegated`. Both then pass the called tool's own Permission and Approval. A call outside
  its origin's set is refused as an unknown tool, even when the tool is bound. The origin is the
  existing parent-call scope (`current_parent_call_id`), so a hidden tool cannot be reached by
  naming it directly. The check runs before a handler's `claim`, so a refused call registers no
  waiter and takes no effect.
- A Code Mode program discovers and calls exactly the `delegated` set: `tools.list` and
  `tools.describe` project it, and `tools.call` admits inside it. `codemode` itself is never
  delegated, so a program cannot nest one.
- The Loop holds the turn's mode: a turn pins it when it starts — when its driving input is
  applied — and every request of the turn uses that pin, together with the callable set ToolBinding
  pins today. A switch takes effect with
  the next turn and needs no re-adoption: the branch's configuration, the version pins and the
  source sets stay as they are. A request retried through its own source (an in-process budget
  recovery) keeps the pin; a process restart settles the in-flight request as the loop already does
  and does not reissue it; the request that follows a tool result keeps the turn's pin.
- A tool that is only delegated never appears in a prompt, a declaration or the usage counter;
  `skill` is in both sets.
- The mount list and an Agent profile's tool list narrow the surface and never widen it. `normal`
  and `shell` hide the other modes' entry tools even when a list names them.

Registration is dynamic, so the projection follows the current set and the current mode: a Session
whose entry tool is not registered, is withdrawn, or is excluded runs `normal` until it appears.
The two product clients also read which modes the daemon's assembly can serve: `hello.features`
carries `session.tool_mode.codemode` and `session.tool_mode.shell`, so a picker can mark an
unavailable mode instead of silently falling back.

## 5. Configuration, creation and switching

The mode is Session configuration; there is no daemon-wide default to maintain.

- `session.create` may name the mode in `toolMode`; a Session created without one starts `normal`.
  The field is additive and advertised as `session.create.toolMode`, like `session.language`,
  because an older daemon rejects an unknown field. `Create` carries it as `tool_mode`, which
  serializes as `toolMode`.
- The App Server records the mode in `SessionConfig.toolMode`. `normal` omits the member, so a
  normal Session's stored configuration has the same bytes as before this field existed; an old
  record reads as `normal`. A fork copies the value, and a restore starts from it.
- The product clients remember the last mode the operator chose in that client and send it when they
  create a Session, so a new Session starts the way the operator last worked. That memory is client
  state; it is not a daemon setting.

The mode is live for the Session's lifetime:

- `ToolRegistry` holds the Session's current mode, initialised from the configuration. A turn reads
  it when it starts; nothing re-adopts a tool set when it changes.
- `session.tool_mode.get` returns `{mode, effective}`: the mode the operator chose and the one in
  force. A mode whose entry tool is absent stays chosen and is not in force; that is the fallback.
  `session.tool_mode.update` follows `session.language.update`'s order: a failure before the record
  write changes nothing; once the record is written the live value is replaced, and a later
  cancellation keeps the new value stored, so the two do not diverge. The pair is advertised as
  `session.tool_mode`, like `session.language`.
- The model has no tool for this; it is a client method only.

```text
session.create.toolMode ──▶ SessionConfig.toolMode ──▶ ToolRegistry (live mode)
                                                          ├─ a turn pins the mode ──▶ visible ──▶ model request, usage
                                                          ├─ a top-level call checks `visible` ──▶ admit or refuse
                                                          └─ a child call checks `delegated` ──▶ admit or refuse
                     session.tool_mode.update ──▶ live mode + stored record
```

A member Session records the creating Session's current mode when it is created. Switching the
creator later does not change it: the member is its own Session with its own operator control.

## 6. Shell mode's tool

The product's local-execution profile does not mount the persistent shell today; `execute` exists
only in the minimal-execution profile, where `ShellTools` takes the Session's single `ToolHost`.

- The product profile mounts `ShellSession` (branch) and the shell tool in every Session, so a
  `shell` or `codemode` Session can use it. `ShellSession` starts no OS process until its first
  command, so an unused mode costs a registered part, not a PTY.
- `execute` becomes an ordinary registered tool: the `ShellTools` Session part registers its
  declaration and handler with `ToolRegistry` instead of providing `ToolHost`, and still resolves
  the calling branch's `ShellSession` per call. The registry's registration already carries what a
  host tool needs: declaration, handler, risk, mount-list filtering and per-Session lifetime.
- The minimal-execution profile is `shell` by construction: its host sets `toolMode: shell` when it
  creates a Session and when it restores one whose record lacks the field, because `normal` would
  filter the only tool it has. It binds `ToolHost` with an empty driver, because `ShellTools` no
  longer provides one.

Why not conditional mounting: a Session has one `ToolHost` (`WorkspaceTools` provides it in the
product), and mounting `ShellTools` instead of `WorkspaceTools` for shell Sessions would also remove
the client methods that part registers (`session.files` mention search, command-output paging), so a
shell Session would lose client features that have nothing to do with its tools.

Stopping a task is a separate route. Today `session.exec.stop` resolves the branch's
`BackgroundExec` and stops a native process; a live persistent-shell command belongs to
`ShellSession`, which provides the branch's `dyn ExecStop`. With `execute` in the product, both
kinds exist in one Session:

- The stop route resolves the live owner first: the branch's `ExecStop` for a live shell task, then
  `BackgroundExec` for a native process. A task neither owns is read from shared history for its
  terminal result; history never answers for a live task.
- Both parts stay mounted when the mode changes, so a task started in `shell` or `codemode` can
  still be stopped after the Session switches to `normal`.
- `session.stop` remains the turn's stop and stops no background task.

## 7. What the entry tools must provide

A mode's entry is one tool, so that tool must carry everything the mode promises. The items below
are its requirements; the ones marked missing are follow-up work, not part of this change.

`codemode`:

- Discovery and call (`tools.list`, `tools.describe`, `tools.call`) over the turn's pinned set,
  everything except `codemode`. Exists.
- **Content blocks on return.** A child result can carry image blocks (`read` of an image, an MCP
  result), and they reach the model only if the return path rebuilds them. When the returned value
  is a child result or its `content`, the host rebuilds the blocks it handed out; an image block is
  rebuilt only when it equals one a child result returned, so a program cannot point the model at
  pixels it never read. Text blocks always become text, and anything else stays JSON text. The
  existing output bound applies. Missing.
- **Async lifetime.** Work handed to its owner — a background `exec` or `execute` task, an `events`
  subscription, a `schedule`/`wakeup`/`loop` timer, a branch line, a member — is not tied to the
  child call or the program: cancelling the program does not cancel it, and its completion arrives
  as it does today. A call cancelled before it hands the work over does nothing. Mostly exists;
  verify and pin it.
- Asking the operator from inside a program: a child `ask_user` carries `parent_call_id` and is
  answerable through `session.question.respond`; the clients must show it under the program and let
  the operator answer it. Missing on the client side.
- Progress: each child call is a `step/tool-call`/`step/tool-result` event; the client groups them
  under the program row. The protocol exists (v4); grouping is a client feature.
- A failure manifest: a failed program names the child calls that ran and their effects, beyond
  today's "some child calls may have completed". Missing.
- Stop: the turn's stop and interrupt already cancel the program's children and return the
  may-have-run warning; a per-program stop that leaves the turn running is missing.
- Bounds on time, memory, calls and output. Exists.
- Approval wording: the outer approval says a program will call tools, and the child calls appear
  under it as they happen. Missing.
- The description exists; examples that teach discovery, loop bounds and partial results are
  missing.

`execute`:

- One shell per branch with cwd, environment, shell variables and jobs persisting across calls.
  Exists.
- Background work with a task id, an output path and a wake on completion. Exists; a title for the
  task list is missing.
- Foreground occupancy: a command still running at the timeout returns its output so far and keeps
  the shell occupied. Exists.
- Stop through the route in §6, so a shell task stops and writes `exec/ended`. Missing.
- Pageable output, and operator-visible state: the current directory, the running job, a command
  waiting for input. Output paging exists for native tasks; the state view is missing.
- The description exists; examples for the dialect, backgrounding and timeouts are missing.

The boundary is §2's: a shell Session cannot load a skill, read an image, call an MCP tool, open a
line, start a member, schedule or manage settings and plugins. A task that needs one runs in another
mode.

## 8. Clients

Both clients switch this Session's mode and remember the choice for the Sessions they create next;
there is no default row to set.

- TUI: `/settings` gains a Tools group with one row, "Tool mode", which calls
  `session.tool_mode.update`. The chosen mode is kept in the client's own settings and sent on the
  next Session creation.
- Web: the chat composer menu next to the permission menu switches this Session; the same choice is
  kept in the browser's local state and sent on creation.

```text
╭──────────────────────────────────────────────╮
│ Settings                           esc close │
│   Interface                                  │
│ › Theme                  System ›            │
│   ...                                        │
│   Tools                                      │
│ › Tool mode              Code Mode ›         │
│   Permissions                                │
│   Permission             Workspace write ›   │
│   Process isolation      ›                   │
│ ↑↓ move · enter select · esc close           │
╰──────────────────────────────────────────────╯
```

The picker lists all three modes; `normal` is always available, and `codemode` or `shell` is shown
as unavailable when its `hello.features` entry is absent. A switch takes effect from the next turn,
and the row shows the effective mode, so a mode that fell back to `normal` is not reported as in
force. A client that offers Goal disables `set` and `resume` while `shell` is in force (the
`effective` value) and shows the named refusal otherwise.

## 9. Compatibility

- Stored records: an absent `toolMode` is `normal`; `normal` is omitted when serialized, so the
  native store's text form does not change for existing Sessions.
- A fork copies its source's current configuration and therefore its mode.
- The v3 Code Mode gate stays. `excluded_tools` keeps `codemode` out of that Session's callable set,
  the fallback rule makes the Session `normal`, and a v3 client can neither select nor receive Code
  Mode ([child tool batches §2.6](appserver/child-tool-batches.md#26-app-server-code-mode-gate)).
- [CodeMode](plugins/CodeMode.md) changes in four respects with the implementation: the mode gate
  (offered in `codemode` mode, not to every v4 Session), the declaration set (`codemode` and `skill`
  instead of every adopted tool), the admission split (a child only inside `delegated`), and the
  outer result rebuilding content blocks instead of serializing the program value as JSON. Its
  child-event contract is unchanged.
- [Goal](plugins/Goal.md) gains the shell lifecycle in §2, and
  [PromptSources](plugins/PromptSources.md) gains the mode-aware environment block.
- The remaining contracts gain the mode with the implementation, each by its owner:
  `SessionConfig.toolMode` in [session access](contracts/session-access.md), the live mode plus
  projection in [ToolRegistry](../architecture/rust-agent/plugins/ToolRegistry.md), and the turn's
  mode pin in [Loop](../architecture/rust-agent/plugins/Loop.md).
- A Session restored from a record that predates this field becomes `normal`, except in the
  minimal-execution profile, whose host sets `shell` on create and restore (§6). A Session that used
  the `codemode` tool therefore loses it on upgrade until the operator selects `codemode`. That is
  the intended product change.

## 10. Verification

- Flow: create a Session in each mode (request field; absent = `normal`) and assert the request's
  declarations: `normal` declares every tool except `codemode` and `execute`; `codemode` declares
  `codemode` and `skill`; `shell` declares `execute`, `ask_user` and `recall`.
- Flow: in `codemode` mode a top-level call naming `read`, `ask_user` or `execute` is refused as an
  unknown tool even though the request pinned it for children, and a child call to `codemode` is
  refused as outside `delegated`. In `shell` mode a top-level call naming `read` is refused.
- Flow: a program returns a `read` image result; the outer result carries the same image block, and
  the model request renders it as an image, not as JSON. A fabricated image block falls back to
  text.
- Flow: a program starts a background `exec` and then fails or is stopped; the task keeps running,
  the Session wakes on its completion, and `session.exec.stop` stops it.
- Flow: a child `ask_user` is shown under the program and is answerable through
  `session.question.respond`; its result drives the program and is not duplicated into the model
  context unless the program returns it.
- Flow: switch a running Session in each direction; the next turn declares the new visible set, a
  request after a tool result in the same turn keeps the turn's pin, an in-process budget recovery
  keeps its pin, a process restart settles the in-flight request without reissuing it, and a call
  already admitted finishes.
- Flow: fallback — a mode whose entry tool is absent stays chosen in `get` and is reported as not in
  force; the Session declares the `normal` surface.
- Flow: a member created under `codemode` records it; switching the creator leaves the member
  unchanged.
- Flow: restore a record without the field, and fork a Session; both keep `normal` and the source
  mode respectively, except that the minimal-execution host restores its Sessions as `shell`.
- Flow: a goal with a queued continuation, switched to an effective `shell` before it is applied:
  the continuation is withdrawn, the goal pauses (`mode`) with its pause text, and no turn carrying
  the `goal_update` instruction starts in `shell`. Switched after it is applied, that turn pinned
  the old mode and finishes under it. The same settle runs when `execute` registers and makes
  `shell` effective without an update; a held continuation released in that window can reach one
  turn, whose `goal_update` call is refused. The pause is committed before the record write and the
  live mode, so a turn that starts in `shell` has the pause text in history; a cancelled update
  after the pause leaves the goal paused and the mode unchanged. `set`/`resume` are refused while
  `shell` is in force; a chosen `shell` that runs `normal` (no `execute`) neither pauses nor
  refuses; after leaving, the operator resumes the goal.
- Flow: stop a shell task with `session.exec.stop` after switching the Session to `normal`; the
  route resolves the live `ExecStop` before `BackgroundExec`, and neither answers for a live task
  from history. The minimal-execution profile's Session runs `shell` and keeps `execute`.
- Clients: a web component test for the composer menu and the remembered choice, and one tmux TUI
  case for the row and the picker.
- No event type or envelope changes. The stored configuration gains an optional field that old
  records omit, so the fixture compatibility baseline is untouched and old records load as before.

## 11. Rule check

- §1 Capability: no new Capability. `ToolRegistry` stays a Session part that publishes a tool set,
  `ToolBinding` stays the branch part ToolExecution owns, and the App Server gains one Session
  method pair and no business state.
- §4 rule 1: no module writes another owner's state; the registry owns the live mode, the App Server
  owns the record and the client method.
- §4 rule 2: no handover of a Timeline object.
- §4 rule 3: no `before` callback.
- §4 rule 4: the mode is a configuration value and the two tool names it maps to belong to the
  registry that applies them, not to core infrastructure.
- §4 rule 5: no batches.
- §4 rule 6: the visible set is exactly what the request declares; no new context path.
- §4 rule 7: parts coordinate through the Session configuration, the registry's read, and the Goal
  owner's settle operation (§2), which the App Server calls and the registry signals.
- §4 rule 8: no infrastructure change; the product profile's tool wiring changes (§6), while
  Timeline, Dispatcher, the event envelope, rutis assembly and the Session facades are untouched.
- §4 rule 9: the mode is Session configuration, not a Timeline fact, and follows
  `session.language`: the current value lives in the registry, the durable copy in the stored
  record.
- §7.1 for the live mode: the resource is the value; lifecycle is the Session's; the operations are
  `get` and `update`. A cancellation before the record write leaves the mode pair unchanged; a Goal
  pause already committed stays. After the record write, the new value stays stored and in force.
  Both report the effective mode rather than refusing one.
- §7.2: no object or resource handle crosses an owner; no new service, and the mode methods join
  the existing Session method face; the parameters are data.
- [ADR 002](../adr/002-plugin-scope.md) D1: the mode is Session configuration and the pinned mode is
  branch state. D2: no wiring module. D3: ToolRegistry keeps no hard dependency on Code Mode; an
  absent or excluded `codemode` falls back to `normal`. D4: business configuration, read at creation
  and changed by its owner's operation. D5: the mode methods take data.
## 12. Requirements raised to mechanism owners

Requirements go to other owners:

- **Goal.** Read the mode in force, the value `session.tool_mode.get` reports in `effective`, never
  the chosen mode. Settle on every transition to an effective `shell`: on an update, when the App
  Server calls it with the pending mode before the record write; on a registration or withdrawal, on
  the registry's published effective mode. Fan out from the Goal Session part to every branch with
  an active goal: write `paused` (`mode`), a pause text carrying the reason, and withdraw a
  continuation the InputQueue has not applied. While `shell` is in force, pause new continuations
  and refuse `session.goal.set` and `resume`; leave resumption to the operator after `shell`
  leaves. Its owner carries this into [Goal](plugins/Goal.md).
- **ToolRegistry.** Publish the effective mode to its readers so Goal settles on a registration or
  withdrawal transition; a pre-change signal that lets Goal settle before the projection flips
  closes the one-turn window in §2.
- **PromptSources.** The environment block follows the surface: while `shell` is in force (the
  `effective` value) it drops the Windows file-encoding guidance about `read`/`edit`/`write`. Its
  owner carries this into [PromptSources](plugins/PromptSources.md).
- **Loop.** Read the effective mode when the driving input is applied and hold it for the turn;
  its page gains the turn's mode pin. Its owner carries this into
  [Loop](../architecture/rust-agent/plugins/Loop.md).

Everything else stays within the owners that already exist: ToolRegistry projects its own set and
holds the live mode, the Loop holds the turn's mode pin, ToolBinding pins the callable set,
ToolExecution refuses a call outside the turn's sets, ContextUsage counts `visible`, the App Server
owns the stored record and the Session methods, and each client owns its control and its remembered
choice.
