# SessionTitle

Status: Proposed; Ready confirmation pending. Overall design: [overview](../overview.md) (SessionTitle). Phase: P1. Language: TypeScript first. Scope: [ADR 002](../../adr/002-plugin-scope.md) (Session).

SessionTitle is a Session plugin that gives an interactive leader Session a title stored through SessionStore metadata. It observes its own Session through the existing Session broadcast; when the first operator input on `main` is adopted (`input/applied`), it generates a title from that input's text alone through the Provider `AuxiliaryModel`, with the model from its own setting. The title is SessionStore state: SessionTitle writes it only through SessionStore's typed `setTitle` operation, and SessionStore announces each change with a Session broadcast notice that clients consume (the delivered C6 notice record is described in [§9](#9-requirements-raised-to-mechanism-owners); the SessionStore title metadata and the Rpc forwarding are supplements in [overview Appendix B.2](../overview.md#b2-app-server-process-plugins)). It serves `session.rename` through the Session-scoped `sessionMethods` service keyed `session.rename` ([methods and tool contributions](../contracts/session-boundary.md)). It writes no Timeline event and has no model-visible effect. Contracts are given in neutral form ([ADR 002 D5](../../adr/002-plugin-scope.md#d5-contracts-are-language-neutral)); the Rust binding is in Appendix G.

## 0. Delivered mechanisms and Ready gates

C1/C2, C4 tools/methods #150, S2 #98/#159, S4's Rust host #89,
Settings #166/#175/#177, C6 notices #85 and S5b title metadata #96 are delivered. SessionTitle is not implemented or Ready.

| Boundary | Current contract and Ready decision |
| --- | --- |
| Observation | Use C4's original-row sessionEvents bind, creation configuration and closing, then its own session.event records. C4 broadcast #180 is delivered; #63/#168 are closed. |
| History | Use the implemented process sessions.history service, not private TimelineTree reads or branch (C5) bindings. |
| Configuration | Existing named Settings, row source declarations and change events. Read settings.read({source:"session-title",scope:"user"}) and use its validated/defaulted document directly. A12 proposes source scope/file. |
| Metadata/notice | S5b #60/#96 defines the process service `sessionMetadata` (`read` and `setTitle`, both async), its errors `conflict`/`not_found`/`storage`, the handover and the v3 list fields `title`/`titleSource`/`titleRevision` ([title metadata](../appserver/session-title-metadata.md)); A.2 restates that contract. After each commit SessionStore calls `session_broadcast::notify`, and Rpc forwards the record to attached v3 clients as `session.notice {sessionId, source, kind, data}` (C6 #49/#85, [Session broadcast notices](../appserver/session-notices.md)). The Ready review adopts S5b's names, modes, handover and errors. |
| Auxiliary request | Provider implements AuxiliaryModelHost. S4's auxiliaryModel.complete neutral contract is documented, but its named HostDispatch/catalog binding is absent. Bind the existing owner operation through ordinary C4 injection; credentials stay with Provider. |
| Confirmation | Confirm A1–A12 and owner bindings before implementation. Documentation CI is not runtime acceptance. |

C7 #50 is already unnecessary (#86). The product credential-store decision in #118 does not block
this package: S4 owns its own credential/connection resolution. Once named bindings exist, missing
required providers keep the whole row Pending under existing rules, including rename; native
Session use continues. No optional lookup or alternate rename path.

The C4 leaf callback filters its own Session/main adoption, registers an owned task and returns;
it does not await Settings/history/model work. Native EventReader delivers only published events,
and SessionBroadcast forwards that Dispatcher stream. Thus the existing history({atLeast:N}) read
can capture published T >= N while the branch exists: no extra publication wait, polling or Core
API. Retain T and page with through=T and after=the last returned sequence; never send atLeast and
through together. Use native HistoryPage/AgentEvent data. S2's stable directory reads the index per
call, so index reload does not reload this consumer.

Use the [C4 binding contract](../session-typescript.md): declare
`sessionMethods:session.rename` with async describe/call and `sessionEvents:session-title` with
sync bind in one provides dictionary. Bind registers observation/cleanup and returns JSON null;
it does not start an asynchronous initialization job. Its creation configuration is immutable.

The fixed snapshot ends at returned T, not necessarily at N. Its first adopted operator input
supplies text. With model=null, the latest ContextConfiguredData.settings in that snapshot supplies
providerId/modelId; without an adopted model, the attempt fails model_unavailable rather than
using Provider's current default.

Cancellation before setTitle handover stops work without writing. The handover is the setTitle call
itself: from then on SessionStore settles its own atomic operation, and a lost reply does not authorize
repeating or undoing it ([S5b §6](../appserver/session-title-metadata.md#6-handover-cancellation-and-settlement)). No rollback or
coordination mechanism.

## 1. Structure

```mermaid
flowchart LR
  subgraph PROC["Process"]
    AUX["AuxiliaryModel (Provider)"]
    META["SessionStore title metadata"]
    Settings["Settings source session-title"]
    Rpc["Rpc"]
    TT["sessions (S2): published main history"]
  end
  subgraph SESS["Session"]
    ST["SessionTitle"]:::new
    SB["Session broadcast"]
    CFG["SessionConfig"]
  end
  ST -->|"listen: input/applied on main"| SB
  ST -->|"read fixed prefix T >= N"| TT
  ST --> CFG
  ST -->|"complete(model, text)"| AUX
  ST -->|"setTitle with revision"| META
  META -->|"title notice"| SB
  ST -->|"read at use"| Settings
  Rpc -->|"session.rename dispatched through the Session"| ST
  Rpc -->|"session.list reads title fields"| META
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
```

| Part | Scope | Depends on | Provides |
| --- | --- | --- | --- |
| `SessionTitle` | Session | C4 Session observation/configuration binding; process sessions history, Settings, title metadata `sessionMetadata` (S5b) and the required AuxiliaryModel binding (not implemented; absent providers keep the row Pending after binding is available) | sessionMethods key session.rename and sessionEvents bind from one provides dictionary; Settings source session-title |

Dependencies stay in the Session or enclosing process. C4/S2/Settings supply existing observation, prefix reads and configuration; S5b supplies title metadata and C6 the notices; the named auxiliary binding retains §0's owner gate. The package declares its source in the ordinary row's settings member, alongside provides. Real provider reloads cancel the row's work (ADR 002 D3); S2 index reload does not. Business data changes never reload the package (D4).

## 2. Data and state

```mermaid
flowchart TB
  subgraph TLM["main branch Timeline"]
    FUI["first adopted operator input (input/message, input/applied)"]
    CC["context/configured (adopted model)"]
  end
  subgraph STORE["SessionStore metadata (persistent)"]
    MD["revision, title text, source auto or manual"]
  end
  CONF["SessionConfig: interactive, collaborationRole"]
  CFG["session-title: enabled, maxLength, model"]
  MEM["in-flight generation, failed attempts (memory)"]:::new
  FUI -->|"text only"| MEM
  CC -->|"Session model"| MEM
  CONF --> MEM
  CFG --> MEM
  MEM -->|"setTitle, expected revision"| MD
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
```

SessionStore owns the title: it keeps the text, source and revision counter and accepts changes only through its typed `setTitle` operation, which SessionTitle alone calls. After each committed change SessionStore emits a title notice on the Session broadcast (A11); the title never enters the Timeline. Each decision reads metadata first, then the fixed prefix. In-memory state is lost on reload or Session unload, which only allows another attempt.

## 3. Flows

### 3.1 Automatic title from the first user input

```mermaid
sequenceDiagram
  participant TL as main Timeline
  participant SB as Session broadcast
  participant ST as SessionTitle
  participant Meta as SessionStore metadata
  participant Sessions as sessions (S2)
  participant Aux as AuxiliaryModel
  TL-->>SB: input/applied at N, branch main
  SB-->>ST: Event
  Note over ST: interactive leader, enabled, none in flight
  ST->>Meta: metadata: no title, revision r
  ST->>Sessions: history(main, atLeast N), page fixed T: first adopted operator text
  ST->>Aux: complete(model, text)
  Aux-->>ST: title text
  ST->>Meta: setTitle(expected r, auto)
  Meta-->>SB: title notice
```

Generation starts when the input is adopted, not after the turn. Only the text of the first adopted operator input on `main` is sent; a deleted input is never adopted, so it never titles the Session and its text never reaches the auxiliary model.
The main turn never waits for title completion; the title task follows its own budget.

### 3.2 Manual rename

```mermaid
sequenceDiagram
  participant CLI
  participant Rpc
  participant ST as SessionTitle (in the Session)
  participant Meta as SessionStore metadata
  CLI->>Rpc: session.rename sessionId, title
  Rpc->>ST: resolve Session, sessionMethods call
  ST->>Meta: metadata, revision r
  ST->>Meta: setTitle(expected r, manual)
  Meta-->>ST: revision r+1
  Note over Meta: title notice on the Session broadcast
  ST-->>Rpc: title, titleSource manual, revision
```

Rpc resolves the Session as for any Session method, loading it if needed. On `conflict` SessionTitle rereads and retries up to 3 times.

## 4. Timing and conflicts

```mermaid
sequenceDiagram
  participant G as generation
  participant R as rename
  participant Meta as SessionStore metadata
  G->>Meta: metadata, revision r, no title
  R->>Meta: setTitle(expected r, manual)
  Meta-->>R: r+1
  G->>Meta: setTitle(expected r, auto)
  Meta-->>G: conflict
  Note over G: automatic title discarded
```

The revision check is the recheck: a manual title always wins.

| Scenario | Order | Outcome | Owner |
| --- | --- | --- | --- |
| Member Session (`collaborationRole`), or a non-interactive Session | `SessionConfig` | Never generated; rename allowed | SessionTitle |
| Second input before generation finishes | — | Skipped (in flight) | SessionTitle |
| First input deleted | input < `input/deleted` | Never adopted: no title from it; the next adopted operator input triggers (A9) | SessionTitle |
| Generation failed | — | Next adopted `main` input while untitled retries from the first adopted input (A5) | SessionTitle |
| Reload or Session unload mid-generation | — | Cancel before store handover; the next adopted main input may trigger. An admitted mutation settles with SessionStore | Lifecycle / SessionStore |

Full table: [Appendix B](#appendix-b-full-conflict-table).

## 5. Context mapping

No model-visible effect: SessionTitle writes no input, tool, prompt source or event. The auxiliary request is a separate one-shot call:

| Part | Content |
| --- | --- |
| System | `Write a short title for this conversation: at most 8 words, in the language of the user's message, no quotes, no trailing punctuation. Reply with the title only.` (A2) |
| User message | Text blocks of the first adopted operator `input/message` on `main` (one with `input/applied`), joined by newlines, cut to 2000 characters (A2) |

Never sent: images, raw input blocks, deleted inputs, later inputs, assistant output, system prompt, tool calls and results, child-branch events. Diagnostics record outcome and duration only.

## 6. Interfaces (summary)

| Interface | Kind | Purpose |
| --- | --- | --- |
| `session.rename {sessionId, title \| null}` | `sessionMethods` key `session.rename` | Set or clear a manual title |
| `session-title` source: `enabled`, `maxLength`, `model` | Business configuration (Settings source) | Turn generation off; title length; model for titles (unset: the Session's model) |
| `title`, `titleSource`, `titleRevision` in v3 `session.list`/`session.summaries` | Rpc protocol fields read from SessionStore ([S5b §8](../appserver/session-title-metadata.md#8-session-lists)) | Display; the revision orders a list against notices |
| Title notice, `data: {title, titleSource, titleRevision}` | Session broadcast notice emitted by SessionStore (C6 record, §9; [S5b §7](../appserver/session-title-metadata.md#7-the-title-notice)) | Live title update for subscribed clients |

## 7. Security

The first input's text is sent to the model in the `session-title` source; unset, that is the Session's model. A Session whose model is an `external-agent:*` entry is not titled automatically, so a title request never goes to an external agent bridge. Titles may contain user content and are returned to every client that can list Sessions. No credentials, host settings or tool output enter the request. Rename input is validated by SessionTitle's method (A.1).

## 8. Rule check

| Rule | How this document complies |
| --- | --- |
| Principles §4 rule 1 (commands for others' state) | The title is SessionStore's process-level state with no Timeline; SessionTitle changes it only through SessionStore's typed `setTitle`, and SessionStore decides through its revision check (principles §4 rule 9). |
| Principles §4 rule 2 (handover) | SessionTitle reads an input only after its handover (`input/applied`) and never acts on the input itself. |
| Principles §4 rule 3 (before callbacks only) | No intervention before a write; rename rules are checked by SessionTitle's own method, not at an entry. |
| Principles §4 rule 4 (no business meaning in core) | The delivered C6 Session broadcast notice in §9 is a generic record for any owner of process-level Session state. |
| Principles §4 rule 5 (batch) | The trigger acts on `input/applied` when it is consumed, in sequence order. |
| Principles §4 rule 6 (context from handed-over events) | SessionTitle adds nothing to model context; the auxiliary request contains only adopted input text. |
| Principles §4 rule 7 (public events and services only) | It uses the Session broadcast, fixed-prefix read, SessionStore `setTitle`, `AuxiliaryModel` and Settings, all public. |
| Principles §4 rule 8 (requirements to mechanism owners) | C6 and S5b deliver the notice and metadata contracts in §9. None for #4: SessionTitle uses the C4 Session event binding and named process services, without a runtime typed-service dependency. Requirements to other plugins (SessionStore title metadata and notice emission, Rpc forwarding of notices) are listed in [overview Appendix B.2](../overview.md#b2-app-server-process-plugins). |
| Principles §4 rule 9 (only facts later decisions use) | The title is process-level SessionStore state: it never enters the Timeline, changes through `setTitle` and reaches UIs as a Session broadcast notice. |
| Principles §1 (Capability and App Server boundary) | SessionTitle is a Session Capability in its own plugin package; Rpc only dispatches `session.rename` and reads title fields from SessionStore. |
| ADR 002 D1 (scope follows state) | Session scope: the title belongs to the Session; the stored title is SessionStore's. |
| ADR 002 D2 (no wiring modules) | No wiring module; all dependencies are declared services. |
| ADR 002 D3 (dependencies follow scope) | Dependencies are in the Session or enclosing process scope; their reload cancels a generation, which is accepted. |
| ADR 002 D4 (structural vs business configuration) | Enabling is structural; the `session-title` document is business configuration in its own Settings source, read at use. |
| ADR 002 D5 (language-neutral contracts) | Contracts are neutral; the Rust form is Appendix G. |

## 9. Requirements raised to mechanism owners

**R1. Session broadcast notice for process-level state — delivered.**

C6 #85 supplies the notice record and Rpc forwarding; S5b #96 supplies title metadata and
post-commit title notices. SessionTitle consumes these delivered contracts.

| Item | Content |
| --- | --- |
| Blocked by | Principles §4 rules 9 and 4: process-level state such as the SessionStore title must not enter the Timeline, yet clients must see title changes live; the delivered C6 `Notice` record carries these post-commit changes ([session_broadcast.rs](../../../crates/dim-agent-capabilities/src/session_broadcast.rs)). |
| Existing mechanisms considered | A `session/titled` Timeline event (puts process state on the Timeline; rejected in overview §7); client polling of `session.list` (no live update); a title-specific channel (a business-specific port, rule 8). |
| Generic capability | A notice record `{source, kind, data}` on a Session's broadcast channel, emitted by the owner of process-level state about that Session after a committed change; no business meaning. The title notice is this record with `source: "SessionStore"`, `kind: "title"`, `data: {title, titleSource, titleRevision}` ([S5b §7](../appserver/session-title-metadata.md#7-the-title-notice)). Rpc forwards that record unchanged as `session.notice`, adding the channel's `sessionId` ([C6 D4](../appserver/session-notices.md#10-decisions)), so `data` does not repeat it. |
| Effect on sequence, batch, interrupt, reload | No sequence number and no batch; not replayed: a client that resubscribes reads current state through the owner's query (`session.list`); interrupts do not apply; a reload of the emitter only stops notices. |
| Owner and issue | Runtime C6 [#49](https://github.com/arcships/dim/issues/49), implemented in [#85](https://github.com/arcships/dim/pull/85): the record, `session_broadcast::notify` and Rpc's `session.notice` ([Session broadcast notices](../appserver/session-notices.md)). SessionStore emits the title notice ([S5b](../appserver/session-title-metadata.md), the supplement in [overview Appendix B.2](../overview.md#b2-app-server-process-plugins)). |

**#4: None.** Observation uses C4 `sessionEvents`; history, Settings, `sessionMetadata` and the auxiliary model are process services. SessionTitle injects no branch service.

## Appendix A. Interface details

### A.1 `session.rename`

Served through `sessionMethods` (Session, key `session.rename`): `describe(): async -> MethodDecl[]` returns one declaration `{name: "session.rename", mutating: true, interruption: "report", progress: false}`; `call(MethodCall): async -> MethodReply` handles it. A dropped call is cancelled before `setTitle` or completes atomically after it.

| Params | Result | Errors |
| --- | --- | --- |
| `{sessionId, title: string \| null}` | `{title: string \| null, titleSource: "auto" \| "manual" \| null, revision}` | `invalid_params`, `session_not_found`, `service_reloading`, `storage_failed`; dispatch errors per [session-boundary](../contracts/session-boundary.md) |

`title` is trimmed; 1..=`maxLength` (source `session-title`) characters, no line breaks or control characters (A3). `null` clears the title (A6).

### A.2 Depended-on shapes (owners define them)

| Service | Method | Kind | Request | Reply and errors |
| --- | --- | --- | --- | --- |
| SessionStore `sessionMetadata` (S5b) | `read` | async | `{sessionId}` | `{revision, title: {text, source: "auto" \| "manual"} \| null}`; errors `not_found`, `storage` ([S5b §4](../appserver/session-title-metadata.md#4-operations)) |
| SessionStore `sessionMetadata` (S5b) | `setTitle` | async | `{sessionId, expectedRevision, title: {text, source} \| null}` | `{revision}`; errors `conflict`, `not_found`, `storage`. The call is the handover; after a commit SessionStore emits the title notice |
| SessionStore (S5b) | title notice | Session broadcast notice (C6 record, §9) | — | `{source: "SessionStore", kind: "title", data: {title, titleSource, titleRevision}}` on the Session's broadcast channel; no sequence number, not replayed |
| Provider `AuxiliaryModel` | `complete` | async | `{model: {providerId, modelId}, request: {system, messages: [{role: "user", content: text}], maxOutputTokens}}` ([design](../appserver/auxiliary-model.md)) | `{text, stopReason, usage}`; errors as data. Cancelled by a cancel request or by dropping the call |

### A.3 Configuration

Settings source `session-title`, one document: `enabled: bool` default `true`; `maxLength: u32` default 80, range 10..=200 (A3); `model: {providerId, modelId} | null` default `null`, meaning the model of the latest `context/configured` in the prefix (A1). The schema SessionTitle declares carries the defaults and the range (`minimum`/`maximum`), so Settings rejects an out-of-range value on write and when reading a persisted file, and SessionTitle uses the delivered document without checking it again. Read at each decision. SessionTitle has no tool that writes configuration, so the document is written only through the client methods `settings.read`/`settings.update` (S8b named Settings delivered). A change is a data update with notification, never a reload. Enabling the plugin is structural configuration. An unselectable `model` makes the call fail `model_unavailable`, which counts as a failed attempt.

### A.4 Mechanics

| Step | Path |
| --- | --- |
| Listen | Original C4 row declares `sessionEvents:<part>` `bind(sync)`, subscribes its own session.event, filters main input/applied, registers an owned task and returns; closing/row cleanup ends owned work |
| Eligibility | C4 creation snapshot: `interactive !== false && collaborationRole === "leader"`; native omission means true |
| Trigger | On `input/applied` at `N`: skip if not eligible, disabled, in flight or failed attempts ≥ 3 (A5); otherwise start a generation task owned by the plugin's lifetime |
| Check | `sessionMetadata.read({sessionId})`: skip if any title exists; keep revision `r` |
| Read | Existing sessions.history with atLeast=N fixes published T >= N, then through=T pages (S2 A.5). Select the first operator input with user content and input/applied; no such input/text means no title (A8/A10) |
| Generate | `complete({model: the source's model, else the model of the latest context/configured in the prefix, request})`, 30 s budget (A4). Normalized: first line, quotes and trailing punctuation removed, whitespace collapsed, cut at `maxLength` |
| Write | `setTitle({sessionId, expectedRevision: r, title: {text, source: "auto"}})`; `conflict` or `not_found` discards |
| Timeline | No writes, no Dispatcher registration, no `StateCell` |
| Reload / unload | Cancel before store handover; the owner settles an admitted mutation. Events in the gap are not replayed |

## Appendix B. Full conflict table

| Scenario | Sequence order | Outcome | Owner |
| --- | --- | --- | --- |
| First input's turn interrupted or failed | input < `stop/interrupt` | Generation proceeds | SessionTitle |
| Inputs without `user` content (Goal blocks, source markers) | — | Never trigger, never used | SessionTitle |
| First input withdrawn (`session.input.withdraw`) before it is applied | input < withdraw < `input/deleted` | Never applied: no generation, text never sent; the first applied operator input triggers | SessionTitle |
| First user input carries only images | — | No automatic title (A8) | SessionTitle |
| Rename before the check | rename < check | Title present: skip | SessionTitle |
| Rename during generation | check (r) < rename (r+1) < write | `conflict`: automatic title discarded | SessionStore revision |
| Concurrent renames | revision order | Last committed rename wins | SessionStore |
| Duplicate `session.rename` | — | Rewrites the same text | SessionTitle |
| Steer/Queue/Append inputs after the first | — | Never in the excerpt | SessionTitle |
| Tool batch, compaction | — | Irrelevant: only the input fact is read | — |
| Child branch inputs | branch filter | Never trigger | SessionTitle |
| Member, stage, scheduled or ACP Session | `SessionConfig` | Never generated | SessionTitle |
| Session close during generation | — | Cancel before store handover; an admitted mutation remains owned by SessionStore | Lifecycle / SessionStore |
| Session fork | metadata not copied (A7) | Fork untitled; the next adopted operator input on `main` triggers generation from the first adopted operator input in its prefix | SessionTitle |
| SessionTitle, Provider, SessionStore or Settings reload | — | Before handover, cancel generation; an admitted store mutation follows S5b settlement (D3) | Lifecycle / SessionStore |
| Model or policy unavailable | — | Failure counted; after 3 no further attempts until reload (A5) | SessionTitle |
| Resource node offline | — | No effect | — |
| Client detach | — | No effect | — |

## Appendix C. Errors and recovery

| Case | Result |
| --- | --- |
| Model error, timeout, empty output, unavailable policy | No write; diagnostic; failure counted |
| `setTitle` `conflict` on automatic write | Discarded (diagnostic) |
| `setTitle` `storage` | Automatic: failure counted; rename: `storage_failed` |
| Unknown Session on rename | `session_not_found` |
| SessionTitle reloading | Before handover: service_reloading, no rename. After handover/lost reply: read metadata; do not infer no commit or repeat the write |
| Cancellation | Before handover abort the auxiliary call and discard; after store admission, S5b owns the atomic outcome |

## Appendix D. Acceptance

Flow tests through `session.create`/`session.submit`, `session.rename`, `session.list` and a fake auxiliary model:

1. Hold the main turn open with the existing model fixture. The first `session.submit` on an interactive leader Session starts generation after adoption and produces an `auto` title while that controlled turn is still active; the request contains only the system text and that input's text, and one title notice reaches subscribed clients. Completion before turn end is not a production ordering guarantee.
2. Interrupting or failing the first turn does not prevent the title.
3. Member Sessions and `interactive: false` Sessions get no title; rename works on them.
4. Rename during generation (fake model held): manual kept, automatic discarded.
5. A titled Session is never regenerated; clearing allows regeneration at the next adopted input, still from the first adopted input's text.
6. Rename of an unloaded Session loads it through Rpc and succeeds.
7. Child-branch inputs and inputs without `user` content never trigger; image-only first input yields no title.
8. Reload before store handover cancels generation without writing; the next adopted input may trigger. Reload after admission leaves settlement to SessionStore; reread metadata after a lost reply without repeating the write. Three failures stop attempts.
9. Invalid rename inputs return `invalid_params`.
10. A first input withdrawn before it is applied (`session.input.withdraw`) produces no title and is never sent to the auxiliary model; the next adopted input titles the Session from its own text.

## Appendix E. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | An unset `model` uses the Session's model (latest `context/configured`) | Judgment | A fixed default model |
| A2 | Instruction text, 8-word target, 2000-character excerpt | Judgment | Different prompt or excerpt |
| A3 | Title length default 80 (10..=200), single line | Judgment | Other limits |
| A4 | 30 s generation budget | Judgment | Other timeout |
| A5 | At most 3 failed attempts per Session per plugin generation | Judgment | More retries, or none |
| A6 | `session.rename` with `null` clears the title | Judgment | Manual title permanent |
| A7 | Session fork does not copy the title | Judgment | Fork shows the source title |
| A8 | A first user input without text produces no automatic title | Decision: title from first user input | Later inputs or a placeholder |
| A9 | Generation waits for adoption (`input/applied`); a deleted input never titles the Session | Maintainer decision | Deleted text could reach the auxiliary model |
| A10 | "User input" means operator input: in an interactive leader Session it is the input with `user` content, since Collaboration, Goal and ShellInput write raw blocks only (Scheduler and Workflow inputs carry `user` content but run in non-interactive Sessions, which SessionTitle does not title) | Judgment | Source-written inputs could title a Session |
| A11 | SessionStore announces title changes with a Session broadcast notice; a client that is not subscribed sees the change at its next `session.list` | Principles §4 rule 9 | Clients poll `session.list` |
| A12 | Proposed user-only source file session-title.json, no project merge; confirm before implementation | One process business document, existing S8b row source mechanism | Different source/scope requires explicit confirmation |

## Appendix F. Open questions

| Question | Description |
| --- | --- |
| Session list fields | Resolved by S5b: `title`, `titleSource` and `titleRevision` in v3 entries, recorded in overview B.2 |
| Named bindings | C4 observation, S2 history, Settings, C6 notices and S5b's `sessionMetadata` are delivered paths. S4's named auxiliary binding remains before Ready; its owner keeps its contract. No new Core request |

## Appendix G. Rust binding

| Neutral | Rust |
| --- | --- |
| `sessionMethods` key `session.rename` | C4 infers session.rename from sessionMethods:session.rename and uses SessionMethods::provide_foreign on the original row; Rust providers use SessionMethods::provide with complete MethodDecl values |
| Session broadcast listener | Listener on `session_channel(session_id)` for `Event` records, removed by `ctx.effect` |
| Generation task | Spawned under `ctx.effect` |

```rust
// SessionStore metadata (S5b; named service `sessionMetadata`)
async fn metadata(&self, session_id: &str) -> Result<SessionMetadata, MetadataError>;
fn set_title(&self, session_id: &str, expected_revision: u64, title: Option<Title>)
    -> impl Future<Output = Result<u64, MetadataError>>; // Conflict | NotFound | Storage; the call is the handover
pub struct SessionMetadata { pub revision: u64, pub title: Option<Title> }
pub struct Title { pub text: String, pub source: TitleSource }   // Auto | Manual
// Provider AuxiliaryModel
async fn complete(&self, model: ModelId, request: AuxiliaryRequest,
    cancel: CancellationToken) -> Result<AuxiliaryReply, AuxiliaryError>;
```
