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

Migrations and translations amplify this, but they are not the cause. Each document was allowed
to contain facts owned by another document.

## Decision

### D1. An accepted ADR is a snapshot

- An accepted ADR is immutable. Do not modify or delete it. A proposed ADR may change until it
  is accepted.
- A direction change is a new ADR that supersedes the old one. An accepted ADR is not rewritten;
  the new ADR states the supersession and `docs/adr/README.md` records the relation.
- An accepted ADR may be corrected for a broken link or typo under the `adr-erratum` label. The
  correction may not change the decision.
- An ADR records the context, the decision and its consequences. It does not record
  implementation status, migration progress or delivery history.
- Existing ADRs predate this rule. They stay as they are and are not retroactively cleaned.

### D2. Delivery status lives in issues

- Documents do not add new completion status. ADRs, `INDEX.md`, `roadmap.md` and plugin indexes
  link to the issue.
- Existing status text written before this decision is historical and is not updated.
- A document may record whether it is a draft or a proposal; it does not record what has been
  delivered.

### D3. A design owns only its own scope

- A feature or plugin design states its own inputs, outputs, events, state, errors and declared
  dependencies. It links to another module's contract instead of restating or changing it.
- It may name the public services, events, data shapes and errors it uses; it does not describe
  how another module registers, retries, routes or cleans them up.
- A cross-module requirement is stated in the design that raises it. The owner changes its own
  document in its own change.
- Cross-module flows live in a higher-level flow document.
- Review enforces this rule; there is no automated count of changed design files.

## Checks and guidance

- `scripts/adr-check.mjs` enforces only D1: an accepted `docs/adr/*.md` cannot be modified or
  deleted without `adr-erratum`. It does not scan prose for status words and does not check
  translations. A label change does not start a new run, so add the label and push again.
- Templates live in `docs/templates/`. The README there links the examples.
- `AGENTS.md` states the constraints and routing. It also states the technical-detail layers and
  that document type comes from the existing path, with no manifest.

## Consequences

- Existing ADRs remain valid history, including status text written before this decision.
- A design that needs another module changes that owner's document separately.
- New status-only edits to indexes and accepted ADRs stop.
- An attempted edit to an accepted ADR gets a local, executable failure before review.
- Cross-owner design edits are caught by review, not by a file count.

## Non-goals

- Removing technical detail from designs.
- Freezing design documents. They remain living documents within their own scope.
- Rewriting legacy status text in existing documents.
- Automating whether a decision is correct.
