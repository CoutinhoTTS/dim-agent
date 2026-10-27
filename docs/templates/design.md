# Name

Status: Draft | Ready. Delivery status is tracked in the issue, not here.

Scope: process | Session | branch.
Issue: #NNN.
Rules: [design rules](../architecture/rust-agent/principles.md), [ADR 002](../adr/002-plugin-scope.md).

## 1. Scope and non-goals

What this part owns and what it does not.

## 2. Own contract

The inputs, outputs, services, events, state and errors this part owns. Name public contract
terms where the decision depends on them. Do not describe another module's registration,
routing, retry or cleanup behavior; link to its contract.

## 3. Events and state

The facts this part writes or consumes, their owner, and how state is rebuilt. Say when there
are none.

## 4. Dependencies

The public services this part declares and what it requires from each. Link to the owning
contract.

## 5. Cross-module requirements

Changes needed from another owner, in the format of [design rules §4 rule 8](../architecture/rust-agent/principles.md).
The owner changes its own document; this design does not.

## 6. Rule check

Answer the applicable rules from [design rules §4](../architecture/rust-agent/principles.md),
§1 and [ADR 002](../adr/002-plugin-scope.md) D1–D5. State why a rule does not apply.

## 7. Binding appendix

Exact Rust or TypeScript spelling, if a binding is needed. Semantics stay above this section.
