# Session broadcast subscription

Status: Draft. Overall design: [overview §3 dependency direction and ports](../overview.md#3-dependency-direction-and-ports), [§5.3 observer flow](../overview.md#53-observer-through-the-broadcast-subscription-sessiontitle), [§9 open questions](../overview.md#9-open-questions) (adopted: reuse the Session broadcast, decision E7). Phase: P0. Language: Rust.

A supplement of the existing Session broadcast, not a second bus: a process-level subscription, hosted with [SessionAccess](session-access.md) in the stable plugin `SessionAccessHost` (depends only on `AgentDirectory` and `Loader`), so that a process plugin subscribes once and receives live events of all loaded Sessions, filtered by event type, Session and branch, plus four lifecycle notices: `Created` and `Loaded { restored }` carrying the `SessionConfig` subset, `Ended`, and `Gap`. Delivery is live, unordered across Sessions, non-resumable and lossy under backpressure with an explicit `Gap`. It is an observation trigger only: decisions read the fixed Timeline prefix through [session-access](session-access.md). Consumers include Notifications, SessionTitle, Hooks (session start observes `Created`), Goal, Scheduler, ShellInput and Collaboration (restore). Subscriptions never depend on Agents: AgentIndex registers into the host and announces `Created`/`Loaded` through its registration, so an Agents reload neither reloads subscribers nor ends streams.

## 1. Structure

```mermaid
flowchart LR
  subgraph PROC["Process (App Server)"]
    Dir["AgentDirectory, Loader (stable)"]
    Agents["Agents (AgentIndex, creation and restore)"]
    SE["SessionAccessHost: SessionEvents"]:::new
    Coll["Collaboration"]
    Cons["Consumers: Goal, Hooks, Notifications, ..."]
  end
  subgraph SESS["Session / branch"]
    SB["SessionBroadcast (branch)"]
    SBE["SessionBroadcastEnd (Session)"]
    Chan["session_channel(session_id)"]
  end
  SE --> Dir
  Agents -->|"register index; announce Created, Loaded"| SE
  Coll -->|"subscribe, acts on restored true"| SE
  SB -->|"Event"| Chan
  SBE -->|"Closed"| Chan
  SE -->|"listener per loaded Session"| Chan
  Cons -->|"subscribe"| SE
  classDef new fill:#d3f9d8,stroke:#2b8a3e,color:#000
```

The subscription reuses the per-Session broadcast (`SessionBroadcast`, `session_channel`) rather than adding a second bus ([single-tree](../../architecture/rust-agent/single-tree.md), events). AgentIndex announces `Created` and `Loaded` through its registration at the points where it registers a Session, so no new creation hook exists. Every edge points to the stable host, so subscriptions survive an Agents reload by construction.

| Part | Scope | Depends on | Provides | Status |
| --- | --- | --- | --- | --- |
| `SessionBroadcast` | Branch | `B:Dispatcher`, `B:TimelineIdentity` | `SessionBroadcastEvent::Event` on `session_channel(session_id)` | Existing ([session_broadcast.rs](../../../crates/dim-agent-capabilities/src/session_broadcast.rs)) |
| `SessionBroadcastEnd` | Session | `S:SessionIdentity` | `SessionBroadcastEvent::Closed` on close (not on idle unload) | Existing |
| `SessionAccessHost` | Process | `AgentDirectory`, `Loader` | `SessionEvents` (process subscription) | New (stable host, shared with SessionAccess) |
| Agents | Process | `SessionAccessHost` (registration) | `Created`/`Loaded` announcements at registration, generalizing the crate-private `SessionRestored` | Supplement |
| Collaboration | Process | `SessionAccessHost` | restore handling subscribes and acts on `Loaded { restored: true }` only | Change |
| Consumers | Process | `Arc<SessionEvents>` | — | Per plugin |

Contracts defined here (owner: Agents side, hosted in `SessionAccessHost`): `SessionEvents`, `SessionEventFilter`, `SessionNotice`, `SessionEnd`, `SessionEventStream`, `SubscribeError`, and the delivery guarantees. Contracts depended on: the Session broadcast, `SessionConfig` with the fields proposed in [session-access A.3](session-access.md#a3-sessionconfig-fields), the public data contract of `AgentEvent`/`EventKind::key()`, `Session::closed()`/`is_unloading()`/`failure()`. The provider filters by opaque type keys and never interprets payloads.

## 2. Data and state

```mermaid
classDiagram
  class SessionEvents {
    listeners: map session to channel listener
    subscriptions: list
    subscribe(ctx, filter, capacity)
  }
  class Subscription {
    filter: SessionEventFilter
    queue: bounded
    dropped: u64
    gap_sessions: set
  }
  class SessionNotice {
    Event(agent, session, branch, event)
    Created(session, config subset)
    Loaded(session, config subset, restored)
    Ended(session, reason)
    Gap(dropped, sessions)
  }
  SessionEvents "1" --> "*" Subscription
  Subscription --> SessionNotice : delivers
```

Nothing is persisted and nothing is written to a Timeline. Channel listeners are attached when AgentIndex announces a Session and, when a new AgentIndex generation registers, by enumerating loaded Sessions through `AgentDirectory`; subscriptions belong to their subscribers' contexts.

## 3. Flows

### 3.1 Creation, subscription and a turn end

```mermaid
sequenceDiagram
  participant A as AgentIndex (registration point)
  participant SE as SessionEvents
  participant C as Consumer (e.g. Hooks)
  participant B as SessionBroadcast (branch)
  participant SA as SessionAccess
  C->>SE: subscribe(filter types turn/end, branches main)
  A->>SE: announce Created: attach listener on session_channel(id) (awaited)
  SE-->>C: Created {session, interactive, role, created_by, node}
  A-->>A: creator receives the Session
  C->>SA: session start input when its rules apply
  B-->>SE: Event turn/end at sequence s
  SE-->>C: Event {session, main, s}
  C->>SA: prefix(main, at_least s), decide, write
```

AgentIndex awaits the announcement before handing out the Session, so no input of a new Session precedes it. `Created` is delivered for every creation path (`session.create`, `launch`, Collaboration member creation including `establish`, fork); consumers select by the subset (Hooks runs session start only for `created_by = Client`, fail-open, decision E10). Restore yields `Loaded { restored: true }` instead.

### 3.2 Overflow and resynchronization

```mermaid
sequenceDiagram
  participant B as SessionBroadcast
  participant SE as SessionEvents
  participant C as Consumer
  participant SA as SessionAccess
  B-->>SE: Event e1
  SE->>SE: queue full: drop e1, dropped+1
  B-->>SE: Event e2
  SE-->>C: Gap {dropped 1, sessions}
  SE-->>C: Event e2
  C->>SA: prefix of affected Sessions
```

A gap is never silent: before the next delivered record the subscriber receives the count and affected Sessions, and resynchronizes by reading prefixes.

## 4. Timing and conflicts

### 4.1 Agents reload

```mermaid
sequenceDiagram
  participant A as AgentIndex
  participant SE as SessionAccessHost
  participant C as Consumer
  Note over A: Agents reload starts, index unregistered
  Note over SE,C: subscriptions and channel listeners stay, events keep flowing
  A->>SE: new generation registers its index
  SE->>SE: enumerate loaded Sessions via AgentDirectory, attach missing listeners
  SE-->>C: Loaded {restored false} for re-attached Sessions
```

Sessions, their channels and the host survive an Agents reload, so live events continue and no Session can be created during the window. If a listener had to be re-attached, the subscriber receives `Loaded { restored: false }` for it and treats events of the window as possibly missed.

### 4.2 Other key rows

| Scenario | Order | Outcome | Owner |
| --- | --- | --- | --- |
| `turn/end` delivered, then interrupt or new input committed | `turn/end` < later facts | Consumer reads to the published head and sees the later facts | Consumer |
| Input withdrawn before consumption | `input/message` < `input/withdrawn` | Both delivered as events; pending derivations read the prefix ([session-access A.4](session-access.md#a4-withdrawal)) | Consumer |
| Session restore | `Loaded {restored true}` < `branch/resume` dispatch | `Loaded` then the restored branch's new events; no `Created` | AgentIndex |
| Queue full | — | Records dropped; `Gap` precedes the next record | SessionEvents |

Full table: Appendix B.

## 5. Context mapping

No model-visible effect. The subscription writes no Timeline event, adds no tool, input or prompt source, and delivered records are never projected into model context. A consumer that wants the model to see something writes input through session-access, whose mapping applies. Diagnostics go to the process diagnostics channel, never to the model.

## 6. Interfaces (summary)

| Interface | Purpose |
| --- | --- |
| `SessionEvents::subscribe(ctx, filter, capacity)` | Live filtered stream owned by the subscriber's context |
| `SessionEventFilter` | Type keys (empty = notices only), Sessions, branches |
| `SessionNotice` | `Event`, `Created`, `Loaded { restored }`, `Ended`, `Gap` |

## 7. Security

Subscribers are trusted in-process plugins and see every Session's events, including user content and tool results. The service is never exported over RPC or rutis links by this contract; RemoteGateway and clients keep their per-Session attachment. Consumers must not copy event content into diagnostics or external messages beyond what their own document permits.

## Appendix A. Interface details

```rust
/// Process service of the `SessionAccessHost` plugin; key TypeKey::of::<SessionEvents>().
impl SessionEvents {
    /// Registers a subscription owned by `ctx`; it ends when the stream drops or `ctx` unloads.
    /// `capacity` bounds queued records (>= 1) (A5).
    pub fn subscribe(&self, ctx: &Ctx, filter: SessionEventFilter, capacity: usize)
        -> Result<SessionEventStream, SubscribeError>;
}
pub struct SessionEventFilter {      // (A4)
    pub types: Option<BTreeSet<String>>,    // EventKind::key(); None = all; Some(empty) = notices only
    pub sessions: Option<BTreeSet<String>>, // None = all loaded Sessions, including later ones
    pub branches: Option<BTreeSet<String>>, // branch ids, e.g. {"main"}; None = all
}
pub enum SessionNotice {
    Event { agent_id: String, session_id: String, branch_id: String, event: Arc<AgentEvent> },
    Created { agent_id: String, session_id: String, interactive: bool,
        collaboration_role: CollaborationRole, created_by: SessionOrigin, resource_node: Option<NodeId> },
    Loaded { agent_id: String, session_id: String, interactive: bool,
        collaboration_role: CollaborationRole, created_by: SessionOrigin, resource_node: Option<NodeId>,
        restored: bool },
    Ended { session_id: String, reason: SessionEnd },
    Gap { dropped: u64, sessions: BTreeSet<String> },  // since the previous delivered record
}
pub enum SessionEnd { Closed, Unloaded, Failed }
pub enum SubscribeError { Invalid(String), Closed }
impl SessionEventStream {
    pub async fn next(&mut self) -> Option<SessionNotice>;  // None: subscription ended
}
```

The subset fields are the `SessionConfig` values of [session-access A.3](session-access.md#a3-sessionconfig-fields); other configuration is read through `SessionAccess::config`. `types` uses `EventKind::key()` strings today and registered type keys once #2 lands. Notices are delivered regardless of `types`; `sessions` and `branches` filters apply to them where they name a Session. `types: Some(empty)` delivers notices only (ShellInput's close detection).

| Notice | Emitted by the host when |
| --- | --- |
| `Created` | AgentIndex announces a Session registered by `session.create`, `launch`, Collaboration member creation (including `establish`) or fork; before the creator receives it |
| `Loaded { restored: true }` | Restore inside `acquire`; Collaboration's existing restore listener (`collaboration.rs`) acts only on this (A1) |
| `Loaded { restored: false }` | A listener is re-attached to an already loaded Session when a new AgentIndex generation registers |
| `Ended` | `Closed` on the channel, or `Session::closed()` (`Unloaded` if `is_unloading()`, `Failed` if `failure()`) |
| `Gap` | Records were dropped since the previous delivered record |

Delivery guarantees:

| Guarantee | Statement |
| --- | --- |
| Liveness | Only events dispatched after the subscription is registered and after the Session's channel listener exists. No history, cursor or replay |
| Published | Every delivered event has been dispatched by its branch Dispatcher, hence published; session-access reads through at least `event.sequence` are valid |
| Identity | Each `Event` carries agent, Session and branch ids and the branch-local `event.sequence` |
| Per-branch order | Strictly increasing sequence per branch; gaps are announced, never silent (A2) |
| Cross-branch and cross-Session order | None |
| Loss | On a full queue the record is dropped and counted; `Gap` precedes the next delivered record (A3) |
| Duplicates | None within one subscription; restore does not re-dispatch stored events |
| Completeness | Not guaranteed: events dispatched while a Session is being built (main `branch/start`, initial prompt adoption) and terminal facts written by cleanup after a Dispatcher stopped ([lifecycle §5](../../architecture/rust-agent/lifecycle.md)) are never broadcast |

The rule for consumers: a notice is a hint to look. Decisions read the fixed prefix through session-access, write through session-access, and tolerate events never received.

Mechanics:

| Step | Path |
| --- | --- |
| Source | Branch `SessionBroadcast` registers `Dispatcher::subscribe_all`; per dispatched event it `emit`s on `session_channel(session_id)` and returns without waiting (existing). No Timeline write |
| Session discovery | At AgentIndex's announcement the host attaches a channel listener with `on(host.ctx, &session_channel(id), listener)` on the Agent host ctx (outlives the Session, so `Closed` arrives; precedent `Connection::attach`) and spawns a task awaiting `Session::closed()` |
| Fan-out | The channel listener matches each record against every subscription's filter and `try_send`s; a full queue increments the dropped count. It never awaits |
| Subscriber lifecycle | `subscribe` registers the subscription and its removal with the subscriber's `ctx.effect`; the host's disposer disposes channel listeners, cancels close watchers and ends all streams (the host has only stable dependencies, so this happens only at process shutdown) |
| StateCell and rebuild | None |
| Recheck | Consumers only, from a prefix read through session-access |
| Agents reload | §4.1; no consumer reloads; announcements resume when the new generation registers |
| Process restart | Nothing persisted; no replay |

## Appendix B. Full conflict table

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Event dispatched before subscription | dispatch < subscribe | Not delivered; consumer reads prefix if needed | Consumer |
| New Session's first input | listener attached < first `input/message` | Delivered from the first input on, including a session-start hook's input | Agents |
| `turn/end` delivered, then turn interrupt or new input committed | `turn/end` < later facts | Consumer reads to the published head and sees them | Consumer |
| Turn `stop/interrupt` | interrupt dispatched | Delivered like any event | — |
| Branch-scoped interrupt and branch close | branch interrupt < terminal facts < Dispatcher stop | Facts dispatched before the stop are delivered; cleanup terminal facts are not; a later session-access call returns `BranchMissing` | Consumer |
| Concurrent Steer/Queue/Append inputs | Timeline order | Delivered in sequence order per branch | — |
| `input/withdrawn` of a pending input | input < withdrawn | Delivered; a consumer deriving pending inputs excludes it from the prefix | Consumer |
| Tool batch not settled | `step/tool-call` < results | Only the whole batch's `step/tool-result` is delivered | ToolExecutor |
| Compaction | `context/compaction` | Delivered as an event | — |
| Child branch creation | `branch/create` (parent) < child `branch/start` | Child events arrive with the child's branch id from its first dispatch | — |
| Member or launched Session created | `Created` | Carries `collaboration_role`, `interactive`, `created_by`, `resource_node`; consumers filter (decision E14) | Consumer |
| Queue full | — | Records dropped, `Gap` precedes the next record | SessionEvents |
| Session idle unload | `Session::closed()` without `Closed` | `Ended { Unloaded }` | SessionEvents |
| Session restore | `Loaded {restored true}` < `branch/resume` dispatch | `Loaded` then new events; Collaboration's listener acts; no `Created` | Agents, Collaboration |
| Session close | `Closed` is the last record | `Ended { Closed }` | SessionEvents |
| Agents reload | — | Streams continue; listeners re-attached where missing with `Loaded {restored false}` | Agents |
| Process restart | — | No replay; consumers recover from their storage and prefixes | Consumer |
| Resource node offline | Session capabilities Pending | Fewer events; subscription unaffected | — |
| Dispatcher fault on a branch | — | No more events from that branch | — |
| Client detach | — | No effect | — |
| Duplicate or late notice after resubscribe | — | Cannot occur within a subscription; consumers dedupe by `(session, branch, sequence)` if they persist progress | Consumer |

## Appendix C. Errors and recovery

| Case | Result |
| --- | --- |
| `capacity == 0`, or an empty `sessions` or `branches` set | `SubscribeError::Invalid` |
| Provider unloading | `SubscribeError::Closed`; existing streams return `None`; consumers resubscribe and resynchronize through `SessionAccess` |
| Listener registration fails for a loaded Session (host ctx unloading) | Session skipped with a diagnostic; subscribers get `Ended { Failed }` |
| Subscriber drops the stream without disposing | Removed on the next send attempt and on its ctx unload |

## Appendix D. Acceptance

Flow tests in the App Server test suite:

1. Two Sessions, one subscription on `turn/end`: both turn ends arrive with correct ids and sequences; other types do not.
2. A Session created after subscribing delivers `Created` with the `SessionConfig` subset and then its first input, including a session-start hook's input; fork, `launch` and member creation also deliver `Created`.
3. Branch filter `main` excludes a child branch's events; `types: Some(empty)` delivers only notices.
4. Capacity 1 with a burst: `Gap` with the count precedes the next record; per-branch sequences increase.
5. Close yields `Ended { Closed }`; idle unload `Ended { Unloaded }`; restore `Loaded { restored: true }` without re-delivery; Collaboration's restore handling runs only for restores.
6. Agents reload: streams continue; re-attached Sessions deliver `Loaded { restored: false }`.
7. Subscriber unload removes the subscription.

## Appendix E. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | AgentIndex announces `Created` and `Loaded` where it registers a Session, before the creator receives it | `SessionRestored` precedent; simplification pass | Another hook is needed; first events may be missed |
| A2 | Per-branch delivery order is guaranteed | Judgment (free from Dispatcher order) | Consumers must not rely on it |
| A3 | Overflow drops records with a `Gap` instead of closing the subscription | Overview §4 (no ordering guarantee, no resumable reads); policy is judgment | Slow consumers lose the subscription |
| A4 | Filters are by event type key, Session id and branch id only; Session attributes are filtered by consumers from the `Created`/`Loaded` subset | Judgment | Richer filters added later |
| A5 | Subscribers choose their own queue capacity | Judgment | A shared default and cap would be defined here |

## Appendix F. Open questions

| Question | Description |
| --- | --- |
| Branch-end notice | No broadcast record marks a single branch's close; add one only if a consumer needs it beyond `BranchMissing` |
| Remote nodes | Whether a subscription spans Sessions on other control nodes (overview §6.3 future extension) |
