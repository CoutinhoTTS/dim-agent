# Agent tool modes: normal, codemode and shell

Status: Draft. Scope: process (mode availability), Session (the mode and the tool surface),
branch (the mode a request pins).
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

In `codemode` and `shell` the ordinary tool declarations do not appear. That absence is a property
of the mode — the model acts through one surface — not the goal of the change. Fewer declarations
per request, fewer round trips and less context churn follow from it; the 23 tools declared today
carry about 12 kB of description and about 20 kB with schemas, but saving that is a result, not the
requirement.

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

| Mode | The model sees | How it works |
| --- | --- | --- |
| `normal` (default) | every tool the Session mounts, except `codemode` and `execute` | one tool call per decision |
| `codemode` | `codemode` only | one program uses `tools.list` / `tools.describe` / `tools.call` |
| `shell` | `execute` only | commands in one persistent shell |

Rules that hold in every mode:

- Approval and Permission are unchanged. Each call is admitted under the called tool's own risk,
  including a Code Mode child call; approving `codemode` approves no child.
- Tool sources are unchanged. MCP servers, extension tools and other registered tools still
  contribute to the Session's tool set; the mode decides what the model sees and, in `codemode`,
  what a program may call. It does not enable or disable a source.
- Client surfaces are unchanged. Mention search (`session.files`), command-output paging, skills,
  approvals and settings work in every mode.
- `shell` mode has no `ask_user` tool, so its model cannot ask the operator a question. It can still
  be stopped, and an `execute` approval still reaches the operator.

## 3. Who owns the mode

The mode belongs to a Session, and the operator owns it. A mode is a capability choice — `shell`
removes the file tools, `codemode` hides every other declaration — so nobody else may change it
behind the operator's back.

| Situation | Rule | Why |
| --- | --- | --- |
| A new Session from a client | starts in the mode the request names, or `normal` when it names none; the client sends the mode it last used | the operator should not re-choose for every Session |
| A running Session | the operator can switch it at any time; the next request declares the new mode, and a turn in flight keeps the mode it pinned | otherwise a change of style means abandoning the conversation |
| A branch (line) | has no mode of its own; every line of a Session runs the Session's mode | a line is the same agent; a line in another mode would hand the agent tools the operator excluded |
| A member Session | starts in its creator Session's current mode, and the operator can switch it like any Session | the member is the same agent doing delegated work; the model must not decide how the agent acts |
| The model | never chooses or changes a mode | it is the operator's choice of capability |

A switch does not rewrite the history the model already has: the next request carries the new
declarations.

## 4. The tool surface rule

The Session's tools are one set with one owner. `ToolRegistry` already owns the mount list and the
Session's tool set ([ToolRegistry](../architecture/rust-agent/plugins/ToolRegistry.md)), so it
also owns this projection. Four terms:

- **callable**: what the Session mounts: host tools, registered tools and each source's sets, after
  `SessionConfig.tools` and `excluded_tools`. The branch adopts and pins this set, and it does not
  change when the mode does.
- **entry tool**: `codemode` → `codemode`, `shell` → `execute`, `normal` → none.
- **exposed**: what a call of this mode may reach.
- **visible**: what the request declares to the model.

| Mode | exposed | visible | entry tool absent from callable |
| --- | --- | --- | --- |
| `normal` | callable − {`codemode`, `execute`} | same as exposed | — |
| `codemode` | callable | {`codemode`} | the Session runs `normal` |
| `shell` | {`execute`} | {`execute`} | the Session runs `normal` |

- Only `visible` reaches the model request, its token accounting and compaction. `exposed` never
  appears in a prompt, a declaration or the usage counter.
- A request pins the mode it was prepared under, together with the callable set it pins today. The
  declarations are the projection of that pinned set, so a switch takes effect with the next
  request and needs no re-adoption: the branch's configuration, the version pins and the source
  sets stay as they are. Each request is itself a `model/call` fact carrying its declarations, so a
  restore keeps what past requests declared and the next request projects with the current mode.
- A call outside the request's pinned `exposed` is refused as an unknown tool. The check reads the
  pinned mode, so a turn in flight keeps the mode it started with even if the Session switches
  meanwhile.
- A Code Mode program projects its list from the pinned set: `tools.list` and `tools.describe` are
  `exposed − {codemode}`, and `tools.call` still resolves the pinned handler with Permission and
  Approval ([CodeMode §1.4](plugins/CodeMode.md#14-tool-exposure-and-eligibility)).
- The mount list and an Agent profile's tool list narrow the surface and never widen it. `normal`
  hides the two mode tools even when a list names them.

Registration is dynamic, so the projection follows the current set and the current mode: a Session
whose entry tool is not registered, is withdrawn, or is excluded runs `normal` until it appears.
The two product clients also read which modes the daemon's assembly can serve: `hello.features`
carries `session.tool_mode.codemode` and `session.tool_mode.shell`, so a picker can mark an
unavailable mode instead of silently falling back.

## 5. Configuration, creation and switching

The mode is Session configuration; there is no daemon-wide default to maintain.

- `session.create` may name the mode; a Session created without one starts `normal`. The field is
  additive and advertised as `session.create.toolMode`, like `session.language`, because an older
  daemon rejects an unknown field.
- The App Server records the mode in `SessionConfig.toolMode`. `normal` omits the member, so a
  normal Session's stored configuration has the same bytes as before this field existed; an old
  record reads as `normal`. A fork copies the value, and a restore starts from it.
- The product clients remember the last mode the operator chose in that client and send it when they
  create a Session, so a new Session starts the way the operator last worked. That memory is client
  state; it is not a daemon setting.

The mode is live for the Session's lifetime:

- `ToolRegistry` holds the Session's current mode, initialised from the configuration. Requests
  read it when they are prepared; nothing re-adopts a tool set when it changes.
- `session.tool_mode.get` returns `{mode, effective}`: the mode the operator chose and the one in
  force. A mode whose entry tool is absent stays chosen and is not in force; that is the fallback.
  `session.tool_mode.update` writes the stored record (native storage only, as
  `session.language.update` does) and the live value. The pair is advertised as `session.tool_mode`,
  like `session.language`.
- The model has no tool for this; it is a client method only.

```text
session.create.mode ──▶ SessionConfig.toolMode ──▶ ToolRegistry (live mode)
                                                       ├─ a request pins the mode ──▶ visible ──▶ model request, usage
                                                       └─ a call checks the pinned mode ──▶ exposed ──▶ admit or refuse
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
- The minimal-execution profile, which has no other host tools, binds an empty host table.

Why not conditional mounting: a Session has one `ToolHost` (`WorkspaceTools` provides it in the
product), and mounting `ShellTools` instead of `WorkspaceTools` for shell Sessions would also remove
the client methods that part registers (`session.files` mention search, command-output paging), so a
shell Session would lose client features that have nothing to do with its tools.

## 7. What each mode still needs

This design fixes the unit of action and the tool surface. Making each mode good is separate work:
the items below are follow-up requirements, not part of this change.

`codemode`:

- A failure manifest: when a program fails, the result names the child calls that ran and what they
  declared, beyond today's "some child calls may have completed".
- Approval wording: the operator approves a program and then each child by its own risk, so the
  prompt must say that and show the child calls as they happen.
- Grouping: v4 carries `parent_call_id`; the TUI and web must group a program's children under one
  row, with their own detail.
- Asking the operator from inside a program, and a surface to answer it.
- Stopping a program and its children with a clear terminal result.
- Description and examples that teach the model when to call `tools.describe`, how to bound loops
  and how to report a partial result.

`shell`:

- A command title for the background task list (`exec` takes one; `execute` does not today).
- Command-level approval wording and output paging; the permission command rules already name
  `exec` and `execute`.
- Operator-visible shell state: the current directory, running jobs, and a command waiting for
  input.
- Description and examples for the dialect, background work and timeouts.

Both:

- A mode-aware environment block: on Windows it carries guidance about `read`/`edit`/`write` text
  encoding, which is noise in `shell` mode; the block should follow the surface the model has.

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

The picker lists all three modes; a mode whose `hello.features` entry is absent is shown as
unavailable and cannot be chosen. A switch takes effect from the next request, and the row shows the
effective mode, so a mode that fell back to `normal` is not reported as in force.

## 9. Compatibility

- Stored records: an absent `toolMode` is `normal`; `normal` is omitted when serialized, so the
  native store's text form does not change for existing Sessions.
- A fork copies its source's current configuration and therefore its mode.
- The v3 Code Mode gate stays. `excluded_tools` keeps `codemode` out of that Session's callable set,
  the fallback rule makes the Session `normal`, and a v3 client can neither select nor receive Code
  Mode ([child tool batches §2.6](appserver/child-tool-batches.md#26-app-server-code-mode-gate)).
- [CodeMode](plugins/CodeMode.md) and [child tool batches](appserver/child-tool-batches.md) change
  in one respect: the `codemode` tool is offered in `codemode` mode, not to every v4 Session. The
  child-event contract is unchanged.
- A Session restored from a record that predates this field becomes `normal`. A Session that used
  the `codemode` tool therefore loses it on upgrade until the operator selects `codemode`. That is
  the intended product change.

## 10. Verification

- Flow: create a Session in each mode (request field; absent = `normal`) and assert the request's
  declarations: `normal` = the mounted tools without `codemode`/`execute`, `codemode` = only
  `codemode`, `shell` = only `execute`.
- Flow: switch a running Session in each direction; the next request declares the new visible set,
  the turn in flight finishes under its pinned mode, and a line created before the switch uses the
  new mode at its next request.
- Flow: in `codemode` mode, a program calls a tool the model does not see; the child
  `step/tool-call` and `step/tool-result` carry `parent_call_id`, and Permission admits the child by
  its own risk. A call outside the pinned `exposed` set is refused as an unknown tool.
- Flow: fallback — a mode whose entry tool is absent stays chosen in `get` and is reported as not in
  force; the Session declares the `normal` surface.
- Flow: a member created under `codemode` records it; switching the creator leaves the member
  unchanged.
- Flow: restore a record without the field, and fork a Session; both keep `normal` and the source
  mode respectively.
- Clients: a web component test for the composer menu and the remembered choice, and one tmux TUI
  case for the row and the picker.
- No event type, envelope or stored format changes, so the fixture compatibility baseline is
  untouched.

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
- §4 rule 7: parts coordinate through the Session configuration and the registry's own read.
- §4 rule 8: no infrastructure change; Timeline, Dispatcher, the event envelope, assembly and the
  Session facades are untouched.
- §4 rule 9: the mode is Session configuration, not a Timeline fact, and follows
  `session.language`: the current value lives in the registry, the durable copy in the stored
  record.
- §7.1 for the live mode: the resource is the value; lifecycle is the Session's; the operations are
  `get` and `update`, whose cancellation stops the record write and which report the effective mode
  rather than refusing one.
- §7.2: no object or resource handle crosses an owner; no new service, and the mode methods join
  the existing Session method face; the parameters are data.
- [ADR 002](../adr/002-plugin-scope.md) D1: the mode is Session configuration and the pinned mode is
  branch state. D2: no wiring module. D3: ToolRegistry keeps no hard dependency on Code Mode; an
  absent or excluded `codemode` falls back to `normal`. D4: business configuration, read at creation
  and changed by its owner's operation. D5: the mode methods take data.
## 12. Requirements raised to mechanism owners

None. The change stays within the owners that already exist: ToolRegistry projects its own set and
holds the live mode, ToolBinding pins the mode with the callable set, ToolExecution refuses a call
outside it, ContextUsage counts `visible`, the App Server owns the stored record and the Session
methods, and each client owns its control and its remembered choice.
