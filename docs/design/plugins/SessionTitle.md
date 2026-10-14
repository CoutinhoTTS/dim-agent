# SessionTitle

Status: Draft. Overall design: [overview §5.6](../overview.md#56-sessiontitle). Phase: P1. Language: TypeScript first ([overview §11.3](../overview.md#113-assignment)).

SessionTitle gives each Session a title stored as SessionStore metadata. It generates an automatic title with an auxiliary model after the first normally completed turn on the main branch, observed through the [broadcast subscription](../contracts/session-broadcast-subscription.md), and serves `session.rename` for manual titles. It is the single writer of title metadata, writes no Timeline event and has no model-visible effect on any Session.

## 1. User flows

1. The user sends the first message of a new Session; the turn ends with `completed`. SessionTitle receives the `turn/end` notice for `main`, finds no title, reads the excerpt (§9) through [session-access](../contracts/session-access.md), asks the model selected for its purpose key, and stores `{text, source: auto}`. The Session list shows the title.
2. The first turn is interrupted or fails. Nothing is generated; the next normally completed `main` turn triggers generation.
3. The user renames the Session from the scrollback panel or `dimcode session rename`. The CLI calls `session.rename`; SessionTitle stores `{text, source: manual}`, also for a Session that is not loaded. An automatic generation in flight loses the revision check and is discarded.
4. The user clears a manual title (`title: null`) (A6). The next normally completed `main` turn generates an automatic title.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Deciding when to generate, the excerpt, the auxiliary request, output normalization | Model selection policy and the model call itself (Provider supplement) |
| Writing title metadata with source and revision check | Title storage, revision counter, Session list fields (SessionStore supplement, Rpc) |
| `session.rename` validation | Displaying titles (CLI); internationalization of UI text (CLI) |
| Its purpose key `session-title` and configuration keys `sessionTitle.*` | Session lifecycle; any Timeline write |

## 3. Scope and dependencies

**Contracts depended on:** [`SessionEvents`](../contracts/session-broadcast-subscription.md) (trigger), [`SessionAccess`](../contracts/session-access.md) (read-only use: `get`, `prefix`; no input is ever written), the public data contracts of `turn/end`, `input/message`, `step/text` and `context/configured`, the SessionStore metadata supplement (overview §7.2) and the Provider purpose-based selection supplement (overview §7.2) plus an auxiliary completion port (proposed, §14). SessionTitle does not know which plugin provides each contract.

**Contracts defined here:** the `SessionTitles` service used by Rpc for `session.rename`, the opaque Provider purpose key `session-title` (A1), and configuration keys `sessionTitle.enabled` and `sessionTitle.maxLength`.

| Part | Scope | Injects | Provides | Status |
| --- | --- | --- | --- | --- |
| `SessionTitle` | Process (App Server) | `Arc<SessionEvents>`, `Arc<SessionAccess>`, `Arc<SessionMetadataStore>`, `Arc<ModelSelectionHost>`, `Arc<AuxiliaryModel>` | `SessionTitles` | Proposed |
| SessionStore | Process | existing | `SessionMetadataStore` (title, source, revision) | Supplement (overview §7.2) |
| Provider | Process | existing | `ModelSelectionHost`, `AuxiliaryModel` | Supplement (overview §7.2); `AuxiliaryModel` proposed here |
| Rpc | Process | `When<Arc<SessionTitles>>` | `session.rename`; `title`, `titleSource` in Session list results | Supplement (see §14) |

Depended-on shapes, as this document needs them (authoritative definitions belong to the owners' documents):

```rust
// SessionStore supplement
async fn metadata(&self, session_id: &str) -> Result<SessionMetadata, MetadataError>;
async fn set_title(&self, session_id: &str, expected_revision: u64, title: Option<Title>)
    -> Result<u64, MetadataError>;                    // Conflict | NotFound | Storage
pub struct SessionMetadata { pub revision: u64, pub title: Option<Title> }
pub struct Title { pub text: String, pub source: TitleSource }   // Auto | Manual
// Provider supplement
async fn select(&self, purpose: &str, inherit: Option<ModelSelection>) -> Result<ModelSelection, SelectError>;
async fn complete(&self, model: ModelSelection, request: AuxiliaryRequest, cancel: CancellationToken)
    -> Result<String, AuxiliaryError>;               // text output only
```

## 4. Events

None. The title is directory metadata that no branch decision consumes (overview §6 "`session/titled`"). Generation attempts and failures go to the diagnostics channel. Events consumed, all through the prefix or notices:

| Event | Use |
| --- | --- |
| `turn/end` on `main` (notice) | Trigger; only `status: completed` |
| `input/message` | First operator content for the excerpt |
| `step/text` | Assistant text of the triggering turn |
| `context/configured` | Latest model selection, passed as `inherit` to Provider |

## 5. State

| State | Owner | Rebuild |
| --- | --- | --- |
| Title, source, revision | SessionStore metadata (written only by SessionTitle) | Persistent |
| In-flight generation per Session | SessionTitle, memory | Not rebuilt; cancelled on unload |
| Failed attempts per Session | SessionTitle, memory | Not rebuilt; resets on reload or restart (A5) |

Nothing is derived from Timeline into owned state; every decision reads metadata first.

## 6. Interfaces

### Session method

| Method | Params | Result | Errors |
| --- | --- | --- | --- |
| `session.rename` | `{sessionId, title: string \| null}` | `{title: string \| null, titleSource: "auto" \| "manual" \| null, revision}` | `invalid_params`, `session_not_found`, `service_reloading`, `storage_failed` |

`title` is trimmed; it must be 1..=`sessionTitle.maxLength` characters after trimming and contain no line breaks or control characters (A3). `null` removes the title (A6). The call works whether or not the Session is loaded and never loads it.

```rust
/// Process service consumed by Rpc; key TypeKey::of::<SessionTitles>().
impl SessionTitles {
    async fn rename(&self, session_id: &str, title: Option<String>) -> Result<SessionMetadata, TitleError>;
}
```

Session list (`session.list`, `session.summaries`) reads `title` and `titleSource` from metadata (Rpc and SessionStore supplements).

Configuration (loader editable layer): `sessionTitle.enabled: bool` default `true`; `sessionTitle.maxLength: u32` default 80, range 10..=200 (A3). Provider policy for purpose `session-title` is configured in Provider's per-purpose policy; unconfigured means inherit (A1). No tools, no management methods. CLI: the scrollback rename panel and `dimcode session rename` call `session.rename`.

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Subscribe | In `apply`: `subscribe(ctx, {types: {"turn/end"}, sessions: None, branches: {"main"}}, 64)`; the stream task is registered with `ctx.effect` |
| Trigger | On `Event` with `turn/end {status: completed}` at sequence `N`: skip if disabled, a generation for the Session is in flight, or attempts ≥ 3 (A5); otherwise register a generation task with the plugin's task tracker and return to `next()` |
| Check | `metadata(session)`: skip if `title` is present (any source); keep `revision = r` |
| Read | `access.source("session-title").get(session)` (loaded only; skip if not loaded) → `prefix("main", N)`; the lease is dropped right after the read, before the model call |
| Select | `select("session-title", inherit = model of the latest context/configured in the prefix)` |
| Generate | `complete(model, request(§9), cancel)` with a 30 s budget (A4); output normalized: first line, surrounding quotes and trailing punctuation removed, whitespace collapsed, cut at `maxLength` on a character boundary; empty output is a failure |
| Write | `set_title(session, expected_revision = r, Some({text, auto}))`. `Conflict` (a rename or another write happened) discards the result; `NotFound` (deleted) discards |
| Rename | `rename` reads `metadata`, then `set_title(expected = revision, {text, manual})`; on `Conflict` it rereads and retries up to 3 times; a manual write always wins because only SessionTitle writes titles and its automatic writes yield on conflict |
| Timeline writes | None. No Dispatcher registration, no `StateCell` |
| Recheck | The revision check is the recheck: the automatic title is written only if metadata is unchanged since the check that found no title |
| Reload / restart | Generation tasks are cancelled through `ctx.effect` (the auxiliary call is aborted); nothing is written. The new generation subscribes afresh; missed notices are not replayed; the next completed `main` turn of an untitled Session triggers generation |

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| First turn interrupted | `stop/interrupt` < `turn/end {cancelled}` | No generation | SessionTitle |
| First turn failed | `turn/end {failed}` | No generation; next completed turn triggers | SessionTitle |
| Interrupt after a completed `turn/end` | `turn/end` < interrupt | Generation proceeds; it is process work, not turn work | SessionTitle |
| Rename before the check | rename < check | Title present: skip | SessionTitle (metadata) |
| Rename during generation | check (r) < rename (r+1) < write | `Conflict`: automatic title discarded | SessionStore revision |
| Two completed turns before generation finishes | — | Second notice skipped (in flight) | SessionTitle |
| Concurrent renames | revision order | Last committed rename wins; each returns its revision | SessionStore |
| Duplicate or late `session.rename` | — | Rewrites the same text; idempotent in effect | SessionTitle |
| Steer/Queue/Append inputs around the turn | input after `N` | Not in the excerpt (prefix fixed at `N`'s published head; text taken only up to `N`'s turn) | SessionTitle |
| Tool batch not settled | — | `turn/end` exists only after the batch result; no partial trigger | Loop |
| Compaction | — | Irrelevant: the excerpt reads facts, not model context | — |
| Child branch turns | branch filter | Never trigger | SessionEvents filter |
| Branch-scoped interrupt / Session close during generation | — | Write proceeds (metadata outlives loading); deleted Session gives `NotFound` | SessionStore |
| Session idle unload during generation | lease already dropped | Generation completes and writes | SessionTitle |
| Session not loaded when notice handled | — | Skip; a later completed turn triggers | SessionTitle |
| Session fork | metadata not copied (A7) | Fork untitled; generated after its first completed `main` turn | SessionTitle |
| SessionTitle reload or process restart mid-generation | — | Cancelled, nothing written; regenerated at the next completed turn | rutis lifecycle |
| Gap notice | — | Ignored; next `turn/end` triggers | SessionTitle |
| Provider selection or model unavailable | — | Failure counted; after 3 failures no further attempts in this generation (A5) | SessionTitle |
| Resource node offline | — | No effect (no host resources used) | — |
| Client detach | — | No effect | — |

## 9. Context mapping

No model-visible effect on any Session: SessionTitle writes no input, no tool, no prompt source and no event; the title never enters model context of the Session.

The auxiliary request (a separate model call, not part of any Session's context) contains exactly:

| Part | Content |
| --- | --- |
| System | `Write a short title for this conversation: at most 8 words, in the language of the user's message, no quotes, no trailing punctuation. Reply with the title only.` (A2) |
| User message 1 | `User:` + text of the first `input/message` in `main`'s prefix that carries `user` content: its text blocks joined by newlines, each image replaced by `[image]`, cut to 2000 characters (A2) |
| User message 2 | `Assistant:` + concatenated `step/text` deltas of the last request of the triggering turn (`turn_id` of `turn/end` N), cut to 1000 characters; omitted if empty |

Never sent: raw input blocks (rules, state, goals, notices from any source), system prompt, tool calls and results, thinking, images, inputs after the first operator input, events of child branches. Diagnostics record attempt outcome and duration only, never excerpt or output text.

## 10. Errors and recovery

| Case | Result |
| --- | --- |
| Selection fails, model error, timeout, empty output | No write; diagnostic; failure counted |
| `set_title` `Conflict` on automatic write | Discarded silently (diagnostic) |
| `set_title` `Storage` | Automatic: failure counted; rename: `storage_failed` |
| Unknown Session on rename | `session_not_found` |
| SessionTitle reloading | Rpc returns `service_reloading`; the rename did not happen |
| Cancellation | Auxiliary call aborted before any write; no partial title |

## 11. Security

The excerpt is user and assistant text sent to the model chosen for `session-title`. With the default inherit policy it goes to the same provider as the Session; a policy pointing at another provider sends it there, which is the user's configuration choice and documented in Provider's settings. Titles are stored in SessionStore and returned to every client that can list Sessions; they may contain user content. No credentials, host settings or tool output enter the request. Rename input is validated at the RPC boundary (§6).

## 12. Acceptance

Flow tests through `session.create`/`session.submit`, `session.rename`, `session.list` and a fake auxiliary model:

1. First completed `main` turn produces an `auto` title; the request contains only the parts in §9 (no raw blocks, no tool output).
2. Interrupted or failed first turn produces no title; the next completed turn does.
3. Rename during generation (fake model held): manual title kept, automatic discarded.
4. A titled Session is never regenerated; clearing the title allows regeneration.
5. Rename of an unloaded Session succeeds without loading it.
6. Child branch turns never trigger; a non-loaded Session is skipped.
7. Reload during generation writes nothing; the next completed turn generates.
8. Three model failures stop attempts until reload.
9. Invalid rename inputs (empty, too long, line breaks) return `invalid_params`.

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Purpose key `session-title`, owned by SessionTitle; unconfigured policy inherits the Session's model | Overview §3 constraint 10 (consumer-defined purposes); overview §5.6 names "auxiliary" (§14) | Shares the `auxiliary` key with compaction |
| A2 | Instruction text, 8-word target, excerpt parts and limits (2000/1000 characters) | Judgment | Different prompt or excerpt |
| A3 | Title length default 80 (10..=200), single line | Judgment | Other limits |
| A4 | 30 s generation budget | Judgment | Other timeout |
| A5 | At most 3 failed attempts per Session per plugin generation; retries only on later completed turns | Judgment | More retries, or none |
| A6 | `session.rename` with `null` clears the title | Judgment | Clearing impossible; manual title permanent |
| A7 | Session fork does not copy the title | Judgment (overview §7.2 copies facts, not metadata) | Fork shows the source title |
| A8 | Titles are generated for every Session, including non-interactive ones, unless `sessionTitle.enabled` is false | Judgment | Some Sessions excluded by a configuration attribute |

## 14. Open questions

| Question | Description |
| --- | --- |
| Purpose key vs overview | Overview §5.6 says the model is selected for the "auxiliary" purpose; per overview §3 constraint 10 purposes are consumer-defined keys, so this design uses `session-title`. Maintainers confirm |
| Auxiliary completion port | Overview §7.2 adds purpose-based selection to Provider but no one-shot completion callable by process plugins; `AuxiliaryModel::complete` is proposed here and should be registered as a Provider supplement (compaction's production host has a similar need) |
| Session-method contribution | `session.rename` makes Rpc inject `When<SessionTitles>`; a generic Session-method contribution point would remove the named dependency (same question as Goal) |
| Session list fields | Adding `title`/`titleSource` to `SessionSummary` is an Rpc protocol change not listed in overview §7.2 |
| Overview §13 "Process-level subscription" | Depends on the subscription contract's answer |
