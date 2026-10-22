# Read-only member conversations in the CLI

Base detail contract: accepted in [#300](https://github.com/arcships/dim/pull/300),
with its implementation in [#324](https://github.com/arcships/dim/pull/324).
Sections 1–6 describe that single selected-member view.

Sections 7–9 propose the remaining transcript-card slice of
[#263](https://github.com/arcships/dim/issues/263); this extension is **not Ready
or implemented**. Its observation lifetime extends the base contract only after
approval. Model-policy configuration remains separate business scope in #263.

## 1. Existing mechanisms

`collaboration/delivered` binds a delegation's request event to the actual member
Session. The task panel already retains that identity in #287. A member is a
Session, so the existing client methods can read it:

| Existing interface | Use |
| --- | --- |
| `session.attach {sessionId}` | Observe the selected member while keeping the leader attached |
| `session.history {sessionId,branchId,after,limit,through?}` | Read the member's main conversation through a fixed watermark |
| `session.event`, `session.model.progress`, `session.state`, `session.closed` | Update the member view through the existing Session subscription |
| `session.detach {sessionId}` | Release this connection's member observation |
| `session.stop {sessionId,branchId,reason}` | The existing explicit member interruption control |

The native [connection subscription](../../crates/dim-app-server/src/connection/subscription.rs)
stores attachments by Session ID; attaching a member does not replace the leader's
attachment. [History](../../crates/dim-app-server/src/connection/tree.rs) reads the
named Session and branch through the existing authorization and dispatch boundary.
Attaching can restore an unloaded member through the existing acquire path and
keeps it pinned while attached. This does not invoke the CLI's editable resume
path or apply `--permission` to that member.
No new server method, notice, Permission preset or core/rutis capability is needed.

The ordinary resume path is unsuitable for this view: on `rust-tui`,
`Worker::enter` can apply `--permission` to a resumed Session and makes it the
editable conversation. Viewing a member must not select that path, change its
policy or expose a composer for it.

## 2. Entry and layout

In `/tasks`, Enter opens output for a command or a conversation for a delivered
member. With an empty list filter, `x` invokes the existing stop control for either
kind; in a nonempty filter it remains ordinary filter text. The detail has no
filter, so its `x` control remains available. A starting member
has no Session ID: Enter or `x` explains that its target is not available yet and
sends no request. Do not derive an ID from an agent name or prompt.

```text
+-- Background tasks --------------------------------------------+
| > Survey the crates                                agent       |
| cargo build                                      command       |
| Enter details   x stop (empty filter)   Esc close              |
+----------------------------------------------------------------+

+-- Survey the crates - read-only -------------------------------+
| Member Session: <delivered Session ID>                         |
| user       Survey the crates                                   |
| assistant  I am checking the dependency boundaries.            |
| tool       read ...                                            |
| ...                                                            |
| x interrupt member   Esc back                                  |
+----------------------------------------------------------------+
```

The detail is a modal read-only view using the existing conversation row
renderers, wrapping, scrolling, theme and localization. It opens at the tail;
Up/Down, PageUp/PageDown and Home/End navigate the conversation. The existing
minimum-size screen applies. Sanitize externally supplied text before styling;
keep the identity, state and key hints within the overlay's width.

Enter and typed text in the detail never submit input. Esc is consumed by the
detail and returns to the task list; it must not fall through to the leader's
interrupt shortcut. The leader remains the active conversation, with its draft,
selected model, permission display and transcript position preserved. Its
notifications continue to be processed. Existing leader approval/question
prompts take precedence and close this observation through the same cleanup path.
The detail does not expose member approval, question or configuration actions.

The displayed history belongs to the delivered Session's `main` branch, not to a
synthetic transcript assembled from the leader's tool result. A reused member can
contain earlier or later turns; its Session ID stays visible. The leader's real
collaboration result determines the delegation outcome. A member being idle does
not establish completion of a particular delegation.

## 3. Read and observation lifetime

1. Keep the leader attached. Attach the selected delivered member on the same
   connection, then read its main history from `after:0`, using the existing page
   limit and cursor validation. The first page fixes `through`; every following
   page uses that watermark and the returned `nextAfter` until `done`.
2. Route the selected member's main-branch events, state and progress by Session
   and branch ID into a temporary instance of the existing conversation projection.
   Route `session.closed` by Session ID before handling a leader close; a member
   close marks only its detail closed and must not end the leader worker.
   Other branches do not supply this detail's transcript or running state. Fold the
   initial history before draining the member notices accumulated during it,
   discarding already folded sequences. Reuse existing progress offsets; progress
   is display data and does not create a history fact.
3. Send history requests without blocking the Session worker's handling of leader
   notifications and controls. At most one member-history page is outstanding;
   no periodic history polling or second connection is needed. Keep a single
   selected member view, not a catalog or another task tracker.
4. Leaving the detail stops further page requests and releases only its member
   attachment with `session.detach`. An admitted attach/page request settles;
   if the view closed while attach was pending, detach after that attach settles.
   Do not let an obsolete reply reopen or replace the current detail. Process a
   subsequent selection after the previous observation's cleanup; connection
   shutdown retains the server's existing attachment cleanup.

This temporary projection is client view state. It owns no task, member, durable
result or agent configuration. Detach does not stop or close either Session and
does not withdraw input. Removing a completed delegation from the running list
does not close an already-open detail; it remains bound to the same member until
the user leaves or another prompt replaces it.

## 4. Visible states and controls

| Condition | Display and behavior |
| --- | --- |
| Attach/history pending | `Loading conversation...`; Esc remains available |
| Successful complete history with no rows | `No conversation yet`; continue observing |
| Live member events/progress | Update the existing read-only rows, without changing leader state |
| Attach or history error | Show the error and any rows already read; stop loading/observing and detach the owned observation; reopening makes a fresh attempt |
| Member `session.closed` | Retain the rendered conversation with `Session closed`; no invented delegation result or automatic reopening of that closed Session |
| Explicit `x` | Existing `session.stop(member, main)` with the user-facing reason; an error uses the existing notice path |
| Escape or leader prompt replaces detail | Detach only the member observation and return to the leader's existing UI |

A stop reply does not fabricate a collaboration result or remove another member.
Stopping still targets the member's current turn: it cannot retract adopted input
and may race a later turn. This view adds no delegation-specific cancellation or
delivery fence. Detach errors use the existing connection error path; they do not
turn into a Session-close request or an automatic re-attach loop.

## 5. Implementation and verification

The affected CLI surfaces are the existing task panel/key hints, Session worker,
conversation projection/rendering and localization. Keep the leader projection
and the selected read-only projection distinct; reuse their canonical event/row
types. The server and Collaboration retain their current contracts.

Extend the existing CLI PTY flows to verify:

- The delivered ID selects member attach/history/stop, while the leader stays
  attached and neither Session receives input or configuration writes from the
  detail. Include `--permission` to prove viewing does not apply it to the member.
- History paging keeps its watermark and handles an event arriving during the
  read exactly once. Live text/state belongs to the selected member; simultaneous
  leader updates remain available after Esc.
- Starting and failed delivery, loading/empty/error/closed states, close during
  attach/history, an obsolete reply, a real delegation result while viewing,
  reopening, narrow wrapping and terminal-control text.
- Esc causes neither stop nor Session close, restores the leader draft and allows
  continued leader input. `x` keeps the existing current-turn semantics.

Reuse a real App Server Collaboration flow for member history plus two simultaneous
attachments and member-only detach; verify the leader still accepts input. These
are implementation acceptance requirements, not evidence that the proposed UI
already works. A documentation-only change runs no runtime test suite.

## 6. Rule check and ablation

| Rule | Application |
| --- | --- |
| §1 / ADR 002 D1 | Existing client process owns transient view state; native Session/branch owners keep their work |
| §4.1 commands/results | Read and observe through public methods; explicit stop keeps the existing owner path and results |
| §4.2 handover | No adopted input or accepted work is revoked by leaving a view |
| §4.3 before | No before callback |
| §4.4 core neutrality | No server or core behavior changes |
| §4.5 batches | No new Timeline writes or batch rule |
| §4.6 context | Client display adds no model-context path |
| §4.7 public access | Public client methods/notices only; no plugin internals or daemon file reads |
| §4.8 infrastructure | Existing attachment, paging, request settlement and connection cleanup; no extension |
| §4.9 necessary facts | Existing history and live progress suffice; no new persistent event |
| ADR 002 D2 | No adapter plugin, event bus, task registry or generic observation framework |
| ADR 002 D3 | Client depends on public server contracts; server never depends on the UI |
| ADR 002 D4 | No loader or business configuration is introduced |
| ADR 002 D5 | Existing JSON RPC/notice contracts; no language-specific service contract |

**Requirements raised to mechanism owners:** none. Existing Session observation
and history cover this requirement. Limitations of turn-scoped interruption remain
recorded in #263 instead of extending a core or cancellation protocol.

Ablation: retain one temporary view projection because replacing the leader's
editable projection would lose its view state or risk writing to the member.
Reuse the existing connection, attachment table, pagination and row rendering;
no extra output stream, polling timer, persistent copy, capability or lock.

## 7. Proposed transcript member cards

The leader's `agent` request block shows member identity/name, the requested task,
actual observed member model/activity, delegation state, elapsed time and the
real result summary. `agent` list/inspect/read/message operations keep their
ordinary tool presentation; a tool-call success is not a delegation completion.
This extension neither changes delegation execution nor adds a model policy.

### Sources and identity

Reuse the branch-local delegation identity and task records from the
[task list](background-task-list.md). Pair a request with its originating tool
call by leader branch plus `request_id` and `call_id`; identify the delegation
itself by its request event ID. Reused tool call IDs, request batches and member
Sessions must not merge separate delegations.

| Existing source | Card data |
| --- | --- |
| `collaboration/request` | Task prompt, request timestamp, target Agent/profile and originating call |
| `collaboration/delivered` | Actual member Agent/Session or the real delivery error |
| `collaboration/result` | Completed/failed/cancelled outcome, text and terminal timestamp |
| `agent.list {}` | Existing Agent IDs/names; use the delivered Agent identity, not a guessed alias |
| Member `session.history` and `session.event` | Existing decoded `context/configured` supplies the adopted member model; conversation facts supply activity |
| Member `session.model.progress`, `session.state`, `session.closed` | Live observed activity or explicit observation unavailability |

Temporary profile names already exist on the leader's request. For an existing
Agent, read the public Agent list after identities are known and share that
result across cards. A missing entry/read error shows the Agent ID rather than
an invented name; the delivered Session ID remains available in detail. Do not
read Agent records or private Collaboration state.

`session.list` is not the current-model source: its `model` comes from the
creation-time Session configuration, including in stored records. Requested
`profile.model` is a requested target, not a replacement for the member's
adopted configuration. Decode the existing member configuration facts once at
the wire boundary and show the latest observed provider/model. While loading,
failed or absent, state that no model has been observed; never copy the leader's
model or infer one from a model name. Model updates remain member observations,
not writes or configuration adoption by the CLI.

The model/activity is explicitly **member activity**, not a historical claim
about which model executed an old delegation. A reused member can run later
turns or receive Steer messages. Each delegation's outcome and elapsed end still
come only from its own leader-side facts. Member idle/close, a stop reply or a
later turn cannot complete or reopen that delegation.

### Observation lifetime

The base contract keeps one selected observation. For this extension, the
existing worker instead owns one observation per distinct delivered member
Session that has an unfinished leader delegation or is selected for detail.
Multiple delegations and the selected detail share that member observation;
they do not issue competing attaches/detaches. There is still one selected
read-only detail and one editable leader.

Reuse the existing attach/main-history/live-notice/detach flow, decoder,
conversation projection and progress offsets. Serialize member history pages
through one worker-owned member-page slot and fair pending queue, including
across leader attachment replacement. Attach/response handling is asynchronous;
a waiting member cannot block leader input, approval handling or other notices.
The task-branch history reader retains its separate existing scope.

History is folded through its fixed watermark before buffered live facts,
with the existing sequence and progress deduplication. Route by actual member
Session and main branch before touching a card/detail. Leader approval/request
and cancellation handling remains filtered by the leader Session. The CLI
neither answers member prompts nor registers another editable UI for a member.

Once the leader records a delegation's terminal result, release that member
observation if no unfinished delegation or selected detail still needs it.
Keep its last observed card metadata with the existing transcript projection;
do not persist another catalog. Restoring unfinished delegations rediscovers
their actual delivered Session and reads it again. Opening an ended member's
card reuses the base finite history view; an unavailable old Session is an
explicit local error, not a missing delegation result.

Closing only detail releases its selection, not an observation still needed by
an unfinished card. Ending the last reference settles admitted attach/page
requests and detaches only the worker-owned observation. Leader replacement or
connection teardown invalidates all owned member observations with the existing
attachment identity before old replies can change another view. This applies
when returning to the same leader or selecting the same member again.

A failed initial read retains validated data, marks activity/model unavailable
and is retried by explicitly opening/reopening the member view, as in the base
contract. Do not add polling, retry loops, a second connection, server reference
counts, a subscription service or a cancellation protocol. Session closure or an
unsettled request does not produce a fabricated terminal delegation result.

### Layout and entry points

```text
+-- Agent: inspect imports --------------------------------------+
| Task: Check crate dependencies                                 |
| Member model: provider / model     Running          00:42       |
| Activity: reading Cargo.toml                                   |
| [Open member conversation]                                     |
+---------------------------------------------------------------+

+-- Agent: inspect tests ----------------------------------------+
| Task: Review the integration coverage                          |
| Member model: last observed provider / model    Completed 01:12 |
| Result: Found two missing restart cases ...                    |
| [Open member conversation]                                     |
+---------------------------------------------------------------+
```

A card is still part of its original leader transcript, not a synthetic model
message. A delivery failure shows its real error and no actionable member
Session. The initial request shows Starting until actual delivery; delivered
unfinished work shows Running, independently of whether the member is between
turns. `collaboration/result` supplies Completed, Failed or Cancelled. Use the
request/result timestamps and existing UI tick; no stop/idle heuristic creates
an end timestamp. Stored result detail remains available without observing a
live member.

Use the existing canonical transcript renderer below the App layer. Collapsed
cards show the task's first display line, model/state and elapsed time. Expanded
cards show up to three wrapped task lines, the latest member activity line and
up to six wrapped result lines; indicate omitted text and keep the full real
result in detail. Do not copy the App renderer into the panel or introduce a
dependency from a lower renderer on the TUI application. Names, activity, model
labels and result text are sanitized before styling, wrapped at narrow widths
and kept out of the leader draft. Model/activity loading/error is distinct from
delegation failure.

In fullscreen mode, add member cards to the existing transcript keyboard focus
order alongside user messages. Tab/Up selects older items, Shift+Tab/Down newer;
Esc returns to the composer without modifying its draft. Enter on a focused
member opens the base read-only member detail. Editing a selected user message
with `e` keeps its current behavior; a member card cannot be copied into the
composer by that action. Ordinary text ends focus and returns to existing
composer input rather than controlling a member.

A fullscreen click on the card's conversation action opens the same detail,
using its stable delegation identity. Drag selection/copy and existing tool
group header expansion retain their behavior; a changing row height cannot
retarget an action to another member. Inline scrollback renders the same card
data and uses the existing `/tasks` keyboard path to reach member detail;
it gains no mouse or transcript-focus mode. Its historical scrollback rewrites
keep the existing terminal rewrite limits.

Both paths preserve the leader draft, viewport anchor and current configuration.
Esc from detail returns to the originating list/card focus without falling
through to leader interrupt. Per-member `x` interruption remains in the existing
detail/list; stop-all remains in the task list, not a new global card shortcut.
Leader approvals/questions preempt observation through the shared cleanup path.
When a card no longer has a delivered Session, navigation reports that condition
instead of opening an editable resume or another member.

### Model-policy boundary

The existing `agent` request accepts a temporary `profile.model`, and named
Agents carry their own profile model. Collaboration resolves that profile
through its existing model selection host. These do not constitute an exposed
user setting for inherit/fixed/agent-choice behavior, and `agent.list` returns
Agent summaries rather than a policy configuration.

This presentation extension displays what the member actually adopted; it does
not silently set a user policy or claim Provider's client-preference proposal
provides one. The remaining user-facing model-policy requirement stays in #263
with Collaboration as its business owner. Resolve it against existing profile
and Settings behavior before proposing any API or implementation; it is not a
request for a core, Prompt, Provider or rutis capability.

## 8. Transcript extension acceptance

Extend the existing PTY and actual two-attachment member flows. Cover two active
members, a reused member and a live model change, with distinct leader and member
models. Actual member `context/configured` must determine the displayed model;
the creation model and leader's configuration must not overwrite it.

- Pair repeated tool call IDs in different request batches and distinct requests
  sharing a member without merging their outcomes. Non-request `agent` actions
  retain ordinary tool output. Real result and elapsed time update the correct
  card even while detail is closed.
- Verify one attach per needed member, shared detail ownership, one member-page
  slot, history/live ordering, fair reads, failure/reopen, closed/removed members,
  leader replacement away/back and cleanup while requests are still admitted.
  No detached old reply or member approval cancellation can affect the leader.
- Keyboard focus/Enter and fullscreen click open the correct read-only Session.
  Preserve user-message edit/navigation, selection/copy, leader draft, scroll
  anchor and approval priority. Inline `/tasks` still reaches that member.
  Include multiple cards, growing progress, sanitization and 48-column layout.
- Only actual delegation facts set its end. Include cancelled reports that start
  another leader turn, later member work, a stop before delivery and unavailable
  observation. Keep the known case where no terminal collaboration fact arrives
  visible as unfinished/unavailable rather than inventing completion.

These are implementation gates for the proposed extension, not passing evidence.
The base #324 acceptance remains applicable only to its delivered scope. #263
stays open for this extension and model-policy configuration.

## 9. Extension rule check and ablation

### Requirements raised to mechanism owners

None. Public Agent metadata, member attachments/history/notices and addressed
stops already supply the view. The owner of the existing CLI member observation
must change its lifetime; no backend subscription, event or host port is missing.

| Rule | Application |
| --- | --- |
| §4.1 commands/results | Existing owner facts determine outcomes; existing stop handles control |
| §4.2 handover | Observation changes do not revoke accepted work or adopted inputs |
| §4.3 before | No callback or Timeline write |
| §4.4 core neutrality | No business case added to core, Rpc or Session facade |
| §4.5 batches | Per-branch order only; no cross-Session atomic snapshot or time correlation claimed |
| §4.6 context | Card metadata/progress never enters model context |
| §4.7 public access | Existing named RPCs, typed facts and public Agent summaries only |
| §4.8 infrastructure | Existing connection/response ownership and private attachment identity; no new mechanism |
| §4.9 facts | No event, persistent card index or copied configuration |
| §1 / ADR 002 D1 | Existing CLI owns derived display/I/O; Collaboration and model owners keep execution |
| ADR 002 D2 | Reuse the member observer and canonical renderer; no registry or wiring plugin |
| ADR 002 D3 | A failed member read affects its display, not leader availability |
| ADR 002 D4 | No loader option or new business setting; model policy remains its owner's scope |
| ADR 002 D5 | Existing JSON methods/facts; no Rust-only public interface |

Ablation: one member observation is shared by every card/detail needing the same
Session. The Session-keyed ownership set exists to prevent duplicate attach and
premature detach, not to create another task registry. A single selected detail
and the existing delegation projection remain. No progress-forwarding service,
periodic read, separate client tree, lock, generation protocol or server field
is needed. Actual model facts are read instead of inventing a default.
