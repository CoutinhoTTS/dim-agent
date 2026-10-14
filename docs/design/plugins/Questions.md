# Questions

Status: Draft. Overall design: [overview §5.1](../overview.md#51-questions). Phase: P1. Language: Rust.

The model asks the user one or more questions through the `ask_user` tool and waits for the answer: single choice, multiple choice, several questions in one call, custom answer. A Session part contributes the tool and resolves the calling branch's service per call (following [Wait](../../architecture/rust-agent/plugins/Wait.md)); a branch part records the pending question and the answer as Timeline facts and decides answer versus interrupt by sequence number (following [Approval](../../architecture/rust-agent/plugins/Approval.md)).

## 1. User flows

1. The model calls `ask_user` with two questions. ToolExecutor admits the call (risk `session`, allowed under every preset), the handler asks the branch service, which writes `question/asked`. The CLI receives the event, shows each question with numbered options and reads the user's choices. It calls `session.question.respond`; the branch writes `question/answered`, consumes it, and the handler returns the answers. When the whole tool batch has settled, ToolExecutor publishes `step/tool-result` and the model continues with the answers.
2. The user picks "Other" and types a custom answer; the answer carries the text instead of an option label.
3. The user presses `Ctrl+C` while the question is shown. The CLI calls `session.stop`; the turn `stop/interrupt` is consumed before any answer, the branch writes `question/cancelled`, the call ends with a cancelled error result and the turn ends cancelled.
4. The client disconnects while a question is pending. The question stays pending. Another client (or the same CLI after `/resume`) attaches, reads history, finds the pending question and answers it.
5. The daemon restarts while a question is pending. On restore the branch writes `question/cancelled` ("did not survive recovery"); ToolExecutor settles the call as not executed; the turn ends Failed. The user asks the model again.
6. A Session created non-interactively (`exec`, scheduled job) never declares `ask_user`.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| `ask_user` declaration, input validation, result text | Admission (Approval, Permission) and batch settlement (ToolExecutor) |
| Pending question registry per branch; `question/*` facts | Turn state, interrupt delivery (Loop, ToolExecutor) |
| Answer validation at the Session boundary; answer-versus-interrupt ordering | Rendering and keyboard handling (CLI layer, §6.4 is the CLI contract only) |
| Settling unanswered questions on interrupt, branch close, reload, restart | Timeouts or reminders (none, A7) |
| Omitting the tool in non-interactive Sessions | Deciding whether a Session is interactive (Agents, creation configuration) |

## 3. Scope and dependencies

Questions has no process part (the overview §3.2 category row lists it under "Process plugin + Capability"; the placement table, which this document follows, has no process part). The Session method lives in Rpc, like `approval.respond`.

| Part | Scope | Injects | Provides |
| --- | --- | --- | --- |
| `QuestionsPlugin` | Session | `S:TreeCommands`, `S:SessionConfig` | `S:SessionToolSource#questions` (one set containing `ask_user`, or none) |
| `QuestionsBranchPlugin` | Branch | `B:Timeline`, `B:Dispatcher` | `B:Questions` |
| Rpc (supplement) | Process | existing | `session.question.respond`, resolving `B:Questions` of the named branch through the `Session` facade (`session.service::<Questions>(branch)`, as `approval::respond` does) |

Assembly: `builder.session_tool_source("questions").session_plugin(QuestionsPlugin::new).timeline_plugin(QuestionsBranchPlugin::new)`.

**Contracts depended on**: the Session tool source contract (`SessionToolSource`, `ToolProvider`, `ToolSourceSet`, `ToolHandler`, `ToolExecutionContext`) of [tool contributions](../../architecture/rust-agent/tool-contributions.md); `TreeCommands::get(branch).service::<T>()` (existing, as Wait); the `ToolDefinition.risk` category contract of Permission (`session`); ToolExecutor's per-batch `CancellationToken` and settlement contract (one result per call, batch result after all calls); the `Timeline` append contract (`append`, `append_registered`) and the Dispatcher subscription and reload protocol of [single-tree](../../architecture/rust-agent/single-tree.md); `SessionConfig.interactive` (proposed field, overview §7.2 Agents supplement; also used by [process-tool-contribution](../contracts/process-tool-contribution.md)); dynamic event registration with a client-visibility flag (#2, A10). Questions names no other plugin's internal type; it never reads Loop, ToolExecutor or Approval state.

**Contracts defined here**: the `ask_user` tool (name, input schema, result), the `question/asked`, `question/answered`, `question/cancelled` data and consumption contracts, the `B:Questions` service (`ask`, `respond`), the Session method `session.question.respond` and its errors. Producers of `question/answered` other than the branch service do not exist: the only write path is `respond`.

Why not the other paths: a process-level tool contribution cannot reach the branch registry, and the [process-tool-contribution](../contracts/process-tool-contribution.md) contract excludes Session-scoped contributions that resolve the calling branch's service per call. `SessionToolSource` is used instead of a fixed `ToolContribution` so that Questions itself decides presence from `SessionConfig.interactive`; Agents and Agent profiles do not need to know the tool name (A1).

## 4. Events

Declared through #2 registration by `QuestionsBranchPlugin`. All three are client-visible (A10) and never rendered into model history.

| Type | Payload | Producer and write path | Consumers and rule |
| --- | --- | --- | --- |
| `question/asked` | `call_id: String`, `turn_id: String`, `request_id: String`, `questions: Vec<Question>` | `Questions::ask`, `Timeline::append_registered`; the callback registers the pending entry under the Questions lock | Questions rebuild (pending set). Clients: show the question |
| `question/answered` | `call_id: String`, `answers: Vec<Answer>`, `declined: bool` | `Questions::respond` (behind `session.question.respond`), `Timeline::append_registered`; the callback registers the response waiter and marks the entry answering | Questions Dispatcher subscription: accepted only if the entry is pending and its batch token is not cancelled at consumption (§7); otherwise no effect. Clients: close the prompt |
| `question/cancelled` | `call_id: String`, `reason: String` | Questions only, `Timeline::append` (no registration), from the ask future, the consumer, `close()` or rebuild | Questions rebuild (pending set). Clients: close the prompt |

```rust
pub struct Question { pub id: String, pub header: Option<String>, pub question: String,
    pub options: Vec<QuestionOption>, pub multi_select: bool, pub allow_custom: bool }
pub struct QuestionOption { pub label: String, pub description: Option<String> }
pub struct Answer { pub question_id: String, pub selected: Vec<String>, pub custom: Option<String> }
```

Outcome derivation, used by rebuild and by clients reading history: a question is **pending** while it has `asked` and neither `answered` nor `cancelled`; it is **answered** when its first `answered` exists and no `cancelled` exists for the same `call_id`; otherwise it is **cancelled**. `cancelled` always wins, because it is written exactly when an answer was not delivered to the handler (§8).

Why not derivable (overview §6): between the handler start and `step/tool-result` the call has no fact; results are published only after the whole batch, so "waiting for an answer" and "answered" are invisible without these events; the answer is user content that must be a fact to be compared with `stop/interrupt` by sequence and to survive the window until the batch result. `turn_id` and `request_id` are added to the overview payload so clients and diagnostics correlate the question with the call without reading `step/tool-call` (A3).

## 5. State

| State | Owner | Rebuild |
| --- | --- | --- |
| Pending entries: `{asked: QuestionAskedData, cancel: CancellationToken, response: oneshot, answering: Option<u64>}` in asked order; `respond` waiters by event id; `closed`; `rebuilt` | `B:Questions`, `Mutex` (no `StateCell`, like Approval) | Not carried across generations. Load folds `local(..=S)`; every `asked` without `answered`/`cancelled` gets `question/cancelled` ("did not survive recovery"); handler futures of the old generation already returned |
| Whether the tool is offered | `QuestionsPlugin`, from `SessionConfig.interactive` | Fixed at Session creation |

The ancestor prefix is not read: a child branch never owns its parent's questions.

## 6. Interfaces

### 6.1 Tool `ask_user`

Risk `session`; `ToolSource::Extension`. Declared only when `SessionConfig.interactive` is true (absent field = true, A2).

| Parameter | Type | Rule |
| --- | --- | --- |
| `questions` | array, 1..=4 items (A4) | Required |
| `questions[].id` | string | Optional; defaults to `"1"`, `"2"`, ... by position; unique in the call |
| `questions[].header` | string ≤ 40 chars | Optional short label |
| `questions[].question` | non-empty string | Required |
| `questions[].options` | array of `{label, description?}`, 2..=6 items, unique labels | Required (A4) |
| `questions[].multiSelect` | boolean, default false | Multiple choice |
| `questions[].allowCustom` | boolean, default true | "Other" free text (A5) |

Validation happens in the handler (the tool's external boundary); invalid input returns an error `ToolResult` and writes nothing.

Result `content` (the only model-readable text):

```
User answered:
1. <question>
   -> <label>, <label>
2. <question>
   -> (custom) <text>
```

`declined` yields `User declined to answer the question(s).` (A6). `structured_content` carries `{answers, declined}` for clients; it is not model text. Cancellation is reported by ToolExecutor from the handler's `HostError::Cancelled` (`Tool execution cancelled` or the reload settlement).

### 6.2 Branch service

```rust
impl Questions {   // key BranchScope::key::<Questions>()
    /// Registers and waits. `cancel` is the batch token from ToolExecutor.
    pub async fn ask(&self, call_id: &str, ctx: &ToolExecutionContext, questions: Vec<Question>,
        cancel: CancellationToken) -> Result<Answered, HostError>;
    /// `Ok` means the answer was dispatched and delivered to the waiting call.
    pub async fn respond(&self, call_id: &str, answers: Vec<Answer>, declined: bool)
        -> Result<(), QuestionRespondError>;
}
pub enum QuestionRespondError { Unknown, AlreadyAnswered, Cancelled, Invalid(String), Closed }
```

### 6.3 Session method (Rpc)

`session.question.respond {sessionId, branchId?: "main", callId, answers: [{questionId, selected: [label], custom?}], declined?: false}` → `{}`.

| Error | Condition |
| --- | --- |
| `invalid_params` `Question is absent or already resolved` | `Unknown` |
| `invalid_params` `Question already answered` | `AlreadyAnswered` (a concurrent answer was registered first) |
| `invalid_params` `Question was cancelled` | `Cancelled` (an interrupt preceded the answer) |
| `invalid_params` `<reason>` | `Invalid`: unknown question id, label not offered, more than one label without `multiSelect`, `custom` without `allowCustom`, nothing selected and no `custom` while not `declined` |
| `service_reloading` / closed | `Closed` on a live branch / closed Session, mapped like `approval.respond` |

Validation at this boundary uses the pending entry's `asked` data. Accepted commands belong to the Session and continue after disconnect.

### 6.4 CLI entry (contract for the CLI layer)

The CLI treats `session.event` notices with type `question/asked`, `question/answered`, `question/cancelled` of **any** branch of the attached Session (A9), like approvals:

| Situation | CLI behavior |
| --- | --- |
| `question/asked` and no prompt open | Queue it; prompts are shown one call at a time in arrival order. Print `[question] <header>` and, per question, the text and numbered options, plus `0. Other (type your answer)` when `allowCustom`. Prompt `Choose 1-N` (single) or `Choose numbers, comma separated` (multi). Empty line on the first question declines the whole call (A6) |
| Prompt open | Other notices are deferred and rendered after the prompt closes (same as approvals) |
| All questions of the call collected | `session.question.respond`; error replies are printed; `Cancelled`/`Unknown` close the prompt silently |
| `question/answered` or `question/cancelled` for the shown call | Close the prompt; print `[question answered elsewhere]` or `[question cancelled]` |
| `Ctrl+C` while prompting | `session.stop` (turn), as during approval; the question is cancelled by the interrupt |
| Input closed (EOF) | `session.stop`; exit code 130 |
| No interactive input (`-p` / `exec`) and a question arrives (Session created interactively elsewhere) | Print `Interactive answer required; stopping`, `session.stop`, exit code 3 (A8) |
| Attach or resume | After `session.attach`, read `session.history` of each branch through its head and derive pending questions (§4); merge with live notices by `call_id` |

The attach race is closed by order: the listener is registered before history is read; a question dispatched before the listener is already published, hence in history; one dispatched later arrives live; duplicates are merged by `call_id`. No Questions-specific attach replay is added to Rpc.

## 7. Timeline and capability mechanics

| Step | Mechanics |
| --- | --- |
| Tool resolution | Handler: `TreeCommands::get(ctx.branch_id)?.service::<Questions>()`; `AgentError::Reloading` maps to `HostError::Cancelled(CAPABILITY_RELOADING)` (as Wait) |
| Ask | If `cancel` is already cancelled, return Cancelled without writing. Else `append_registered(question/asked)`; the callback (under the commit lock) fails if `closed` or another pending entry has the same `call_id`, else pushes the entry with the batch token. If the commit fails, the registered entry is removed and the call returns Cancelled (closed) or an error |
| Wait | `select! biased { response, cancel.cancelled() }`. On cancel: take the entry under the lock; if taken, write `question/cancelled {reason: "Tool execution cancelled"}` after releasing the lock; return Cancelled. Dropped sender (close/reload): Cancelled with `CAPABILITY_RELOADING` |
| Respond | Pre-check pending and validate; `append_registered(question/answered)`; callback fails with `Closed` if closed, `Cancelled` if the entry is gone, `AlreadyAnswered` if `answering` is set; else sets `answering` and stores the waiter by event id. Await the dispatch result |
| Consume | Dispatcher subscription `"Questions"` on `["question/answered"]`, under the rebuild gate, skipping `sequence <= rebuilt`. Takes the entry: if its token is cancelled, send `None`, reply `Cancelled`, write `question/cancelled`; else send the answer and reply `Ok`. Missing entry: reply `Unknown`. The handler only resolves channels and returns; the ask future builds the result |
| Recheck | The token reflects only interrupts ToolExecutor has already consumed. The Dispatcher delivers each event to all subscribers before the next (`serial`), so a token cancelled at consumption of `answered` means a `stop/interrupt` with a lower sequence was consumed (or the generation is unloading). No committed-but-unconsumed tail is read (time bus §4) |
| Terminal facts | `question/asked` and `question/cancelled` are written by Questions (the initiator of the pending state); `question/answered` is the boundary fact; the call's terminal fact is ToolExecutor's `step/tool-result` |
| Close / reload (unload) | `ctx.effect` disposer: set `closed`, drop respond waiters (they report `Closed`), take all entries, write `question/cancelled {reason: "capability reloading"}` for each, then drop their senders. A closing Timeline may reject the write; the branch is not restored |
| Load | Subscribe first, then under the gate fix `S = committed()`, fold `local(..=S)`, write `question/cancelled {"Question cancelled: execution did not survive recovery"}` for each unresolved `asked`, set `rebuilt`, provide `B:Questions` |
| Restart | Same as load; ToolExecutor's rebuild independently settles the batch as not executed. Nothing is re-asked |

Questions reloads with its injected `B:Timeline`/`B:Dispatcher` only; it is not on ToolExecutor's injection chain, so a Questions reload during a call ends that call through `close()` (Cancelled, reload-settled by ToolExecutor) without reloading other capabilities.

## 8. Timing and conflicts

`A` = `question/answered`, `I` = turn `stop/interrupt`, `Q` = `question/asked`, `C` = `question/cancelled`, `R` = `step/tool-result`.

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Interrupt before the batch is dispatched | I < `step/tool-call` consumed | No `Q`; call not executed | ToolExecutor |
| Interrupt before the question is registered | I consumed, then `ask` | Token cancelled; nothing written; Cancelled | Questions (token check) |
| Answer, then interrupt | Q < A < I | Answer accepted at A; handler result kept as truly completed; turn ends cancelled after R | Questions; ToolExecutor |
| Interrupt, then answer committed before cancel is written | Q < I < A < C | At A the token is cancelled (I consumed first): `respond` → `Cancelled`; `C` written; outcome cancelled | Questions |
| Interrupt, cancel written, then answer | Q < I < C | `respond` registration fails (`Cancelled`), nothing written | Questions |
| Two clients answer concurrently | Q < A1 (< A2 rejected) | Second registration fails `AlreadyAnswered`; first decides | Questions (registration under lock) |
| Duplicate or late RPC after resolution | — | `Unknown` or `Cancelled`; nothing written | Questions |
| Branch-scoped interrupt, branch close | Q < branch I | Token cancelled → C; then close settles any remaining with C | Questions; ToolExecutor |
| Answered, other calls of the batch still running | Q < A < R | Answer held in the handler result; model sees it only at R | ToolExecutor |
| Answered, then reload before R | Q < A < reload | ToolExecutor marks every result of the batch reload-settled; turn Failed. Questions outcome stays answered (no C, A was consumed) | ToolExecutor |
| Answer committed, Questions reloads before consuming | Q < A, unload | `close()` writes C (Q < A < C): outcome cancelled; `respond` gets `Closed` → `service_reloading` | Questions |
| Steer/Queue/Append while pending | Q < input | Not consumed until R (Steer/Append) or `turn/end` (Queue); does not end the question (A11) | Loop, InputQueue |
| Several `ask_user` in one batch | Q1 < Q2 | Both pending; answered independently in any order; results published together | Questions; ToolExecutor |
| Same `call_id` asked twice while pending | — | Second registration fails; error result for that call | Questions |
| Compaction between Q and R | Q < compaction < R | No effect: compaction happens at request preparation, not while the batch runs; answer enters at R | ContextPreparation |
| Child branch cut between Q and A | Q < cut < A | Child rebuild reads only local events: nothing pending; the inherited call has no result and projects as not executed | Questions (child); history projection |
| Session unload | pending question | Turn running → Session not idle → not unloaded | AgentIndex |
| Process restart mid-question | Q, no A/C | Load writes C; ToolExecutor settles the batch; turn Failed | Questions; ToolExecutor; Loop |
| Resource node offline | — | Questions uses no host resource; unaffected unless a dependency goes Pending (then reload rows) | rutis lifecycle |
| Client detach | — | No effect; question remains pending | — |
| Non-interactive Session | — | Tool absent; a model call by name gets `Unknown tool` | ToolRegistry, ToolBinding |

## 9. Context mapping

| What the model sees | Form | Placement |
| --- | --- | --- |
| `ask_user` name, description, schema | Tool declaration from the `questions` source set | Adopted `context/configured`, interactive Sessions only |
| Answers, decline or cancellation | `ToolResult.content` (§6.1); error text from ToolExecutor when cancelled | Tool result position of the batch, after all calls of the batch settle |

Never visible: `question/*` events, `structured_content`, the CLI rendering. Compaction keeps the declaration (configuration) and keeps answers only through the summary. Child branches inherit the declaration and the answers whose `step/tool-result` precedes the cut. Non-interactive Sessions see no declaration.

## 10. Errors and recovery

| Case | Result |
| --- | --- |
| Invalid tool input | Error `ToolResult`; nothing written |
| Branch service missing (reloading) | Cancelled with `capability reloading`; ToolExecutor reload-settles |
| `asked` commit fails | Entry removed; Cancelled or error result |
| Interrupt / branch close | `question/cancelled`, handler `HostError::Cancelled` |
| Reload, restart | `question/cancelled` for every unresolved question; never re-asked |
| `respond` on a reloading branch | `service_reloading`; client retries; the question was cancelled, the retry returns `Unknown` |

## 11. Security

`ask_user` needs no host resource and is `session` risk (allowed under all presets; `disallowed_tools` still applies). Answers are user content written as Timeline facts and replayed to the model through the tool result; clients must not put secrets in them. `session.question.respond` is a Session method available to any client attached with Session access (including remote clients, overview §7.2 Transport); it confers no authority beyond answering. Questions never widens permissions; replay never re-asks.

## 12. Acceptance

Flow tests through public interfaces (RPC, tool calls, events):

1. Single, multiple, multi-question and custom answers produce the specified `content`; invalid answers are rejected at `respond` without facts.
2. `ask_user` absent in a non-interactive Session; present otherwise.
3. Answer then stop: result contains the answer; stop then answer: `Cancelled`, `question/cancelled`, cancelled result. Both orders forced by committing `I` before `A` with a held Dispatcher.
4. Two concurrent `respond`: exactly one accepted.
5. Two `ask_user` plus a slow tool in one batch: answers held until the single `step/tool-result`.
6. Branch close and Questions reload settle pending questions; restart writes "did not survive recovery" and the turn fails; no re-ask.
7. Detach and re-attach: history derivation shows the pending question; answering after re-attach works.
8. CLI: prompt rendering, deferral of notices, `Ctrl+C`, EOF, non-interactive exit code 3.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | The tool is a `SessionToolSource` whose presence Questions decides from Session configuration | Overview §5.1; brief; judgment | Agents puts `ask_user` in `SessionConfig.tools` only for interactive Sessions |
| A2 | `SessionConfig.interactive` absent means interactive | Judgment (existing Sessions) | Existing Sessions lose the tool |
| A3 | Payload adds `turn_id`, `request_id`, `declined` to the overview fields | Judgment | Correlation through `step/tool-call`; decline modeled as an answer |
| A4 | 1..=4 questions, 2..=6 options | Edge-case reference; judgment | Limits change; schema only |
| A5 | Custom answers allowed by default | Feature list "custom answers"; default is judgment | Model must opt in |
| A6 | The user may decline a whole call | Judgment | User must answer or stop the turn |
| A7 | No timeout | Judgment | A timeout writes `question/cancelled {timeout}` |
| A8 | CLI without input stops with exit code 3 | Precedent: CLI approval handling | Different exit policy |
| A9 | Questions of every branch are presented, not only `main` | Approval precedent | Child-branch questions need another client |
| A10 | #2 registration marks events client-visible and Connection forwards them as `session.event` | Overview §1 (#2); judgment | Rpc needs dedicated notifications like `approval.request` |
| A11 | Steer input does not end a pending question | Judgment | Steer cancels with reason `user steer` |

## 14. Open questions

| Question | Description |
| --- | --- |
| Overview §13 "How events enter context" | Designed for tool results only (ADR 001) |
| Client visibility of dynamic events | Whether #2 projects plugin events to clients by a declared flag (A10) |
| Overview §3.2 category | Questions has no process part; the category row should read "Capability (Session + branch)" |
