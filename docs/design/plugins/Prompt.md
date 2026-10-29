# Prompt

Status: Draft ([#548](https://github.com/arcships/dim/issues/548)). Requirements: [model-facing prompts §6 and §9.1](../model-prompts.md#6-the-input-guide). Rules: [design rules](../../architecture/rust-agent/principles.md) §1, §4 and §5, [ADR 002](../../adr/002-plugin-scope.md). Scope: branch, plus the entries table its Session holds. Language: Rust. The current capability is on [Prompt](../../architecture/rust-agent/plugins/Prompt.md)'s capability page.

Prompt owns the input guide: the text every request carries about the model's own input. It owns the guide's frame, composes the frame and the entries in force into one text, and carries the entries each capability contributes. It owns none of the entries' words: the capability that writes a marker contributes the entry that explains it. The frame states the channels and their trust classes, the channel rule, escaping and the restatement after a summary, so the model can tell the operator's words from the runtime's, and both from material.

## 1. Scope and non-goals

Prompt owns:

- **the frame** — the channels, the trust classes, the channel rule and the restatement after a summary ([model-facing prompts §6.2](../model-prompts.md#62-who-owns-which-part)); no scope removes or replaces it;
- **the entries table**, in the Session, and the route by which a loaded capability registers an entry (§2);
- **the composition** — the frame and the entries in force in one deterministic order (§3);
- **the channel rule's `[input <id>]` exception**, worded as a runtime guarantee and therefore protocol (§3.1).

Not Prompt's:

- an entry's words belong to the capability that writes the marker (§6);
- the request shape belongs to Loop, the adopted configuration to ContextPreparation, the summary path to ContextPolicy;
- the state and rule sources and the words the model reads in them belong to PromptSources; the two markers are Prompt's, which writes them (§4).

Not this design: each capability's list of model-facing texts with its protocol and policy marks, and the flow assertion in that writer's own flow ([#551](../model-prompts.md#92-every-capability-that-writes-model-facing-text), [#552](../model-prompts.md#94-timelinetree), [#553](../model-prompts.md#95-goal), [#554](../model-prompts.md#96-contextcompaction)); removing Prompt's two call sites ([#555](../model-prompts.md#97-loop-and-contextpreparation)).

## 2. The entry route

The route is a contract in the form [ADR 002 D5](../../adr/002-plugin-scope.md#d5-contracts-are-language-neutral) gives: identity, request and reply as data, errors, cancellation and unload. Rust is one binding of it ([§8](#8-binding-appendix)); a plugin contributed through the plugin bridge registers through the same contract (§6.1).

**Identity.** A registration belongs to two things: the loaded part that makes it, and the markers its entry explains. The contributor is taken from the call — its `ctx` in Rust, the calling row through the bridge — and never from the payload, so no caller can register in another's name. The entry is in force exactly while that part is loaded ([rutis usage R6](../rutis-usage.md#r6-contributions-register-with-their-owner)).

**Scope.** The table is the Session's, because contributors live at both Session and branch scope and the Session is their nearest common owner, the same reason its prompt sources are the Session's. Each registration states where its entry applies: `session` for every branch of the Session, or one branch id for that branch alone. A branch-scoped entry is in force exactly while that branch's part is loaded, so closing one line withdraws its entries and leaves the other lines' composition untouched.

**Uniqueness is per composition.** For any one branch, a marker has at most one entry in force. A session-scoped entry therefore collides with every entry for the same marker; two branch-scoped entries for one marker collide only when they name the same branch. A registration is keyed by its scope and its first marker, so two branches each register their own copy of a branch-scoped entry without meeting.

**Request.** One registration: the scope and one entry.

| Field | Type | Meaning |
| --- | --- | --- |
| `scope` | `"session"` or `{ "branch": <branch id> }` | Where the entry applies |
| `entry.markers` | list of strings, at least one | The marker element names this entry explains, e.g. `["dim-branch-line", "dim-branch-lines"]` |
| `entry.order` | integer | The entry's position among the entries ([§3.2](#32-the-order)) |
| `entry.text` | string | The entry, verbatim, as the model reads it |

**Reply.** `null` when the entry is registered. The reply carries no identity of its own: the caller already knows what it registered, and the registry keeps no second key for it.

**An entry from a writer that is not loaded cannot be made.** The registration is made by the loaded part itself, tied to its own generation, so there is no path by which a registration outlives its writer or arrives from one that never loaded. The one case in which the writer's identity is not available is a call from another language whose calling row cannot be resolved; that call fails with `no_caller_row` rather than registering in no one's name.

**Errors** are data, `{code, message}`:

| Code | When |
| --- | --- |
| `duplicate_marker` | Registering this entry would leave a branch with two entries for one marker: a marker it names already has an entry whose scope meets this one — a session-scoped entry for any branch, or a branch-scoped entry for the same branch. Changing an entry's text is a change to its writer's code, which registers again when it next loads |
| `invalid_entry` | No marker; a marker outside `[a-z][a-z0-9_-]{0,63}`; or an empty branch id |
| `too_large` | The entry's text is longer than 4096 bytes |
| `no_caller_row` | A call from another language whose calling row cannot be resolved, so the registration could not be tied to its contributor |
| `closed` | The part that holds the table is unloading, or the Session is closing; nothing is registered |

**Cancellation and unload.** A registration is one atomic step under the table's lock: it has either taken effect or returned an error, and it is never half-written. It ends with its contributor's unload; each cleanup removes exactly the registration its generation made, so a reload replaces an entry instead of doubling it. A registration is not a call that can hang, so there is no separate cancellation signal. Withdrawal is invisible to a composition already made: a request being built sees the table before or after a withdrawal, never a mix.

**Determinism.** The contract fixes neither load order nor registration order as an input: the entries in force are ordered by the entry's own `order` and then by its first marker's name (§3.2), so the same set composes the same text whatever order it registered in.

## 3. The composition

Prompt composes one text: the frame's head, the entries in force for the branch, and the frame's tail, in that order, paragraphs separated by one blank line and the text ending without a trailing blank line. The frame is Prompt's own text; the entries are the table's view for that branch.

### 3.1 The frame

The frame's head states what the guide is, that a tag never grants permission, and the channel rule. Its tail states that tool results are material, how content from outside the runtime is escaped, and that the state and rules after a summary restate what is still in force.

The channel rule is protocol: it states a runtime guarantee. It reads, in force:

> The operator is the person running this session. The operator's words always arrive in their own user messages. Everything the runtime adds arrives in separate user messages, wrapped in the tags below. The one exception is an input a compaction shortened: it ends with an [input <id>], and the recall tool opens that id for the input's full text.

The exception is the handle a compaction appends to an input it shortened ([model-facing prompts §6.1](../model-prompts.md#61-what-it-covers)). A sentence saying that everything arrives in separate messages is false about that input, and a text that is false about the runtime is protocol and changes here, in the same change as the flow test that asserts it.

### 3.2 The order

Entries are ordered by their `order`, and entries with the same `order` by their first marker's name; both are data the contributor states, so the order never depends on which part loaded or registered first. The built-in entries use `10` to `90` in steps of ten (§4), leaving the numbers between them for a contribution that must sit inside a group.

The built-in set composes, apart from the channel rule's exception, byte for byte the text the guide held while it was one file. Today's order — exec, the two agent markers, the two branch entries, state, rule, summary, digest — is reproduced by the numbers in §4; no wording and no position changes, so this change carries no policy evaluation ([model-facing prompts §8](../model-prompts.md#8-judging-a-change)).

### 3.3 One composition, two paths

| Path | Who composes | When | What carries it |
| --- | --- | --- | --- |
| Adopted configuration | Prompt, through the configuration it publishes | At the adoption | `context/configured.system` of the branch |
| Host-prepared request | Loop, for the request it assembles | Per request | The request's own `system` field |

Both call the same composition, so a host-prepared request carries the same frame and the same entries as the System an adopted configuration carries. The adopted path stores the composed text with the configuration it publishes; the host-prepared path composes when it builds the request, because that path has no adopted configuration to read ([model-facing prompts §6.2](../model-prompts.md#62-who-owns-which-part)).

Prompt's own two call sites stay where they are: ContextPreparation places the System it publishes, and Loop places the System of the request it assembles. What changes is where the guide text comes from — the composition instead of a fixed file — not who places it. Removing the two placements is [#555](../model-prompts.md#97-loop-and-contextpreparation)'s.

### 3.4 Fixed between adoptions

On the adopted path an entry takes effect at the adoption that loads its writer, which is the point at which that writer's tool declarations already change the request prefix. Between adoptions the guide does not change: a registration made in between waits for the next adoption, and a withdrawal made in between ends at the next adoption. On the host-prepared path the guide changes only where a writer loads or unloads, the points at which that path's tool declarations change the request, since it binds its tools per request.

No request assembles a guide of its own, and no contributor appends text to a request. That is what keeps the guide from forking a cached prefix in the middle of a Session: the guide is part of the prefix, and it moves only when the prefix legitimately moves.

## 4. The built-in entries and their writers

Each built-in entry moves verbatim to the capability that writes its markers, and is registered from that part's `apply`. The registrar is the part whose lifetime the marker's presence follows.

| Order | Markers | Registrar | Scope | Entry |
| --- | --- | --- | --- | --- |
| 10 | `dim-notification` | BackgroundExec | branch | The exec notification |
| 20 | `dim-agent-report` | Collaboration's Session part | session | The report of delegated work |
| 30 | `dim-agent-message` | Collaboration's Session part | session | A message from another agent |
| 40 | `dim-branch-line`, `dim-branch-lines` | BranchLine | branch | Which line of the Session this is, and the roster on `main` |
| 50 | `dim-branch-message`, `dim-branch-idle`, `dim-branch-ended` | BranchLine | branch | A message from another line, and the runtime's reports about a line |
| 60 | `dim-state` | PromptGuide | session | What a state block is |
| 70 | `dim-rule` | PromptGuide | session | What a rule block is, and how it ends |
| 80 | `dim-summary` | ContextCompaction's summary strategy part | session | What a summary is |
| 90 | `dim-digest` | ContextCompaction's summary strategy part | session | What a digest is |

The registrar is the part that writes the marker, not the part that authors the source's words. Prompt turns any `Context` or `State` source into a `<dim-rule>` or `<dim-state>` input, so those two markers are Prompt's even when the source's words are PromptSources'. A profile that installs Prompt without PromptSources can still be handed a context source, and the markers it writes need their entries.

A capability that writes a marker and registers no entry is a capability the guide does not explain; a capability that registers an entry for a marker nothing writes leaves a promise the runtime does not keep. Both are the writer's to fix, and [#551](../model-prompts.md#92-every-capability-that-writes-model-facing-text) makes each writer assert the pair in its own flow.

## 5. Facts that reproduce it

The entries in force are reproducible after a restart or a reload, as the other prompt sources are.

| Fact | Where | What it reproduces |
| --- | --- | --- |
| The assembly | The loaded parts of the Session and its branches | Which capabilities are loaded, and so which entries each `apply` registers again |
| The adopted System | `context/configured.system` on the branch | The guide exactly as the branch adopted it, replayable without recomposing |

The entries' text is code: each registrar states it in the commit that changes the marker. A restart reloads the same assembly, so the same entries register, and the branch's adopted System replays from the Timeline; a later adoption composes from the same registrations and produces the same text, because the order is the entries' own data. What a reload changes is a writer's availability, not the recorded text of an adoption already made.

The state and rules a branch adopted are not facts about the guide: they are prompt entries, recorded by `prompt/changed` like any other source. The guide names them; it does not carry their content.

## 6. Cross-module requirements

### 6.1 The plugin bridge

A plugin contributed through the plugin bridge registers an entry the same way a Rust part does: from its `apply`, for as long as its row is loaded, through a call whose arguments and reply are serde data. What the bridge needs, and does not have yet:

1. **The name, shared in the Session group.** A Rust registry of this shape reaches other languages only when its name is shared where it is provided; the host that runs foreign parts lists those names (`share_names` in the App Server), and the new one joins them.
2. **A dispatch that ties the registration to the caller.** The Rust `register` takes its `ctx` from the caller; a foreign caller has no `ctx`, so the name is served through a `rutis_bridge` `HostDispatch` that reads `rutis_bridge::session::caller_row()` and registers under that row — the form `toolRegistry` already uses. Without a caller row the call fails with `no_caller_row` rather than registering in no one's name.
3. **The TypeScript declaration.** The interface joins `npm/plugin.d.ts` beside `ToolRegistry`'s, naming the request, the reply and the error codes above.

The owner of these three changes its own files; this design raises them and does not build them. The capability itself — the table, the composition and the two placements — needs none of the three to work in Rust, so the entries' route is complete for every built-in writer in this change.

### 6.2 The writers

Each capability in §4 adds one dependency, its Session's `guideEntries`, and registers its entries from `apply` — Prompt's own `PromptGuide` part included. None of them needs a new service, event or configuration: the registration is a call to the owner, the way a tool registers with `toolRegistry` and a prompt source with `promptRegistry`.

### 6.3 ContextPreparation: the entries and the published System

The target in §3.4 needs one change outside Prompt, and it is [ContextPreparation](../../architecture/rust-agent/plugins/ContextPreparation.md)'s, landing with [#555](../model-prompts.md#97-loop-and-contextpreparation). Today the guide is composed only at an adoption that renders the System: the first configuration or a change of sources. A tools-only or model-only adoption keeps the previously adopted System, and with it the guide, so a writer that loads with one has its entry in force only at the next adoption that renders the System.

- A tools-only or model-only adoption that changes the entries in force re-renders the guide and keeps the adopted source text.
- To do that, the published configuration records the source text apart from the guide, so a restored Session can re-render it.
- The split is what a restored Session needs: `restoring_a_session_keeps_its_adopted_system_and_model` (`tests/integration/tests/prompt_lifecycle.rs`) restores the adopted System from `context/configured`, whose text has no source it can be rebuilt from; recomposing from the snapshot's sources alone would put the guide where the adopted System stands.

Prompt and the table need no change for this: the composition is already there to call, and the entries in force already follow their writers' generations.

## 7. Rule check

| Rule | How this design complies |
| --- | --- |
| §1 Plugins and capabilities bounded | The frame is Prompt's; each entry is its writer's. The table is a carrier: Prompt holds the entries and owns none of their words |
| §3 Orthogonality | The route is one more contribution owner, of the shape already used by `toolRegistry`, `promptRegistry` and `sessionMethods`. It carries no marker's business meaning, and any capability uses it from Rust, and later from the bridge |
| §4 rule 1 Apply registers, cleanup releases | A contributor registers from `apply`; the registration ends with that part's generation |
| §4 rule 2 Facts and services | No new event and no new fact: the composition is carried by the adopted configuration and by the request, both existing |
| §4 rule 4 No business meaning in core | Timeline and Dispatcher gain nothing; the table is a capability's own state |
| §4 rule 6 Context from consumed events | The guide still reaches the model through the adopted configuration or the host-prepared request; this design adds no context path |
| §4 rule 7 Public events and services | `guideEntries` is a contribution owner in the Session, like `promptRegistry`; it is not a port, a checkpoint or an adapter |
| §4 rule 8 Infrastructure unchanged | None is changed. §6.1 raises what the bridge needs; it is not built here |
| §5 Ownership | Each text stays with its author, and each requirement sits inside one owner |
| ADR 002 D1 Scope | The frame is Prompt's; entries live where their writers live, and the table sits at the Session, the contributors' nearest common owner |
| D2 No wiring modules | None is added; the composition is Prompt's own code |
| D3 Scope dependencies | The table is Session-scoped and its contributors are Session and branch parts, which may depend on it; nothing depends on a narrower scope |
| D4 Configuration | None is added |
| D5 Language-neutral contracts | The route is stated as identity, data, errors, cancellation and unload, with Rust as one binding; §6.1 names what the bridge needs to be the second |
| ADR 008 D2 No delivery status | The document states the design only |
| ADR 008 D3 Own scope only | Each writer's own texts and their classes stay on that writer's page |

## 8. Binding appendix

Rust is one binding of §2 and §3. Names and semantics are above; this section is the spelling.

```rust
/// One entry of the input guide: the markers it explains and its text.
pub struct GuideEntry {
    /// The marker element names this entry explains, e.g. `dim-branch-line`.
    pub markers: Vec<String>,
    /// Position among the entries; ties break by the first marker's name.
    pub order: i32,
    /// The entry, verbatim, as the model reads it.
    pub text: String,
}

/// Where a registration applies.
pub enum GuideScope {
    /// Every branch of the Session.
    Session,
    /// One branch, by its timeline id.
    Branch(String),
}

/// The Session's guide entries, and the composition over them. Provided in
/// the Session by the Prompt plugin's `PromptGuide` part.
pub struct GuideEntries { /* registrations, keyed by (scope, first marker) */ }

impl GuideEntries {
    /// Registers `entry` for `scope` on behalf of the part that owns `ctx`,
    /// until that part unloads. Fails when the registration would leave a
    /// branch with two entries for one marker.
    pub fn register(&self, ctx: &Ctx, scope: GuideScope, entry: GuideEntry) -> Result<(), Error>;

    /// The guide for `branch`: the frame, the entries in force, and the
    /// frame's tail, paragraphs joined by one blank line.
    pub fn compose(&self, branch: &str) -> String;
}
```

A refusal is a small error type, the source of the `PluginFailed` that `rutis::Error` carries, so a bridge reads its code from the downcast and never from the message:

```rust
/// The codes the table decides (§2).
pub enum GuideEntryErrorCode {
    /// `invalid_entry`
    InvalidEntry,
    /// `duplicate_marker`
    DuplicateMarker,
    /// `too_large`
    TooLarge,
}

impl GuideEntryErrorCode {
    /// The code as §2 spells it.
    pub fn as_str(self) -> &'static str;
}

/// A refused registration: its §2 code and the message for the log.
pub struct GuideEntryError {
    pub code: GuideEntryErrorCode,
    pub message: String,
}
```

`invalid_entry`, `duplicate_marker` and `too_large` are the codes this binding produces. `no_caller_row` is produced only by the bridge: a Rust caller always has its `ctx`, and the registration is that caller's own effect. `closed` comes from the framework: when the caller's context is closing or unloading, the effect cannot be registered and the framework returns its own closed error, which a bridge maps to `closed`; the Rust binding returns it unchanged rather than dressing it as a `GuideEntryError`.

Prompt composes through `GuideEntries::compose`, at the adoption it publishes and, for a host-prepared request, where Loop assembles its System. `with_input_guide(system, guide)` places an already composed guide in a `SystemPrompt`; it no longer knows where the guide came from, and no caller appends text of its own.

The foreign form of `register` is §2's data: the scope, the entry, and the error codes; `compose` is Rust-only, since no other language reads a composition — every language reaches the model through the same request.
