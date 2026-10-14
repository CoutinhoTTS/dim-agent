# Session broadcast subscription

Status: Draft. Overall design: [overview §7.2 "Session broadcast" (To confirm)](../overview.md#72-app-server-process-plugins), [§3.2](../overview.md#32-capabilities-and-process-plugins), [§13 "Process-level subscription"](../overview.md#13-open-questions). Phase: P0. Language: Rust.

A process plugin subscribes once and receives live events of all loaded Sessions, filtered by event type, Session and branch. Delivery is live, unordered across Sessions, non-resumable and lossy under backpressure with an explicit gap notice. It is an observation trigger only: consumers that need correctness read the fixed Timeline prefix through [session-access](session-access.md). The contract knows no consumer: any process plugin may subscribe, and consumers depend on this contract, not on the plugin that provides it.

This contract answers the overview §13 question by reusing the existing per-Session broadcast (`SessionBroadcast`, `session_channel`) rather than adding a second bus ([single-tree "only one bus"](../../architecture/rust-agent/single-tree.md)).

## 1. User flows

1. A consumer subscribes to `turn/end` of all Sessions. A turn ends; the consumer receives `{session, branch, sequence}`, reads the prefix through session-access and decides whether to act.
2. A consumer subscribes to `turn/end` on branch `main` only and ignores other branches.
3. A consumer subscribes to several event types. A burst overflows its queue; it receives a gap notice and resynchronizes by reading prefixes.
4. The daemon restarts. Subscribers start fresh; events dispatched while the process was down are not replayed.

## 2. Scope

| Responsible | Not responsible |
| --- | --- |
| Subscribe API, filters, delivery guarantees, gap notices | Wire subscriptions of clients (`session.attach`, `session.event`, fences) |
| Session load/unload/close notices to subscribers | History, resumable cursors, durable delivery |
| Backpressure policy and subscriber lifecycle | Any decision of a consumer |

## 3. Scope and dependencies

**Contracts defined here** (owner: `SessionEventsPlugin`): the `SessionEvents` service, `SessionEventFilter`, `SessionNotice`, `SessionEventStream` and the delivery guarantees in §6. **Contracts depended on**: the existing Session broadcast (`session_channel`, `SessionBroadcastEvent`), the AgentIndex load notice (supplement below), the public data contract of `AgentEvent`/`EventKind::key()`, and `Session::closed()`/`is_unloading()`/`failure()`. Subscribers depend on this contract only; the provider filters by opaque type keys and never interprets event payloads. Any plugin providing `SessionEvents` with these guarantees can replace it.

| Part | Scope | Injects | Provides | Status |
| --- | --- | --- | --- | --- |
| `SessionBroadcast` | Branch | `B:Dispatcher`, `B:TimelineIdentity` | emits `SessionBroadcastEvent::Event` on `session_channel(session_id)` | Existing ([session_broadcast.rs](../../../crates/dim-agent-capabilities/src/session_broadcast.rs)) |
| `SessionBroadcastEnd` | Session | `S:SessionIdentity` | emits `SessionBroadcastEvent::Closed` on close (not on idle unload) | Existing |
| `AgentsPlugin` | Process | existing | `AgentIndex`; emits `SessionLoaded` (generalizes the existing crate-private `SessionRestored`) | Supplement |
| `SessionEventsPlugin` | Process (App Server) | `AgentIndex` | `SessionEvents` | Proposed |
| Consumer plugins | Process | `Arc<SessionEvents>` | — | Proposed per plugin |

`AgentIndex` today emits `SessionRestored` (`agent/session-restored`) with `parallel` inside `acquire` before the restoring caller receives the Session. The supplement emits the same notice, renamed `SessionLoaded { agent_id, session, restored }`, from every path that registers a newly loaded Session (`session.create`, collaboration member creation, restore), before the creator receives the handle (A1).

## 4. Events

None. The subscription adds no Timeline event and writes nothing to any Timeline. Records delivered to subscribers are process notifications on the rutis bus; Session/branch lifecycle notices are derived from existing runtime signals (`SessionLoaded`, `Session::closed()`, `SessionBroadcastEvent::Closed`).

## 5. State

| State | Owner | Rebuild |
| --- | --- | --- |
| Per-Session channel listener and close watcher | `SessionEvents` | On `apply`: register the `SessionLoaded` listener first, then enumerate loaded Sessions (`AgentIndex::hosts()` and `Agent::sessions()`), deduplicating by Session id and incarnation |
| Subscriber queue, filter, dropped count | `SessionEvents`, per subscription | Not rebuilt; a new generation starts with no subscriptions |
| Consumer progress (last handled sequence, pending work) | Each consumer, in memory or process storage | Consumer's own document |

## 6. Interfaces

```rust
/// Process service; key TypeKey::of::<SessionEvents>().
impl SessionEvents {
    /// Registers a subscription owned by `ctx`; it ends when the handle drops
    /// or `ctx` unloads. `capacity` bounds queued records (>= 1) (A5).
    pub fn subscribe(&self, ctx: &Ctx, filter: SessionEventFilter, capacity: usize)
        -> Result<SessionEventStream, SubscribeError>;
}

pub struct SessionEventFilter {      // (A4)
    pub types: Option<BTreeSet<String>>,    // EventKind::key(), e.g. "turn/end"; None = all
    pub sessions: Option<BTreeSet<String>>, // None = all loaded Sessions, including later ones
    pub branches: Option<BTreeSet<String>>, // branch ids, e.g. {"main"}; None = all
}

pub enum SessionNotice {
    Event { agent_id: String, session_id: String, branch_id: String, event: Arc<AgentEvent> },
    Loaded { agent_id: String, session_id: String, restored: bool },
    Ended { session_id: String, reason: SessionEnd },          // Closed | Unloaded | Failed
    Gap { dropped: u64, sessions: BTreeSet<String> },          // since the previous delivered record
}

impl SessionEventStream {
    pub async fn next(&mut self) -> Option<SessionNotice>;    // None: subscription ended
}
```

`types` uses `EventKind::key()` strings today and registered type keys once #2 lands. `Loaded`, `Ended` and `Gap` are delivered regardless of `types`; `sessions` and `branches` filters apply to them where they name a Session.

Delivery guarantees, stated exhaustively:

| Guarantee | Statement |
| --- | --- |
| Liveness | Only events dispatched after the subscription is registered and after the Session's channel listener exists. No history, no cursor, no replay after reload or restart |
| Published | Every delivered event has been dispatched by its branch Dispatcher, hence published; session-access reads through at least `event.sequence` are valid |
| Identity | Each `Event` carries agent, Session and branch ids and the branch-local `event.sequence` |
| Per-branch order | Delivered events of one branch have strictly increasing sequence (Dispatcher order, FIFO queue); gaps are announced, never silent (A2) |
| Cross-branch and cross-Session order | None. Consumers must not infer order between branches or Sessions |
| Loss | When the queue is full the record is dropped and counted; before the next delivered record the subscriber receives `Gap` with the count and affected Sessions (A3) |
| Duplicates | None within one subscription. Restoring a Session does not re-dispatch stored events |
| Completeness | Not guaranteed even without gaps: events dispatched while a Session is being built (main `branch/start`, initial prompt adoption), and terminal facts written by cleanup after a Dispatcher stopped ([lifecycle §5](../../architecture/rust-agent/lifecycle.md)), are never broadcast |

The rule for consumers: a notice is a hint to look. Any decision reads the fixed prefix through session-access (which also covers missed events), writes through session-access input, and tolerates events it never received.

## 7. Timeline and capability mechanics

| Step | Path |
| --- | --- |
| Source | Branch `SessionBroadcast` registers `Dispatcher::subscribe_all`; per dispatched event it `emit`s `SessionBroadcastEvent::Event` on `session_channel(session_id)` and returns without waiting (existing). No Timeline write |
| Session discovery | `SessionEvents` listens to `SessionLoaded` on AgentIndex's process event key; the handler registers a channel listener with `on(host.ctx, &session_channel(id), listener)` on the Agent host ctx, which outlives the Session so `Closed` is received (precedent `Connection::attach`), and spawns a task awaiting `Session::closed()` tracked by the plugin. Because AgentIndex awaits `parallel` listeners before handing out the Session, no input of a new Session precedes the listener |
| Fan-out | The channel listener matches each record against every subscription's filter and `try_send`s to its bounded queue; on a full queue it increments the subscription's dropped count. It never awaits (listeners on one channel are serialized) |
| End of Session | `Closed` on the channel or completion of `Session::closed()` produces `Ended` (`Closed` if the channel ended, `Unloaded` if `Session::is_unloading()`, `Failed` if `Session::failure()` is set); the channel listener is disposed |
| Subscriber lifecycle | `subscribe` registers the subscription and its removal with `ctx.effect` of the subscriber's plugin. `SessionEvents` registers its own `ctx.effect` disposer that disposes every channel listener, cancels close watchers and ends all streams (`next()` returns `None`) |
| StateCell and rebuild | None: no business state is owned; nothing is rebuilt from a Timeline |
| Recheck | Belongs to consumers: before writing, they decide from a prefix read through session-access; facts committed after that prefix are ordered before their write and handled by the input owners (InputQueue, Loop) |
| Reload | `SessionEvents` or `AgentsPlugin` reload reloads every consumer through `Deps`; consumers subscribe again in `apply` and resynchronize from session-access if they need to (e.g. list loaded Sessions and read prefixes). No gap notice spans generations |
| Process restart | Same as reload; nothing persisted by this contract |

The subscription has no model-visible effect and does not participate in `session.idle`, stop or close.

## 8. Timing and conflicts

| Scenario | Sequence order | Outcome | Owner of the decision |
| --- | --- | --- | --- |
| Event dispatched before subscription | dispatch < subscribe | Not delivered; consumer reads prefix if it needs it | Consumer |
| New Session's first input | `SessionLoaded` handled < first `input/message` | Delivered from the first input on; build-time events are not | `SessionEvents` |
| `turn/end` delivered, then turn interrupt or new input already committed | `turn/end` < later facts | Consumer reads to the published head and sees the later facts | Consumer |
| Turn `stop/interrupt` | interrupt dispatched | Delivered like any event; no special handling | — |
| Branch-scoped interrupt and branch close | branch interrupt < terminal facts < Dispatcher stop | Facts dispatched before the stop are delivered; cleanup terminal facts are not; no branch-end notice exists, a later session-access call returns `BranchMissing` | Consumer |
| Concurrent Steer/Queue/Append inputs | Timeline order | Delivered in sequence order per branch | — |
| Tool batch not settled | `step/tool-call` < results | Only `step/tool-result` after the whole batch is delivered; no partial result exists | ToolExecutor |
| Compaction | `context/compaction` | Delivered as an event; does not alter delivered sequences | — |
| Child branch creation | `branch/create` (parent) < child `branch/start` | Child events arrive with the child's branch id from its first dispatch on | — |
| Queue full | — | Records dropped, `Gap` precedes the next record | `SessionEvents` |
| Session idle unload | `Session::closed()` without `Closed` record | `Ended { Unloaded }` | `SessionEvents` |
| Session restore | `SessionLoaded { restored: true }` < `branch/resume` dispatch | `Loaded` then the restored branch's new events | `SessionEvents` |
| Session close | `Closed` is the last record | `Ended { Closed }` | `SessionEvents` |
| `SessionEvents` or Agents reload | — | Streams end; consumers resubscribe; window events lost | rutis lifecycle; consumer |
| Process restart | — | No replay; consumers recover from their own storage and prefixes | Consumer |
| Resource node offline | Session capabilities Pending | Fewer events; subscription unaffected | — |
| Dispatcher fault on a branch | — | No more events from that branch | — |
| Client detach | — | No effect (subscription is not a client attachment) | — |
| Duplicate/late notice after resubscribe | — | Cannot occur within a subscription; consumers dedupe by `(session, branch, sequence)` if they persist progress | Consumer |

## 9. Context mapping

No model-visible effect. The subscription writes no Timeline event, adds no tool, input or prompt source, and delivered records are never projected into model context. A consumer that wants the model to see something writes input through session-access; that input's mapping is defined there. Diagnostics of `SessionEvents` (drops, listener failures) go to the process diagnostics channel and never reach the model.

## 10. Errors and recovery

| Case | Result |
| --- | --- |
| `capacity == 0` or empty filter set | `SubscribeError::Invalid` |
| `SessionEvents` unloading | `SubscribeError::Closed`; existing streams return `None` |
| Listener registration fails for a loaded Session (host ctx unloading) | That Session is skipped with a diagnostic; subscribers get `Ended { Failed }` |
| Subscriber drops the stream without disposing | Subscription removed on next send attempt and on its ctx unload |

## 11. Security

Subscribers are trusted in-process plugins and see every Session's events, including user content and tool results. The service is never exported over RPC or rutis links by this contract; RemoteGateway and clients keep using their per-Session attachment. Consumers must not copy event content into diagnostics or external messages beyond what their own document permits (their own redaction rules).

## 12. Acceptance

Flow tests through public interfaces in the App Server test suite:

1. Two Sessions, one subscription filtered on `turn/end`: both Sessions' turn ends arrive with correct ids and sequences; other event types do not.
2. A Session created after subscribing delivers its first input; events before subscribing are not delivered.
3. Branch filter `main` excludes a child branch's events.
4. Capacity 1 with a burst: a `Gap` with the dropped count precedes the next record; per-branch sequences stay increasing.
5. Close yields `Ended { Closed }` after the last event; idle unload yields `Ended { Unloaded }`; restore yields `Loaded { restored: true }` and no re-delivery of stored events.
6. Subscriber plugin unload removes the subscription; `SessionEvents` reload ends streams.
7. A consumer reading the prefix after `turn/end` sees a later interrupt (row 3).

## 13. Assumptions

| # | Assumption | Basis | Effect if rejected |
| --- | --- | --- | --- |
| A1 | Session discovery uses a generalized AgentIndex `SessionLoaded` notice emitted before the creator receives the Session | Judgment; existing `SessionRestored` precedent | Another discovery hook is needed; a new Session's first events may be missed |
| A2 | Per-branch delivery order is guaranteed even though the overview only requires "no ordering guarantee" across Sessions | Judgment (free from Dispatcher order) | Consumers must not rely on per-branch order either |
| A3 | Overflow drops records with a `Gap` notice instead of closing the subscription | Overview §7.2 (no ordering guarantee, no resumable reads); policy is judgment | Slow consumers would lose the subscription and resubscribe |
| A4 | Filters are by event type key, Session id and branch id only | Judgment from the observation needs listed in overview §3.3 | Richer filters (agent id, payload predicates) added later |
| A5 | Subscribers choose their own queue capacity; there is no global limit | Judgment | A shared default and cap would be defined here |

## 14. Open questions

| Question | Description |
| --- | --- |
| Overview §13 "Process-level subscription" | This contract proposes reusing the Session broadcast; maintainers confirm |
| Branch-end notice | No broadcast record marks a single branch's close today; add one only if a consumer needs it beyond `BranchMissing` from session-access |
| Remote nodes | Whether a subscription spans Sessions on other control nodes (overview §3.1 future extension) |
