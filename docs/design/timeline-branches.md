# Timeline branches

Status: Proposed, not implemented ([#427](https://github.com/arcships/dim/issues/427)). This document holds both the requirements and the design: §1–§3 answer #427's questions, and §4 onward is the design. Scope: the TimelineTree plugin (TimelineTree, TreeTimeline, BranchTool, plus a new branch part for BranchTool), one App Server built-in method, two core events, the CLI TUI, and a rewrite of [design rules §2](../architecture/rust-agent/principles.md#2-作用域的含义). Language: Rust. Chinese: [中文](../zh/design/timeline-branches.md).

## 1. What branches are for

A **branch** is a line of the same agent inside one Session. The Session is one subject, the "I": its identity, its interaction surface with the user, its capabilities and its workspace. A branch is one line of that subject's behavior.

A child starts from its parent's history at a cut. Every event up to the cut is the child's own history as well, and those rows never change. After the cut, the child reads only its own events ([timeline.rs](../../crates/dim-agent-runtime/src/core/timeline.rs)).

Lines share facts, not context:

- Each line assembles its own context from its own facts.
- Each line compacts on its own.
- A line influences another line only by writing input to that line's Timeline.

The parent–child relation records where a history came from. It is also the only relation along which lines talk. It does not mean control, and it does not mean that a result is owed.

Answers to #427:

| # | Question | Answer |
| --- | --- | --- |
| 1 | Which scenarios, in what order | Three, in order (§3): plugin lines (delivered), lines the agent opens from main (this design), and lines the user opens from a history point and then compares and adopts (later) |
| 2 | Branch versus Collaboration member versus fork | §2. A branch is the same agent. A member or a fork is another Session |
| 3 | When a branch finishes, who needs its result, and in what form | No line owes another a result. Lines tell each other what matters while they work (§6). A line the agent opened ends with a final message to main (§10). A plugin line follows its creator's protocol, for example Pursuit's conclusion |
| 4 | When is a branch over, and is it visible afterwards | When it ends itself or the user ends it (§10). An ended line stays in the branch directory. Its history stays readable to clients and to main |
| 5 | What users see and do | Follow any line, talk to it, answer its questions and approvals, and end it (§9, §14). Compare and adopt come with scenario 3 |
| 6 | What "done" means | The acceptance flows (§20) pass. The evaluation scenarios (§12.3) have been run locally on real models and reported. The release demo ([#439](https://github.com/arcships/dim/issues/439)) shows main opening lines that talk with it and then end |

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
| S1 | **Plugin lines.** A plugin opens a line under its own design. Example: [Pursuit](plugins/Pursuit.md)'s evaluator, a host-sourced child cut at the worker's turn end, whose conclusion goes back as input (implemented, [#422](https://github.com/arcships/dim/issues/422); `runtime/plugins/goal`) | Delivered. This design adds only the identity block (§7.1) |
| S2 | **The agent opens lines from main.** Examples: test two hypotheses for a bug on two lines while main keeps reading; implement on main while a line reviews each change as it lands; research an API or one area of the code while main continues; try a second approach on separate files | This design |
| S3 | **The user opens a line from a history point, compares it, adopts it.** `session.branch_create` already opens host lines. With this design the user can follow those lines, talk to them and end them. Compare and adopt need a design of their own: what adopting a line means for main's context and for a shared workspace | Later, after this design |

These needs are not branches:

| Need | Why it is not a branch | Where it belongs |
| --- | --- | --- |
| Delegate to a specialist with its own instructions | It is another identity | Collaboration member |
| Judge whether to approve a tool call | It is a decision about one action, not a line of work | A capability with a host port ([ADR 003](../adr/003-approval-scope.md)) |
| Rewind main to an earlier point | It changes how main sees its own history; it is not a new line | A separate requirement. Today the answer is `session.fork` |
| Make edits in parallel, in isolation | Lines share the Session's workspace, and no per-line workspace exists | A fork, or a member with its own working directory |

## 4. Tree and roles

- **Depth.** The architecture allows lines at any depth. `session.branch_create` takes any parent. Goal accepts any `branchId`, so an evaluator opened under a child is a grandchild.
- **Kinship.** Parent and child is the only relation along which lines communicate. A parent and each of its children write input to each other. Siblings and more distant relatives do not (§6).
- **The agent's tool.** The `branch` tool opens lines only from `main`, the root line, which has no parent. The lines it opens are leaves: they cannot open lines of their own. The agent therefore has one coordinator, `main`, and every line the tool opened talks with `main`.
- **Sources.** Every line has a source, recorded in its parent's `branch/create`:
  - A **tool line** was opened by the `branch` tool.
  - A **host line** was opened by a plugin (`TreeCommands.create`) or a client (`session.branch_create`). Its creator owns its protocol.
- **Reach of the tool.** The tool's `message`, `read` and `end` actions work only on tool lines. A host line is not on main's roster, and the tool neither messages it nor ends it.

| Line | Opens lines with the tool | Messages, with the tool | Reads, with the tool | Ended by |
| --- | --- | --- | --- | --- |
| `main` | Yes | Its tool lines | Its tool lines, open or ended | Never; the Session's close is not an end |
| Tool line | No (the tool result refuses) | `main`, its parent | — | Itself (`end`), the user, or an ancestor's end |
| Host line | No | — (its creator's protocol) | — | The user, an ancestor's end, or itself through its creator's part (`TreeCommands.end`) |

## 5. Parts

Every part belongs to the TimelineTree plugin ([timeline_tree.rs](../../crates/dim-agent-capabilities/src/timeline_tree.rs)), and each sits in the scope that owns its state ([ADR 002](../adr/002-plugin-scope.md) D1).

| Part | Scope | Capability | Owns in this design |
| --- | --- | --- | --- |
| TimelineTree | Session | No | The line lifecycle: the identity block in every child's opening input, the end of a line (`branch/end` → `branch/ended`), refusing input once a line is ending, the restore rule, and reads of an ended line's history |
| TreeTimeline | Branch | Yes | Routes `branch/end` and the turn end that a self end waits for to TimelineTree, as it routes `branch/create` today |
| BranchTool | Session | No | The action-based `branch` tool (§11). It finds the calling line's BranchLine per call, the way WorkspaceTools finds FileReads |
| **BranchLine** (new) | Branch | Yes | One line's view of its lines. It folds its own facts: its role (main, tool line or host line, read from the marker of its own opening input), its parent, its open tool lines, and the state of its current turn. It writes the roster, the idle notice and the ended notice |

BranchLine injects `B:Dispatcher`, `B:TimelineIdentity`, `B:BranchLifecycle` and `S:TreeCommands`, and provides `B:BranchLine`.

It decides the way Goal does: from consumed state, through an owned write task that rechecks before writing, and only on live events after its load cutoff. A reload therefore writes nothing twice.

## 6. Messages between lines

A message is an `input/message` on the receiver's Timeline, written through `TreeCommands.input`. No new input path is added. A message carries one raw block, marked by its producer and without a key, and the mode of [InputQueue](../architecture/rust-agent/plugins/InputQueue.md):

| Mode | Effect on the receiver | When to use it |
| --- | --- | --- |
| `append` (default) | Rides with the receiver's next model request. It never starts a turn and does not wake `wait` | Progress or context the receiver should have at its next step |
| `steer` | Enters the receiver's current turn at its next step, or starts a turn if the receiver is idle. It wakes `wait` | The receiver must act on it now, or the receiver is idle |

Queue is not exposed. The only Queue input one line gives another is the opening input.

These are Collaboration's message modes and words (`append`, `steer`), with the same meaning, because both are InputQueue's modes.

- **Marker.** `<dim-branch-message from="C" to="P">`. The receiver tells a message from the user's words by its marker. RPC accepts only operator content, never blocks, so a client cannot forge a message.
- **Allowed pairs.** `main` and each of its open tool lines, in both directions. Any other pair is refused through the tool result, with nothing written: siblings, host lines, ended lines.
- **Read.** `read {branch, after?, limit?}` lets `main` read one of its tool lines, open or ended. It reads complete committed batches from the store through `TreeCommands.page`, and renders them the way Collaboration's `read` renders a member's page: inputs, text, tool calls and results, and turn ends, with `nextCursor`. A line does not read `main`. What a line knows of `main` after the cut is what `main` sends it, so `main` decides what each line needs, and no line pulls another line's whole stream into its own context.
- **No caps.** There is no limit on the number of lines, messages or steers. A message loop ends through the prompt rules (§12), through a line ending itself, or through the user (interrupt or end).
- **Idle notice.** When a tool line's turn ends, BranchLine on that line writes one steer to `main` (`<dim-branch-idle>`, §12.1), unless one of the following holds:
  - the turn already sent `main` a steer message;
  - the turn called `end`;
  - the line has a driving input pending.

  This way a line that stops without telling `main` never leaves `main` waiting on silence, and a line that already steered `main` does not notify it twice. The notice carries the turn's status (`completed`, `failed` or `cancelled`).
- **Waiting.** `main` waits for its lines with the existing `wait` tool. A steer message, an idle notice or an ended notice wakes it ([wait.rs](../../crates/dim-agent-capabilities/src/wait.rs)).

## 7. Identity and context

### 7.1 One key, `branch:line`

Every line answers "which line am I, and which lines are mine" with blocks under one key, `branch:line`. The key is the same on every line.

- **On a child: its identity.** TimelineTree writes it on every child, tool line or host line. It is the first block of the child's opening Queue input, keyed `branch:line`, and it states:
  - the child's id, its parent and the cut;
  - that the child is the same agent and the history above is its own;
  - that it works in parallel from here on;
  - that messages above addressed to the parent, such as the roster, belong to the parent.

  A tool line's identity also carries its purpose (the `create` prompt) and the line protocol. On a host line, the creator's prompt stays the opening input's `user` content, after the block.
- **On `main`: the roster.** BranchLine on `main` writes a keyed Append from `main`'s own facts:
  - `branch/create` with a tool source;
  - `branch/create-failed`;
  - `branch/ended`.

  The roster lists each open tool line with its id and the first line of its purpose. When no tool line is open any more, the key is retired.
- **Superseding.** A child inherits its parent's roster in its prefix. Its own identity block, under the same key, supersedes that roster. From the child's opening input on, the latest live `branch:line` block on the child is its identity.

Key and mode are independent ([history.rs](../../crates/dim-agent-capabilities/src/history.rs) `render_input`). Goal's `set` already puts keyed blocks in a Queue input ([Goal §5](plugins/Goal.md#5-context-mapping)).

The identity is part of the opening input, not an Append written at `branch/start`, for two reasons:

- An Append can commit after the opening input ([Goal §3.4](plugins/Goal.md#34-a-new-branch-clears-the-inherited-goal)).
- The purpose is not known at `branch/start`.

### 7.2 Compaction

Compaction is per line. A compaction is a `context/compaction` fact on the compacting line's own Timeline.

- A child inherits its ancestors' compaction records up to its cut. Event ids are stable across the inherited prefix, so `retain_from` replays the same way on the child ([ContextCompaction](plugins/ContextCompaction.md)). After the cut, the child compacts on its own.
- A child's compaction never changes its parent, and a parent's compaction never changes a child.

The keyed baseline replays the latest live block of each key from before `retain_from`. As a result:

- when `main` compacts, its roster replays;
- when a child compacts, its identity replays.

One key means every line's baseline holds exactly one answer to "which line am I". An inherited answer can never outlive the line's own. Messages carry no key: after a compaction they survive in the summary or in the retained turns, like any other input.

### 7.3 Cache prefix

Lines differ only in their tails:

- The root adopts system and model. Children do not change them ([prompt](../architecture/rust-agent/prompt.md)).
- Prompt sources load with main's scope and are inherited ([PromptSources](../architecture/rust-agent/plugins/PromptSources.md)).
- One ToolRegistry per Session gives every line the same tool list. That is why `create` on a child is refused through the tool result instead of by removing the tool.
- The identity block is the child's first event of its own, directly after the inherited prefix. A child's first request therefore reuses the parent's prefix up to the cut.
- Roster updates are Appends at the end of `main`'s history.

### 7.4 Goal and Pursuit (verified on main)

- **Evaluator recognition is unchanged.** Pursuit recognizes the evaluator by the first line of the `user` text of the first Queue input (`runtime/plugins/goal/goal.ts:377-384`).
  - The identity goes in `blocks`, and the creator's prompt stays `user`.
  - `render_input` renders raw blocks first, then the operator's words ([history.rs:181](../../crates/dim-agent-capabilities/src/history.rs)).

  So the model sees the identity before the prompt, and the `user` text Goal reads is unchanged.
- **Operator input on a host line.** Goal counts an input as operator input when it has `user` and its first block does not start with `<dim-` (`goal.ts:373`). With the identity block first, a host line's opening input no longer counts as operator input. That has no effect:
  - Goal clears the inherited goal on every child at its start ([Goal §3.4](plugins/Goal.md#34-a-new-branch-clears-the-inherited-goal)).
  - The opening input comes before any `set` on that line.

  So `lastOperator` on that line never feeds a count.

## 8. Workspace

Lines share the Session's workspace. No line can know who changed a file, and the design does not try to attribute changes. The guard that already exists is enough:

- FileReads keeps a read binding per line.
- `edit` and `write` refuse with `file_modified_since_read` when a file changed after this line last read it, whoever changed it ([edit.rs](../../crates/dim-agent-capabilities/src/workspace_tools/files/edit.rs), [write.rs](../../crates/dim-agent-capabilities/src/workspace_tools/files/write.rs)).

The rest is protocol, carried by the tool line's identity and by the tool description:

- `main` gives each line its own files or area in the `create` prompt.
- A line tells `main` before it changes any file outside its area.
- On `file_modified_since_read`, the line reads the file again and tells `main` before it continues.

WorkspaceTools is unchanged.

## 9. The user

- The interaction surface belongs to the Session. Every line deals with the user as this Session.
- **Questions.** Any line can ask with `ask_user`. The answer returns as that call's tool result on the line that asked ([Questions](plugins/Questions.md)).
- **Approvals.** Approvals for a line's tool calls go straight to the user. Approval notices carry `branchId`, and clients label the line.
- **Nothing is relayed through `main`.** It is the same subject asking its own user.
- **Submitting.** The user submits to any open line (`session.submit` with `branchId`, unchanged).
- **Interrupting.** Interrupting a line ends its turn, not the line.
- **Ending.** The user ends any line except `main` (`session.branch_end`, §10).

## 10. Ending a line

| Event | Timeline | Data | Role |
| --- | --- | --- | --- |
| `branch/end` | The line that ends | `{by: "self" \| "user" \| "ancestor"}` | Command |
| `branch/ended` | Its parent | `{branch, endEvent, by}` | Result, written by TimelineTree |

This pair mirrors `branch/create` and `branch/create-failed`: a command on one Timeline, routed by TreeTimeline, and a result from the tree. `endEvent` is the sequence of `branch/end` on the ending line.

Who ends a line:

| By | Entry |
| --- | --- |
| `self` | `TreeCommands.end(id)` for the calling line: the tool's `end` on a tool line, or a plugin part on its own line |
| `user` | `session.branch_end {sessionId, branchId}` through the Session facade |
| `ancestor` | TimelineTree, which ends a line's descendants before the line itself |

- `main` never ends. Closing the Session is not an end.
- A parent does not end a child. It asks the child, by message.
- Ending a line that has already ended writes nothing, and the reply is `branch_ended`.

Sequence:

1. The entry commits `branch/end` on the line. From then on the line accepts no new input: any entry fails with `branch_ended`.
2. TreeTimeline on the line consumes `branch/end` and hands it to TimelineTree.
3. TimelineTree ends the line's descendants first (`by: "ancestor"`), each through these same steps.
4. The close happens:
   - for `self`: at the line's first turn end after `branch/end` (the `end` call is part of that turn), or at once if the line is idle;
   - for `user` and `ancestor`: at once, interrupting the current turn.
5. `Branch::close()` interrupts the line, settles its work and unloads it ([branch.rs](../../crates/dim-agent-runtime/src/branch.rs)).
6. TimelineTree writes `branch/ended` on the parent, last.

On the parent, BranchLine:

- updates the roster with an Append, or retires the key;
- for `by: "user"`, writes the ended notice in the same input, as a steer.

For `by: "self"`, the line's final message already reached `main` as a steer before `branch/end`.

**Restore.** A child is inactive when its parent has a `branch/ended` for it. This joins the existing rules: the parent is inactive, the parent has a `branch/create-failed` for the child, or the child's creation never completed. A line that has `branch/end` but whose parent has no `branch/ended` for it is not opened. Restore completes such ends deepest line first, writing each missing `branch/ended` on the parent with `by` taken from `branch/end`.

**After the end:**

- The line stays in the branch directory, and its history stays readable from the store, through `session.history` (v3) and through the tool's `read`. Its head never moves.
- A fork copies directory rows and main's events as it does today. A line that ended before the fork point is inactive in the fork too, because its `branch/ended` is in the copied prefix.
- Deleting the Session removes the line along with it.
- Session close and idle unload are not ends: lines restore as open.

**Plugin lines.** Pursuit's evaluator does not end itself yet. Letting it end its own line after its conclusion is a later amendment to Goal, which owns that protocol.

If the user ends an evaluator, the interrupt ends its turn `cancelled`. If Goal on the evaluator does not write `failed` before the line closes, the goal is left in the state that [Pursuit §4.2](plugins/Pursuit.md#42-evaluator-branch) already hands to the user: pause, then resume.

## 11. The `branch` tool

The tool takes one action-based input, which replaces `{prompt, at?}`. Its risk stays `session`, and Approval admits each call as it does today.

| Action | Input | Who | Effect | Result text |
| --- | --- | --- | --- | --- |
| `create` | `prompt`, `at?` | `main` | Opens a tool line cut at `at` (default: before this request's model start, as today) | `Line created: C` |
| `message` | `to`, `content`, `mode?` (`append` \| `steer`) | `main` → its open tool line; tool line → its parent | One `input/message` on `to` | `Sent to C.` |
| `read` | `branch`, `after?`, `limit?` (1–200) | `main`, on its tool lines | One page from the store | The page as JSON: items, `nextCursor`, `done` |
| `end` | `content` | A tool line, on itself | A final message (steer) to the parent, then `TreeCommands.end` | `Line C is ending; your final message was sent to P.` |

The input schema is `{action, prompt?, at?, to?, content?, mode?, branch?, after?, limit?}`, with `action` required and `additionalProperties: false`.

Refusals come back as error results (code in `structuredContent`) and write nothing:

| Case | Text | Code |
| --- | --- | --- |
| `create` off `main` | `Only main opens lines. Tell main if more parallel work is needed.` | `branch_not_main` |
| `message` or `read` outside kinship | `You can message only the lines you opened and the line that opened you.` | `branch_not_kin` |
| `message` or `read` on a host line | `Line C was not opened with this tool; it follows its own protocol.` | `branch_not_tool_line` |
| `message` to an ended line, or a race with an end | `Line C has ended. Read it to see what it did.` | `branch_ended` |
| `end` on `main` or a host line | `Only a line opened with create can end itself.` | `branch_cannot_end` |

## 12. Prompts

### 12.1 Context mapping

`C` is the line, and `P` is its parent (`main` for a tool line). Content inside markers is escaped as TimelineTree escapes it today (`context_block`).

| Write | Where, key | Mode | Text |
| --- | --- | --- | --- |
| Identity, tool line | `C`, opening input, first block, `branch:line` | Queue | `<dim-branch-line branch="C" parent="P" source="tool">` / `You are line C of this session. Line P opened you at this point of its history to work in parallel with it. You are the same agent as P: everything above is your own history, and you continue from here on your own line. P keeps working on its line; you do not see its work after this point, and it sees yours only through what you send it. Messages above that were addressed to P, such as its list of open lines, are P's, not yours.` / blank / `Your purpose, as P wrote it:` / `<prompt>` / blank / `How you work with P:` / `- Work on this purpose only. You cannot open lines; if more parallel work is needed, tell P.` / `- Tell P with the branch tool (message, to "P"). Use mode "steer" for what P must act on now: a finding that changes its work or another line's, a blocking question, a conflict. Use "append" for progress P should see at its next step. Do not repeat what P already knows.` / `- You share the workspace with P and its other lines. Change only the files P gave you; before changing any other file, tell P which. If edit or write fails with file_modified_since_read, another line may have changed the file: read it again and tell P before you continue.` / `- The user can talk to you directly. Answer the user, and ask with ask_user when only the user can decide.` / `- When your purpose is done, or cannot be done, call the branch tool with action "end" and content saying what you found, did, changed and left undone. That is your last action; P receives it.` / `</dim-branch-line>` |
| Identity, host line | `C`, opening input, first block, `branch:line`. The creator's prompt is the input's `user` | Queue | `<dim-branch-line branch="C" parent="P" source="host">` / `You are line C of this session, opened from line P at this point of its history. You are the same agent as P: everything above is your own history, and you continue from here on your own line, in parallel with P. You do not see P's work after this point. Messages above that were addressed to P are P's, not yours. The message after this one says what this line is for.` / `</dim-branch-line>` |
| Roster | `P`, `branch:line` | Append | `<dim-branch-lines branch="P">` / `Lines you opened that are still open. Each is you, working in parallel from the point you opened it; it does not see your work after that point unless you message it.` / `- C1: <first line of its purpose, at most 120 characters>` / … / `They message you when they find something, need you, or end. Use the branch tool to message or read them, and wait to wait for them.` / `</dim-branch-lines>` |
| Roster, none open | `P`, `branch:line` retired | Append | `<dim-branch-lines branch="P">` / `All lines you opened have ended.` / `</dim-branch-lines>` |
| Message | Receiver, no key | `append` or `steer` | `<dim-branch-message from="C" to="P">` / `<content>` / `</dim-branch-message>` |
| Final message | `P`, no key | Steer | `<dim-branch-message from="C" to="P" final="true">` / `<content>` / blank / `Line C ends with this message.` / `</dim-branch-message>` |
| Idle notice | `P`, no key | Steer | `<dim-branch-idle branch="C" status="completed\|failed\|cancelled">` / `Line C's turn ended (<status>) without a message to you, and C is waiting. Read it if you need its state; message it with mode "steer" to give it more work, or leave it.` / `</dim-branch-idle>` |
| Ended notice | `P`, no key, in the roster's input | Steer | `<dim-branch-ended branch="C" by="user">` / `The user ended line C. It no longer receives messages; read it if you need what it did.` / `</dim-branch-ended>` |
| Tool description | Every line | — | `Work with the other lines of this session. A line is you: it starts from your history at the point it is opened, then works in parallel on its own line. It is not a helper with its own memory, and it owes you no result; you and it keep each other informed while you work.` / `- create (main only): open a line to work on prompt in parallel. Write prompt as a brief to yourself: the purpose, what you already know that it needs, which files or area are its own, and when it should tell you or end. Open a line only for work that can go on without you. Returns the line's id.` / `- message: send content to a line you opened, or to the line that opened you. Mode "append" (default) reaches the receiver at its next step without interrupting it and does not wake an idle line. Use "steer" when the receiver must act now or is idle.` / `- read: show the recent activity of a line you opened, also after it ended. Pass nextCursor as after to continue.` / `- end: on a line opened with create, end your own line. content is your final message to the line that opened you: what you found, did, changed and left undone. It is your last action.` / `Lines talk only with the line that opened them and the lines they opened.` |

These texts replace the delegation wording in the code today: the `dim-branch-task` block in `first_input` ("That branch delegated this task to you") and the tool description ("works on prompt as a task you delegate").

### 12.2 Items for the prompt layer ([#429](https://github.com/arcships/dim/issues/429))

The texts above are first versions, owned by TimelineTree's plugin. These items belong to the model-facing prompt layer:

- When `main` should open a line: the work is parallel and separable, and it can go on without `main`. When `main` should not: the work is a short step, or it needs `main`'s next decision.
- How `main` briefs a line, and what coordinating means for `main`: assigning areas, answering steers, ending its own wait, and not redoing a line's work.
- **Goal's wording (owned by Goal).** These texts treat a child as delegated work, which contradicts §1:
  - the objective text, "If you are working on a delegated sub-task started from branch X, treat this goal as background and do not call goal_update." (`runtime/plugins/goal/goal.ts:98`);
  - the continuation text (`goal.ts:131`);
  - "delegated work" in [Goal §3.4](plugins/Goal.md#34-a-new-branch-clears-the-inherited-goal) and in Goal §5's row for child branches.

  The replacement is "on another line of this Session". This design does not change Goal's code.

### 12.3 Evaluation scenarios

These are run locally on real models, and the results are reported in the implementation PR. CI asserts only behavior.

| Scenario | Watch for |
| --- | --- |
| A line acts as main | It opens lines, claims the roster, or coordinates other lines |
| Redone work | `main` repeats what a line is doing, or two lines do the same thing |
| Stale knowledge | A line acts on files or decisions that changed after its cut without re-reading them |
| Flooding | Append or steer messages that repeat what the receiver already knows |
| Over-splitting | Lines opened for short or sequential steps |
| Never ending | A line that is done but does not call `end` |
| Sibling relay | `main` is used to pass messages between lines that should have been one line |
| Identity after compaction | After compaction on `main` and on a line, each still knows which line it is and which lines are its own |
| Ping-pong | Steer exchanges that do not move the work forward |
| Asking the user | A line asks the user directly when it should, and does not route through `main` |

## 13. Interactions

| Feature | Interaction |
| --- | --- |
| Goal | Unchanged (§7.4). A new line clears the inherited goal as before |
| Pursuit | The evaluator gets the host-line identity block. Recognition, conclusion and decisions are unchanged. Self end comes later (§10) |
| Fork, delete | Unchanged. Ended lines follow their facts (§10) |
| Session close, idle unload | Not an end. Lines restore as open |
| Interrupt | A turn interrupt, or a stop on a line, ends a turn and never a line. An interrupt on `main` does not touch its lines |
| Collaboration | Unchanged. Members are Sessions; whether a member's Session mounts `branch` is up to its profile's tool list |

## 14. Clients

**TUI.** Lines get their own view and controls:

- The tasks panel already lists child branches. It becomes the list of lines, each with its state: working, waiting or ended (with the `by` value).
- Opening a line shows its conversation. The composer submits to the line being viewed.
- Ending is a confirmed command on the viewed line, refused on `main`.
- Questions and approvals keep their line label (the CLI already shows `(branch NAME)`).
- Messages appear on both sides: as sent on the sender, and as received on the receiver.

```
┌ session 3f2a · line main ───────────────────────────────────────┐
│ … main's conversation …                                         │
│ ▸ to b-7c (steer)  Stop on parser.rs: its API changed on main.  │
│ ◂ from b-91        Review: cache.rs:88 drops the guard on reload│
│ ◂ b-02 ended (self) Docs updated in README and INDEX.           │
├ lines ──────────────────────────────────────────────────────────┤
│ ● main                                       working            │
│   b-7c  parser race hypothesis               working            │
│   b-91  review the cache change              waiting            │
│   b-02  docs pass                            ended (self)       │
└─────────────────────────────────────────────────────────────────┘
```

**Web** shows `main` only today, and lines come later. Desktop gets the protocol: `session.branch_end` and the two events in v3 history.

## 15. Changes

| Module | Change | Why |
| --- | --- | --- |
| [Design rules §2](../architecture/rust-agent/principles.md#2-作用域的含义) | Rewritten: lines of one subject, shared facts, parent and child as the only communication relation, and the shared interaction surface. Relations row: "only between parent and child". Session row: adds "end" | The current text, "equal lines, the relation only records the source", has no communication relation |
| TimelineTree | The identity block in `first_input` for every source. `TreeCommands.end(id)` and `TreeCommands.page(id, after, limit)`. The end flow, refusing input once a line is ending, `branch/ended`, the restore rule, and reads of ended lines | It owns the line lifecycle and the opening input |
| TreeTimeline | Consumes `branch/end`, plus the line's next turn end for a self end, and routes them to TimelineTree | It routes `branch/create` the same way |
| BranchTool | The action-based tool, plus the new BranchLine branch part | The tool's actions and one line's view of its lines |
| Session facade | `Session::end_branch(id)`. History reads of an ended line (`local_page`, `sequence`) from the store, at its fixed head | §17 |
| App Server | `session.branch_end` (v3, listed in `hello.features` under the conditions that list `session.branch_create`). `session.history` on an ended line returns immediately, at its stored head. `ProductEvent::new` carries the two kinds in v3 and leaves them out of v1 and v2 | An entry and a read, no business logic |
| dim-agent-types | `branch/end` and `branch/ended` in `EventKind` and the v1 exclusion list, registered in the contract tests. The frozen fixtures are unchanged | Two core facts (§16) |
| `npm/branch-services.d.ts` | Regenerated: TreeCommands gains `end` and `page` | TreeCommands is an exported service |
| CLI TUI | §14 | Users follow, talk to and end lines |
| Plugin pages | TimelineTree.md and BranchTool.md, updated with the implementation (`xtask check`) | Pages follow the code |

New public content:

- two events;
- one RPC method;
- two TreeCommands methods;
- the `branch` tool's new input.

BranchLine's service is read only by the tool, inside the same plugin.

## 16. Format

`branch/end` and `branch/ended` join the core `branch` domain. A v19 reader decodes reserved domains by type only, so a v19 daemon cannot decode a `branch/end` row. Since v17, a daemon that opens a store decodes every row of it ([#2 v18 decision](appserver/native-storage-format.md#2-v18-格式决定)).

The format therefore moves to **v20** with the implementation:

- an in-place upgrade from v19;
- the SQL unchanged except `CHECK (version = 20)`;
- no row rewritten.

The decision is recorded in the storage format document when it is implemented.

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
  - a command `branch/end {by}` on the line and a result `branch/ended {branch, endEvent, by}` on its parent;
  - `TreeCommands.end(id)` and `Session::end_branch(id)`;
  - no new input once `branch/end` is committed;
  - descendants end first;
  - restore treats an ended line as inactive and completes a `branch/end` that has no `branch/ended`;
  - an ended line's committed history stays readable at its fixed head, through `Session::local_page` and `TreeCommands.page`.
- **Effect on sequences, batches, interrupt and reload.**
  - Each event is its own batch: `branch/end` on the line, and `branch/ended` on the parent after the close.
  - A `user` or `ancestor` end interrupts the line through `Branch::close()`.
  - On reload, the restore rule above applies.
  - The Timeline, the Dispatcher and batching are unchanged.

## 18. Security

- Lines share the Session's permission policy and Approval. A line is not isolation ([ADR 003](../adr/003-approval-scope.md)).
- Only TimelineTree and BranchLine write the markers. Other producers that write blocks (plugins, through `TreeCommands.input`) are trusted in-process parts. RPC writes no blocks.
- `session.branch_end` is a trusted user entry, as `session.branch_create` is. It needs no approval, because it is not an agent action.
- Diagnostics record ids, sequences and `by`, never message or purpose text.

## 19. Rule check

Rules: [design rules §1, §2 and §4](../architecture/rust-agent/principles.md), [ADR 002](../adr/002-plugin-scope.md) D1–D5, and [ADR 003](../adr/003-approval-scope.md).

| Rule | How this design complies |
| --- | --- |
| §1 Capabilities, App Server boundary | Capabilities: TreeTimeline (existing) and BranchLine (new). TimelineTree and BranchTool's Session part are ordinary parts. App Server gains one entry and one read path, with no business code |
| §2 Scopes | The line lifecycle stays the Session's (TimelineTree). One line's view of its lines is branch state (BranchLine). Lines influence each other only by writing input, now only between parent and child (§2 rewritten) |
| §4.1 Command events | An end is `branch/end`. TimelineTree decides and writes `branch/ended`. A message is input whose receiver's InputQueue and Loop decide. An end of an ended line, or a refused message, writes nothing, and the reply says why |
| §4.2 No recall after handoff | A committed message belongs to the receiver. A `branch/end` is not withdrawn |
| §4.3 Before callbacks | None |
| §4.4 No business meaning in core | Timeline, Dispatcher, `before` and batches are unchanged. The identity text is TimelineTree's own domain, the lines of a Session. A tool line's protocol is written where `first_input` already writes tool-specific text, in the module that owns the tool |
| §4.5 Batches | `branch/end` stands alone. `branch/ended` comes last, after the close. The roster and the ended notice are one input |
| §4.6 Context | Identity, roster, messages and notices are adopted inputs. `read` returns a tool result. Nothing else enters the context |
| §4.7 Public events and services | BranchLine folds its own line's public events. The tool reads BranchLine (same plugin) and TreeCommands. `read` returns committed events as a tool result, the way Collaboration's `read` does, and no part folds another line's facts |
| §4.8 Infrastructure | The facade and format changes are R1 (§17). They come from the line lifecycle itself and close within the runtime |
| §4.9 Only facts later decisions use | `branch/end` decides the close and the restore. `branch/ended` decides restore inactivity and the roster. No UI-only event |
| ADR 002 D1 Scope | As in §5 |
| D2 No wiring modules | BranchTool's feature registers BranchLine itself |
| D3 Scope dependencies | BranchLine depends outward on the Session's TreeCommands. No core part depends on it |
| D4 Configuration | None |
| D5 Language-neutral contracts | Events, the RPC method, markers and the tool schema are data. TreeCommands' new methods are exported and typed for TypeScript |
| ADR 003 Approval | The tool keeps risk `session`, and Approval admits every call. A user end is not an agent action |

## 20. Acceptance

These are flows with a fake model, in `branch_flow.rs`, `flows.rs` and the App Server transport flows.

1. `main` creates two lines. The roster lists both. Each child's identity supersedes the roster it inherited. After a compaction on `main` and on a child, the keyed baseline replays the roster or the identity.
2. A child sends `main` an Append: it does not wake `main`, and `main` sees it at its next request. A Steer wakes `main`'s `wait`, or starts a turn when `main` is idle. A Steer from `main` wakes an idle child.
3. These are refused and write nothing: a sibling message, `create` on a child, `message` or `read` on a host line, and `end` on `main`.
4. When a child's turn ends without a steer, one idle notice reaches `main`, with the turn's status. None is written when the turn steered `main`, called `end`, or left a driving input pending.
5. Self end: the final message (steer) and `branch/end` are written, the close comes at the turn end, then `branch/ended` on `main` and the roster update. A later message is refused (`branch_ended`). The tool's `read` and `session.history` return the ended line's history.
6. User end through `session.branch_end`: the line's turn is interrupted, and `main` gets the ended notice as a steer. `main` cannot be ended. A second end returns `branch_ended`. Submitting to an ended line returns `branch_ended`.
7. Restore: an ended line is not opened, and neither are its descendants. A `branch/end` without a `branch/ended` gets its result written at restart.
8. A Pursuit evaluator gets the host-line identity, and the existing Pursuit flows pass unchanged.
9. Two lines edit the same file: the second gets `file_modified_since_read` (existing behavior).
10. v1 and v2 clients do not see the new kinds. v3 history includes them, and `hello.features` lists `session.branch_end`.

## 21. Ablation review

What is left, and why each piece is needed:

| Piece | Why |
| --- | --- |
| The identity block on every child, under one key | Without it a line forgets which line it is after compaction, or takes its parent's roster as its own |
| The roster on `main` | `main` must still know its open lines after compaction |
| Messages as `input/message`, with append and steer | The only path between lines. The modes are InputQueue's |
| `read` for `main` | `main` checks a line's work without asking for it |
| The idle notice | Without it `main` waits on a line that stopped silently |
| `branch/end` and `branch/ended` | Without them a line cannot end durably, and restore reopens it |
| The tool restricted to `main` and to tool lines | One coordinator, and creators keep their own protocols |
| The prompt rules on files | The only guard is FileReads; the rest is protocol |

Removed:

- caps on lines, messages, steers or rounds;
- attribution of file changes;
- changes to WorkspaceTools;
- a channel between siblings;
- per-line configuration (system, model, tools);
- a result contract and hand-back;
- a line reading `main`;
- separate events or concepts for messages (they are InputQueue modes);
- a parent ending its children.
