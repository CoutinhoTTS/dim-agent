# Model-facing prompts

Status: Draft ([#429](https://github.com/arcships/dim/issues/429)). Requirements, not a design: this document fixes what the product's model-facing text must achieve, which channel carries each text, which text is protocol and which is policy, who may change what, and how a change is judged. Each owner designs its own part (§9). Scope: every text the model reads from the product — the system prefix, rule and state blocks, tool declarations, the markers the runtime writes, the compaction replacement and the input guide — across a Session, its branches and its plugins. Chinese: [中文](../zh/design/model-prompts.md).

The model works the way the product intends when what it reads is true about the runtime, it can tell the operator's words from the runtime's, and the product can change its behaviour on evidence. This document answers [#429](https://github.com/arcships/dim/issues/429)'s five questions, which [Agent context §3](agent-context.md#3-给模型的提示词) raises, and hands the consequences to the owners of each text.

## 1. Answers

**Policy and contract.** Read a text as the model reads it and edit only the words: if a sentence becomes false about the runtime, or a mechanism can no longer be used as it is written, the text is **contract**; otherwise it is **policy**. Contract is the model-facing protocol — channel and marker names and their attribute values, handles such as `[input <id>]`, refusal codes, tool schemas and result formats, and the sentences that state a runtime guarantee. Protocol text is recorded in facts, in history and in the adopted configuration, so a change to it keeps a history resumed with the older form understandable, or it is a record-format change for the text's owner; and it lands with the behaviour and the flow test that assert it. Policy is guidance about how to work: it is replaceable, and every change to it is evaluated (§8). A text may hold both, such as a declaration with a schema and a description; the class is per sentence (§4).

**Who may change what.** A scope reaches the model only through a channel it owns: the product through its built-in capabilities; the host or the profile through the Session system prompt and the trusted sources it loads; the project through its `AGENTS.md` and its skills; the user through their messages, their `AGENTS.md` and the settings that select values; a plugin through what it registers or writes, on the same channels as the built-ins. No central repository takes wording away from its author: Prompt, PromptRegistry and ToolRegistry carry and never own (principles §1, §5; ADR 002). No scope changes the protocol or another author's text, a tag never grants permission, the guide's frame is removable by none, and System holds only static text (§5).

**The input guide.** The input guide is the text that explains the other channels to the model: each channel's trust class, the channel rule, escaping, the restatement after a summary, and, for each marker, what it is, whose words they are, how to treat it, when it stops applying, and its trust rule (§6). Its frame belongs to the prompt layer, which is Prompt. Each marker's entry is contributed by the capability that writes the marker, through the route every contribution uses — registered from the contributor's own `apply` and withdrawn when it unloads — so the entry is in force exactly when its writer is loaded, contributed plugins included; one fixed text can never cover them. The entries in force are reproducible from facts, and adding one never rewrites an adopted System mid-Session; how Prompt meets both properties is Prompt's design. Consistency is by construction: a writer that is not loaded contributes no entry; a loaded writer's own flow asserts that the request carrying its marker carries its entry; removing or changing a marker changes its entry in the same change. There is no central scan and no central marker list.

**Language.** Model-facing text, protocol and policy alike, is written in English, in one copy: each policy text carries one evaluation, a second language doubles every text and its evaluation, a text that varies by language forks the Session's cached prefix and its evidence, and the protocol names stay English in any translation. "Agent prompt language" is the language the model writes to the operator in, carried as a runtime state source; it is neither a translation of the prompt texts nor a choice between translated copies, and without it the base sentence — that the model answers in the user's language — is the default. Summaries and marker bodies are data: they keep the conversation's language and its identifiers verbatim (§7).

**Judging a change.** A change to a policy text is judged against the current text on the same cases, the same model with the same reasoning, the same budget, and one fixed judge for both arms: each metric is at least `current − δ` with `δ = 0.05`, violations and failures are no higher, the candidate text's own token cost is reported beside the scores, and no absolute pass line is used. The candidate runs through the shipped assembly, put in place at its author's factory configuration and never by patching strings in an evaluation-only path. The summary prompt has `summary-eval --candidate`; a behavioural text is evaluated by scenarios in the shape of [Timeline branches §12.3](timeline-branches.md#123-evaluation-scenarios), and a change to a text with no harness yet brings its own scenario set, run locally and reported in the pull request. No text is frozen for lack of a harness (§8).

## 2. Basis

1. **What the model reads is a projection of facts.** Model context holds only the events consumed after handoff — adopted inputs, tool results and adopted prompts — and the configuration adopted for the branch carries the system prefix and the tool declarations (principles §4 rule 6). Every text therefore reaches the model through one of a small, fixed set of channels (§3), and nothing else is in its context.

2. **A channel has a carrier, and each text has an author.** The carrier delivers and never owns wording; the author registers or writes the text and owns it. Owning a text where its state is owned is the plugin contract's own rule (principles §5, ADR 002 D1): a central text store would take the wording away from the module whose behaviour it describes.

3. **The channels carry different trust.** A rule binds within its scope, state describes a situation, a marker is information and never permission, the operator's words are the operator's, and tool material is material. The model has to tell which one it reads, and the guide is where it is told (§6).

4. **Policy and protocol change on different licences.** Policy changes on evidence (§8); protocol changes with the behaviour it names, and the facts it is recorded in keep the older form readable (§4).

5. **The channels are closed, the authors are open.** A new channel would change the runtime's delivery and is an infrastructure change under principles §4 rule 8; a text is added by becoming its author inside an existing channel, built-in or contributed. Because the authors are open, the guide's entries are contributed by the writers rather than written once, so one fixed text never has to cover a contributed plugin's markers (§6).

## 3. Channels

Every model-visible text fits one channel below. The table names the carrier, the author, the lifetime and the trust class; §4 and §5 build on it.

| Channel | Carrier | Author | Lifetime | Trust |
| --- | --- | --- | --- | --- |
| System prefix | Prompt: registered System sources, adopted when the root branch starts | base: PromptSources; the Session system prompt: the host or the profile; the skills instructions: Skills | Static for the Session | Instructions |
| Rule and state blocks (`<dim-rule>`, `<dim-state>`) | Prompt: Append inputs, keyed, replayed after a compaction | the capability whose source it is; the body of user and project instructions is the user's or the project's own | Keyed: replaced, revoked or expired by key | Rules: binding within their scope. State: the situation |
| Tool declarations | ToolBinding and ToolRegistry: the configuration's tools part | each tool's owner, built in or contributed | The adopted configuration | Instructions for that tool |
| Runtime-written inputs (markers) | the input path: InputQueue, then Loop | the capability that writes them — TimelineTree, Collaboration, BackgroundExec, ShellInput, and every contributing plugin | History entries; a compaction may replace them | Information, never permission |
| Operator input | the input path | the operator | History | The operator's words |
| Tool results | Loop | material from outside; the refusals and handles the runtime writes into a result belong to the tool's owner | History | Material, never binding; the skill tool's result is the one exception |
| Compaction replacement | ContextPolicy | the summary: the strategy; the digest and the pinned inputs with their handles: ContextCompaction | Replaces a range of history | Background; the live conversation wins |
| The input guide | request assembly | the frame: Prompt; each marker's entry: the capability that writes the marker (§6) | Every request | Instructions about the other channels |

**Verification.** The shipped texts fit the table without a new channel: base, the Session system prompt and the skills instructions in the system prefix; the environment and date state, the user and project instructions and the skills list as rule and state blocks; every built-in tool declaration, and the declarations of MCP servers and contributed plugins, in the configuration's tools part; the branch, goal, exec, collaboration and shell-input markers as runtime-written inputs; the operator's messages; tool results, with the refusals, codes and handles the runtime writes into them; the summary, the digest and the pinned inputs in the compaction replacement; and the guide. A text that fits none of them would need a new channel, which is an infrastructure change (principles §4 rule 8); no text does.

## 4. Protocol and policy

**The test.** Read the text as the model reads it and edit only the words.

- If the edited sentence becomes false about the runtime, or a mechanism stops working — a marker the model cannot place, a call with the wrong shape, a code the model cannot act on — the text is **protocol** (contract).
- If the runtime behaves exactly as before and only the model's choice of how to work changes, the text is **policy**.
- If the text is the operator's, the project's or a profile's own words, it is **content**: the product carries it, escaped, and never edits it.

**Protocol** is the model-facing side of a mechanism:

- channel and marker names, attributes and their values (`op`, `cleared`, …);
- handles such as `[input <id>]`;
- refusal codes;
- tool schemas and result formats;
- sentences that state a runtime guarantee.

Three requirements follow. It changes only with the behaviour it describes, in the same change as that behaviour's flow test. Because it is recorded in facts — in history and in the adopted configuration — a change keeps a history resumed with the older form understandable; where it cannot, it is a record-format change for the text's owner. And an owner that changes it states the compatibility it keeps.

**Policy** is guidance about how to work: the base prompt's rules, the shell guidance, the usage guidance in tool declarations, the coordination text of the branch and collaboration tools, the Goal wording, the summary prompt's wording. It is replaceable, and every change to it is evaluated (§8).

**Mixed texts.** A text may hold both. A tool declaration is the usual case: its schema and result format are protocol, its description's guidance is policy. The class is per sentence, and the owner marks the split on its own design page (§9.2).

## 5. Scopes

| Scope | What it reaches the model through | Limits |
| --- | --- | --- |
| Product | the texts of its built-in capabilities | it changes no protocol without the behaviour, and no other author's text |
| Host or profile | the Session system prompt, and the trusted sources loaded through the host's prompt loading | the base and the skills instructions are not its to remove; the guide is appended after it |
| Project | its `AGENTS.md`, carried as project-scope rules, and its skills | it changes no protocol, and a project rule grants no permission |
| User | their own messages; `~/AGENTS.md` as user-scope rules; the settings that select values — the model, the execution shell, the answer language, the summary's reasoning setting ([ContextCompaction §13.2](plugins/ContextCompaction.md#132-summaryreasoning)) | it changes no protocol, and a user rule grants no permission |
| Plugin, built-in or contributed | the texts it registers or writes: its sources, its tool declarations, its markers, and their guide entries | the same limits, and no entry that is not its own marker's |

Three limits hold for every scope:

1. **No scope changes the protocol or another author's text.** The protocol changes with its behaviour (§4), and a text belongs to its author (§2.2).
2. **The guide's frame is not removable.** The frame — channels, trust, escaping, the channel rule, the restatement — is Prompt's, and no operator, project, plugin or profile removes or replaces it (§6).
3. **System holds only static text.** Text that changes during a Session arrives as a rule or state input, so the cached prefix stays one for the Session (§7).

A user's or a project's words shape how the model works within their scope; they never change what the runtime does and never grant permission. Content inside a marker is material, and a tag describes content — it does not authorize it (§6).

## 6. The input guide

The input guide is the text the model reads about its own input, on every request.

### 6.1 What it covers

- **Each channel's trust class**: instructions, rules binding within their scope, state describing a situation, information, material, and the operator's words.
- **The channel rule**: the operator's words appear only in the operator's own messages, and everything the runtime adds arrives separately, marked and escaped. The one exception is the `[input <id>]` handle a compaction appends to an input it shortened, which the recall tool opens for the full text. The guide's sentence that everything the runtime adds arrives in separate messages ([input-guide.txt:5](../../crates/dim-agent-capabilities/src/input-guide.txt)) contradicts that handle; the rule the requirement states is the corrected one.
- **Escaping**: content from outside the runtime cannot forge a marker, and unescaped tags inside a marker are the runtime's own.
- **The restatement after a summary**: the state and rules that follow a summary are what is still in force, not new changes.
- **For each marker**: what it is, whose words it carries, how to treat it, when it stops applying, and its trust rule.

### 6.2 Who owns which part

- **The frame** — the channels, the trust classes, escaping, the channel rule and the restatement — belongs to the prompt layer, which is Prompt.
- **Each marker's entry** is contributed by the capability that writes the marker. It takes the route every contribution takes: registered from the contributor's own `apply`, withdrawn when that contributor unloads, so the entry is in force exactly when its writer is loaded ([PromptRegistry](../architecture/rust-agent/plugins/PromptSources.md#promptregistry); rutis usage R6). A contributed plugin contributes its entries the same way, which one fixed text can never cover.
- **Prompt provides the means.** It accepts the entries, composes them with the frame, and keeps two properties of the entries in force: they are reproducible from facts, as the other sources are, and adding or removing one never rewrites an adopted System mid-Session. The mechanism is Prompt's design (§9.1).

### 6.3 Consistency by construction

- A writer that is not loaded contributes no entry, so no entry outlives its writer.
- A loaded writer's own flow asserts that a request carrying its marker also carries its entry.
- Removing or changing a marker changes its entry in the same change.
- There is no central scan and no central marker list.

## 7. Language

**Model-facing text is English, in one copy, protocol and policy alike.**

- Each policy text carries one evaluation (§8); a second language doubles every text and its evaluation, and the two languages drift apart.
- The system prefix is the Session's cached prefix; a text that varies by language forks the prefix and its evidence with it.
- Protocol names, attribute values and codes are written in one spelling by the runtime, and a translation carries the English names anyway.
- Instruction-following is not equal across languages, and the product does not claim an evaluation it has not run.

**"Agent prompt language"** is the language the model writes to the operator in. It settles the case where the operator's environment language and the language of their messages differ — English logs or code pasted into a Chinese session, or a first message that is material rather than a request — so the answer language is an explicit choice instead of an inference from the last message. The requirements:

- The value reaches the model as one state source, a `<dim-state>` entry naming the language the model writes to the operator in.
- Without it, the base sentence's rule stands: the model answers in the user's language.
- It is the operator's setting; a project states its language in its own rules, which are content.

Rejected: translating the prompt texts per locale. It doubles the policy texts and their evaluations, forks the cached prefix, and leaves the protocol names in English anyway.

**Data keeps its language.** A summary, a goal objective, a branch's purpose and a tool result stay in the language they arrived in, and every identifier — a path, a symbol, a command, an error — is kept verbatim in every language ([Compaction requirements §3](compaction-quality.md#3-what-compaction-keeps)).

## 8. Judging a change

### 8.1 The rule

**The comparison.** A change to a policy text is judged against the current text, on the same cases, the same model with the same reasoning settings, and the same budget — the same window, tools, turn limit and starting workspace — with **one fixed judge** for both arms. A candidate passes when each metric is at least `current − δ` with `δ = 0.05`, and its violations and failed runs are no higher than the current arm's. The candidate text's own token cost, which every request carries, is reported beside the scores. No absolute pass line is used: an absolute line — a task-success rate, or a judged score on a nondeterministic model — varies by scenario, model and run, and needs many runs to separate a difference from noise, while a relative comparison at the same budget holds across scenarios ([Compaction requirements §6](compaction-quality.md#6-how-we-judge-that-it-works)). The judge is named with the result, and a judge from the model's own family is reported as a limitation of the measurement.

**The candidate runs through the shipped assembly.** It is put in place at its author's factory configuration, as the summary strategy's optional system prompt member specified in [ContextCompaction §13.4](plugins/ContextCompaction.md#134-evaluation) allows a candidate prompt to replace the fixed one — the product profile never sets it — and never by patching strings in an evaluation-only path, so both arms run the shipped request building, chunking and failure handling.

**Where it runs.** Real-model runs are local and reported in the pull request. CI holds only deterministic protocol checks.

### 8.2 Harnesses

- **The summary prompt and its length sentence** have `summary-eval --candidate`, which runs the current and the candidate prompt on the same cases, model and target rule ([ContextCompaction §13.4](plugins/ContextCompaction.md#134-evaluation)).
- **A behavioural text** is evaluated by scenarios in the shape of [Timeline branches §12.3](timeline-branches.md#123-evaluation-scenarios): scripted input, the real tools, probes for what a run must show and must not show, both arms judged by the fixed judge. The requirement to the evaluation tooling is a scenario runner with the same candidate injection and the same relative rule (§9.7).
- **Until a text has a harness, a change to it brings its own scenario set**, run locally current against candidate and reported in the pull request. No text is frozen for lack of a harness.
- **Protocol text is not evaluated.** Its flow test changes with it (§4).

## 9. Requirements raised to owners

The format follows [Timeline branches §17](timeline-branches.md#17-requirements-raised-to-mechanism-owners). Each owner changes its own document.

### 9.1 Prompt, PromptRegistry and PromptSources

**Owners:** Prompt, PromptRegistry and PromptSources.

- Prompt owns the guide's frame (§6.2) and reviews it; no scope removes or replaces it.
- The frame states the channel rule with the `[input <id>]` exception (§6.1).
- Prompt provides the means by which a capability contributes its markers' entries: registered from the contributor's own `apply` and withdrawn on unload; the entries in force reproducible from facts; adding or removing one never rewrites an adopted System mid-Session (§6.2). The mechanism is Prompt's design.
- PromptSources keeps `dim/base` and the shell guidance as policy, replaceable at its factory configuration so a candidate can run (§8.1); the environment and date sources state facts only; an instruction body is the user's or the project's words, carried escaped and never edited.
- PromptSources provides the "Agent prompt language" state source (§7).
- Prompt and PromptRegistry carry text and own none of it (§2.2).

### 9.2 Every capability that writes model-facing text

**Owners:** every capability that writes a text — built-in or contributed.

- It lists its model-facing texts on its own design page, each marked protocol or policy, sentence by sentence where mixed. This document does not restate them, as [Timeline branches §12.1](timeline-branches.md#121-context-mapping) does for its own.
- It contributes the guide entry for each marker it writes, and its own flow asserts that the request carrying the marker carries the entry (§6.3).
- It exposes its policy texts at its factory configuration, so a candidate can run through the shipped assembly (§8.1).
- It ships a protocol change with the behaviour, the flow test and the compatibility of histories resumed under the older form (§4).

### 9.3 ToolRegistry and ToolBinding

**Owners:** ToolRegistry and ToolBinding.

- They carry the declarations and own no wording.
- In a declaration, the description's guidance is policy; a schema, a result format and a refusal code are protocol (§4).
- A declaration's policy change follows §8; its protocol change follows §4.

### 9.4 TimelineTree

**Owner:** TimelineTree.

- **Protocol.** The marker names and attributes, the identity blocks' factual sentences, the branch tool's actions and modes, and its refusal codes change only with the branch mechanism, in the same change as the flow tests that assert them (§4).
- **The coordination text for `main` covers**: how to brief a line — its purpose, what `main` knows that the line needs, which files or area are the line's own, and when it should report or end; assigning those areas; answering a line's steers; ending its own wait when what it waited for is settled; and not redoing a line's work ([Timeline branches §12.2](timeline-branches.md#122-items-for-the-prompt-layer-429)).
- **Evaluation.** These texts are policy: a change is evaluated with the scenarios of [Timeline branches §12.3](timeline-branches.md#123-evaluation-scenarios) — redone work, over-splitting, flooding, never ending, sibling relay, ping-pong, identity after compaction, asking the user — current against candidate (§8).

### 9.5 Goal

**Owner:** Goal.

- **The wording.** Goal's own design page describes work on another line as delegated work: the objective block's and the continuation block's "delegated sub-task", and "delegated work" in §3.4 and in §5's row for child branches ([Goal §A.5](plugins/Goal.md#a5-model-visible-texts), [§3.4](plugins/Goal.md#34-a-new-branch-clears-the-inherited-goal)). A line is not delegated work: it is the same agent on another line of the Session ([Timeline branches §1](timeline-branches.md#1-what-branches-are-for)). The requirement: each of those texts says "on another line of this Session", and the meaning stays — the goal belongs to another line, and this line neither continues it nor calls `goal_update` for it.
- **The classes.** The marker name, its `kind` values and its keys are protocol; the sentences above are policy.
- **The guide.** Goal contributes the entry for its marker when it writes it (§6.2, §9.2).
- **Evaluation.** A goal scenario set — a goal set on `main` continues; a line does not continue the goal it inherited; a completed or blocked goal stops — current against candidate (§8).

### 9.6 ContextCompaction

**Owner:** ContextCompaction.

- **The declared parts.** The summary prompt declares the parts of a substantive summary in its own words — the task or goal in force first; constraints and preferences still in force; standing decisions and their reasons; approaches ruled out and why; open items with the user; the current judgment; the next step; the exact identifiers in use ([Compaction requirements §3](compaction-quality.md#3-what-compaction-keeps)) — and the substantive-summary check verifies that those parts are present ([ContextCompaction §5.2](plugins/ContextCompaction.md#52-productsummarystrategy)).
- **Language.** The prompt is English, and the summary it asks for keeps the conversation's language, identifiers verbatim (§7).
- **Policy.** The prompt and its length sentence are policy: a change is evaluated with `summary-eval --candidate` (§8.2).
- **The guide.** Each marker it writes has its entry: the summary envelope, the `[input <id>]` handle, and the digest message (§6.2, §9.2).

### 9.7 The evaluation tooling

**Owner:** the evaluation tooling, `xtask`.

- **A scenario runner** with the candidate injection of §8.1 and the relative rule of §8.1: the scenarios are the cases, both arms run the shipped assembly, and the result is reported in the pull request.
- **No absolute pass line and no CI run** (§8.1).

## 10. Rule check

Rules: [design rules](../architecture/rust-agent/principles.md) §1, §3, §4 rules 4, 6, 7 and 8, and §5; [ADR 002](../adr/002-plugin-scope.md); [ADR 008](../adr/008-documentation-ownership.md).

| Rule | How this document complies |
| --- | --- |
| §1 Plugins and capabilities bounded | The guide's frame is the prompt layer's, and each capability contributes its own entries. No part gains another's state |
| §3 Orthogonality | The text travels on existing contracts — sources, inputs and declarations. Where the guide needs a contribution route, it is raised to Prompt as a generic capability: it carries no text's business meaning and any capability can use it (step 3 of §3's order) |
| §4 rule 4 No business meaning in core | The delivery classes stay generic; the guide's content is the prompt layer's text and the entries are the writers' texts. Timeline and Dispatcher gain nothing |
| §4 rule 6 Context from consumed events | Every channel is a consumed event or the adopted configuration (§3); the document adds none |
| §4 rule 7 Public events and services | No new event or service: entries and texts travel on existing sources, inputs and declarations |
| §4 rule 8 Infrastructure unchanged | None is changed: the contribution route is a Prompt-level capability inside its own scope, not a port, checkpoint or adapter on infrastructure |
| §5 Ownership | Each text belongs to its author, and each requirement stays inside one owner (§5) |
| ADR 002 D1 Scope | Each text lives where its state lives; the guide's entries follow their writers' lifetimes |
| D2 No wiring modules | None is added |
| D3 Scope dependencies | No dependency is added; entries follow the existing registration direction |
| D4 Configuration | No configuration is added; the candidate injection uses an author's existing factory configuration |
| D5 Language-neutral contracts | No wire contract is added |
| ADR 008 D2 No delivery status | The document states requirements only |
| ADR 008 D3 Own scope only | Per-text inventories stay on each owner's design page (§9.2); this document names no per-string list |

Being requirements, the document adds no part, service, event, method or configuration.

## 11. Later options

- **Per-model variants of a policy text**: only with evidence, as a policy change under §8.
- **A harness for every policy text**: the scenario runner covers more texts over time; no text waits for it (§8.2).
- **A per-project answer language**: the answer language is the operator's; a project states its own in its rules, which are content.
- **A translated prompt set**: rejected in §7; it would need its own requirement and evaluation for every text.
