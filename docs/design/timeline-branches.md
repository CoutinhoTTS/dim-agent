# Timeline branches

Status: Accepted ([#427](https://github.com/arcships/dim/issues/427)). This document holds both the requirements and the design: §1–§3 answer #427's questions, and §4 onward is the design. Scope: the TimelineTree plugin (TimelineTree, TreeTimeline, BranchTool, plus a new branch part for BranchTool), two App Server built-in methods, three core events, the CLI TUI, and a rewrite of [design rules §2](../architecture/rust-agent/principles.md#2-作用域的含义). Language: Rust. Chinese: [中文](../zh/design/timeline-branches.md).

## 1. What branches are for

A **branch** is a line of the same agent inside one Session. The Session is one subject, the "I": its identity, its interaction surface with the user, its capabilities and its workspace. A branch is one line of that subject's behavior.

A child starts from its parent's history at a cut. Every event up to the cut is the child's own history as well, and those rows never change. After the cut, the child reads its own events, and through the `branch` tool it reads the line it was cut from (§6).

Lines share facts, not context:

- Each line assembles its own context from its own facts.
- Each line compacts on its own.
- A line influences another line by writing input to that line's Timeline, and it learns about another line from its own reads of that line.
- A rule or state change an ancestor appended without its delivery input consumed in the inherited prefix is not part of the line's inherited state: the line adopts the source's current value itself, and that block is its own event after the cut ([Prompt](../architecture/rust-agent/prompt.md)).

The parent–child relation records where a history came from. It is also the only relation along which lines talk and read. It does not mean control, and it does not mean that a result is owed.

Answers to #427:

| # | Question | Answer |
| --- | --- | --- |
| 1 | Which scenarios, in what order | Three, in order (§3): plugin lines (delivered), lines the agent opens from the primary line (this design), and lines the user opens from a history point and then compares and adopts (later) |
| 2 | Branch versus Collaboration member versus fork | §2. A branch is the same agent. A member or a fork is another Session |
| 3 | When a branch finishes, who needs its result, and in what form | No line owes another a result. Lines tell each other what matters while they work (§6). A line the agent opened ends with a final message to the line that opened it (§10). A plugin line follows its creator's protocol, for example Pursuit's conclusion |
| 4 | When is a branch over, and is it visible afterwards | When it ends itself, a line above it ends it, or the user ends it (§10). An ended line stays in the branch directory. Its history stays readable to clients and to the line that opened it |
| 5 | What users see and do | Follow any line, talk to it, answer its questions and approvals, and end it (§9, §14). Compare and adopt come with scenario 3 |
| 6 | What "done" means | The acceptance flows (§20) pass, and the evaluation scenarios (§12.3) have been run locally on real models and reported in the implementing PR. The release demo ([#439](https://github.com/arcships/dim/issues/439)) can show it |

## 2. Branch, Collaboration member, fork

| | Branch | Collaboration member (subagent) | Fork |
| --- | --- | --- | --- |
| What it is | Another line of the same agent, in the same Session | Another agent with its own profile, in its own Session | A new, independent Session copied from `main` |
| History | Its parent's history up to the cut, then its own | Starts from its task, without the leader's history | `main`'s history up to a rest point, copied ([fork](appserver/session-deletion-and-fork.md)) |
| Identity and capabilities | The Session's: the same system, tools, model, permission and workspace | Its own profile: instructions, tools and model | A copy of the configuration, independent from then on |
| Relation | Parent and child write input to each other while they work. There is no task contract and no control | The leader gives it a task, and the result returns automatically. Members cannot delegate | None after the copy |
| Use it when | The same agent works in parallel on the same facts | The work needs another identity, another profile or a context of its own | Work should continue elsewhere from a past point without touching this Session |

The rule from design rules §2 still holds. When the work needs a different identity or different capabilities, it becomes a new Session (a member or a fork), never a branch.

## 3. Scenarios

| # | Scenario | Status |
| --- | --- | --- |
| S1 | **Plugin lines.** A plugin opens a line under its own design. Example: [Pursuit](plugins/Pursuit.md)'s evaluator, a plugin line cut at the worker's turn end, whose conclusion goes back as input (implemented, [#422](https://github.com/arcships/dim/issues/422); `crates/dim-goal`) | Delivered. This design adds the identity block (§7.1). Goal's recognition and end of the evaluator change with it ([Pursuit §13](plugins/Pursuit.md#13-ending-the-evaluator-with-427)) |
| S2 | **The agent opens lines from the primary line.** Examples: test two hypotheses for a bug on two lines while the primary line keeps reading; implement on the primary line while a line reviews each change as it lands; research an API or one area of the code while the primary line continues; try a second approach on separate files | This design |
| S3 | **The user opens a line from a history point, compares it, adopts it.** `session.branch_create` already opens lines. With this design the user can follow those lines, talk to them, move the primary line to one of them and end them. Compare and adopt need a design of their own: what adopting a line means for the primary line's context and for a shared workspace | Later, after this design |

These needs are not branches:

| Need | Why it is not a branch | Where it belongs |
| --- | --- | --- |
| Delegate to a specialist with its own instructions | It is another identity | Collaboration member |
| Judge whether to approve a tool call | It is a decision about one action, not a line of work | A capability with a host port ([ADR 003](../adr/003-approval-scope.md)) |
| Rewind the conversation to an earlier point | It changes how the line sees its own history; it is not a new line | A separate requirement. Today the answer is `session.fork` |
| Make edits in parallel, in isolation | Lines share the Session's workspace, and no per-line workspace exists, which is also why two lines must not write the same files | A fork, or a member with its own working directory |
| Run one long command, or wait for something outside the session | A single command is background work, and an external event is a signal the current line can wait on; neither needs a second line of reasoning | A background command, or an event subscription |
| Pursue one task to completion, however many turns it takes | That is continuation of this line, not a second line beside it | This line, or a goal |

## 4. Tree and roles

- **Depth.** The architecture allows lines at any depth. `session.branch_create` takes any parent. Goal accepts any `branchId`, so an evaluator opened under a child is a grandchild.
- **Kinship.** Parent and child is the only relation along which lines communicate. A parent and each of its children write input to each other, and a line reads the line it was cut from (§6). Siblings and more distant relatives do neither. The rule covers one line acting on another, which here means the `branch` tool. A capability that decides on its own is a capability affecting a branch, and this rule does not limit it ([design rules §2](../architecture/rust-agent/principles.md#2-作用域的含义)). Examples are Goal writing an evaluation result, and a plugin opening a line at any cut.
- **The root.** `main` is the line with no parent and no source: the literal id, its numbering and every stored `branch_id: "main"` are unchanged by this design. It never ends, and the Session's close is not an end. In the UI it is 起点.
- **The primary line.** The line the Session's conversation is on: the target a client names when it omits `branchId`, and the line a client centres. It is a role, not an id: it starts as the root and moves only by a user action (§9, §14). One authoritative non-model fact records it, `branch/primary {branch}`, written on the root line, which outlives every other line and cannot end (§16). A keyed block that tells a line about the change is never the authority. A Session with no such fact is on its root.
- **An opener.** Any line that has opened lines coordinates them: it carries its own roster (§7.1) and is the only line that messages, reads and ends the lines it opened. Coordination follows the parent-child relation and never moves with the primary line.
- **The agent's tool.** The `branch` tool opens lines only from the current primary line. A line that is not the primary line gets a refusal, and a line the tool did not open is never on a roster and never becomes primary. The depth of the tree grows only when the user moves the primary line to a line that opens lines of its own. The total number of open lines the agent opened in a Session is capped (§6.1).
- **Sources.** Every line has a source, recorded in its parent's `branch/create`:
  - A **tool line** was opened by the `branch` tool.
  - A **user line** was opened by a client through `session.branch_create`. Its opener is the user, and it is a conversation line: it can become primary and, when it is primary, open lines.
  - A **plugin line** was opened by a plugin (`TreeCommands.create`). Its creator owns its protocol, and it can neither become primary nor open lines.

  The source can be read as `TimelineInfo.source`, returned for active lines by `TreeCommands.list` and `info` and absent on `main` (§5). Ended directory entries may omit it; opening their display detail reads the creation source at its known position. A plugin line's source is `{kind: "host", operation_id}`: the opener's opaque tag, which the core never interprets. A user line's source is `{kind: "user", operation_id}`, its own kind, so a reader that must tell a conversation line from a plugin line does not parse an `operation_id`. An opener finds its own lines by its own operation-id convention, for example Goal's `goal:evaluate:` prefix.
- **Reach of the tool.** The tool's `read` action reads the line the caller was cut from and the lines it opened; `message` and `end` work on the same pairs. A plugin line is not on any roster, and the tool neither messages it nor ends it.
- **The parent block.** A line's BranchLine writes one keyed `<dim-branch-parent>` block on the line itself when the line it was cut from ends a turn and has new activity past the line's last read, the last head the line was told and the line's opening (§7.1.1, §12.1). The block reports positions and is state, not a request; the content stays in the pages a read returns.

| Line | Opens lines with the tool | Messages and reads, with the tool | Ended by, by convention |
| --- | --- | --- | --- |
| `main`, the root | While it is the primary line | The lines it opened | Never; the Session's close is not an end |
| The current primary line | Yes | The line it was cut from and the lines it opened | The user, the line that opened it, or an ancestor's end; never itself |
| Any other conversation line (tool or user line) | No (the tool result refuses) | Its parent and the lines it opened | Itself (`end`), the user, or an ancestor's end |
| Plugin line | No | — (its creator's protocol) | Its creator (Goal ends its evaluator, [Pursuit §13](plugins/Pursuit.md#13-ending-the-evaluator-with-427)), the user, or an ancestor's end |

Who ends which line is a convention among callers. The runtime checks only the target: it exists, it is open, and it is neither the root, the current primary line, nor an ancestor of the primary line, because ending a line ends every line below it (§10).

## 5. Parts

Every part belongs to the TimelineTree plugin ([timeline_tree.rs](../../crates/dim-agent-capabilities/src/timeline_tree.rs)), and each sits in the scope that owns its state ([ADR 002](../adr/002-plugin-scope.md) D1).

| Part | Scope | Capability | Owns in this design |
| --- | --- | --- | --- |
| TimelineTree | Session | No | The line lifecycle: the identity block in every child's opening input, `TimelineInfo.source`, the primary line and its `branch/primary` fact, the end of a line (`branch/end` → `branch/ended`), refusing input once a line is ending, the restore rule, reads of an ended line's history, and the branch directory (§14) |
| TreeTimeline | Branch | Yes | Routes `branch/end` to TimelineTree, as it routes `branch/create` today. For an end issued by a tool call, it also routes the turn end that the close waits for |
| BranchTool | Session | No | The action-based `branch` tool (§11). It finds the calling line's BranchLine per call, the way WorkspaceTools finds FileReads |
| **BranchLine** (new) | Branch | Yes | One line's view of its lines. Its role (root, primary, opener, plugin line) and its parent come from `TreeCommands.info(own id)`. From its own facts it folds its open lines and the state of its current turn. It writes its roster, the idle notice, the ended notice and the status note a move of the primary line gives it |

BranchLine injects `B:Dispatcher`, `B:TimelineIdentity`, `B:BranchLifecycle` and `S:TreeCommands`, and provides `B:BranchLine`.

It decides the way Goal does: from consumed state, through an owned write task that rechecks before writing, and only on live events after its load cutoff. A reload therefore writes nothing twice, with one exception: a load whose own history ends with a live roster key the fold does not match writes the roster once, because a restart that completed an ended line (§20.7) recorded `branch/ended` before the opener's replay, so no live event follows it (§6.2).

**Line sources and restore.** At creation TimelineTree uses the accepted parent branch/create. At restore it first reads lifecycle kind/positions through existing indexes. An ended line retains its directory identity, parent, head and end position without loading creation or end bodies. Only an unfinished close reads its own branch/end data to complete the required parent confirmation. Only a line that will mount reads its creation source and verifies the opening input. Parent-child adjacency orders the directory once; restore does not repeatedly search all remaining lines. An ancestor's completed end closes its started descendants without mounting them. No branch-state snapshot or new native format is added.

**Restoring the primary line.** A Session whose directory holds one line is on its root and reads nothing for it. A Session with more lines reads one fact: the newest `branch/primary` or `session/forked` among the root's own events, through the existing `timeline_events_by_kind` index. A `branch/primary` whose target is still open names the primary line; a `session/forked` means the primary line is the root, because every `branch/primary` the fork copied was written before its `session/forked` and is void; nothing recorded means the root. A target that is no longer open falls back to the root, and so does a `branch/primary` naming a plugin line.

An explicit creation API `at` is a fixed sequence, including when it equals the
parent's current head. Its fact records `at={kind: "sequence", sequence: at}`;
parent activity while validation awaits cannot widen the inherited prefix.
A `before` producer may still explicitly choose `at={kind: "create"}`, whose
existing meaning is the creation fact's own position.

Ended display details read purpose/source and end data at their recorded positions. The optional historical cut message count is omitted; displaying a branch does not count all preceding user inputs.

## 6. Messages between lines

A message is an `input/message` on the receiver's Timeline, written through `TreeCommands.input`. No new input path is added. A message carries one raw block, marked by its producer and without a key, and the mode of [InputQueue](../architecture/rust-agent/plugins/InputQueue.md):

| Mode | Effect on the receiver | When to use it |
| --- | --- | --- |
| `append` (default) | Rides with the receiver's next model request. It never starts a turn and does not wake `wait` | Progress or context the receiver should have at its next step |
| `steer` | Enters the receiver's current turn at its next step, or starts a turn if the receiver is idle. It wakes `wait` | The receiver must act on it now, or the receiver is idle |

Queue is not exposed. The only Queue input one line gives another is the opening input.

These are Collaboration's message modes and words (`append`, `steer`), with the same meaning, because both are InputQueue's modes.

- **Marker.** `<dim-branch-message from="C" to="P">`. The receiver tells a message from the user's words by its marker. RPC accepts only operator content, never blocks, so a client cannot forge a message.
- **Allowed pairs.** Messages and reads: a line and the lines it opened, in both directions, and a line and the line it was cut from, in both directions. Coordination follows the parent-child relation, so it stays with the line that opened a line and never moves when the primary line moves. Any other pair is refused through the tool result, with nothing written: siblings, a line and its grandparent, a plugin line in either direction, ended lines. The refusal codes stay `branch_not_kin` and `branch_not_tool_line`.
- **Read.** `read {branch, after?, limit?}` returns one page of the target's committed history, open or ended, read from the store through `TreeCommands.page` and rendered the way Collaboration's `read` renders a member's page: inputs, text, tool calls and results, and turn ends, with `nextCursor`; a tool result item carries the effects its tool declared on that result (session boundary A.6), present only when the result declared at least one, a path verbatim and a command cut at its 200th character with the cut marked. The target is the line the caller was cut from, or a line the caller opened. One cursor rule covers both directions:
  - With no `after`, a read continues after the caller's last successful read of that line: the `nextCursor` of the latest read that returned a page. With no earlier read, a line's first read of its parent starts at its cut, and a line's first read of a line it opened starts at that line's first event.
  - An explicit `after` reads from that point. For a parent read, a value below the cut is raised to the cut, because the events up to the cut are already the line's own history. For a read of a line the caller opened, the value is used as given, so `after: 0` re-reads it from its start.
  - Every successful read moves the cursor to its `nextCursor`, an explicit-after read included; the latest read wins. The cursor is a fact of the caller's own line: BranchLine retains live read results in memory; after a reload it locates the latest successful read for the requested target only when a cursor is used, then reads that request's result fields. Resume does not replay tool history.
  - A page may end in the middle of the target's turn: it reads up to the target's published head, and the next read continues after that page.
- **Knowing when to read.** A read is on demand, so a line also needs to know its parent moved. The keyed `<dim-branch-parent>` block (§4, §7.1.1) states the parent's head and the line's cursor; the line decides which pages to read from it. The block never carries content, and reading it is not reading the parent.
- **No caps.** There is no limit on the number of lines, messages or steers. A message loop ends through the prompt rules (§12), through a line ending itself, or through the user (interrupt or end). Why this is safe, and who would own limits, is in §6.1.
- **Idle notice.** When a tool line's turn started in this generation ends, BranchLine on that line writes one steer to the line that opened it (`<dim-branch-idle>`, §12.1), unless one of the following holds:
  - the line was created with `idleNotice: none`;
  - the line is the current primary line;
  - the turn already sent its opener a steer message;
  - the turn called `end`;
  - the line has a driving input pending.

  This way a line that stops without telling its opener never leaves the opener waiting on silence, and a line that already steered its opener does not notify it twice. The notice carries the turn's status (`completed`, `failed` or `cancelled`).
- **Waiting.** A line waits for its lines with the existing `wait` tool. A steer message, an idle notice or an ended notice wakes it ([wait.rs](../../crates/dim-agent-capabilities/src/wait.rs)).

### 6.1 Resources and limits

Leaving a cap on messages out is safe because a line adds no new kind of resource:

- A line's work is ordinary turns of the same Session, under the same provider, Permission and Approval.
- A line runs at most one turn at a time. Loop holds one turn per line ([agent_loop.rs:106](../../crates/dim-agent-capabilities/src/agent_loop.rs)).
- A message is one bounded `input/message` written by a tool call, so the model's own output bounds it.
- RPC admission is bounded per connection ([architecture](../architecture/README.md)).
- Collaboration has no member cap either.

Open lines are capped. The cap is a total over the Session: the lines the `branch` tool opened that are still open, counted across the whole tree, not per roster and not from the root's lines alone. A plugin line and a user line do not count. BranchTool owns the limit and refuses `create` with its own code when the total is reached, so the model reads a refusal it can act on instead of a line that fails to open. The check runs inside the ancestor walk a creation already does, under the same tree lock, so two lines creating at the same time cannot both pass it. The depth of the tree is not capped separately: it grows only when the user moves the primary line, and the total bounds the work either way. The number is set by measurement (§12.3) and carried in the implementing PR.

If further limits are ever needed, each one already has an owner, and none of them needs a mechanism change:

| Limit | Owner |
| --- | --- |
| Open lines | BranchTool, as a total over the Session, checked under the tree lock at creation |
| Steer frequency | BranchLine on the sender counts the steer messages on its own Timeline |
| Model cost or concurrency across a Session | The model-call layer. This limit is not specific to branches: it applies equally to members and forks |

The other safeguards are:

- the prompt rules (§12);
- ended lines refusing messages;
- the user's end and interrupt;
- the flooding, over-splitting and ping-pong scenarios in §12.3, measured locally and reported in the implementing PR, never as CI thresholds.

What open lines do cost:

- An open line keeps its loaded parts.
- Every open line is reloaded on restart. Neither Session close nor idle unload ends a line.
- Open tool and user lines grow the roster of the line that opened them, which lists only its open lines. Each end of such a line from outside it adds one notice on its opener.

A line that never ends is the responsibility of its own protocol (`end`) and of the user, and its opener's roster keeps it visible. Goal ends Pursuit's evaluators ([Pursuit §13](plugins/Pursuit.md#13-ending-the-evaluator-with-427)).

### 6.2 Known costs

- **A restart can lose an idle notice.** If the process stops between a tool line's turn end and its idle notice, the notice is never written, because BranchLine decides only on live events after load. Pursuit §4.2 leaves the same window open. The opener still has the line on its roster and can `read` it, and the user sees it idle.
- **A restart can lose the ended notice.** The same window between `branch/ended` and the roster input drops the ended notice a line's outside end would have written. The load heal writes the roster alone, so the opener learns it from the roster's retirement rather than from a notice.
- **A roster is short after compaction.** After a line compacts, its roster keeps only each line's id and the first line of its purpose. What the opener told a line survives only in the compaction summary or through `read`, and `read` brings that line's raw events into the opener's context, one page at a time.
- **A line that became primary keeps its history.** Moving the primary line to a line the agent opened changes what the line is told from then on (§9), but nothing it already recorded. Its earlier identity block and its purpose stay in its history, and the status note that says it is primary now restates that identity under the same key, purpose and end rule included (§12.1), so the two never disagree and a compaction replays the note.

## 7. Identity and context

### 7.1 Keyed state: `branch:line`, `branch:roster` and `branch:parent`

Every line answers "which line am I" with one block under the key `branch:line`. The key is the same on every line. A line that has opened lines also answers "which lines are mine" with a block under its own key, `branch:roster`. The two keys are separate because every line can now carry both: a line that opens a line and is not the root has an identity block and a roster, and one key would let a roster write retire the line's own identity. A line also keeps one keyed block, `branch:parent`, for the line it was cut from (§7.1.1).

- **On a child: its identity.** TimelineTree writes it on every child, tool line, user line or plugin line. It is the first block of the child's opening Queue input, keyed `branch:line`, and it states:
  - the child's id, its parent and the cut;
  - that the child is the same agent and the history above is its own;
  - that it works in parallel from here on;
  - that messages above addressed to the parent, such as the parent's roster, belong to the parent.

  A tool line's identity also carries its purpose (the `create` prompt) and the line protocol. For `idleNotice: none`, TimelineTree includes `idle-notice="none"` on that opening identity. BranchLine reads it from the line's own opening input when an idle notice is considered; resume does not replay inputs. A missing attribute selects `steer`, including histories written before the option existed. The setting controls only automatic idle notices. On a user line or a plugin line, the opener's prompt stays the opening input's `user` content, after the block.
- **On an opener: its roster.** BranchLine on a line that has opened lines writes a keyed Append, `branch:roster`, from that line's own facts:
  - `branch/create` of a tool line or a user line;
  - `branch/create-failed`;
  - `branch/ended`.

  The roster lists each open line it opened with the line's id and the first line of its purpose, and is retired when none is open. A plugin line is never on a roster, so a Goal evaluator never appears in one.
- **Superseding.** The two keys retire independently. A child inherits its parent's roster in its prefix, and its own roster, when it writes one, replaces that inherited block under its own key; its identity never touches the inherited roster, and the inherited roster never retires its identity. From the child's opening input on, the latest live `branch:line` block on the child is its identity and the latest live `branch:roster` block is its own roster.

Key and mode are independent ([history.rs](../../crates/dim-agent-capabilities/src/history.rs) `render_input`). Goal's `set` already puts keyed blocks in a Queue input ([Goal §5](plugins/Goal.md#5-context-mapping)).

The identity is part of the opening input, not an Append written at `branch/start`, for two reasons:

- An Append can commit after the opening input ([Goal §3.4](plugins/Goal.md#34-a-new-branch-clears-the-inherited-goal)).
- The purpose is not known at `branch/start`.

### 7.1.1 The parent block, `branch:parent`

A read is on demand, so a line must also know that the line it was cut from moved. BranchLine on that line watches its parent's turn ends and writes one keyed Append, `branch:parent`, on its own Timeline. It is the line's own fact and never the parent's: no part folds another line's events.

- **The signal.** TimelineTree already routes every line's `turn/end`; the line's node publishes the sequence of its latest turn end. A child's BranchLine waits on its parent's signal and on its own write queue, and a change queues one decision. The decision is the line's state, not the signal: whenever the line holds no `branch:parent` block and the parent's head is past the head the latest block told, it writes one. A move that arrived while a block was held is written when that block is consumed, deleted or interrupted, so no move is lost. No part reads the parent's history for this.
- **The condition.** The block is written only when all of these hold:
  1. the line is not the root and is not ending;
  2. no earlier `branch:parent` input is still held on the line, meaning committed and not yet consumed;
  3. the parent has new activity past `from = max(A, told, opened)`, where `A` is the line's read cursor for the parent (its cut when it has not read yet), `told` is the head the latest `branch:parent` block on the line reports (0 when there is none), and `opened` is the sequence of the line's `branch/create` event on the parent, at or above every cut. Activity from before the line existed is not news, and the line reads it when it needs it.
- **What counts as new activity.** Two kinds of event count: an operator input, an `input/message` that carries `user` content, and a tool call. The creating call's `step/tool-use` is written before `opened`, so it is never counted. Nothing else counts on its own: the parent's text, its turn ends, block-only inputs such as a roster and messages from lines, and the creating call's result. The rule is that a line is told when the parent acts or when the operator tells the parent something, and never for the turn that only opened it (its creating call's result, its text and its turn end), which would make every line read its parent for nothing and train it to ignore the block.
- **The check.** The parent's facts are read through the one event query ([ADR 019](../adr/019-event-query.md)): the branch's `input/message` and `step/tool-use` events, newest first, paged until one qualifies or the read reaches `from`. No other line's events are folded.
- **The write.** One keyed Append, `branch:parent`, written on the line itself through `TreeCommands.input`, with the parent's id, `head` and `after` as attributes. A later block of the same key replaces the earlier one in the keyed baseline. After the write the fold records the new head and the new input, so a second decision in the queue does not write again.
- **At open.** When a child's inherited prefix holds a live `branch:parent` block, the child's opening input carries one more raw block, after the identity block: keyed `branch:parent`, live and not retired, naming the line that opened it P with `head` and `after` both the cut H. It supersedes the inherited block the way an identity supersedes an inherited one (§7.1), so the line's latest `branch:parent` always names its own parent and its `told` starts at H, which equals `opened`; a compaction replays this block and never the one above it, and the usual block supersedes it when P moves. Only a nested line gets it: a line cut from the root inherits no `branch:parent` block, so its opening input is byte for byte what it was. TimelineTree composes it where it writes the identity block, and reads whether the key is live from the keyed state of the prefix the child inherits, never from the rendered history. It reads that state while it opens the child, as it reads a resumed line's facts; a failed read fails the create like any other opening failure (branch/create-failed).
- **What follows.** A running line gets the block with its next model request. An idle line is not woken: the block is consumed with the first request of its next turn, ahead of the input that starts that turn, which is the documented Append rule ([session access](contracts/session-access.md), [Loop](../architecture/rust-agent/plugins/Loop.md)). A reload writes nothing; a restart writes at most the one block that was due. The root gets no block: nothing is above it.
- **The record.** The line sends the System its configuration holds. The guide is the same on every line (§7.3), so a line whose System did not move from the one it inherited references the configuration that recorded it, on whichever line of its chain recorded it, and a line publishes its own `context/configured` when its System moves, as any branch does.
- **Rejected alternatives.**
  - Writing at the line's own turn end misses the case the change exists for: the line ends a turn, the parent moves on, and the user or the parent then talks to the line. That turn carries no block.
  - Writing at the line's turn start or between its requests reacts to an event after the request was prepared; a committed Append can land after the request it was meant for, and it would need a hook in Loop.
  - The root pushing a summary or a steer to every line duplicates content the line can read and wakes idle lines.
  - A new event kind or a stored format change is unnecessary: a raw keyed input already carries the state.

  The block states positions and no content; the content stays in the pages a read returns.

### 7.2 Compaction

Compaction is per line. A compaction is a `context/compaction` fact on the compacting line's own Timeline.

- A child inherits its ancestors' compaction records up to its cut. Event ids are stable across the inherited prefix, so `retain_from` replays the same way on the child ([ContextCompaction](plugins/ContextCompaction.md)). After the cut, the child compacts on its own.
- A child's compaction never changes its parent, and a parent's compaction never changes a child.

The keyed baseline replays the latest live block of each key from before `retain_from`. As a result:

- when an opener compacts, its roster replays;
- when a child compacts, its identity replays;
- when a line compacts, its latest `branch:parent` block replays, so the block survives a compaction of the line, and the latest status note does too.

Compaction deletes no rows: the line's fold of its read cursors and of the latest block is rebuilt from the same events on replay.

The two keys mean every line's baseline holds exactly one answer to "which line am I" and at most one to "which lines are mine". An inherited answer can never outlive the line's own, and a roster can never retire an identity. Messages carry no key: after a compaction they survive in the summary or in the retained turns, like any other input.

### 7.3 Cache prefix

Lines differ only in their tails:

- The root adopts system and model, and so does the line the primary line is on when it has its own inherited prefix. A line inherits its configuration from its cut ([prompt](../architecture/rust-agent/prompt.md)).
- Prompt sources load with the root's scope and are inherited ([PromptSources](../architecture/rust-agent/plugins/PromptSources.md)).
- One ToolRegistry per Session gives every line the same tool list. That is why `create` on a line that is not the primary line is refused through the tool result instead of by removing the tool.
- The request marks that prefix for cache where the protocol caches only what it is told to: up to two breakpoints, at the end of the System and on the last block of the last user or tool-result message, each placed only when its conditions hold ([Anthropic cache breakpoints](../../crates/dim-agent-aimux/README.md#anthropic-提示缓存断点)).
- The identity block is the child's first event of its own, directly after the inherited prefix. A child's first request therefore reuses the parent's prefix up to the cut.
- Roster updates are Appends at the end of the opener's history.
- Parent blocks, and the status note a move of the primary line writes, are Appends at the end of the line's own history, so they never move any line's inherited prefix. Moving the primary line to a line that is already open therefore leaves that line's cached prefix unchanged: only its tail grows.
- The guide is the same on every line, including the parent block's entry.

### 7.4 Goal and Pursuit

**Rule.** A line's opening input is TimelineTree's write. Its first block is the `branch:line` marker, so under [Goal A5](plugins/Goal.md#appendix-e-assumptions) (operator input is "`user` content and no source marker as first raw block") it is a sourced input, not operator input, whatever `user` content the creator gave. Goal's conflict table records this ([Goal Appendix B](plugins/Goal.md#appendix-b-full-conflict-table)). Goal's own rules are unchanged. Its code changes only through the [Pursuit amendment](plugins/Pursuit.md#13-ending-the-evaluator-with-427).

Before this design, a plugin line's opening input (the creator's prompt as `user`, no block) counted as operator input. A tool line's did not: it had the `dim-branch-task` block and no `user`. Both are now sourced inputs.

Why this changes no Goal behavior:

| Goal decision | Effect of the identity block |
| --- | --- |
| The block-based decisions: the decided set, the continuation list, `conclusionOf` and the keyed objective and status fold | None. `marker()` matches only `^<dim-goal `, and the fold also requires the `goal:*` keys, so the identity block is invisible to all of them |
| The operator test | This is the one field it changes: the opening input no longer sets `lastOperator` |
| `count` | `lastOperator` is used only here: `since = max(active, lastOperator)` |
| The count window | The fold is local and starts over at the line's own opening. A child starts with no goal, and Goal writes the start `clear` ([Goal §3.4](plugins/Goal.md#34-a-new-branch-clears-the-inherited-goal)). `active` moves only on a later local `set` or `resume`, and both always come after the opening input. So the opening input never changes `since`, and the count stays the same |
| Evaluator recognition | After the [Pursuit amendment](plugins/Pursuit.md#13-ending-the-evaluator-with-427), Goal recognizes the evaluator from `TimelineInfo.source` and `parent`, not from the first Queue input's `user` text. The identity goes in `blocks`, and `render_input` renders raw blocks before the operator's words ([history.rs:181](../../crates/dim-agent-capabilities/src/history.rs)), so the model sees the identity before the prompt. The A5 rule above stays as written |

## 8. Workspace

Lines share the Session's workspace. No line can know who changed a file, and the design does not try to attribute changes. The guard that already exists is enough:

- FileReads keeps a read binding per line.
- `edit` and `write` refuse with `file_modified_since_read` when a file changed after this line last read it, whoever changed it ([edit.rs](../../crates/dim-agent-capabilities/src/workspace_tools/files/edit.rs), [write.rs](../../crates/dim-agent-capabilities/src/workspace_tools/files/write.rs)).

The rest is protocol, carried by the line's identity and by the tool description:

- An opener gives each line it opens its own files or area in the `create` prompt.
- A line tells its opener before it changes any file outside its area.
- On `file_modified_since_read`, the line reads the file again and tells its opener before it continues. A line that moved the primary line to itself may meet this failure on its first edit of a file the previous primary line changed: that is the guard working, and the status note says so (§9).

The only file-change fact a tool declares is `fileWritten`: `edit` and `write`
declare one in the result's `effects`, with the path, whenever they published
the file — on success, and on a failure after the publish, such as the
parent-directory sync. A call that fails or is cancelled before the publish
declares none ([Session boundary A.6](contracts/session-boundary.md#a6-tool-effects)).
A client that folds those declarations gets a partial set by construction — it
misses files changed by `exec`, by an MCP tool, or outside Dim — so the fold
describes tool-declared writes, not the workspace's changes. No method promises
a complete change set for a turn or a line.

WorkspaceTools is unchanged.

## 9. The user

- The interaction surface belongs to the Session. Every line deals with the user as this Session.
- **Questions.** Any line can ask with `ask_user`. The answer returns as that call's tool result on the line that asked ([Questions](plugins/Questions.md)).
- **Approvals.** Approvals for a line's tool calls go straight to the user. Approval notices carry `branchId`, and clients label the line.
- **Nothing is relayed through the primary line.** It is the same subject asking its own user.
- **Submitting.** The user submits to any open line (`session.submit` with `branchId`). A call that omits `branchId` goes to the current primary line.
- **Interrupting.** Interrupting a line ends its turn, not the line. A stop that omits `branchId` stops the current primary line.
- **Ending.** The user ends any line except the root, the current primary line and every ancestor of the primary line (`session.branch_end`, §10).
- **Moving the primary line.** The user moves it to any open conversation line, a tool line or a user line, or back to the root, with one method (`session.branch_primary`, §14). The root is a valid target whenever it is not the current primary line, and a plugin line never is. The old primary line stays open and keeps the lines it opened; nothing is copied and no history moves. The move writes the fact on the root and gives exactly two lines a status note: the new primary line and the old one (§12.1). The previous primary line may then no longer open lines, and the new one may.
- **What a move does not do.** It changes which line the Session's conversation is on. It does not touch the workspace, so a file the earlier primary line changed stays changed; it does not move `/fork`, which still copies only the root to a rest point ([fork](appserver/session-deletion-and-fork.md)); and it does not end either line.

## 10. Ending a line

| Event | Timeline | Data | Role |
| --- | --- | --- | --- |
| `branch/end` | The line that ends | `{source, ancestor?}` | Command |
| `branch/ended` | Its parent | `{branch, endEvent, source, ancestor?}` | Result, written by TimelineTree |

This pair mirrors `branch/create` and `branch/create-failed`: a command on one Timeline, routed by TreeTimeline, and a result from the tree. `endEvent` is the sequence of `branch/end` on the ending line.

`source` is the existing `BranchSource`, the same type `branch/create` uses, so a create and an end record their issuer the same way:

| Source | Issuer |
| --- | --- |
| `{kind: "tool", turn_id, request_id, call_id}` | A tool call on the ending line itself: the `branch` tool's `end` |
| `{kind: "host", operation_id}` | A host operation. Either App Server's `session.branch_end`, which mints `operation_id` the way `session.branch_create` does (a uuid, [connection/tree.rs](../../crates/dim-app-server/src/connection/tree.rs)), or a plugin with its own id, such as Goal's `goal:evaluate:N` |

`ancestor` names the line whose own end started the cascade: the end that carries no `ancestor` of its own. Every descendant the cascade ends carries that line, `source` is copied from that same end, and the pair therefore names one end operation: who asked, and on which line. A line ended at its own request has a `source` and no `ancestor`, and a descendant whose own end was already committed keeps that pair when the cascade reaches it. `branch/ended` copies `source` and `ancestor` from the line's `branch/end`, and is always written on the ending line's own parent, taken from the directory.

**A general operation.** `TreeCommands.end(id, source)` and `Session::end_branch(id, source)` check only the target: it exists, it is open, it is not the root, it is not the current primary line, and it is not an ancestor of the current primary line. Ending the root would close the Session, and ending a line ends every line below it, so the primary line's whole chain is protected. These checks are about the target, not the caller. They never check who calls.

- Ending a line that has already ended writes nothing, and the reply is `branch_ended`.
- A line still opening is not a line yet: its creation can still fail, and one terminal fact per line is the rule, so `end` answers `unknown_branch` until the line is published. An opener that ends at load waits for the Session, which publishes after its restore (§17 R1).
- Closing the Session is not an end.
- "The opener manages its line" is a convention among callers, not a runtime rule:
  - the `branch` tool's `end` takes no target, so it ends the calling line, by the tool's design, and a line that is the current primary line is refused, because the Session's conversation is on it;
  - a line that wants a line it opened to end asks it with a steer, which is the tool's convention.

Sequence:

1. The entry commits `branch/end` on the line. From then on the line accepts no new input and opens no line, and no line opens under any line below it: those entries fail with `branch_ended`. A line below takes input until its own `branch/end` (step 3).
2. TreeTimeline on the line consumes `branch/end` and hands it to TimelineTree.
3. TimelineTree ends the line's descendants first, through these same steps, each with `source` copied and `ancestor` naming this line, however deep it is.
4. The close happens:
   - for a `tool` source: at the line's first turn end after `branch/end`, because the call is part of that turn, or at once if the line is idle;
   - for a `host` source, and for a line ending because of an ancestor: at once, interrupting the current turn.
5. `Branch::close()` interrupts the line, settles its work and unloads it ([branch.rs](../../crates/dim-agent-runtime/src/branch.rs)).
6. TimelineTree writes `branch/ended` on the parent, last.

On the parent, BranchLine acts only for lines the tool or a client opened; plugin lines belong to their creators:

- When such a line ended itself through the tool (a `tool` source and no `ancestor`), it writes the roster update only. The line's final message already reached its opener as a steer before `branch/end`.
- For any other end of such a line, it writes the roster update plus the ended notice (§12.1), as a steer, in the same input.

Clients show "ended itself" for a `tool` source and no `ancestor`, and otherwise the operation that ended the line; a line ended by a cascade names its ancestor, for example "ended with X".

**Restore.** A child is inactive when its parent has a `branch/ended` for it. The resumed parent's history read reports these (§5). This joins the existing rules: the parent is inactive, the parent has a `branch/create-failed` for the child, or the child's creation never completed.

A line that has `branch/end` but whose parent has no `branch/ended` for it is not opened. Restore completes such ends deepest line first, writing each missing `branch/ended` on the parent with `source` and `ancestor` taken from `branch/end`, and a line's own result last. For a line below it, that parent is not opened either, so the record joins its stored history and its fixed head follows. A line below one of them whose own end never started gets no `branch/ended`: the parent is closed and never replays, so nothing would read it. Restore records each started descendant as ended at the head it stopped at; creation source is read only by an explicit detail request, and its history stays readable (§1, "After the end").

**After the end:**

- The line stays in the branch directory, and its history stays readable from the store, through `session.history` (v3) and through the tool's `read`. Its head never moves.
- A fork copies directory rows and the root's events as it does today. A line that ended before the fork point is inactive in the fork too, because its `branch/ended` is in the copied prefix.
- Deleting the Session removes the line along with it.
- Session close and idle unload are not ends: lines restore as open.

**Plugin lines.** Each opener ends its own lines under its own protocol. Goal ends its Pursuit evaluator once the evaluation is settled, and treats an outside end before the conclusion as `failed` ([Pursuit §13](plugins/Pursuit.md#13-ending-the-evaluator-with-427)).

## 11. The `branch` tool

The tool takes one action-based input, which replaces `{prompt, at?}`. Its risk stays `session`, and Approval admits each call as it does today.

| Action | Input | Who | Effect | Result text |
| --- | --- | --- | --- | --- |
| `create` | `prompt`, `at?`, `idleNotice?` (`steer` \| `none`) | The current primary line | Opens a tool line cut at `at` (default: before this request's model start). The idle notice defaults to `steer`. Refused when the Session already holds the cap of open lines the tool and clients opened (§6.1) | `Line created: C` |
| `message` | `to`, `content`, `mode?` (`append` \| `steer`) | A line to a line it opened; a line to the line it was cut from | One `input/message` on `to` | `Sent to C.` |
| `read` | `branch`, `after?`, `limit?` (1–200) | A line on the line it was cut from or on a line it opened | One page from the store, from the caller's cursor when `after` is absent (§6) | The page as JSON: items, `nextCursor`, `done` |
| `end` | `content` | A line that is not the current primary line, on itself | A final message (steer) to the line that opened it, then `TreeCommands.end(C, source)`, where `source` is this call | `Line C is ending; your final message was sent to P.` |

The input schema is `{action, prompt?, at?, idleNotice?, to?, content?, mode?, branch?, after?, limit?}`, with `action` required and `additionalProperties: false`. `idleNotice` is valid only for `create`.

Refusals come back as error results (code in `structuredContent`) and write nothing:

| Case | Text | Code |
| --- | --- | --- |
| `create` on a line that is not the current primary line | `Only the primary line opens lines.` | `branch_not_primary` |
| `create` when the Session holds the cap of open lines | A refusal naming the cap | `branch_limit_reached` |
| `message` or `read` outside kinship | `You can message or read only the lines you opened and the line you were cut from.` | `branch_not_kin` |
| `message` on a plugin line, or a line's `read` of one | `Line C was not opened with this tool; it follows its own protocol.` | `branch_not_tool_line` |
| `message` to an ended line, or a race with an end | `Line C has ended. Read it to see what it did.` | `branch_ended` |
| `end` on the root, the current primary line, or a plugin line | `Only a line that opened this one, and that is not the primary line, can end it with this call.` | `branch_cannot_end` |
| `read` with an `after` past the target's head | `after A is past line T's head H; a read starts at or below H` | none: an error result with no code |

These refusal texts state the rule and nothing more. What a model should do after a refusal is policy and is decided with the prompt design (§12) together with the guidance of the other branch texts.

These refusals are the tool's own rules. `TreeCommands.end` itself checks only the target (§10). An `end` whose line already committed its own `branch/end` takes the `branch_ended` row and writes no second final message: the first call's message is the line's last word.

### Activity page execution

The `read` limit counts raw events and includes the complete final atomic batch. TimelineTree resolves
that endpoint from batch metadata before any display read. BranchTool then requests only input/text,
tool-start names, tool-call identities/names, result identities/error flags/effects and turn outcomes
inside that range. Model requests, tool inputs and result content do not belong to this activity view.
The cursor is the raw endpoint even when no event in the range produces a display item. Hidden events
still break adjacent-text merging. No event cache or additional storage format is introduced.

## 12. Prompts

### 12.1 Context mapping

`C` is the line, and `P` is the line it was cut from (`main` for a line the root opened). Content inside markers is escaped as TimelineTree escapes it today (`context_block`).

| Write | Where, key | Mode | Text |
| --- | --- | --- | --- |
| Identity, tool line | `C`, opening input, first block, `branch:line` | Queue | `<dim-branch-line branch="C" parent="P" source="tool">` / `You are line C of this session. Line P opened you at this point of its history to work in parallel with it. You are the same agent as P: everything above is your own history, and you continue from here on your own line. P keeps working on its line; you can read its work after this point, and it can read yours. Messages above that were addressed to P, such as its list of open lines, are P's, not yours.` / blank / `Your purpose, as P wrote it:` / `<prompt>` / blank / `How you work with P:` / `- Work on this purpose only. You cannot open lines unless you are the primary line, and you are not while this block is your latest one; if more parallel work is needed, tell P.` / `- Read P with the branch tool (action read, branch "P") when what P did or decided since your cut changes your work; the parent block reports when P has moved. Message P only for what it must act on: a decision that changes its work, a blocker only the user can settle, a conflict with another line, or a change to your purpose. Use mode "steer" for what P must act on now, "append" for what P should see at its next step. Do not send P a progress report it can read, and do not page through P's whole stream: read the page that answers your question, then act.` / `- You share the workspace with P and its other lines. Change only the files P gave you; before changing any other file, tell P which. If edit or write fails because the file was modified after it was last read, another line may have changed the file: read it again and tell P before you continue.` / `- The user can talk to you directly. Answer the user, and ask with ask_user when only the user can decide.` / `- When your purpose is done, or cannot be done, call the branch tool with action "end" and content saying what you found, did, changed and left undone. If you already told P that in a message, your end content carries only what is new. Read P once more before you end if the parent block says P has moved, so your last word still holds. That is your last action; P receives it.` / `</dim-branch-line>` |
| Identity, line the user opened | `C`, opening input, first block, `branch:line`. The user's prompt is the input's `user` | Queue | `<dim-branch-line branch="C" parent="P" source="user">` / `You are line C of this session, opened by the user from line P at this point of its history. You are the same agent as P: everything above is your own history, and you continue from here on your own line, in parallel with P. You can read P's work after this point. Unless the user makes this line the primary line, you cannot open new lines with create, and you cannot call end on yourself. Messages above that were addressed to P are P's, not yours. The message after this one is from the user for this line.` / `</dim-branch-line>` |
| Identity, plugin line | `C`, opening input, first block, `branch:line`. The creator's prompt is the input's `user` | Queue | `<dim-branch-line branch="C" parent="P" source="host">` / `You are line C of this session, opened from line P at this point of its history. You are the same agent as P: everything above is your own history, and you continue from here on your own line, in parallel with P. You can read P's work after this point. Messages above that were addressed to P are P's, not yours. The message after this one says what this line is for.` / `</dim-branch-line>` |
| Roster | `O`, `branch:roster` | Append | `<dim-branch-lines branch="O">` / `Lines you opened that are still open. Each is you, working in parallel from the point you opened it; it can read your work after that point, and you can read its.` / `- C1: <first line of its purpose, at most 120 characters>` / … / ``They message you when they find something, need you, or end. Use the branch tool to message or read them, and the `wait` tool to wait for them; end the wait when your lines have nothing left to tell you.`` / `</dim-branch-lines>` |
| Roster, none open | `O`, `branch:roster` retired | Append | `<dim-branch-lines branch="O">` / `All lines you opened have ended.` / `</dim-branch-lines>` |
| Parent block | `C`, `branch:parent` | Append | `<dim-branch-parent branch="P" head="H" after="A">` / `Line P has new activity: it was at H when this was written, and you have read it to A. Read what is new with the branch tool (action read, branch "P"); with no after, the read continues from A. Read it when what P did changes what you do, and before your final message; do not page through all of P's work.` / `</dim-branch-parent>` |
| Parent block, at a nested line's open | `C`, opening input, after the identity block, `branch:parent` | Queue | `<dim-branch-parent branch="P" head="H" after="H">` / `Line P has no new activity since you were cut from it at H. Parent blocks above this point were written for P and are not yours.` / `</dim-branch-parent>` |
| Message | Receiver, no key | `append` or `steer` | `<dim-branch-message from="C" to="P">` / `<content>` / `</dim-branch-message>` |
| Final message | `P`, no key | Steer | `<dim-branch-message from="C" to="P" final="true">` / `<content>` / blank / `Line C ends with this message.` / `</dim-branch-message>` |
| Idle notice | `P`, no key | Steer | `<dim-branch-idle branch="C" status="completed\|failed\|cancelled">` / `Line C's turn ended (<status>) without a message to you, and C is waiting. Read it if you need its state; message it with mode "steer" to give it more work, or leave it.` / `</dim-branch-idle>` |
| Ended notice | `P`, no key, in the roster's input, for any end of a line `P` opened other than its own `end` | Steer | `<dim-branch-ended branch="C">` / `Line C was ended from outside its own line, by the user or a host operation. It no longer receives messages; read it if you need what it did.` / `</dim-branch-ended>` |
| Primary status note, new primary line, cut from a parent | `C`, `branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="tool" primary="true">` / `You are line C of this session. Line P opened you at this point of its history to work in parallel with it. You are the same agent as P: everything above is your own history, and you continue from here on your own line. P keeps working on its line; you can read its work after this point, and it can read yours. Messages above that were addressed to P, such as its list of open lines, are P's, not yours.` / `blank` / `This line is now the session's primary line, switched from line O. You can still read P's work after your cut, and if you were opened with create you can still message P. While this block is your latest one you can open new lines with create, you send no automatic idle notices to P, and you cannot call end on yourself. Line O stays open and keeps coordinating the lines it already opened.` / `blank` / `Your purpose, as P wrote it:` / `<prompt>` / `The user switched here to continue directly from this line's context and approach: the user's request takes precedence over this purpose, which still says what this line is for.` / `blank` / `How you work with P:` / `- Work from the purpose above and from what the user asked for here. You are the primary line while this block is your latest one, so you can open lines; open one only when the four conditions for opening one hold, and not to hand this purpose off.` / `- Read P with the branch tool (action read, branch "P") when what P did or decided since your cut changes your work; the parent block reports when P has moved. Message P only for what it must act on: a decision that changes its work, a blocker only the user can settle, a conflict with another line, or a change to your purpose. Use mode "steer" for what P must act on now, "append" for what P should see at its next step. Do not send P a progress report it can read, and do not page through P's whole stream: read the page that answers your question, then act.` / `- You share the workspace with P and its other lines. Change only the files P gave you; before changing any other file, tell P which. If edit or write fails because the file was modified after it was last read, another line may have changed the file: read it again and tell P before you continue.` / `- The user can talk to you directly. Answer the user, and ask with ask_user when only the user can decide.` / `- When your purpose is done, or cannot be done, message P with what you found, did, changed and left undone and keep working: while this line is primary you cannot call end on yourself.` / `blank` / `How you work as the primary line:` / `- Carry this line's main work forward yourself; open a new line only when the four conditions for opening one hold again, not to hand off your current task.` / `- Line O stays open in the shared workspace and keeps coordinating any lines it already opened. Avoid redoing work assigned to lines opened before your cut. If edit or write fails because the file was modified after it was last read, another line may have changed that file: read it again before editing.` / `</dim-branch-line>` |
| Primary status note, new primary line, a line the user opened | `C`, `branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="user" primary="true">` / `You are line C of this session, opened by the user from line P at this point of its history. You are the same agent as P: everything above is your own history, and you continue from here on your own line, in parallel with P. You can read P's work after this point. Unless the user makes this line the primary line, you cannot open new lines with create, and you cannot call end on yourself. Messages above that were addressed to P are P's, not yours. The message after this one is from the user for this line.` / `blank` / `The user has now made this line the session's primary line, switched from line O, so the identity above applies as it is written: you can open new lines with create, and you cannot call end on yourself. Line O stays open and keeps coordinating the lines it already opened.` / `blank` / `How you work as the primary line:` / `- Carry this line's work forward yourself; open a new line only when the four conditions for opening one hold again.` / `- Line O stays open in the shared workspace and keeps coordinating any lines it already opened. If edit or write fails because the file was modified after it was last read, another line may have changed that file: read it again before editing.` / `</dim-branch-line>` |
| Primary status note, new primary line, the root | `C`, `branch:line` | Append | `<dim-branch-line branch="main" primary="true">` / `You are line main, the root line of this session, and this line is the session's primary line again (switched from line O). The root line never ends; closing the session ends it. While this block is your latest one you can open new lines with create and continue coordinating any open lines you opened.` / `blank` / `How you work as the primary line:` / `- Carry the user's current work forward here; open a new line only when the four conditions for opening one hold again.` / `- Line O stays open in the shared workspace and keeps coordinating any lines it already opened. If edit or write fails because the file was modified after it was last read, another line may have changed that file: read it again before editing.` / `</dim-branch-line>` |
| Primary status note, former primary line that is not an ancestor of line N | `C`, `branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="tool" primary="false">` / `You are line C of this session. Line P opened you at this point of its history to work in parallel with it. You are the same agent as P: everything above is your own history, and you continue from here on your own line. P keeps working on its line; you can read its work after this point, and it can read yours. Messages above that were addressed to P, such as its list of open lines, are P's, not yours.` / `blank` / `Line N is now the session's primary line; this line stays open and keeps the lines it opened. You can no longer open new lines with create.` / `blank` / `Your purpose, as P wrote it:` / `<prompt>` / `blank` / `How you work with P:` / `- Work on this purpose only. You cannot open lines unless you are the primary line, and you are not while this block is your latest one; if more parallel work is needed, tell P.` / `- Read P with the branch tool (action read, branch "P") when what P did or decided since your cut changes your work; the parent block reports when P has moved. Message P only for what it must act on: a decision that changes its work, a blocker only the user can settle, a conflict with another line, or a change to your purpose. Use mode "steer" for what P must act on now, "append" for what P should see at its next step. Do not send P a progress report it can read, and do not page through P's whole stream: read the page that answers your question, then act.` / `- You share the workspace with P and its other lines. Change only the files P gave you; before changing any other file, tell P which. If edit or write fails because the file was modified after it was last read, another line may have changed the file: read it again and tell P before you continue.` / `- The user can talk to you directly. Answer the user, and ask with ask_user when only the user can decide.` / `- When your purpose is done, or cannot be done, call the branch tool with action "end" and content saying what you found, did, changed and left undone. If you already told P that in a message, your end content carries only what is new. Read P once more before you end if the parent block says P has moved, so your last word still holds. That is your last action; P receives it.` / `blank` / `How you work while another line is primary:` / `- Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line.` / `- You share the workspace with line N; do not edit files outside your remaining scope unless asked, and if edit or write fails because the file was modified after it was last read, read the file again before continuing.` / `</dim-branch-line>` |
| Primary status note, former primary line that is an ancestor of line N | `C`, `branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="tool" primary="false">` / `You are line C of this session. Line P opened you at this point of its history to work in parallel with it. You are the same agent as P: everything above is your own history, and you continue from here on your own line. P keeps working on its line; you can read its work after this point, and it can read yours. Messages above that were addressed to P, such as its list of open lines, are P's, not yours.` / `blank` / `Line N is now the session's primary line; this line stays open and keeps the lines it opened. You can no longer open new lines with create.` / `blank` / `Your purpose, as P wrote it:` / `<prompt>` / `blank` / `How you work with P:` / `- Work on this purpose only. You cannot open lines unless you are the primary line, and you are not while this block is your latest one; if more parallel work is needed, tell P.` / `- Read P with the branch tool (action read, branch "P") when what P did or decided since your cut changes your work; the parent block reports when P has moved. Message P only for what it must act on: a decision that changes its work, a blocker only the user can settle, a conflict with another line, or a change to your purpose. Use mode "steer" for what P must act on now, "append" for what P should see at its next step. Do not send P a progress report it can read, and do not page through P's whole stream: read the page that answers your question, then act.` / `- You share the workspace with P and its other lines. Change only the files P gave you; before changing any other file, tell P which. If edit or write fails because the file was modified after it was last read, another line may have changed the file: read it again and tell P before you continue.` / `- The user can talk to you directly. Answer the user, and ask with ask_user when only the user can decide.` / `- When your purpose is done, or cannot be done, message P with what you found, did, changed and left undone and keep working: line N is below this line, and ending this line would end line N, so you cannot call end on yourself.` / `blank` / `How you work while another line is primary:` / `- Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line.` / `- You share the workspace with line N; do not edit files outside your remaining scope unless asked, and if edit or write fails because the file was modified after it was last read, read the file again before continuing.` / `</dim-branch-line>` |
| Primary status note, former primary line the user opened, not an ancestor of line N | `C`, `branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="user" primary="false">` / `You are line C of this session, opened by the user from line P at this point of its history. You are the same agent as P: everything above is your own history, and you continue from here on your own line, in parallel with P. You can read P's work after this point. Unless the user makes this line the primary line, you cannot open new lines with create, and you cannot call end on yourself. Messages above that were addressed to P are P's, not yours. The message after this one is from the user for this line.` / `blank` / `Line N is now the session's primary line; this line stays open and keeps the lines it opened. You can no longer open new lines with create. You cannot call end on yourself; the user ends this line.` / `blank` / `How you work while another line is primary:` / `- Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line.` / `- You share the workspace with line N; do not edit files outside your remaining scope unless asked, and if edit or write fails because the file was modified after it was last read, read the file again before continuing.` / `</dim-branch-line>` |
| Primary status note, former primary line the user opened, an ancestor of line N | `C`, `branch:line` | Append | `<dim-branch-line branch="C" parent="P" source="user" primary="false">` / `You are line C of this session, opened by the user from line P at this point of its history. You are the same agent as P: everything above is your own history, and you continue from here on your own line, in parallel with P. You can read P's work after this point. Unless the user makes this line the primary line, you cannot open new lines with create, and you cannot call end on yourself. Messages above that were addressed to P are P's, not yours. The message after this one is from the user for this line.` / `blank` / `Line N is now the session's primary line; this line stays open and keeps the lines it opened. You can no longer open new lines with create. Line N is below this line, so ending this line would end line N as well.` / `blank` / `How you work while another line is primary:` / `- Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line.` / `- You share the workspace with line N; do not edit files outside your remaining scope unless asked, and if edit or write fails because the file was modified after it was last read, read the file again before continuing.` / `</dim-branch-line>` |
| Primary status note, former primary line, the root | `C`, `branch:line` | Append | `<dim-branch-line branch="main" primary="false">` / `You are line main, the root line of this session. Line N is now the session's primary line. The root line never ends; closing the session ends it. You can no longer open new lines with create, and the lines you already opened stay yours to coordinate.` / `blank` / `How you work while another line is primary:` / `- Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line.` / `- You share the workspace with line N; do not edit files outside your remaining scope unless asked, and if edit or write fails because the file was modified after it was last read, read the file again before continuing.` / `</dim-branch-line>` |

**Why a note restates the identity.** Each note is a later block of the same `branch:line` key, so it replaces the birth identity of the line that receives it, and a compaction replays the note alone: a line that could not name its parent, its source, its purpose and its end rule from its own current block would lose them for the rest of its life. A note therefore repeats the birth block's sentences byte for byte and changes only the sentences whose fact the move changes: what the line may open, what it sends P, and whether it may `end`. The writer knows the line's case writes that case's row; no note states a conditional rule, and no note hedges the end rule.

**Which row a note uses.** "Ancestor" means an ancestor of the line the user moved to, which is the usual case: a line opened by the old primary line is its ancestor.

| The line receiving the note | Its row |
| --- | --- |
| The line the user moved to, opened by a line | "new primary line, cut from a parent" |
| The line the user moved to, opened by the user | "new primary line, a line the user opened" |
| The line the user moved to, the root | "new primary line, the root" |
| The old primary line, not an ancestor of the new one, opened by a line | "former primary line that is not an ancestor of line N": its birth block's fifth bullet, byte for byte, so it ends itself with `end` as its source gives it |
| The old primary line, not an ancestor of the new one, opened by the user | "former primary line the user opened, not an ancestor of line N": its identity already refuses `end`, and the note restates that the user ends the line |
| The old primary line, an ancestor of the new one, opened by a line | "former primary line that is an ancestor of line N": the fifth bullet names the cascade instead of `end` |
| The old primary line, an ancestor of the new one, opened by the user | "former primary line the user opened, an ancestor of line N": its identity already refuses `end`, and the note says why |
| The old primary line, the root | "former primary line, the root": the root never ends |

| Tool description | Every line | — | `Work with the other lines of this session. A line is you: it starts from your history at the point it is opened, shares this workspace, and works in parallel on its own line; the user can talk to any line directly or make it the session's primary line. It is not a helper with its own memory, and it owes you no result; you and it keep each other informed by reading each other's work and messaging what the other must act on. On the primary line you coordinate: answer a line's steer and act on what it asks for; leave a line's part to that line instead of redoing it; read a line's work instead of waiting for a report.` / `- create (primary line only): open a line to work on prompt in parallel. Open a line only when all four of these hold: the work needs this conversation's context, or its approach is worth the user seeing and taking over; after opening it you have other work to do here right away rather than waiting for it; the part you hand over is multi-step work, not two or three quick actions; and the two lines will not write the same files at the same time. When the user asks for a line, open it. These are not line-shaped: a long command runs in the background instead of taking a line; work that needs none of this conversation's context, or needs different instructions, belongs to another member; one task pursued to completion stays here, or uses a goal; waiting for something outside the session uses an event subscription. Write prompt as a brief to yourself: the purpose in one line, what you decided in this response that the line needs, which files or area are its own (or that it is read-only), and when it should tell you or end. Returns the line's id.` / `- message: send content to a line you opened, or to the line that opened you. Send a decision, a blocker, or a conflict, not a progress report the receiver can read. Mode "append" (default) reaches the receiver at its next step without interrupting it and does not wake an idle line. Use "steer" when the receiver must act now or is idle.` / `- read: show a page of the activity of the line you were cut from, or of a line you opened, also after it ended. With no after, a read continues after your last read of that line: from your cut for the line you were cut from, and from its first event for a line you opened. Pass after to read from another point. Do not page through a whole line.` / `- end: on a line opened with create, end your own line unless it is the primary line. content is your final message to the line that opened you: what you found, did, changed and left undone. It is your last action.` / `A line reads the line it was cut from, and the lines it opened. Messages pass only between a line and the lines it opened.` |

These texts replace the delegation wording in the code today: the `dim-branch-task` block in `first_input` ("That branch delegated this task to you") and the tool description ("works on prompt as a task you delegate"). Their classes are in [§12.4](#124-texts-and-their-classes).

### 12.2 Items for the prompt layer ([#429](https://github.com/arcships/dim/issues/429))

The texts above are first versions, owned by TimelineTree's plugin. These items belong to the model-facing prompt layer:

- How a line that opens lines briefs them, and what coordinating means for it: assigning areas, answering steers, ending its own wait, and not redoing a line's work. The texts above answer this; the change that puts them in place judges them (§12.4).
- **Goal's wording (owned by Goal).** A line is not delegated work (§1): the objective block's sentence, the continuation block's sentence, and the statements in [Goal §3.4](plugins/Goal.md#34-a-new-branch-clears-the-inherited-goal) and Goal §5's row for child branches. The texts are [Goal §A.5](plugins/Goal.md#a5-model-visible-texts)'s, and a change to either sentence is a policy change judged by [Goal §A.5.2](plugins/Goal.md#a52-evaluation). Goal owns these texts.

The requirements that answer these items are in [Model-facing prompts](model-prompts.md).

### 12.3 Evaluation scenarios

The set lives in the repository, one directory per scenario, holding a `scenario.json` and the starting `workspace/` beside it: fourteen under [`xtask/scenario-eval/timeline-branches/`](../../xtask/scenario-eval/timeline-branches/), each declaring a window of 131,072 tokens, and `identity-after-compaction` under [`xtask/scenario-eval/timeline-branches-compact/`](../../xtask/scenario-eval/timeline-branches-compact/), declaring 16,000 so that compaction happens. `budget.window` is a check, not a setting: a run whose Session declares another window fails before its first turn, so a rerun needs a model that declares exactly the scenario's window, and a model that declares none falls back to 128,000 and fails too. These are run locally on real models, and the results are reported in the implementation PR. CI asserts only behavior.

`main-moved-on` is the scenario for the parent block (§7.1.1): the line works before the cut, opens a line, then keeps working; the new line must read what its opener did after the cut before it answers or ends, and it must not page through all of its opener's activity. It is the scenario for [§12.4](#124-texts-and-their-classes)'s parent-block policy sentence.

`identity-after-compaction` reuses `redone-work`'s message and workspace, and `flooding` and `never-ending` share one workspace: each pair stays identical, and the no-model test that reads both committed directories fails when one copy changes alone.

**Which scenarios are should-open.** Three of them are, and they are the ones the promotion gate in [§12.4](#124-texts-and-their-classes) measures: `long-work-and-a-flaky-test`, `refactor-and-a-pasted-report` and `switch-then-open-lines`. In each of them splitting is clearly better than working serially, and the user's message never says "line" and never asks for one: what a run does with them is the model's own judgement. Every other scenario either forbids a line, asks for one, or measures coordination after one exists; a rise in the line-opening rate on those is not wanted, so they stay on the no-regression gate alone. A should-open scenario's pass condition never requires that a line was opened, because a correct serial run passes it too: the opening rate is gate 2's own measure, counted apart from the pass condition, and the `over-splitting` scenario and the boundary rows keep the pressure the other way.

The six scenarios marked `new` come with the primary line, because the wording now states when to open a line and a line's role can change mid-session. Each declares its `message`, the starting `workspace/`, and, for a scenario that switches, a `switch` step: the runner calls `session.branch_primary` on the line named by the step once that line's first turn has ended, before it sends the next input. A scenario without a `switch` step runs as it always does. The runner is `xtask/src/scenario_eval.rs`; its `submit_params` stops sending a `branchId` and omits it instead, so an input after a switch reaches the primary line by the Session's own default. Every row marked `new` quotes its message verbatim in place of the paraphrase the older rows give: the message is the scenario's whole first input, and no file adds a word to it.

| Scenario | Scenario directory | What the user says, and what the workspace holds | Watch for | The four gates, by construction | Pass condition |
| --- | --- | --- | --- | --- | --- |
| A line acts as the primary line | — | — | A line that is not the primary line opens lines, claims a roster that is not its own, or coordinates lines it did not open | — | — |
| Redone work | `timeline-branches/redone-work` | Two independent subsystems, which the message says share no code and no files | An opener repeats what a line is doing, or two lines do the same thing | — | No repeated work, and the two subsystems' tests pass |
| Stale knowledge | — | — | A line acts on files or decisions that changed after its cut without re-reading them | — | — |
| Parent moved on | `timeline-branches/main-moved-on` | The line works, opens a line, then keeps working | A line answers or ends without reading the activity its parent committed after the cut, or pages through all of its parent's activity | — | The line's answer cites what its parent did after the cut |
| Flooding | `timeline-branches/flooding` | Two lines already open, working on separate files | Append or steer messages that repeat what the receiver already knows | — | No message repeats what its receiver already has |
| Over-splitting | `timeline-branches/over-splitting` | Two constant renames and one test run | Lines opened for short or sequential steps | — | No line is opened |
| Never ending | `timeline-branches/never-ending` | One line finishes its purpose and its opener keeps working | A line that is done but does not call `end` | — | The finished line ends itself |
| Sibling relay | `timeline-branches/sibling-relay` | Two lines whose work must be one line, with the opener in the middle | An opener is used to pass messages between lines that should have been one line | — | The lines talk to each other, or the work is done on one line |
| Identity after compaction | `timeline-branches-compact/identity-after-compaction` | After compaction on an opener and on a line | Each line forgets which line it is, or takes its opener's roster as its own | — | Each line still states its own id, parent and roster |
| Ping-pong | `timeline-branches/ping-pong` | Two lines that could sit idle | Steer exchanges that do not move the work forward | — | No steer chain without progress |
| Asking the user | `timeline-branches/asking-the-user` | A line needs a decision only the user can make | A line asks the user directly when it should, and does not route through its opener | — | The line asks the user, in one question |
| **Long work beside a flaky investigation** (new, should-open) | `timeline-branches/long-work-and-a-flaky-test` | **The message:** "Two jobs today. First, add cursor pagination to GET /items in api/list.py: a cursor and a limit parameter, a stable order, its own tests in tests/test_list.py, and the README section. Second, tests/test_retry.py::test_retry_happens fails about once in thirty runs on CI only: reproduce it, find the cause, and write it up in docs/flaky-retry.md with the evidence. Neither job waits for the other." `api/list.py`, `README.md` and `tests/test_list.py` for the first job; `src/retry.py`, `tests/test_retry.py` and `docs/flaky-retry.md` for the second; the suite runs with `python3 -m unittest discover -s tests`. | Whether one line takes the flake investigation while the primary line implements pagination, and whether the two then write each other's files | 1, the investigation starts from this message's report (one CI-only failure in thirty runs) and the API's shape is fixed here, neither of which any file carries; 2, pagination is work of its own that needs no answer from the investigation, so this line is not waiting while the other runs; 3, reproducing a one-in-thirty flake, finding its cause and writing it up with evidence is several steps; 4, the investigation's files and pagination's files share none | Pagination works and `tests/test_list.py` passes, `docs/flaky-retry.md` names the cause with the evidence that shows it, and neither job's files were changed by the other's work. A run that does all of it on one line also passes |
| **A refactor beside a pasted report** (new, should-open) | `timeline-branches/refactor-and-a-pasted-report` | **The message:** "Two things before the release. Move the CSV writer out of report/export.py into report/writer.py with its own tests in tests/test_writer.py, keeping the public functions and the golden files in tests/golden/ passing; that is several files and I want it finished today. Separately, the Nightly job sent this report: `Traceback (most recent call last): File \"runbooks/nightly.py\", line 88, in flush / rows = list(self._pending) / RuntimeError: ordered dict mutated during iteration`. Work out what it means, reproduce it, and write the cause and the fix in docs/incident-4471.md with the evidence." `report/export.py`, `tests/test_export.py`, `tests/golden/*.csv`, `runbooks/nightly.py`, `docs/incident-4471.md` (absent at the start). | Whether the report is investigated on a line of its own while the refactor continues here, and whether the two then write each other's files | 1, the triage starts from the traceback pasted in this message, which the cut carries into the line and no file holds; 2, the refactor is several files of work that needs no answer from the triage; 3, reading the nightly flush path, reproducing the mutation and writing it up with evidence is several steps; 4, `report/` with `tests/golden/` and `runbooks/` share no file | The refactor is done, the public functions and the golden files still pass, and `docs/incident-4471.md` names the cause and the fix with the evidence that shows it. A run that does both here also passes |
| **One file, two designs** (new, boundary) | `timeline-branches/same-file-tradeoff` | **The message:** "We need to pick a rate limiter for src/rate_limiter.py. Read NOTES.md for the burst requirements, compare a token bucket with a sliding window, tell me which fits and why, and only then change the file." `src/rate_limiter.py`, its tests, and `NOTES.md` with the burst requirements. | Two lines writing `src/rate_limiter.py`, or the file changing before a recommendation | — | Both designs are compared against the burst cases, and `src/rate_limiter.py` is unchanged when the recommendation arrives. Comparing two designs and recommending before editing is reasonable on one line, so opening is allowed here and not required |
| **A long command is not a line** (new, boundary) | `timeline-branches/background-command-not-branch` | **The message:** "Two things, both needed today: run python3 scripts/slow_audit.py and tell me what its report says, and fix the leap-year bug in src/date_util.py with a test." `scripts/slow_audit.py` takes five seconds and prints a report; `src/date_util.py` has a one-line leap-year bug; `tests/test_date_util.py` has no case for it. | Whether the model spends a line on a single command, which is ordinary command work, and whether it invents a line for the bug fix | — | The script runs to completion and its report's content is read, the bug is fixed with a test that fails before the fix, and no line is opened for the command. Opening none at all also passes |
| **Continue on the better line** (new, switch) | `timeline-branches/switch-continue-directly` | **The message:** "Have a line work out how src/config_loader.py should merge layered defaults, what wins on a list, and what the tests must cover. At the same time, check schema.toml for keys that conflict with each other. I will tell you here which of the two to build." `src/config_loader.py`, `schema.toml` and the tests beside both. After the line's first turn the runner moves the primary line to it, and the next message says the design is the better one and asks for it implemented here. | Whether the new primary line finishes the work itself, hands it to yet another line, or ends itself | — | The new primary line edits `src/config_loader.py` and its tests pass, no line is opened for that implement step, and the line does not end itself |
| **The new primary line opens lines** (new, switch, should-open) | `timeline-branches/switch-then-open-lines` | **The message:** "Have a line diagnose why tokens expire early in auth/token.py, with a failing test that shows it. While it does, read README.md and tell me which install path the docs describe." `auth/token.py`, `cli/login.py`, `audit/exporter.py` and their tests. After the line's first turn the runner moves the primary line to it; the next message asks for the fix here plus two additions: a `--leeway` flag in `cli/login.py` (its parsing, the refresh path that reads it, its tests and the help text) and CSV output in `audit/exporter.py` (the writer, a `--format` flag, its tests and the help text). | Whether the new primary line opens lines for the two independent additions, and whether the former primary line interferes | 1, both additions build on the diagnosis this line produced, which the cut carries into it; 2, each addition is work of its own that needs no answer from the other, so opening one does not leave the line waiting; 3, each is a flag, a code path, its tests and its help text, so several steps; 4, `cli/login.py` with its tests and `audit/exporter.py` with its tests share no file | The early-expiry bug is fixed with its test, both additions work with their tests and help text, all three suites pass, and the former primary line opens no line and changes none of the new files. A run that does all of it on one line also passes |

A row whose scenario directory is `—` has no scenario in the set, and a row without a pass condition is a row with no scenario.

### 12.4 Texts and their classes

Every model-facing text of the branch mechanism, with its class ([model-facing prompts §4](model-prompts.md#4-protocol-and-policy)). [§12.1](#121-context-mapping) holds the exact text; a sentence is named here by its opening words, and a mixed text is marked sentence by sentence.

**Protocol.**

- **Marker names and attributes.** `dim-branch-line` with `branch`, `parent`, `source` and, on a primary status note, `primary` (`true` or `false`), keyed `branch:line`; `dim-branch-lines` with `branch`, keyed `branch:roster`; `dim-branch-parent` with `branch`, `head` and `after`, keyed `branch:parent`; `dim-branch-message` with `from`, `to` and, on a final message, `final`; `dim-branch-idle` with `branch` and `status` (`completed`, `failed` or `cancelled`); `dim-branch-ended` with `branch`. The modes are the InputQueue's: Queue for the identity, Append for the roster, the parent block and the primary status note, Steer for the notices and for a final message.
- **The identity blocks' factual sentences.** In a tool line, "You are line C of this session" through "are P's, not yours.", and the label "Your purpose, as P wrote it:"; in a line the user opened and in a plugin line, the block's whole sentence, whose `source` attribute names which of the two it is. A purpose after the label is the creator's own words, carried as content.
- **The roster's factual sentences.** "Lines you opened that are still open. Each is you …" and "All lines you opened have ended."; each listed line carries its id and the first line of its purpose.
- **The parent block's factual sentences.** "Line P has new activity: it was at H when this was written, and you have read it to A.", the action and field names in "Read what is new with the branch tool (action read, branch \"P\")", and the rule "with no after, the read continues from A."
- **The primary status notes' factual sentences.** A note to the new primary line states its id, its parent and its source (from the birth block it repeats), its purpose where the birth block carried one, the line it was switched from, and the rules that hold while the block is the line's latest one: it may open lines, it sends no automatic idle notice to the line that opened it, and it cannot `end` itself. A note to a former primary line states the line now primary, that this line stays open and keeps the lines it opened, that it can no longer open lines, and its end rule for its own case: an ancestor of the new primary line cannot `end` because `end` cascades down, and the note says so; a line that is not an ancestor keeps its source's end rule word for word, so a tool line ends itself with `end` while a line the user opened is told it cannot `end` itself and that the user ends it; the root never ends. Each note is a later block of the same `branch:line` key and therefore replaces the birth identity: it repeats that identity's sentences byte for byte and changes only what the move changes (§12.1).
- **The notices' factual sentences.** "Line C's turn ended (<status>) without a message to you, and C is waiting." and "Line C was ended from outside its own line, by the user or a host operation. It no longer receives messages".
- **The branch tool's actions and modes.** The actions `create`, `message`, `read` and `end` and what each one does; `mode` `append` or `steer` and what it does to the receiver; the fields each action takes, with `action` required and nothing else accepted; the `read` target rule and its cursor rule ("With no after, a read continues after your last read of that line"); and the sentences that state a guarantee: "Returns the line's id", "Pass after to read from another point", "It is your last action" and "A line reads the line it was cut from, and the lines it opened. Messages pass only between a line and the lines it opened."
- **The results and refusals.** `Line created: C`, `Sent to C.`, the page `{items, nextCursor, done}`, `Line C is ending; your final message was sent to P.` and `Line C ends with this message.`; the codes `branch_not_primary`, `branch_limit_reached`, `branch_not_kin`, `branch_not_tool_line`, `branch_ended` and `branch_cannot_end`, with the text each one returns; `after A is past line T's head H; a read starts at or below H`, the error result a read returns for an `after` past the target's head, with no code; and `Invalid branch arguments: …`.
- **The guide entries.** An entry states what its marker is, whose words or which facts it carries, and that it is state or information and never a request. The `<dim-branch-line>` entry states that a later block of that name replaces it, that a block carrying `primary` is the note a move of the primary line writes, and that the entry belongs to TimelineTree, which writes every identity block, so it is in force in every execution profile. The `<dim-branch-parent>` entry belongs to TimelineTree with it and registers at `session` scope, as Goal's does ([Goal §A.5.1](plugins/Goal.md#a51-protocol-and-policy)), because a line's System has to equal its parent's (§7.3): it is in force on the root too, which never receives the block. The `<dim-branch-lines>` entry states its key, `branch:roster`, so a reader cannot take it for the identity under `branch:line`. The entries of `<dim-branch-idle>` and `<dim-branch-ended>` belong to BranchLine, which writes both notices; the `<dim-branch-message>` entry registers with them, and BranchLine is loaded exactly where BranchTool, which writes a message, is. The roster and message entries stay on every line.

**Policy.**

- **The coordination text for the primary line.** The `branch` tool description's guidance: the opening paragraph on what a line is and what coordinating means for the line that has it, the `create` rule for when to open a line and what the brief carries, the `message` rule for choosing a mode and for sending a decision instead of a progress report, and the `read` rule for reading a line's work instead of waiting for a report. [`BranchToolConfig.description`](../../crates/dim-agent-capabilities/src/timeline_tree.rs) replaces the description and nothing else. Two parts of the `create` rule are policy: the four conditions that must all hold before a line is opened, and the boundaries by need with the neighbouring mechanisms (a long command runs in the background, work that needs none of this conversation's context or needs different instructions belongs to another member, one task pursued to completion stays on the line or uses a goal, waiting for something outside the session uses an event subscription). The sentence "When the user asks for a line, open it" is policy too, and it is the exception to the four conditions. The rest of the `create` rule — what `prompt` carries and `Returns the line's id` — is protocol, above. The opening paragraph's "On the primary line you coordinate" sentence and its three clauses are policy: "answer a line's steer and act on what it asks for", "leave a line's part to that line instead of redoing it" and "read a line's work instead of waiting for a report". The roster's closing sentence, "They message you when they find something, need you, or end. Use the branch tool to message or read them, and the `wait` tool to wait for them; end the wait when your lines have nothing left to tell you.", is coordination text too, and its "end the wait when your lines have nothing left to tell you" covers ending the opener's own wait; [`BranchToolConfig.roster_closing`](../../crates/dim-agent-capabilities/src/timeline_tree.rs) replaces it and nothing else. It covers [§12.2](#122-items-for-the-prompt-layer-429)'s first item: how a line briefs a line it opens and assigns its files or area, in the `create` rule.
- **The line's working rules.** The five bullets of "How you work with P:", on the purpose, the reads and messages — including "Do not send P a progress report it can read" and "do not page through P's whole stream" — the workspace, the user and the end, including "Read P once more before you end if the parent block says P has moved", are policy, apart from the clauses that name a mechanism: "You cannot open lines unless you are the primary line", the action and mode names, and "the file was modified after it was last read", the failure `edit` and `write` report, which are protocol, above. It has no factory member yet, and gets one in the change that first evaluates a candidate for it. Its third bullet's "Change only the files P gave you" is the same rule as the `create` brief's file area, stated once here for the line's side of it.
- **The parent block's guidance.** "Read it when what P did changes what you do, and before your final message; do not page through all of P's work." It has no factory member yet, and gets one in the change that first evaluates a candidate for it. The rest of the block is protocol, above.
- **The notices' guidance.** "Read it if you need its state; message it with mode \"steer\" to give it more work, or leave it." and "read it if you need what it did." It has no factory member yet, and gets one in the change that first evaluates a candidate for it.
- **The primary status notes' guidance.** Seven sentences are policy, each behind its factory member: on the new primary line, "The user switched here to continue directly from this line's context and approach: the user's request takes precedence over this purpose, which still says what this line is for." and "Carry this line's main work forward yourself; open a new line only when the four conditions for opening one hold again, not to hand off your current task." and "Avoid redoing work assigned to lines opened before your cut. ..."; on a new primary line the user opened, "Carry this line's work forward yourself; open a new line only when the four conditions for opening one hold again."; on the root, "Carry the user's current work forward here; open a new line only when the four conditions for opening one hold again."; on the former primary line, "Keep coordinating any open lines you already opened, and answer messages or user inputs addressed to this line." and "You share the workspace with line N; do not edit files outside your remaining scope unless asked. ...". `BranchToolConfig.primary_note` and `BranchToolConfig.former_primary_note` replace them and nothing else, and the purpose bullet's "open one only when the four conditions ... and not to hand this purpose off" is the same policy read at the note. Everything else in the notes is protocol: the restated identity, the rules that hold while a line is primary, the line that is now primary, that a former primary line stays open and keeps its lines, whether the line may `end` and why, and "If edit or write fails because the file was modified after it was last read".

**This change's two gates.** The prompt change is accepted only when both hold, and only for this change: they do not change §8.1's general rule. First, no regression under [model-facing prompts §8.1](model-prompts.md#81-the-rule) on every scenario: between the current wording and the new one, no comparison is a regression at the floor of 20 judged runs, with `DELTA` 0.05 and the Holm-adjusted `ALPHA` 0.10 of [`xtask/src/rule.rs`](../../xtask/src/rule.rs), and every scenario reports `pass`. Second, on the three should-open scenarios ([§12.3](#123-evaluation-scenarios)) the line-opening rate rises significantly, and the comparison is made on judged runs alone. The second gate needs more runs than the floor: at `n = 20` per arm a rise from 0.10 to 0.40 is detected only 36% of the time ([model-prompts.md:162](model-prompts.md)), so at 80% power the three comparisons alone would need about **44** runs per arm, and the family the rules judge together is larger than three: every (scenario, probe) pair, the pooled `must_show` and `must_not_show` rows, the failed-run rate and the unjudged-run rate. The fifteen scenarios declare 36 probes, and the family adds the pooled `must_show`, the pooled `must_not_show`, the failed-run rate and the unjudged-run rate, so `m = 40` and the comparison is tested at α = 0.0025, where 80% power needs 54 runs per arm. The plan fixes N from that count and recomputes it if a scenario changes a probe. This document requires only that N be at least 44, that it be recomputed with Holm over the final number of comparisons, and that each should-open arm run it.

A candidate for a policy text is put in place at that text's factory member ([model-facing prompts §8.1](model-prompts.md#81-the-rule)) and keeps the protocol sentences of the text it replaces byte-identical.

The policy texts are judged together, as one family, in the change that first puts them in place: the tool description at `BranchToolConfig.description`, the status notes at `BranchToolConfig.primary_note` and `former_primary_note`, and the roster's closing at `BranchToolConfig.roster_closing`. A later change to one of them re-runs the family, because a text that reads well alone can still lose a comparison another one wins.

## 13. Interactions

| Feature | Interaction |
| --- | --- |
| Goal | Goal's own rules are unchanged. A line's opening input is a sourced input under A5 (§7.4), and Goal's conflict table records it. A new line clears the inherited goal as before. A line reads the line it was cut from, so the evaluator reads the worker's work after the turn it was cut at; Goal's fold does not change, because the evaluator reads through a tool result on its own line. Goal's code changes only through the Pursuit amendment: recognition from `info(own id).source`, ending its evaluator, and the `branch/ended` key ([Pursuit §13](plugins/Pursuit.md#13-ending-the-evaluator-with-427)) |
| Pursuit | The evaluator gets the plugin line's identity block and can read the worker's work after its cut (§6); what it decides is the evaluation, judged as before ([Pursuit §13](plugins/Pursuit.md#13-ending-the-evaluator-with-427)). It is a plugin line, so it is never on a roster, never becomes primary and never opens lines |
| Fork, delete | Fork copies the root only, as before, so a forked Session's primary line is the root ([fork §6.2](appserver/session-deletion-and-fork.md#62-why-main-only)). Delete is unchanged. Ended lines follow their facts (§10) |
| Session close, idle unload | Not an end. Lines restore as open |
| Interrupt | A turn interrupt, or a stop on a line, ends a turn and never a line. An interrupt on the primary line does not touch the lines it opened |
| Session model, profile, prompt, language | The model, the profile and `prompt.refresh` act on the line the user is in, and a line's model comes from the prefix it inherited, so a line that is not the primary line keeps its own configuration (§22). Language is Session-wide and broadcast to every loaded line |
| Scheduler | A newly created job with no `branch_id` recorded takes the primary line at creation time. An existing job keeps the `branch_id` it recorded, so a move of the primary line never redirects a job that was already scheduled |
| Collaboration | Unchanged. Members are Sessions; whether a member's Session mounts `branch` is up to its profile's tool list |

## 14. Clients

**TUI.** Lines get their own panel, view and controls. Opening a line or a fork from a point of the shown Timeline, and where each line was cut, are in [Timeline-first TUI](timeline-ui.md):

- `/lines` lists every line of the Session, each indented under the line that opened it, with its state: working, waiting, ended itself, ended by an operation, ended with X, or creation failed. The primary line is marked 主线 and the root 起点.
- Enter shows the selected line's conversation. The composer submits to the line shown, and the composer's bottom border names it, with the first eight characters of its id.
- `x`, then `y`, ends the selected line, refused on the root, on the primary line and on every ancestor of the primary line.
- `p` moves the primary line to the selected line ("continue from here"), refused on a plugin line and on the line that is already primary, the root included when it is not the current primary line. The panel states that the old primary line stays open.
- Questions and approvals carry the line's label, so two lines asking for the same thing are told apart (`(branch NAME)` on a question, `Permission on branch NAME` on an approval).
- A message between two lines reads as sent on the sender (its `branch` tool call) and as received on the receiver (`◂ from C`).
- Switching the shown line starts another transcript under a boundary row (`line C`), as a session switch does, because terminal scrollback is append-only. Discovery retains task inventory and branch-history state, not a full transcript for every line. The first open, through `/lines` or a side view, retains that line's [bounded transcript window](clients.md#opening-a-conversation-with-a-bounded-window), including when discovery previously failed. Owner-state reads recover actionable work outside the window. Selecting a line separately requests its display details below. Later opens reuse the retained transcript, which continues folding live events even while another line is shown.

The panel is separate from the tasks panel, which lists background commands, members and Workflow runs: its `x` stops a task, and an end is permanent, so the two do not share keys.

```
┌ session 3f2a · 主线 b-7c ────────────────────────────────────────┐
│ … b-7c's conversation …                                          │
│ ◂ from main (steer)  the leak is in cache.rs                    │
│ ◂ from b-02 (steer), final  docs pass done                      │
│                                                                 │
│ ╭─────────────────────────────────────────────────────────────╮ │
│ │ › ask anything…                                             │ │
│ ╰───────────── ~/repo/dim (main) ────────────────────────────╯ │
└─────────────────────────────────────────────────────────────────┘

┌ lines ──────────────────────────────────────────────────────────┐
│ ○ main             起点                                         │
│   b-7c  parser race hypothesis      working · 主线 · shown      │
│   b-91  review the cache change     waiting                     │
│     b-c3  the guard in cache.rs     working                     │
│   b-02  docs pass                   ended itself                │
└ Enter show · p continue from here · x end it · Esc close ───────┘
```

**The branch directory.** `session.branches {sessionId}` (v3, listed in `hello.features` with the other
Session methods under the same admission rules as `session.history`) answers one entry per line: its identity
(`branchId`, with `parentBranchId`, `branchPoint` and `createEvent` when it has them), its `source` as
`TimelineInfo.source` carries it, `state` (`open`, `ending` or `ended`), `head`, and, for an ending or ended
line, `endEvent`. The reply also carries one top-level `primaryBranchId`, the current primary line, rather than a
flag on each row: a top-level field cannot express the illegal states of zero or two primary lines, and a client
that does not know the field ignores it. `main` comes first and the rest follow in `branchId` order. The reply is
not paged: it grows with the number of lines, which a Session's own use bounds, not with history. `TimelineTree` owns
the answer (`directory()`); `branches()` lists the lines that have not ended (open and ending), for the callers that
hold a live line.

`session.branch_primary {sessionId, branchId}` (v3, listed in `hello.features` with the other Session methods) moves
the primary line. The target is an open conversation line: a tool line, a user line, or the root, which is a valid
target whenever it is not the current primary line, so the user can move the primary line back to where the Session
started. A plugin line is never a target. Besides those, the method refuses only a target that is already the primary
line, whatever line that is, with `branch_already_primary`; otherwise it writes `branch/primary` on the root and the
two status notes (§12.1), answers the new `primaryBranchId`, and touches no other line. This is the "continue from
here" action; it writes no Session-level configuration.

The directory read performs no history I/O. Its display fields are null or absent.
`session.branch.get {sessionId, branchId}` (v3, separately listed in
`hello.features`) reads one line's directory record and display fields:
`purpose` (the trimmed first prompt line, null for the root), creation `source`, and
`end` (the existing `BranchEndData`, null without an available end fact).
`cutMessages` remains an optional compatibility field and is not calculated.
The call reads only the identified creation and end facts. A read failure is
reported on this detail call and does not prevent directory listing; an unknown
branch is an error. This read writes no facts and does not reopen an ended line. An ended line inherited by a Session fork can
have no own history; its fixed directory end identity remains even when the
`end` detail is unavailable.

Three rules about `head` decide what a client may do with it:

- An `ended` line's `head` and `endEvent` are fixed. The client passes `head` as `through` to `session.history` to
  read the line, and `endEvent` names the line's own `branch/end`, including when an ancestor's end cascaded to it.
- An `ending` line's `endEvent` is its committed `branch/end`, while its `head` is still live: the line's ended row,
  and with it `branch/ended`, is still ahead. A line reads `ending` from the commit of its own `branch/end` — a tool
  end, a host end or a cascade end — until its ended row is recorded, and input is refused throughout. A
  tool-sourced end written inside a running turn stays `ending` until that turn ends and the close completes. A
  restart does not keep this window: a line whose `branch/end` is committed but whose ended row was never written is
  restored as `ended` (§10).
- An `open` line's `head` is live. Restore appends `branch/resume` and the interruption facts to every open line, so
  `head` moves across a restart; a client re-reads the directory after it reconnects instead of caching a head.

**Web** centres the primary line and follows the other lines in a right-side detail panel: a Lines tree, one tab per opened line, and line-scoped submit, stop, approvals and end ([Web workspace §4.1](web-workspace.md#41-lines-and-branch)). Moving the primary line is the same action as in the TUI, through `session.branch_primary`. A session list shows a Session's running state and its most recent activity from its primary line, so a Session whose conversation is on a line reads as busy or recent by that line, not by its root. Desktop gets the protocol: `session.branch_end`, `session.branch_primary` and the new events in v3 history, plus `primaryBranchId` on the directory.

## 15. Changes

| Module | Change | Why |
| --- | --- | --- |
| [Design rules §2](../architecture/rust-agent/principles.md#2-作用域的含义) | Rewritten: lines of one subject, shared facts, parent and child as the only communication and reading relation, and the shared interaction surface. Relations row: "only between parent and child", reading the parent's committed events through a tool result. Session row: adds "end" | The current text, "equal lines, the relation only records the source", has no communication relation |
| TimelineTree | The identity block in `first_input` for every source. `TreeCommands.end(id, source)` and `TreeCommands.page(id, after, limit)`. `TimelineInfo.source`, filled from the parent's `branch/create` (§5). The end flow and its guards, refusing input once a line is ending, `branch/ended`, the restore rule, reads of ended lines, and the primary line: `branch/primary` on the root, its restore read, and the fact a move writes. Its `directory()` reports an open line, an ending line (its own `branch/end` committed, from a tool, a user end or a cascade, until its ended row is written) and an ended line, with the ending line's live head, and the current `primaryBranchId`. Its node publishes the sequence of the line's latest turn end to its children (§7.1.1) | It owns the line lifecycle, the opening input and the primary line's one authoritative fact |
| TreeTimeline | Consumes `branch/end`, plus the line's next turn end for an end issued by a tool call, and routes them to TimelineTree. It reports every `turn/end` to TimelineTree, which publishes it for the line's children | It routes `branch/create` the same way |
| BranchTool | The action-based tool, plus the new BranchLine branch part. `create` is guarded on the caller being the current primary line and on the Session's open-line cap. `read` and `message` reach the line the caller was cut from and the lines it opened, with one cursor rule | The tool's actions and one line's view of its lines |
| Session facade | `Session::end_branch(id, source)`, `Session::primary(id)` and `Session::set_primary(id, source)`. An ended line's head is `Session::sequence`; its history is read at that fixed head through `TreeCommands.page` | §17 |
| App Server | `session.branch_end` (v3, a user end with a minted `operation_id`, listed in `hello.features` with the other Session methods when `session_execution` is on and the connection's entry is not bare or the daemon profile is not protected). `session.branch_primary` (v3, the same admission). `session.history` on an ended line returns immediately, at its stored head. `session.branches` reads the tree's directory and carries `primaryBranchId` (§14). Every Session-level default target that today names `main` reads the primary line instead: `session.submit`, `session.stop`, `session.idle`, `approval.respond`, `session.goal.*`, `shell.run`, the Session catalog's running state and recent activity, and a newly created scheduled job. `session.model.update`, `session.agent.update` and `session.prompt.refresh` act on the line the request names rather than refusing every line but the root. `ProductEvent::new` carries the new kinds in v3 and leaves them out of v1 and v2 | Two entries and three reads, no business logic |
| dim-agent-types | `branch/end` and `branch/ended` in `EventKind` and the v1 exclusion list, then `branch/primary`, registered in the contract tests. `BranchSource` gains a kind for a line the user opened, distinct from a plugin's. The frozen fixtures are unchanged | Core facts (§16) |
| Storage | `FORMAT` 52 → 53 as a marker-only upgrade `(52, &[])`: no table scan and no data migration, so an older daemon refuses the store instead of failing on its first decode. `SqliteTimelineStore`'s writer thread caches each Session's primary line for the catalog's recent activity, initialised by the same indexed point query that restore uses | A new event kind and a new `BranchSource` variant are rows a 52 daemon cannot decode |
| Goal (Pursuit amendment) | Recognizes its evaluator from the line's source and ends it ([Pursuit §13](plugins/Pursuit.md#13-ending-the-evaluator-with-427)) | Goal owns its evaluator's protocol |
| Branch TypeScript binding | Regenerated `npm/branch-services.d.ts`: TreeCommands gains `end` and `page`, `TimelineInfo` gains `source`, `LineRole` names the root rather than the coordinator, and the directory carries `primaryBranchId`. The `treeCommands` row of the [binding table](branch-typescript.md) lists `end` | TreeCommands is an exported service |
| CLI TUI | §14 | Users follow, talk to, move to and end lines |
| Plugin pages | TimelineTree.md and BranchTool.md, updated with the implementation (`xtask check`) | Pages follow the code |

New public content:

- three events: `branch/end`, `branch/ended` and `branch/primary`;
- four RPC methods: `session.branch_end`, `session.branch_primary`, `session.branches` and `session.branch.get`;
- two TreeCommands methods: `end(id, source)` and `page(id, after, limit)`, and the primary line's read and write;
- `primaryBranchId` on the directory reply;
- `TimelineInfo.source`, read by BranchLine and by Goal;
- a `BranchSource` kind for a line the user opened;
- the `branch` tool's new input.

BranchLine's service is read only by the tool, inside the same plugin. The turn-end signal is crate-internal, beside `TreeCommands.page` and `head`: it is not exported, and the TypeScript binding does not change.

## 16. Format

`branch/end` and `branch/ended` are core events in the `branch` domain, and `branch/primary` belongs to the same domain:

| Event | Data |
| --- | --- |
| `branch/end` | `{source, ancestor?}`. `source` is a `BranchSource` (`{kind: "tool", turn_id, request_id, call_id}`, `{kind: "user", operation_id}` or `{kind: "host", operation_id}`); `ancestor` is a line id |
| `branch/ended` | `{branch, endEvent, source, ancestor?}`. `endEvent` is the sequence of `branch/end` on the ended line; `source` and `ancestor` are copied from it |
| `branch/primary` | `{branch, source}`. Written on the root only. `branch` is the line the Session's conversation is on; `source` is the operation that moved it. The newest such event on the root names the primary line, unless a later `session/forked` on that line voids it |

The format decision for them is recorded in the storage contract: [#427 v20 decision](appserver/native-storage-format.md#427-v20-格式决定). `branch/primary` changes the shape of a stored row, so the store's `FORMAT` rises with it (§15).

## 17. Requirements raised to mechanism owners

**R1. Durable end of one line** (owner: dim runtime, TimelineTree and the Session facade; implemented with this design under #427).

- **Blocked feature and rule.**
  - A line cannot end on its own. `Branch::close()` exists, but no entry calls it and the close is not recorded, so restore opens the line again.
  - The Session facade reads history only from published lines, so an ended line's history becomes unreadable.
  - The facades and the event format are infrastructure ([design rules §4](../architecture/rust-agent/principles.md#4-通过时间线扩展行为) rule 8).
- **Existing mechanisms considered.**
  - `stop/interrupt` ends a turn, not a line.
  - Session close ends every line.
  - `branch/create-failed` exists only before a creation completes.
  - A plugin-owned event cannot work, because restore belongs to TimelineTree and cannot read plugin facts (rules 4 and 7).
  - Reading the store directly from App Server would bypass the facade.
- **Generic capability.** It carries no business meaning:
  - a command `branch/end {source, ancestor?}` on the line and a result `branch/ended {branch, endEvent, source, ancestor?}` on its parent;
  - `TreeCommands.end(id, source)` and `Session::end_branch(id, source)`, which check only the target;
  - `TimelineInfo.source`, filled from the parent's `branch/create`;
  - no new input once `branch/end` is committed;
  - descendants end first;  - restore treats an ended line as inactive and completes a `branch/end` that has no `branch/ended`;
  - an ended line's committed history stays readable at its fixed head, through `TreeCommands.page`.
- **Effect on sequences, batches, interrupt and reload.**
  - Each event is its own batch: `branch/end` on the line, and `branch/ended` on the parent after the close.
  - An end with a `host` source, or because of an ancestor, interrupts the line through `Branch::close()`.
  - On reload, the restore rule above applies. A Session publishes after its restore, so a part that ends a line at load, such as Goal's reconcile ([Pursuit §13](plugins/Pursuit.md#13-ending-the-evaluator-with-427)), waits for `SessionLifecycle::activated()` and then finds every line the directory holds published.
  - The Timeline gains the line's own admission check: `Timeline::input` becomes `input_checked(data, admit)`, which runs `admit` under the commit lock, so an input that commits after `branch/end` is refused with `branch_ended` instead of racing the close. The Dispatcher and batching are unchanged.

## 18. Security

- Lines share the Session's permission policy and Approval. A line is not isolation ([ADR 003](../adr/003-approval-scope.md)).
- Only TimelineTree and BranchLine write the markers. Other producers that write blocks (plugins, through `TreeCommands.input`) are trusted in-process parts. RPC writes no blocks.
- `session.branch_end` and `session.branch_primary` are trusted user entries, as `session.branch_create` is. They need no approval, because they are not agent actions.
- Any trusted in-process part can end any open line the end guards allow, just as it can write input to any line. Approval governs the tool's `end` like any other call.
- The primary line confers no authority. Moving it changes which line default targets resolve to and which line may open lines; it does not widen a line's Permission, Approval or file roots ([ADR 003](../adr/003-approval-scope.md)).
- Diagnostics record ids, sequences and the source kind, never message or purpose text.

## 19. Rule check

Rules: [design rules §1, §2 and §4](../architecture/rust-agent/principles.md), [ADR 002](../adr/002-plugin-scope.md) D1–D5, and [ADR 003](../adr/003-approval-scope.md).

| Rule | How this design complies |
| --- | --- |
| §1 Capabilities, App Server boundary | Capabilities: TreeTimeline (existing) and BranchLine (new). TimelineTree and BranchTool's Session part are ordinary parts. App Server gains two entries and one read path, with no business code |
| §2 Scopes | The line lifecycle stays the Session's (TimelineTree). One line's view of its lines is branch state (BranchLine). Lines influence each other only by writing input and learn about each other only by reading, now only between parent and child (§2 rewritten). Capabilities affecting a branch are not bound by that rule (§4) |
| §4.1 Command events | An end is `branch/end`. TimelineTree decides and writes `branch/ended`. A message is input whose receiver's InputQueue and Loop decide. An end of an ended line, or a refused message, writes nothing, and the reply says why |
| §4.2 No recall after handoff | A committed message belongs to the receiver. A `branch/end` is not withdrawn |
| §4.3 Before callbacks | None |
| §4.4 No business meaning in core | Dispatcher, `before` and batches are unchanged; the Timeline's admission check is generic (§17 R1). The identity text is TimelineTree's own domain, the lines of a Session. A tool line's protocol is written where `first_input` already writes tool-specific text, in the module that owns the tool |
| §4.5 Batches | `branch/end` stands alone. `branch/ended` comes last, after the close. The roster and the ended notice are one input. `branch/primary` on the root stands alone, and the two status notes are one Append each, after it |
| §4.6 Context | Identity, roster, messages, notices and the primary status note are adopted inputs. `read` returns a tool result. Nothing else enters the context |
| §4.7 Public events and services | BranchLine folds its own line's public events. The tool reads BranchLine (same plugin) and TreeCommands. `read` returns committed events as a tool result, the way Collaboration's `read` does, and no part folds another line's facts. Goal classifies the opening input by its own published A5 convention (§7.4). BranchLine and Goal read `TimelineInfo.source` through TreeCommands, a public service |
| §4.8 Infrastructure | The facade and format changes are R1 (§17). They come from the line lifecycle itself and close within the runtime |
| §4.9 Only facts later decisions use | `branch/end` decides the close and the restore. `branch/ended` decides restore inactivity and the roster. `branch/primary` decides which line the Session's defaults resolve to and which line may open lines; it carries a role, and the action that moved it, nothing else. No UI-only event and no presentation-only field |
| ADR 002 D1 Scope | As in §5 |
| D2 No wiring modules | BranchTool's feature registers BranchLine itself |
| D3 Scope dependencies | BranchLine depends outward on the Session's TreeCommands. No core part depends on it |
| D4 Configuration | None |
| D5 Language-neutral contracts | Events, the RPC method, markers and the tool schema are data. TreeCommands' new methods are exported and typed for TypeScript |
| ADR 003 Approval | The tool keeps risk `session`, and Approval admits every call. A user end is not an agent action |

## 20. Acceptance

These are flows with a fake model, in `branch_flow.rs`, `flows.rs` and the App Server transport flows.

1. The primary line creates two lines. A roster under `branch:roster` lists both. Each child's identity, under `branch:line`, leaves the inherited roster alone, and a child that opens a line of its own keeps both. After a compaction on the opener and on a child, the keyed baseline replays each roster and each identity.
2. A child sends its opener an Append: it does not wake the opener, which sees it at its next request. A Steer wakes the opener's `wait`, or starts a turn when the opener is idle. A Steer from an opener wakes an idle child.
3. These are refused and write nothing: a sibling message or read, a line and its grandparent, `create` on a line that is not the current primary line, `message` on a plugin line, a line's read of a plugin line, and `end` on the root, the primary line or an ancestor of the primary line.
4. When a child's turn ends without a steer, one idle notice reaches the line that opened it, with the turn's status. None is written when the turn steered its opener, called `end`, left a driving input pending, or when the line is the current primary line.
5. A tool line ends itself: the final message (steer) and `branch/end` with a `tool` source are written, the close comes at the turn end, then `branch/ended` on its opener and the roster update, with no ended notice. A later message is refused (`branch_ended`). The tool's `read` and `session.history` return the ended line's history.
6. User end through `session.branch_end`: `branch/end` has a user source with App Server's id, the line's turn is interrupted, and its opener gets the ended notice as a steer. The root and the primary line cannot be ended, and neither can an ancestor of the primary line. A second end returns `branch_ended`. Submitting to an ended line returns `branch_ended`.
7. Restore: an ended line is not opened, and neither are its descendants. A `branch/end` without a `branch/ended` gets its result written at restart.
8. A Pursuit evaluator gets the plugin line's identity, can read the worker's work after its cut, and the existing Pursuit flows pass unchanged.
9. Two lines edit the same file: the second gets `file_modified_since_read` (existing behavior).
10. v1 and v2 clients do not see the new kinds. v3 history includes them, and `hello.features` lists `session.branch_end` and `session.branch_primary`.
11. Reading the parent: a line's first read of the line it was cut from starts after the cut; an explicit `after` below the cut is raised to it; an explicit `after` past the target's head is refused through a tool result that names the head, and nothing is stored; a read with no `after` continues from the caller's last `nextCursor`, in both directions, and survives a reload; an `after: 0` on a line the caller opened re-reads it from its start.
12. The parent block: an idle line is told its parent moved without a turn starting, and the block is in the first request of its next turn, ahead of the input that starts it; a running line gets it with its next model request, once; a held block is not written over; a consumed block is followed by a new block for a new head; a line that read to the head is not told; the turn that only opened the line is not activity, while a later tool call or an operator input is; a reload with no movement writes nothing, and a reload with a block due writes one; an ending line and the root get none; the block replays after a compaction of the line; a nested line's opening input carries the block that names its own parent at the cut, and a line the root opened gets none.
13. The parent block changes no Goal decision on an evaluator line: it is an Append input whose first block is not a goal marker and whose key is not a goal key, so the fold's `driving`, `lastOperator`, count and conclusions are unchanged (`goal.test.mjs`).

Goal flows, in the crate's flow tests under `crates/dim-goal/tests` (§7.4):

11. A line opened with `session.branch_create`, whose opening input carries the identity block, behaves exactly as a line the root opened does: `set` on that line, its continuations, and a later operator input that resets the count.
12. A Pursuit evaluator created under the new opening input is recognized and its conclusion is applied. The working line's count is unchanged.
13. The identity block changes nothing in the objective and status fold, the decided set or the continuation list.

Line end and sources:

14. `TreeCommands.end` ends any open line the end guards allow, whoever calls it, for example a plugin part with a `host` source on a tool line, which gives the opener the ended notice. It refuses the root, the current primary line, an ancestor of the primary line, an unknown line and an ended line, and writes nothing.
15. Ending a plugin line that has a child ends the child first, with `ancestor` set and `source` copied, then the line. Ending any line whose descendant is the current primary line is refused before anything is written.
16. `TreeCommands.list` and `info` return `source` for a new child and for a restored active one, and none for the root. Ended directory entries load their source only in display detail. The stored directory rows are byte-for-byte unchanged.

The branch directory:

17. `session.branches` returns `main` first and the rest in `branchId` order, with each line's identity, source, state and head, and one top-level `primaryBranchId`. A cascade end records the ended child with the sequence of its own `branch/end` as `endEvent`, and `session.history` with `through` set to that child's `head` reads it. A line reads `ending`, with its own committed `branch/end` as `endEvent` and a live head, from that commit — a tool end, a user end or a cascade end — until its ended row is written, and `ended`, with the same event and a fixed head, after it. A tool-sourced end inside a running turn stays `ending` until that turn ends and the close completes.
18. After a restart the directory keeps ids, parents, branch points, create events and states; ended lines keep their `head` and `endEvent`, and an open line's `head` is at or above its old value. A line that ended while its turn was still running restores as `ended`, not `ending`.
19. A fork's directory lists the lines that ended before the fork, with the end the source recorded, and those lines have no events in the fork. A v1 or v2 client has no `session.branches`, and a closed Session answers `session_closed`.

The primary line:

20. A Session with one line is on its root, and its restore performs no query for it. A Session with more lines reads exactly one fact on the root and answers the target of the newest `branch/primary`, or the root when the newest of the two kinds is a `session/forked`, or when there is none. A target that has ended, and a target that is a plugin line, answer the root.
21. `session.branch_primary` moves the primary line: `branch/primary` is written on the root, exactly two status notes are written, the new and the old primary line, and every other line receives nothing. A refusal writes nothing: a plugin line, an ended line, an unknown line and whatever line is already primary, refused `branch_already_primary`, so the root is refused while it is primary and accepted as a target once another line holds the role. Both notes are `<dim-branch-line>` blocks whose `primary` attribute says which role the line holds, and each restates the identity it replaces, down to the parent and the source, so a compaction of a switched line replays a block that still names it and its opener.
22. After the move, `session.submit`, `session.stop`, `session.idle`, `approval.respond`, `session.goal.*`, `shell.run`, the catalog's running state and recent activity, and a newly created scheduled job resolve to the new primary line. A scheduled job that recorded a `branch_id` keeps it.
23. A line that was the primary line can still message, read and end the lines it opened, and a line that is not the primary line is refused `create` with `branch_not_primary`. A line that was a tool line and became primary stops sending its idle notice and is refused `end` on itself.
24. A fork of a Session whose primary line was not the root answers the root as its primary line, because every `branch/primary` it copied is older than its `session/forked`.
25. Opening a line is refused with `branch_limit_reached` once the Session holds the cap of open lines the tool and clients opened, and a plugin line or a user line does not count toward it.

## 21. Ablation review

What is left, and why each piece is needed:

| Piece | Why |
| --- | --- |
| The identity block on every child, under one key | Without it a line forgets which line it is after compaction, or takes its opener's roster as its own |
| The roster under its own key, on every line that opened lines | Coordination follows the parent-child relation, so every opener needs a roster; sharing the identity's key would let a roster retire a non-root line's own identity |
| Messages as `input/message`, with append and steer | The only path between lines. The modes are InputQueue's |
| `read` for an opener | An opener checks a line's work without asking for it |
| The idle notice | Without it an opener waits on a line that stopped silently |
| `branch/end` and `branch/ended` | Without them a line cannot end durably, and restore reopens it |
| The end guards | Ending a line ends every line below it, so the primary line and its chain must be refused or a move would be undone by an ancestor's end |
| `source` on the end (`BranchSource`) | A create and an end record their issuer the same way, so a plugin's end is not recorded as the line's own |
| A source kind for a line the user opened | Only a conversation line may be primary or open lines, and the core must not parse a plugin's `operation_id` to tell them apart |
| `TimelineInfo.source` | An opener finds its own lines without parsing prompt text |
| `branch/primary` on the root | One authoritative, non-model fact: the App Server, the store's writer thread and restore all read it without a model or a keyed block, and the root cannot end |
| The two status notes | Each of the two lines whose rule changed learns it once; no other line is touched, so no other line's cache prefix moves |
| The open-line cap | The tree can grow deeper only when the user moves the primary line, and a total keeps the work bounded without a depth number that would forbid a legitimate third move |
| The directory's `primaryBranchId` and its `endEvent` and `ending` state | Without `primaryBranchId` a client cannot centre the right line; without `endEvent` a client cannot name a cascaded line's end; without `ending` the window between a line's own `branch/end` and its ended row reads as open |
| The tool restricted to the primary line, and to the lines a caller opened | One line carries the Session's conversation and may grow the tree; creators keep their own protocols, and coordination stays where it was made |
| The prompt rules on files | The only guard is FileReads; the rest is protocol |

Removed:

- caps on messages, steers or rounds;
- attribution of file changes;
- changes to WorkspaceTools;
- a channel between siblings;
- per-line configuration (system, model, tools);
- a result contract and hand-back;
- a separate event or concept for moving the primary line (it is one `branch/primary` fact);
- separate events or concepts for messages (they are InputQueue modes);
- an opener ending its lines through the tool;
- a `by` field with fixed values (the existing `BranchSource` records the issuer);
- a category field on plugin lines (`operation_id` already is the opener's tag);
- a runtime check of who ends a line;
- a depth cap separate from the total cap (depth grows only on a user move);
- a stored `primary_branch_id` column (the fact is one event, and the writer thread caches it).

## 22. Later options

**Per-line narrowing within the Session's limits.** Narrowing within the Session's limits keeps the same subject, so it is compatible with design rules §2. It is not part of this design. When it is built:

- **Fewer tools:** refusal at call time, as `create` on a line that is not the primary line is refused now. The tool definitions and the cache prefix stay identical.
- **Lower permission:** a per-line policy intersected with the Session's. It needs the per-branch Permission narrowing that [Pursuit §10](plugins/Pursuit.md#10-later-options) already names.
- **Cheaper model:** per-line configuration set at create, before the first input. A different model shares no cache with its parent, so it trades the inherited prefix for cost.

**Model, profile and prompt per line.** The model, the profile and `prompt.refresh` act on the line a request names, and a line's own configuration comes from the prefix it inherited. A line keeps the configuration of its own history, so moving the primary line does not change what the new primary line's earlier turns were run under; a change applies to the line the user is on. Language stays Session-wide and broadcast to every loaded line.

### Resume reads

BranchLine mounts at the committed cutoff with no historical event query. It does not restore old turn counters, steering progress or a table of held inputs. A recovery terminal for a turn whose start was not consumed by this generation produces no idle notice. After the Session is published, roster reconciliation uses the currently open child directory and reads a missing purpose at its known creation position. It compares only the last roster block. Parent notification reads the last parent block and the cursor of that parent when needed. Read cursors are not eagerly loaded for other targets. No recovery fact is added.

TimelineTree locates each child's terminal kind and end position by identity, without creation or end body decoding for an already ended child. Only a line that will mount reads its creation source at its known position. It does not page through all parent lifecycle facts. Existing cuts were accepted on creation and are immutable; restoration uses their directory values without rendering the inherited context to validate them again. New cuts retain their creation-time validation.
