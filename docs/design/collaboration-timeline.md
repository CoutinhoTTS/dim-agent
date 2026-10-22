# Collaboration Timeline

Status: Proposed; not implemented. Scope: the Collaboration Timeline mechanism, a
time bus of its own beside the agent branch Timeline. Business built on it
(collaboration spaces, teams, delegation, membership) is out of scope: it is
extended by state machines and designed by its owners.

## 0. Concept

Dim has one kind of time bus today: the branch Timeline of an agent Session
([time bus](../architecture/rust-agent/time-bus.md)). An event is written, the
Timeline assigns its sequence, the Dispatcher consumes committed events in order
and delivers each to its subscribers, and every subscriber is a state machine that
decides from what it consumed and writes its results as new events. Causality is
that loop; nothing is linked by reference or called directly.

The **Collaboration Timeline** is a second kind of bus with the same loop, for what
happens between agents rather than inside one:

```text
   agent Session A          agent Session B          system owners       external
   (state machine,          (state machine,          (lifecycle, reload,  (webhook, ACP,
    its own branch buses)    its own branch buses)    schedule, ...)       bridges)
         │   ▲                    │   ▲                     │                 │
  writes │   │ consumes    writes │   │ consumes            │ writes          │ writes
         ▼   │                    ▼   │                     ▼                 ▼
   ┌──────────────────────── Collaboration Timeline ─────────────────────────────┐
   │  seq 1  seq 2  seq 3  …       sequence, atomic batches, before, publish      │
   └──────────────────────────────────┬──────────────────────────────────────────┘
                                      │ driver = Dispatcher: consumes committed events
                                      ▼ in order, delivers to subscribers
                 collaboration state machines (business extensions)
                 each consumes, decides, writes result events back
```

- It is **not an agent Session**. It has no model, Loop, context or tools. It is a
  bus, its driver and the state machines subscribed to it.
- **Every agent Session is a state machine** relative to it: it affects the
  Collaboration Timeline only by writing to its entry, and is affected only by input
  written to its own buses.
- **System changes and external influences** are written to it by their owners and
  consumed like any other event.
- **Everything that gives the bus meaning is business**: which events exist, what a
  collaboration space is, who takes part, how work is assigned and settled. Business
  is added by registering state machines on the bus. The mechanism defines none of
  it.

## 1. Why

Today collaboration has no bus. The Collaboration plugin
([collaboration.rs](../../crates/dim-app-server/src/collaboration.rs)) keeps its facts
on the leader's branch, its Team in process memory, and decides completion by
waiting on another Session. Its relations do not survive a restart, its decisions
read other Sessions' live state, and system changes reach it through ad-hoc
observers. These are symptoms of one missing mechanism: there is no bus whose
owner is the collaboration itself, so collaboration state has nowhere to be a fold
of consumed events. How Collaboration's business is rebuilt on the new bus is
Collaboration's own design, not this document's.

## 2. Mechanism contract

The Collaboration Timeline reuses the agent Timeline's contracts unchanged. Only
its owner differs.

| Aspect | Contract |
| --- | --- |
| Identity | A Collaboration Timeline root has its own id (not a Session id); each of its branches has a branch id. `(root, branch)` addresses one bus |
| Order | One sequence per branch; only the sequence orders events (time-bus §1) |
| Write | `append`, atomic batches, `before` callbacks, commit then publish: exactly the agent Timeline's rules ([timeline storage §3](../architecture/rust-agent/timeline-storage.md)) |
| Driver | A Dispatcher per branch, unchanged: the single ordered reader of published events; delivers by key filter to subscribers; handlers register their own work and return (principles §4 rule 3) |
| Events | Envelope and storage as for agent events. Business event types are `EventKind::Extension` (dynamic events, [ADR 001 #2](../adr/001-harness-foundation.md#events-and-ownership-issue-2)); the core defines only the generic ones it already has (`branch/*`, `stop/interrupt`) |
| State | Every subscriber's state is the fold of what it consumed, rebuilt from the bus on load; side effects are not replayed; a new generation settles what the previous one left open (time-bus §3, §5) |
| Tree | Branches with cuts, created and closed by the same TimelineTree rules (`branch/create` at a cut, `branch/start`, `branch/create-failed`); a branch inherits its parent's history up to the cut |
| Persistence | The same `TimelineStore`; durable like agent history |
| Lifecycle | Load, publish, unload and close with the same phases as a branch (Loading → Active → Closing → Closed); closing settles accepted work before services go, and close does not cascade to agent Sessions |
| Readers | History, local, trace and live observation, as for a branch |

What the mechanism does **not** do: define collaboration events, membership,
spaces, roles, addressing, admission or completion; decide when a Collaboration
Timeline is created or closed; read or write agent Sessions by itself.

## 3. State machines

A state machine on the Collaboration Timeline is an ordinary capability of the
bus's scope: registered by a factory of that scope, declaring `Deps`, providing
services, subscribing through the bus's Dispatcher and writing through its
`Timeline` service (principles §1, AGENTS rule 1). Business is any number of them.

| Kind | Lives | Reaches the Collaboration Timeline by | Reaches agents by |
| --- | --- | --- | --- |
| Bus state machines | The Collaboration Timeline's scope | Its own Dispatcher subscription and `Timeline` service | Writing input to an agent's public input entry (`Session::input`, `stop`) after consuming an event |
| Agent-side state machines | An agent Session or branch, as capabilities of that scope | Writing to the Collaboration Timeline's public input entry after consuming an event on the agent's own bus | Their own branch's Dispatcher and `Timeline` |
| System and external writers | Their owners (process plugins, boundary owners) | Writing to the public input entry when their own state changes | — |

One rule connects the buses: **a state machine writes to another bus only after
consuming an event on its own bus, and only through that bus's public input
entry.** It never reads another bus's or state machine's live state to decide, and
it never writes another owner's result events. This is the existing rule "a branch
affects another branch only by writing input" ([principles §2](../architecture/rust-agent/principles.md#2-作用域的含义)),
applied to both kinds of bus. Reading another bus's history, and when such a read
counts as complete, remains the reading state machine's business (time-bus §6).

The public input entry of a Collaboration Timeline accepts business events from
in-process writers that hold its address; validation of their content belongs to
the state machines that consume them (principles §4 rule 3: entries check shape and
authorization only).

## 4. Business is extension

Collaboration spaces, teams, delegation, messages, membership, nested work, how a
collaboration branch relates to agent branches, and how a closed participant is
treated are all decisions of business state machines on this mechanism. Each
business owner designs its events and state machines in its own document, with the
usual rule check. The existing Collaboration plugin is the first such owner; its
redesign on this bus is a separate document.

## 5. Infrastructure changes

This is an infrastructure change. Its need is the infrastructure's own: the runtime
has a time bus only for agent branches, and a bus whose owner is not an agent
Session does not exist. Nothing in the change carries collaboration meaning, and it
is implemented and verified inside the runtime (principles §4 rule 8).

| Owner | Change |
| --- | --- |
| `TimelineKey`, `TimelineStore` | Address a bus by a root that is either an agent Session or a Collaboration Timeline root; root records for Collaboration Timelines without `SessionConfig`. Storage format decision (next version) |
| Scope tree | A Collaboration Timeline scope under the process root, beside `SessionScope`, providing branch-equivalent scopes with `Timeline`, `Dispatcher`, identity and lifecycle ([single tree](../architecture/rust-agent/single-tree.md)) |
| TimelineTree | Reused for the Collaboration Timeline's branches |
| Assembly | A factory for Collaboration Timeline capabilities, as `session_plugin` / `timeline_plugin` are for agents |
| Entries | A public input entry addressed by `(root, branch)`, available to agent capabilities and process owners through a language-neutral service, so TypeScript state machines use it by name (ADR 002 D5) |
| Client protocol | Read and observe Collaboration Timelines; carrying Extension events to clients needs the dynamic protocol that ADR 001 #2 leaves unimplemented |

Unchanged: Timeline write rules, Dispatcher, `before`, the event envelope, agent
Sessions and their facades.

## 6. Rule check

| Rule | Application |
| --- | --- |
| Time bus §1–§6 | Same bus contract; only the owner differs |
| Principles §1–§2 | A new scope for a new owner; state machines are capabilities of the scope that owns their state |
| §4.1 commands/results | Writers write commands or facts on a bus; the consuming state machine writes its results |
| §4.2 hand-over | Crossing buses is an explicit input write after consumption |
| §4.3 before | Same before contract |
| §4.4 core neutrality | The mechanism defines no collaboration event and no business handling |
| §4.5 batches | Same batch contract |
| §4.6 context | The Collaboration Timeline never enters model context; agents see only input written to their own buses |
| §4.7 public access | Only public entries and subscriptions; no live-state reads |
| §4.8 infrastructure | §5: need, implementation and verification within the runtime |
| §4.9 necessary facts | Business state machines decide which facts they need |
| ADR 002 D2–D5 | No wiring module; business is optional extension; configuration stays with business owners; the entry is a language-neutral service |

**Requirements raised to mechanism owners:** this document is itself the requirement
to the runtime (§5); nothing is raised to rutis.

## 7. Open questions

Mechanism only:

1. Storage: one store with a root kind for both bus owners, or a separate root table
   for Collaboration Timelines?
2. Entry service: one generic input entry for both kinds of bus, or one per kind?
3. Client protocol: deliver Collaboration Timeline events only once the dynamic
   protocol exists, or define it together with this mechanism?
