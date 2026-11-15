# Capability design arguments

Status: Draft.
Scope: how authors express and justify plugin, Capability and Timeline event designs before implementation.
Issue: [#1615](https://github.com/arcships/dim/issues/1615).

This guide owns the design method, not event or lifecycle semantics. Use the existing
[design template](../templates/design.md). The [responsibility review](capability-document-review.md)
routes runtime questions to their existing contracts.

## 1. Existing contracts and the proposed change

State placement, ordered facts, work settlement, open event contracts and precise-query recovery
already belong to the [time bus](../architecture/rust-agent/time-bus.md),
[principles](../architecture/rust-agent/principles.md), [plugin contract](plugin-contract.md) and
[property-query contract](timeline-property-query.md). This guide does not add a writer registry,
close the set of consumers, change reload outcomes or require snapshots.

The proposed authoring requirement is new: a design states the assumptions, definitions,
transitions and propositions of its Timeline-facing contract, and supplies an argument for those
propositions before implementation. Plugin-specific execution design remains in its own scope.
An example of successful behavior is not the argument. Unresolved obligations remain explicit;
a design with an unresolved required proposition is not ready to implement as an established contract.
A proposed behavior change still requires owner agreement under AGENTS.md.

## 2. Name the role, not an ambiguous owner

Use these distinctions when writing an argument; their underlying semantics remain in the linked contracts.

| Role | Meaning and source |
| --- | --- |
| State owner | The part that changes a particular business state; other parts may derive their own views from the same facts ([time bus §2](../architecture/rust-agent/time-bus.md#2-事件是总线上带序号的事实)) |
| Event contract maintainer | Defines the meaning and compatibility of the event type; this is not exclusive authority to append it ([ADR 001, Events and ownership](../adr/001-harness-foundation.md#events-and-ownership-issue-2)) |
| Writer | Produces an event satisfying that contract and the applicable work/port authority; several writers may produce the same type (same source) |
| Consumer | Declares the facts its decisions need, including query dependencies; the set of consumers is open ([principles §3](../architecture/rust-agent/principles.md#3-正交)) |
| Work owner | Starts, cancels and settles its own work; a new consumer does not acquire historical work merely by reading it ([time bus §2, §5](../architecture/rust-agent/time-bus.md)) |

Writer unload does not invalidate or delete facts. Unknown extension facts remain data, not
instructions to install code or run effects (ADR 001). A known-consumer list is an illustration,
not an event validity condition. No correctness argument may assume that no new conforming writer
or consumer can be added unless a specific contract actually supplies that restriction.

## 3. Write the problem before choosing the implementation

The common design obligation concerns the part's contract with the Timeline: the facts it
consumes and produces, their effect on its business state, and recovery and consumption handoff.
It does not prescribe the size, internal organization or execution algorithm of a Capability.
The definition of a Capability remains [principles §1](../architecture/rust-agent/principles.md#1-插件部件与-capability-有界).
A transition table describes this contract; it is not a requirement to split a feature into tiny
state machines or implement a pure reducer.

Plugin-specific execution design owns external calls, processes, resource management and external
system behavior. Those may have their own models and proofs, but this guide does not require
proving them as part of every Timeline argument. At the boundary, name only the inputs and
guarantees needed by the proposition. For example, a claim about a terminal fact may depend on
the work owner's settlement guarantee; link that guarantee rather than modeling its entire
execution. If that guarantee is not established, keep the dependency explicit instead of claiming
an unconditional result. A Timeline proof is not a proof of all plugin behavior.

Define the relevant state space S, its initial condition Init, the input/event domain E and
the allowed transition relation Next. A useful notation for one transition is:

```text
(relevant business state, consumed fact or declared boundary input)
    -> (new business state, facts to append, declared work requests)
```

This is design notation, not a new runtime API. Choose the abstraction for the proposition:
append, publication and consumption need separate steps only where their distinction affects
that proposition. Internal execution can remain abstract behind its declared contract. For
example, a cancellation request and a settled result cannot be treated as the same boundary
input when their difference changes which terminal facts are allowed.

## 4. State and discharge the propositions

For a safety invariant I, give both obligations:

```text
Init(s) implies I(s)
I(s) and Next(s, s') imply I(s')
```

Check every enabled transition, including irrelevant input and failure outcomes. A temporal
property may need a small history variable in the model; that does not require production code
to retain a history copy. Do not strengthen assumptions solely to exclude an inconvenient path.

For an eventual outcome, state what must eventually happen in the environment: scheduling,
storage completion, resource release, or an external response. Explain why those assumptions
imply progress. A system permitted to crash forever cannot promise eventual successful settlement.

For composition, name the guarantees the part needs from its neighbors and show how they are
provided. Proving two parts independently does not prove their handoff. Consider a new conforming
writer, a consumer disappearing, and a late result where those affect the proposition. Do not model
arbitrary plugins as authorized writers of another part's terminal facts.

## 5. Prove the recovery result, not an implementation technique

Define ExpectedRecovery(H, c) from the owning business contract for history H at cutoff c.
Then establish:

```text
Recover(Query(H, c)) = ExpectedRecovery(H, c)
```

The equality may be observational equivalence if the two states use different representations;
name the observations. It does not require equality with all pre-unload memory. Goal pauses on
reload, Wait discards old arrivals, and resource owners settle their unfinished work according to
their own contracts. First installation may have different obligations from replacing an existing
generation; state them rather than adopting historical work implicitly.

Derive the selection and returned fields from the required result. Explain complete correlation
keys, why finished objects are excluded, and why still-effective objects are included. All reads
in a recovery use the same cutoff and relevant ancestor cuts; continuation narrows the outer
result page, not related-event scope ([query §2–3](timeline-property-query.md)). Query errors are
not empty results. The Dispatcher handoff is a separate obligation ([plugin contract §5.2](plugin-contract.md#52-consuming-facts)).

A simple mathematical definition over history is allowed as the proof reference; it is not a
production replay algorithm. If ordered processing is necessary, select required fields and
consume pages without accumulating historical events. Persistence choices follow the
[saving boundaries](owner-state-indexed-reads.md); no per-event snapshot protocol follows from this equation.

Logical correctness does not prove indexed execution cost. State what should scale with current
objects rather than ended history, and measure candidate scans, decoded bytes and retained data
on the implementation under the [resume acceptance](resume-read-design.md#8-验收).

## 6. Evidence and implementation correspondence

| Evidence | What the design must specify | What it does not establish |
| --- | --- | --- |
| Mathematical argument | Definitions, assumptions, proposition, initial case and preservation or progress argument | Machine checking or correctness of source code |
| Machine-checked proof, for example Lean | Exact theorem, definitions, assumptions/axioms, tool version and reproducible command | Correctness of a different model or its Rust implementation |
| TLA+ with TLC | Model, finite configuration, checked invariants/properties, fairness, state constraints and reproducible command | An unbounded theorem from a bounded run |
| Implementation flow test | Model actions mapped to real public boundaries and observable outcomes | All executions beyond those exercised |
| Performance measurement | Workload, current objects versus historical size, scan/decode/retention measurements | A logical safety or liveness proof |

Record outstanding obligations and their effect on the claim. Keep execution results, counterexample
investigation and delivery progress in the issue/PR. Tool references:
[TLC](https://docs.tlapl.us/using:tlc:start) and
[Lean proof validation](https://lean-lang.org/doc/reference/latest/ValidatingProofs/).
No design is required to use all tools. Choose evidence for the proposition, not a tool checklist.

Ground environment assumptions in the selected profile, caller and event-producer contracts,
lifecycle ownership and existing integration flows. Record missing evidence; an isolated model
cannot establish that its inputs represent actual use.
Start with the owner's externally observable promises and map each to an argument, a dependency
or an explicit gap; do not choose the coverage solely from what is easy to model.
Map the design's actions and observation points to implementation entry points. Translate relevant
counterexamples into existing flow tests. Code review checks whether locks, awaits, queries and
effects preserve the modeled boundaries; model correctness alone does not establish that mapping.

The [WaitBranch example](examples/wait-argument.md) gives a scoped mathematical argument and
executable model of recovery/consumption. Its assumptions and excluded behaviors are part of the claim.

The [Wait contract experiment](examples/wait-contract-experiment.md) demonstrates a coverage
inventory and a history-based oracle compared with actual public service results.
