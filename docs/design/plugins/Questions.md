# Questions

Status: Draft. Overall design: [overview B.1](../overview.md#4-plugin-catalog). Phase: P1. Language: Rust. Scope: Session + branch ([ADR 002](../../adr/002-plugin-scope.md)).

The model asks the user one or more questions through the `ask_user` tool and waits for the answer: single choice, multiple choice, several questions in one call, custom answer. The **Session part** provides the `ask_user` tool as a `toolContribution` service (Session scope, key `ask_user`) and the Session method `session.question.respond` through a `sessionMethods` service (Session scope, key `session.question`) ([session-boundary](../contracts/session-boundary.md)). It is installed only in interactive Sessions. The **branch part** records the pending question and the answer as Timeline facts and decides answer versus interrupt by sequence number, following the [Approval](../../architecture/rust-agent/plugins/Approval.md) pattern.

## 1. Structure

```mermaid
flowchart LR
  subgraph S["Session scope (interactive Sessions only)"]
    QS["QuestionsPlugin: toolContribution ask_user, sessionMethods session.question"]:::new
    TR["ToolRegistry"]
    TC["TreeCommands"]
  end
  subgraph P["Process scope"]
    RPC["Rpc"]
  end
  subgraph B["Branch scope"]
    QB["QuestionsBranchPlugin (questions)"]:::new
    TE["ToolExecutor"]
    TL["Timeline + Dispatcher"]
  end
  TR -- "collects toolContribution by name" --> QS
  RPC -- "resolves the Session, calls sessionMethods session.question" --> QS
  QS -- "per call: calling or named branch" --> TC
  TC --> QB
  TE -- "execute(ToolCall), cancel on batch token" --> QS
  QB -- "append_registered / append" --> TL
  TL -- "question/answered (Dispatcher)" --> QB
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
```

Both Session services hold no state; they resolve the `questions` service of the calling (tool) or named (method) branch per call through `S:TreeCommands`, as [Wait](../../architecture/rust-agent/plugins/Wait.md) does. All waiting and ordering lives in the branch part. Contracts are given in language-neutral form (service name, scope, key, methods with data shapes); the Rust form is [Appendix G](#appendix-g-rust-binding).

| Part | Scope | Depends on | Provides |
| --- | --- | --- | --- |
| `QuestionsPlugin` | Session | `S:TreeCommands` | `toolContribution` key `ask_user`; `sessionMethods` key `session.question` (`session.question.respond`, level `session`) |
| `QuestionsBranchPlugin` | Branch | `B:Timeline`, `B:Dispatcher` | `questions` (branch scope; `ask`, `respond`), used only by the Session part |

Installation: both parts declare their services in the plugin's `provides`; the Session part's row is conditioned on `SessionConfig.interactive`, so Agents installs it, and lists `ask_user` in `SessionConfig.tools`, only in interactive Sessions (set by `session.create`; ACP, `exec`, Scheduler and Workflow Sessions are non-interactive). The branch part is installed with it. ToolRegistry and Rpc collect the services by name and scope.

Contracts depended on: `toolContribution` (ToolRegistry accepts named tool services, [tool contribution](../../architecture/rust-agent/tool-contributions.md) supplement); ToolExecutor's batch token and settlement; the `Timeline` append contract and Dispatcher protocol of [single-tree](../../architecture/rust-agent/single-tree.md) (typed runtime services; Questions is Rust); `SessionConfig.interactive` ([session-access](../contracts/session-access.md)); `sessionMethods` ([session-boundary](../contracts/session-boundary.md)); client-visible dynamic events (#2, A10); Permission risk category `session`.

Contracts defined here: the `ask_user` tool, the `question/*` data and consumption contracts, the `questions` branch service, `session.question.respond` and its errors. The only write path of `question/answered` is `questions.respond`.

## 2. Data and state

```mermaid
classDiagram
  class question_asked {
    call_id
    turn_id
    request_id
    questions
  }
  class question_answered {
    call_id
    answers
    declined
  }
  class question_cancelled {
    call_id
    reason
  }
  class PendingRegistry {
    entries in asked order
    batch token per entry
    answering marker
    closed
    rebuilt sequence
  }
  question_asked --> PendingRegistry : rebuild folds local events
  question_answered --> PendingRegistry : consumed by Dispatcher
  question_cancelled --> PendingRegistry : rebuild folds local events
```

All three events are branch Timeline facts written only by the `questions` service; the pending registry is memory under a `Mutex` (no `StateCell`, like Approval) and is never carried across generations.

Outcome rule, used by rebuild and by clients reading history: a question is **pending** while it has `asked` and neither `answered` nor `cancelled`; **answered** when an `answered` exists and no `cancelled`; otherwise **cancelled**. `cancelled` wins because it is written exactly when an answer was not delivered to the handler.

Rebuild on load folds only the branch's local events (a child branch never owns its parent's questions) and writes `question/cancelled` ("did not survive recovery") for every unresolved `asked`.

Why the events exist: between handler start and `step/tool-result` a call has no fact, and results publish only after the whole batch, so "waiting" and "answered" are not derivable; the answer is user content that must be a fact to be ordered against `stop/interrupt`. `turn_id`, `request_id` and `declined` extend the overview payload (A3, F1).

## 3. Flows

### 3.1 Ask and answer

```mermaid
sequenceDiagram
  participant M as Model (Loop)
  participant TE as ToolExecutor
  participant Q as questions (branch)
  participant TL as Timeline
  participant D as Dispatcher
  participant C as Client
  M->>TE: step/tool-call ask_user
  TE->>Q: ask(callId, questions) via toolContribution execute
  Q->>TL: append_registered question/asked (callback registers entry)
  TL-->>C: session.event question/asked
  C->>Q: session.question.respond (Rpc calls sessionMethods)
  Q->>TL: append_registered question/answered (callback marks answering)
  TL->>D: deliver question/answered
  D->>Q: consume: token not cancelled, send answer, return
  Q-->>C: {}
  Q-->>TE: answers (ToolResult)
  TE->>TL: step/tool-result after whole batch settles
```

`respond` returns only after the Dispatcher consumed the answer, so `{}` means the answer reached the waiting call. The Dispatcher handler only resolves channels; the ask future builds the result.

### 3.2 Detach and re-attach

```mermaid
sequenceDiagram
  participant C as Client
  participant R as Rpc
  participant TL as Timeline
  C->>R: session.attach (listener registered first)
  C->>R: session.history per branch through head
  R-->>C: events incl. question/asked without answer
  TL-->>C: live question/* notices
  Note over C: merge by call_id, show pending prompt
  C->>R: session.question.respond
```

Anything dispatched before the listener is in history, anything later arrives live, duplicates merge by `call_id`. No Questions-specific replay is added to Rpc.

### 3.3 Non-interactive Sessions

The Session part is not installed: no declaration, and `session.question.respond` gets `unknown_method`. A model calling `ask_user` by name gets `Unknown tool`.

## 4. Timing and conflicts

### 4.1 Answer versus interrupt

```mermaid
sequenceDiagram
  participant TL as Timeline
  participant D as Dispatcher (serial)
  participant TE as ToolExecutor
  participant Q as questions (branch)
  Note over TL: Q = asked, I = stop/interrupt, A = answered, C = cancelled
  TL->>D: I (seq n)
  D->>TE: cancel batch token synchronously
  TL->>D: A (seq n+1)
  D->>Q: consume A: token already cancelled
  Q->>TL: append C
  Q-->>Q: respond returns Cancelled
```

The Dispatcher delivers each event to every subscriber before the next, so a token cancelled at consumption of `A` means an interrupt with a lower sequence was consumed. If `A` precedes `I`, the answer is kept as a completed result; the turn still ends cancelled after `step/tool-result`.

### 4.2 Reload and restart

```mermaid
sequenceDiagram
  participant G1 as questions gen 1
  participant TL as Timeline
  participant G2 as questions gen 2
  participant TE as ToolExecutor
  G1->>TL: unload: C for each pending (capability reloading)
  G1-->>TE: drop senders, handler returns Cancelled (reload-settled)
  G2->>TL: subscribe, fix S = committed, fold local up to S
  G2->>TL: C did not survive recovery, for any unresolved asked
  Note over G2: rebuilt = S, provide questions
```

Nothing is re-asked. A reload of the Session part propagates to ToolRegistry and its dependents like any contribution reload; in-flight calls end reload-settled and the turn fails. This follows a rare structural operation and is an accepted cost ([ADR 002](../../adr/002-plugin-scope.md) D3).

| Scenario | Outcome |
| --- | --- |
| Two clients answer | Second registration fails `AlreadyAnswered` |
| Client unable to answer (`-p` attached to an interactive Session) | The CLI stops the turn with exit code 3 (A8) |
| Remote paired device answers | Allowed: paired devices may call Session methods |
| `input/withdrawn` while pending | No effect on the question |
| Process restart mid-question | `C` on load; batch settled as not executed; turn Failed |

Full table: Appendix B.

## 5. Context mapping

| What the model sees | Form | Placement |
| --- | --- | --- |
| `ask_user` name, description, schema | Fixed tool declaration | Adopted `context/configured`, interactive Sessions only |
| Answers, decline | `ToolResult.content` (Appendix A.1) | Tool result position, after every call of the batch settles |
| Cancellation | Error text from ToolExecutor | Same position |

Never visible: `question/*` events, `structured_content`, the CLI rendering. Compaction keeps the declaration and keeps answers only through the summary. Child branches inherit answers whose `step/tool-result` precedes the cut; a call cut between `asked` and its result projects as not executed.

## 6. Interfaces (summary)

| Interface | Kind | Purpose |
| --- | --- | --- |
| `ask_user` | `toolContribution` (Session, key `ask_user`), risk `session` | Ask 1–4 questions, wait for the answer |
| `session.question.respond` | `sessionMethods` (Session, key `session.question`), level `session` | Answer or decline a pending call |
| `questions.ask` / `respond` | Branch service `questions`, internal to Questions | Register and wait; boundary write of the answer |
| `question/asked`, `question/answered`, `question/cancelled` | Events | Pending state and answer ordering |
| CLI prompt contract | Client | Rendering, deferral, `Ctrl+C`, attach |

## 7. Security

`ask_user` uses no host resource and is `session` risk (allowed under every preset; `disallowed_tools` still applies). Answers are user content written as facts and replayed to the model; clients must not put secrets in them. `session.question.respond` is available to any client with Session access, including paired remote devices ([RemoteGateway](RemoteGateway.md)); it confers no authority beyond answering. Questions never widens permissions; replay never re-asks.

## Appendix A. Interface details

### A.1 Tool `ask_user`

Service `toolContribution`, Session scope, key `ask_user`, listed only in interactive Sessions. `declaration(): sync -> ToolDeclaration` returns the name, a fixed description, the parameter schema below and risk `session`; `execute(ToolCall): async -> ToolResult` runs the handler. The consumer cancels by a cancel request on the call (ToolExecutor's batch token).

| Parameter | Type | Rule |
| --- | --- | --- |
| `questions` | array, 1..=4 (A4) | Required |
| `questions[].id` | string | Optional; defaults to `"1"`, `"2"`, ... by position; unique in the call |
| `questions[].header` | string, at most 40 chars | Optional short label |
| `questions[].question` | non-empty string | Required |
| `questions[].options` | array of `{label, description?}`, 2..=6, unique labels | Required (A4) |
| `questions[].multiSelect` | boolean, default false | Multiple choice |
| `questions[].allowCustom` | boolean, default true | "Other" free text (A5) |

Validation happens in the handler; invalid input returns an error `ToolResult` and writes nothing. Result `content`:

```
User answered:
1. <question>
   -> <label>, <label>
2. <question>
   -> (custom) <text>
```

A declined call yields `User declined to answer the question(s).` (A6). `structured_content` carries `{answers, declined}` for clients only. A cancelled call ends with the error `{code: "cancelled", reason}`, which ToolExecutor reports.

### A.2 Event payloads

| Type | Payload | Write path | Consumers |
| --- | --- | --- | --- |
| `question/asked` | `call_id, turn_id, request_id: string`, `questions: Question[]` | `questions.ask`, `append_registered`; callback registers the entry under the lock (fails if closed or `call_id` already pending) | Rebuild; clients show the prompt |
| `question/answered` | `call_id: string`, `answers: Answer[]`, `declined: boolean` | `questions.respond`, `append_registered`; callback fails `Closed`/`Cancelled`/`AlreadyAnswered`, else sets `answering` and stores the waiter by event id | Dispatcher subscription `"Questions"`, under the rebuild gate, skipping `sequence <= rebuilt`; accepted only if the entry is pending and its token is not cancelled. Clients close the prompt |
| `question/cancelled` | `call_id, reason: string` | `questions` only, `append`: ask future on cancel, consumer, `close()`, rebuild | Rebuild; clients close the prompt |

All three are declared through #2 registration as client-visible and are never rendered into model history.

| Shape | Fields |
| --- | --- |
| `Question` | `{id: string, header?: string, question: string, options: QuestionOption[], multi_select: boolean, allow_custom: boolean}` |
| `QuestionOption` | `{label: string, description?: string}` |
| `Answer` | `{question_id: string, selected: string[], custom?: string}` |

### A.3 Branch service `questions`

Branch scope, no key, provided by `QuestionsBranchPlugin` and called only by its Session part.

| Method | Mode | Request | Reply | Errors |
| --- | --- | --- | --- | --- |
| `ask` | async | `{callId, turnId, requestId, questions: Question[]}` | `{answers: Answer[], declined: boolean}` | `cancelled {reason}` (cancel request from the consumer, close, reload), `invalid {reason}` |
| `respond` | async | `{callId, answers: Answer[], declined: boolean}` | `{}`: the answer was dispatched and delivered to the waiting call | `unknown`, `already_answered`, `cancelled`, `invalid {reason}`, `closed` |

Cancellation of `ask` is a cancel request on the call; the Session part sends it when ToolExecutor cancels the batch token.

### A.4 Mechanics

| Step | Mechanics |
| --- | --- |
| Tool resolution | Handler resolves the calling branch's `questions` through `S:TreeCommands`; a failed lookup (branch closed or branch part reloading) → error result or Cancelled with `capability reloading`, as Wait |
| Ask | Token already cancelled: return Cancelled, write nothing. Else `append_registered(question/asked)`; on commit failure remove the entry and return Cancelled or an error |
| Wait | `select! biased { response, cancel.cancelled() }`. On cancel: take the entry under the lock; if taken, write `question/cancelled {reason: "Tool execution cancelled"}` after releasing the lock; return Cancelled. Dropped sender: Cancelled with `capability reloading` |
| Respond | Pre-check pending and validate against the entry's `asked` data; `append_registered(question/answered)`; await the dispatch result |
| Consume | Take the entry; token cancelled: send `None`, reply `Cancelled`, write `question/cancelled`; else send the answer, reply `Ok`. Missing entry: `Unknown` |
| Recheck | Only the batch token, which reflects interrupts ToolExecutor already consumed ([time-bus](../../architecture/rust-agent/time-bus.md) §4) |
| Terminal facts | `asked`/`cancelled` by Questions, `answered` is the boundary fact; the call's terminal fact is ToolExecutor's `step/tool-result` |
| Unload | Unload disposer: set `closed`, drop respond waiters (`Closed`), take all entries, write `question/cancelled {reason: "capability reloading"}`, drop senders. A closing Timeline may reject the write; that branch is not restored |
| Load / restart | Subscribe, fix `S = committed()` under the gate, fold `local(..=S)`, cancel unresolved, set `rebuilt`, provide `questions` |

### A.5 Session method `session.question.respond`

Service `sessionMethods`, Session scope, key `session.question`. `describe()` returns one `MethodDecl`: `{name: "session.question.respond", level: session, mutating: true, interruption: report, progress: false, notifications: [], reply: direct}`. Rpc resolves the Session and sends `call(MethodCall)`; the handler resolves `questions` of the named branch through `S:TreeCommands`.

`params`: `{sessionId, branchId?: "main", callId, answers: [{questionId, selected: [label], custom?}], declined?: false}` → `MethodReply` value `{}`. Errors are `MethodError` data:

| Error | Condition |
| --- | --- |
| `invalid_params` `Question is absent or already resolved` | `Unknown` |
| `invalid_params` `Question already answered` | `AlreadyAnswered` |
| `invalid_params` `Question was cancelled` | `Cancelled` |
| `invalid_params` `<reason>` | `Invalid`: unknown question id, label not offered, several labels without `multiSelect`, `custom` without `allowCustom`, nothing selected and no `custom` while not `declined` |
| `service_reloading` | `Closed` on a live branch, or the service is reloading |
| `unknown_method` | Session not interactive (service not installed) |
| `session_closed` | Session closed |

Accepted commands belong to the Session and continue after disconnect.

### A.6 CLI contract

The CLI handles `question/*` notices of every branch of the attached Session (A9), like approvals:

| Situation | CLI behavior |
| --- | --- |
| `question/asked`, no prompt open | Queue; one call at a time in arrival order. Print `[question] <header>`, each question with numbered options, plus `0. Other (type your answer)` when `allowCustom`. Prompt `Choose 1-N` or `Choose numbers, comma separated`. Empty line on the first question declines the call (A6) |
| Prompt open | Other notices deferred until the prompt closes |
| All questions collected | `session.question.respond`; error replies printed; `Cancelled`/`Unknown` close the prompt silently |
| `answered`/`cancelled` for the shown call | Close the prompt; print `[question answered elsewhere]` or `[question cancelled]` |
| `Ctrl+C` while prompting | `session.stop` (turn); the interrupt cancels the question |
| EOF | `session.stop`; exit code 130 |
| No interactive input (`-p`) and a question arrives | `Interactive answer required; stopping`, `session.stop`, exit code 3 (A8) |
| Attach or resume | Listener first, then `session.history` per branch; derive pending; merge by `call_id` |

## Appendix B. Full conflict table

`A` = `question/answered`, `I` = turn `stop/interrupt`, `Q` = `question/asked`, `C` = `question/cancelled`, `R` = `step/tool-result`.

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Interrupt before the batch is dispatched | I < `step/tool-call` consumed | No `Q`; call not executed | ToolExecutor |
| Interrupt before the question is registered | I consumed, then `ask` | Token cancelled; nothing written | Questions (token check) |
| Answer, then interrupt | Q < A < I | Answer accepted; result kept as completed; turn ends cancelled after R | Questions; ToolExecutor |
| Interrupt, then answer committed before cancel | Q < I < A < C | `respond` → `Cancelled`; `C` written | Questions |
| Interrupt, cancel written, then answer | Q < I < C | Registration fails `Cancelled`; nothing written | Questions |
| Two clients answer concurrently | Q < A1 (A2 rejected) | `AlreadyAnswered`; first decides | Questions (registration lock) |
| Duplicate or late RPC | after resolution | `Unknown` or `Cancelled`; nothing written | Questions |
| Branch-scoped interrupt, branch close | Q < branch I | Token cancelled → C; close settles the rest with C | Questions; ToolExecutor |
| Answered, other calls of the batch running | Q < A < R | Answer held in the handler result until R | ToolExecutor |
| Answered, then reload before R | Q < A < reload | Batch reload-settled, turn Failed; question outcome answered | ToolExecutor |
| Answer committed, branch part unloads before consuming | Q < A, unload | `close()` writes C: cancelled; `respond` → `service_reloading` | Questions |
| Session part reload while pending | — | Propagates to ToolRegistry; batch reload-settled, turn Failed; branch part writes C if it reloads too (D3) | ToolExecutor; Questions |
| Steer/Queue/Append while pending | Q < input | Consumed only after R (Steer/Append) or `turn/end` (Queue); question continues (A11) | Loop, InputQueue |
| `input/withdrawn` while pending | Q < withdrawn | No effect; withdrawal concerns unconsumed inputs only | InputQueue |
| Several `ask_user` in one batch | Q1 < Q2 | Answered independently; results published together | Questions; ToolExecutor |
| Same `call_id` asked twice | — | Second registration fails; error result | Questions |
| Compaction between Q and R | — | Cannot happen: compaction runs at request preparation, not during a batch | ContextPreparation |
| Child branch cut between Q and A | Q < cut < A | Child has nothing pending; inherited call projects as not executed | Questions (child) |
| Session unload | Q pending | Turn running, Session not idle, not unloaded | AgentIndex |
| Process restart mid-question | Q, no A/C | Load writes C; batch settled; turn Failed | Questions; ToolExecutor; Loop |
| Client detach, no client ever attached | Q pending | Stays pending, no timeout (A7) | User |
| `-p` CLI on an interactive Session | Q < I | CLI stops the turn; C via the interrupt | CLI; Questions |
| Paired remote device answers | Q < A | Accepted like any client | Questions |
| Non-interactive Session | — | Not installed; call by name → `Unknown tool`; method → `unknown_method` | Agents; ToolBinding; Rpc |

## Appendix C. Errors and recovery

| Case | Result |
| --- | --- |
| Invalid tool input | Error `ToolResult`; nothing written |
| Branch service lookup fails (reloading) | Cancelled with `capability reloading`; ToolExecutor reload-settles |
| `asked` commit fails | Entry removed; Cancelled or error result |
| Interrupt, branch close | `question/cancelled`; `execute` ends `cancelled` |
| Reload, restart | `question/cancelled` for every unresolved question; never re-asked |
| `respond` during reload | `service_reloading`; the retry returns `Unknown` because the question was cancelled |

## Appendix D. Acceptance

Flow tests through RPC, tool calls and events:

1. Single, multiple, multi-question and custom answers produce the specified `content`; invalid answers are rejected at `respond` without facts.
2. Session created with `interactive: false` (incl. `exec` and ACP): no `ask_user`, `session.question.respond` → `unknown_method`; present otherwise and when `interactive` is omitted.
3. Answer then stop: result has the answer. Stop then answer: `Cancelled`, `question/cancelled`, cancelled result. Both orders forced by committing `I` before `A` with a held Dispatcher.
4. Two concurrent `respond`: exactly one accepted.
5. Two `ask_user` plus a slow tool in one batch: answers held until the single `step/tool-result`.
6. Branch close and branch-part reload settle pending questions; restart writes "did not survive recovery" and the turn fails; no re-ask.
7. Detach and re-attach: history shows the pending question; answering after re-attach works; answering from a paired remote device works.
8. A withdrawn queued input while a question is pending does not affect it.
9. CLI: rendering, deferral, `Ctrl+C`, EOF, `-p` exit code 3.

## Appendix E. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | The Session part is installed only in interactive Sessions | [ADR 002](../../adr/002-plugin-scope.md) consequences; decisions §3 (ACP/automation non-interactive) | Tool listed everywhere; non-interactive Sessions would block |
| A2 | `interactive` omitted on `session.create` means interactive | Judgment (existing clients) | Existing Sessions lose the tool |
| A3 | Payload adds `turn_id`, `request_id`, `declined` | Judgment | Correlation through `step/tool-call`; decline modeled as an answer |
| A4 | 1..=4 questions, 2..=6 options | Edge-case reference; judgment | Schema limits change |
| A5 | Custom answers allowed by default | Feature list "custom answers"; default is judgment | Model must opt in |
| A6 | The user may decline a whole call | Maintainer decision | User must answer or stop |
| A7 | No timeout | Maintainer decision | A timeout writes `question/cancelled {timeout}` |
| A8 | CLI without input stops with exit code 3 | CLI approval precedent | Different exit policy |
| A9 | Questions of every branch are presented | Approval precedent | Child-branch questions need another client |
| A10 | #2 registration marks events client-visible and Connection forwards them as `session.event` | Overview §1 (#2); judgment | Rpc needs dedicated notifications like `approval.request` |
| A11 | Steer does not end a pending question | Judgment | Steer cancels with reason `user steer` |

## Appendix F. Open questions

| # | Question | Description |
| --- | --- | --- |
| F1 | Overview §7 payloads | `turn_id`, `request_id`, `declined` are not in the overview's `question/*` fields (A3) |
| F2 | Client visibility of dynamic events | Whether #2 projects plugin events to clients by a declared flag (A10) |

## Appendix G. Rust binding

Questions is implemented in Rust; this binding is one realization of the neutral contracts above.

| Neutral contract | Rust binding |
| --- | --- |
| `toolContribution` key `ask_user` | `S:ToolContribution`, collected by ToolRegistry as `Contributions<ToolContribution>` entry `ask_user`; `execute` maps to the handler `HostFuture<Result<ToolResult, HostError>>`, cancel request to the batch `CancellationToken`, `cancelled` to `HostError::Cancelled` |
| `sessionMethods` key `session.question` | `SessionMethodContribution(Arc<dyn MethodProvider>)`, key `SessionScope::named_key::<SessionMethodContribution>("session.question")`; `describe` = `methods()`, `call` = `call(MethodCall, CallChannel, CancellationToken)` |
| `questions` | `B:Questions`, key `BranchScope::key::<Questions>()` |

```rust
pub struct Question { pub id: String, pub header: Option<String>, pub question: String,
    pub options: Vec<QuestionOption>, pub multi_select: bool, pub allow_custom: bool }
pub struct QuestionOption { pub label: String, pub description: Option<String> }
pub struct Answer { pub question_id: String, pub selected: Vec<String>, pub custom: Option<String> }

impl Questions {
    /// Registers and waits. `cancel` is the batch token from ToolExecutor.
    pub async fn ask(&self, call_id: &str, ctx: &ToolExecutionContext, questions: Vec<Question>,
        cancel: CancellationToken) -> Result<Answered, HostError>;
    /// `Ok` means the answer was dispatched and delivered to the waiting call.
    pub async fn respond(&self, call_id: &str, answers: Vec<Answer>, declined: bool)
        -> Result<(), QuestionRespondError>;
}
pub enum QuestionRespondError { Unknown, AlreadyAnswered, Cancelled, Invalid(String), Closed }
```

The unload disposer is registered through `ctx.effect`.
