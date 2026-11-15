# Capability design arguments

Scope: designing a plugin's Timeline events, state and lifecycle before implementation.
Issue: [#1615](https://github.com/arcships/dim/issues/1615).

Use the [design template](../templates/design.md) to answer four questions. The new requirement
is to justify the design before implementation; event and lifecycle rules stay in their existing
contracts. Reuse arguments for unchanged behavior. Short prose is sufficient; tables and formal
tools are optional.

## 1. Where is it used?

Who calls the part, who produces its inputs, and what do their contracts guarantee? Ground the
assumptions in the selected profile and complete flows. Known plugins are examples, not a closed
list of future writers or consumers.

## 2. What is it responsible for?

State its own inputs, outputs, events and state, and what must happen during normal execution,
cancellation and reload. Link guarantees supplied by others. Internal execution can stay abstract,
but admission and old-work settlement cannot disappear from the argument.

## 3. Why does the design meet those responsibilities?

Define the relevant state and transitions, then explain why the required properties hold from
the initial state through those transitions. Include the environment assumptions needed for
progress. For recovery, show why the structured queries reconstruct the required business state;
that is separate from ending old work. Make unresolved obligations explicit.

## 4. Does the implementation match?

Map the argument to code boundaries and observable results in real flows. Distinguish a manual
argument, a bounded model check and implementation tests; none silently proves the others.
An observed contract violation is a defect, not a new regression expectation.

Follow existing contracts. Do not invent a requirement just because a speculative scenario can
be constructed; first establish that it is permitted and violates an agreed promise. Changes to
the promise follow the existing owner-confirmation process in AGENTS.md.

## Reference, when needed

The [reference](capability-design-reference.md) explains roles, argument techniques, recovery
queries and evidence limits. It is not another required form. The [Wait example](examples/wait-argument.md)
and [implementation experiment](examples/wait-contract-experiment.md) illustrate scoped arguments;
[Wait's usage context](examples/wait-usage-context.md) shows how to check their assumptions.
The [document responsibility review](capability-document-review.md) routes questions to current
contracts. No tool, component size or runtime architecture is prescribed by this method.
