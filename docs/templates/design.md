# Name

Scope: process | Session | branch.
Issue: #NNN. Delivery status belongs in the issue.

Use the [four-question guide](../design/capability-design-arguments.md). Keep the explanation as
short as the affected contract allows; link existing arguments for unchanged behavior.

## 1. Where is it used?

Callers, input producers, declared dependencies and the public guarantees this design relies on.
Link the authoritative contracts and relevant complete flows. Identify the parts that are Capabilities.

## 2. What is it responsible for?

Own inputs, outputs, state and errors; responsibilities during normal execution, cancellation and
reload. For new events, define meaning, establishment condition, correlation identity, permitted
writers and compatibility. Distinguish existing requirements from proposed behavior changes.

## 3. Why does it work?

Relevant states and transitions, the properties they must preserve and the argument that they do.
State assumptions, progress conditions and unresolved obligations. If recovery is involved, explain
why the precise queries return the required state and how consumption resumes. Consult the
[reference](../design/capability-design-reference.md) only for the details this argument needs.

## 4. Does the implementation match?

Code or planned entry points, observable outcomes and the real flows that check them. State what
the evidence establishes and what it leaves open; run results belong in the issue or PR.

## Rule check

Answer [design rules §4](../architecture/rust-agent/principles.md), §1 and
[ADR 002](../adr/002-plugin-scope.md) D1–D5 as required by AGENTS.md; explain inapplicable rules.

## Requirements raised to mechanism owners

Use the format of [design rules §4 rule 8](../architecture/rust-agent/principles.md), or state none.
Link the owner's contract rather than redefine it here.

## Binding appendix, if needed

Exact Rust or TypeScript spelling. Semantics stay in the sections above.
