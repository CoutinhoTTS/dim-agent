# Name

Status: Draft | Ready. Delivery status is tracked in the issue, not here.

Scope: process | Session | branch.
Issue: #NNN.
Rules: [design rules](../architecture/rust-agent/principles.md), [ADR 002](../adr/002-plugin-scope.md).

## 1. Scope and non-goals

State the intended use and affected contract: who calls this part, what produces its inputs,
and which lifecycle boundaries matter. Link the authoritative contracts and available complete
flows that ground those assumptions. Distinguish compliance fixes and wording clarifications
from proposed semantic changes; observed implementation behavior does not override the contract.

Use only the sections and tables needed for this change; explain inapplicable scope briefly and
link existing arguments for unchanged obligations. Retain the rule-check and cross-module
requirement sections required by AGENTS.md. No particular formal tool is required.

## 2. Own contract

The inputs, outputs, services, events, state and errors this part owns. Name public contract
terms where the decision depends on them. Identify affected obligations and who discharges each,
including admission and settlement across lifecycle boundaries. A narrower model does not remove
those obligations. Do not describe another module's registration,
routing, retry or cleanup behavior; link to its contract.

## 3. Assumptions, events and state

Use the [design argument guide](../design/capability-design-arguments.md). Identify which parts
are Capabilities. Define the state and initial conditions relevant to the Timeline contract.
Keep plugin-specific execution design at its own scope; expose only the boundary inputs and
guarantees needed by these propositions. Link each guarantee supplied by another owner. The
model does not prescribe a small component, a fixed implementation shape or a proof of every
external effect.

For each introduced event, state its meaning, establishment condition, correlation identity,
valid writers and compatibility. Consumers are open-ended, not an admission list. Separately
declare this part's live and historical dependencies. Say when there are no events or state.

| State / guard | Event or local action | State after | Facts written | Work started, cancelled or released |
| --- | --- | --- | --- | --- |
| | | | | |

Include relevant completion, cancellation, late arrival, unload and failure transitions. A
rejected or irrelevant input also has a defined outcome. Do not invent transitions solely to
fill the table.

## 4. Propositions and arguments

State the properties this design guarantees and justify them before implementation. Give the
initial case and preservation across each relevant transition for invariants; give progress
assumptions for eventual outcomes. State assumptions at integration boundaries, including
other event writers and dynamically added consumers. A finite model check is not an unbounded
proof or proof of the implementation.

| Proposition | Assumptions | Argument or proof artifact | Scope and unresolved obligations |
| --- | --- | --- | --- |
| | | | |

## 5. Recovery and query argument

| Required recovered state | Fixed scope / cutoff | Selection and correlation | Returned fields / computation | Recovery action |
| --- | --- | --- | --- | --- |
| | | | | |

Show why these queries produce the recovery result required by this part's contract. State what
is deliberately settled, paused or discarded under the existing lifecycle and owner contracts;
new-state recovery does not by itself justify old-work settlement. Use the [property-query contract](../design/timeline-property-query.md),
not full-history collection or a default snapshot per event. Explain pagination and handoff to
live consumption. Distinguish first installation from reload where their obligations differ.
State expected scan, decode and retained-data costs and how implementation measurements will
check them; a small reply alone proves no cost bound. If no recovery is needed, explain why.

## 6. Dependencies

The public services this part declares and what it requires from each. Link to the owning
contract.

## 7. Cross-module requirements

Changes needed from another owner, in the format of [design rules §4 rule 8](../architecture/rust-agent/principles.md).
The owner changes its own document; this design does not.

## 8. Rule check

Answer the applicable rules from [design rules §4](../architecture/rust-agent/principles.md),
§1 and [ADR 002](../adr/002-plugin-scope.md) D1–D5. State why a rule does not apply.

## 9. Implementation correspondence and validation

Map the modeled actions and queries to implementation boundaries or planned entry points.
Name the affected public flows and the observations that distinguish a violation. Keep design
arguments, bounded model-check evidence, implementation tests and performance measurements
distinct. Classify discrepancies against the contract before making them regression expectations.
Reuse applicable evidence; record run results in the issue or PR, not here.

## 10. Binding appendix

Exact Rust or TypeScript spelling, if a binding is needed. Semantics stay above this section.
