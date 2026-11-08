# Timeline read consumers and page execution

Scope: bounded display reads and execution of the existing property query contract.
[Timeline property queries](timeline-property-query.md) owns query semantics;
[Clients](clients.md#opening-a-conversation-with-a-bounded-window) owns window sizes.
This design changes no event format or search matching rule. The persistent store remains v36.

## Consumers

| Consumer | Read and lifetime |
| --- | --- |
| CLI / ACP | Existing initial windows and owner inventories under the Clients contract. Export reads its explicit full prefix. Reconnect consumes the missing live prefix. |
| Web main / selected lines | Initial backward window under the same client limits. Reconnect replaces opened windows. Earlier content requires an explicit action. Raw pages last only until projection. |
| Web unopened lines | Directory identities/lifecycle and pending question state; no transcript read. Pending questions are read at each directory head, then updated by live facts. |
| SessionTitle | Four one-row property queries at one published cutoff: first adopted operator id, triggering operator id, selected user content, latest adopted provider/model. [SessionTitle §0.1](plugins/SessionTitle.md#01-fields-read-for-one-title-attempt) owns selection and eligibility. |
| Branch tool activity | Raw page endpoint from batch metadata, then display fields within that range. [Timeline branches](timeline-branches.md#activity-page-execution) owns rendering and cursor behavior. |
| Recall search | Candidate positions/identity fields, then the required facts for small batches of candidates, then existing narrative rendering and exact matching. Search stops once the result page and continuation evidence are available. |
| Context request preparation | Selected adopted inputs/tool results and required request components for that request. Rebuilding a concrete request may legitimately consume a large selected history. The request owns that memory. |
| Restore integrity / explicit export | Their full-range reads serve integrity or an explicit full export; they do not define the default display or state-query path. |

## Page execution

SQL must distinguish two ranges:

- The outer candidate window advances past the output position cursor.
- Related facts retain the original fixed range, including ancestor cuts. A terminal or an input before the cursor can still decide whether a later origin matches.

For an exactly representable position query, SQL bounds the outer range and takes at most the requested
number plus one before field projection. Physical rows that merge output deltas remain eligible when
they overlap that range. Logical output positions are clipped afterwards. A predicate that still needs
referenced content cannot use that early limit.

Input ids use the event identity index. Request relations use the existing request identity index.
A related disjunction can compile into separate indexed alternatives. Safe text predicates inside
those relations test the text index by event identity or request partition; they do not rebuild a
complete temporary text-candidate set for each output page. Ancestry concatenation and publishing
tails keep the conservative existing candidate path when a direct index predicate cannot preserve it.

General grouping, unsupported relation shapes and content-dependent predicates retain their complete
fixed scope. This design does not promise constant work for arbitrary query expressions.

The store's default batch page first resolves the final batch endpoint and reads only that range.
Memory and SQLite resolve a containing batch from metadata. The trait default can probe existing
batch-boundary validation for custom stores; it preserves their source compatibility without reading
an entire remaining suffix.

## Web flow and controls

```text
attach -> directory -> main window -> owner state at window head -> show -> live
           |
           +-> pending questions on other lines (no transcripts)

select a line -> its window + owner state + selected-line detail -> show -> live
load earlier  -> older window at original head -> prepend display rows
reconnect     -> fresh windows for main and previously opened lines
```

The frontend requires window support and the pending-input, pending-question and current-turn reads.
Read failure leaves the affected conversation retryable; unsupported servers do not trigger full-history
fallback. The current model comes from attach, Goal from its owner, and absolute output character counts
from the current-turn read. A missing start in a window does not establish that work is finished.
Missing tool origins are resolved by request/call identity, including results arriving live. Events
remain ordered while an origin read is pending. Loading older display rows does not replay old controls
into the current owner snapshots.

## Acceptance and limits

Use disposable stores. Run property parity across ancestor cuts, merged output and publishing tails;
record plans and VM instructions for both the first page and **all pages cumulatively**. Scale candidate
sets by at least 10×. Exercise the combined input/model/tool relation shape as well as a simple input
join, hits and misses. A small returned page or zero `FULLSCAN_STEP` is not evidence of a bounded scan.

The full-pagination benchmark intentionally consumes every candidate. Reducing its VM instructions
means less repeated SQL work; it does not mean fewer candidates or fewer body verifications in the
business flow. Recall may still consume all candidates when exact rendering finds too few matches.
Short/non-indexable words and text produced only by rendering require conservative candidate retention.
Rendered search output is not inserted into FTS. Source-field relations without a matching identity
index can still scan the relevant event kind; their cost must not be described as an indexed point read.

Public flows cover whole atomic pages, hidden events, explicit older loads, lazy branch transcripts,
window-external pending controls, output offsets, missing tool origins, title selection and branch
activity rendering. A range-refusing store pins that BranchTool uses property reads; large tool results
exercise its existing displayed effects without requiring full stored-event reads.

## Structure and removal check

The existing facades still establish scope and cutoff; stores execute generic queries; consumers own
selection and display. No Reader service, query session, persistent projection or rendered-text index
is added. Web drops its historical Goal fold and per-branch input history for message counting. The
short-lived raw window is required for backwards pages to project in order; it is released afterwards.
The branch activity enum orders three field projections before rendering. SQL compiler state and
temporary content tables belong only to one query.
