# 008: Documentation ownership and immutable ADRs

Decision status: Proposed.

Supersedes the part of [ADR 006](006-plugin-contract.md) D9 that puts delivery status and
history in ADR delivery tables and design documents. It does not change ADR 006's plugin
contract.

## Context

Documentation changes have a large blast radius because documents do not own stable facts:

- Accepted ADRs are edited for implementation status, current facts and migration steps.
- Feature designs restate other modules' services, registration, routing and error behavior.
- A cross-cutting design edits other owners' documents directly.
- Status and phase are repeated in ADRs, `INDEX.md`, `roadmap.md` and plugin indexes.
- Contract names, language spellings and another module's internals share one design body.

Migrations and translations amplify this, but they are not the cause. Each document was allowed
to contain facts owned by another document.

## Decision

### D1. An ADR is a snapshot

- An accepted ADR is immutable. Do not modify or delete it. A proposed ADR may change until it
  is accepted.
- A direction change is a new ADR that supersedes the old one. An accepted ADR is not rewritten;
  the new ADR states the supersession and `docs/adr/README.md` records the relation.
- An accepted ADR may be corrected for a broken link or typo under the `adr-erratum` label. The
  correction may not change the decision.
- An ADR records the context, the decision and its consequences. It does not record
  implementation status, migration progress or delivery history.
- Existing ADRs predate this rule. They stay as they are and are not retroactively cleaned.

### D2. The directory is the document type

No manifest or per-document metadata is added. Existing paths define the type:

- `docs/adr/` — ADR
- `docs/design/contracts/` — contract
- `docs/design/plugins/<X>.md` — design of X
- `docs/architecture/` — current implementation
- `docs/zh/` — translation

### D3. A design owns only its own scope

- A feature or plugin design states its own inputs, outputs, events, state, errors and declared
  dependencies. It links to another module's contract instead of restating or changing it.
- It may name the public services, events, data shapes and errors it uses; it does not describe
  how another module registers, retries, routes or cleans them up.
- Cross-module requirements are stated in the design that raises them. The owner changes its
  own document in its own change.
- Cross-module flows live in a higher-level flow document.
- A change that modifies more than one `docs/design/plugins/*.md` needs the `cross-module`
  label and review from the affected owners.

### D4. Status lives in issues

- Documents do not add new completion status. ADRs, `INDEX.md`, `roadmap.md` and plugin indexes
  link to the issue.
- Existing status text written before this decision is historical and is not updated.
- A document may record whether it is a draft or a proposal; it does not record what has been
  delivered.

### D5. Technical detail has a layer

- Public contract terms and invariants belong in the contract or design body.
- Language spelling belongs in a binding appendix.
- Current implementation facts belong in architecture documents, generated where possible.
- Another module's internals are linked, not described.

### D6. The rules that can be checked are checked

`scripts/docs-check.mjs` checks the PR diff:

- an existing accepted `docs/adr/*.md` is modified or deleted, unless the `adr-erratum` label is
  present;
- more than one `docs/design/plugins/*.md` changes without the `cross-module` label;
- a new ADR lacks `Decision status`, `## Context`, `## Decision` or `## Consequences`, or has an
  `## Implementation`/`## Delivery` section or an `Implementation:` line.

The check is structural. It does not scan prose for status words, and it does not cover
`docs/zh/adr/` or `docs/zh/design/plugins/`: a translation may be updated to match its English
source and may not add or change a decision.

CI runs the check for pull requests. A label change does not start a new run, so add the label
and push again. Checks enforce boundaries and structure; review decides whether a decision is
right.

### D7. Templates and AGENTS.md have different jobs

- Templates live under `docs/templates/`; examples live under `docs/examples/`.
- `AGENTS.md` states constraints and routing: what not to do, which template to read and which
  check to run. It does not contain template bodies.

## Consequences

- Existing ADRs remain valid history, including status text written before this decision.
- A design that needs another module changes that owner's document separately.
- New status-only edits to indexes and accepted ADRs stop.
- Agents get a local, executable failure instead of a prose reminder.
- A legitimate cross-module change is still possible, but it is explicit and reviewed.

## Non-goals

- Removing technical detail from designs.
- Freezing design documents. They remain living documents within their own scope.
- Rewriting legacy status text in existing documents.
- Automating whether a decision is correct.
