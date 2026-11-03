use crate::{
    BoxFuture, BranchRecord, Result, SessionConfig, TimelineQuery, TimelineQueryPage, TimelineTrace,
};
use dim_agent_types::{
    AgentEvent, ContextCompactionResult, EventKind, RecoverySnapshot, TimelinePosition,
};
use std::{ops::RangeInclusive, sync::Arc};

/// Storage address of one Timeline inside an Agent's shared store.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimelineKey {
    pub session_id: String,
    pub branch_id: String,
}

/// The events `ContextPolicy` needs, without the rest of the Timeline.
#[derive(Debug, Clone, Default)]
pub struct PolicyFacts {
    pub through: u64,
    /// Root first, then sequence: the branch chain's order.
    pub events: Vec<PolicyEvent>,
}

#[derive(Debug, Clone)]
pub struct PolicyEvent {
    pub position: TimelinePosition,
    pub event: Arc<AgentEvent>,
}

impl PolicyFacts {
    /// Builds facts from an already decoded context, for callers that have it.
    pub fn from_context(context: &crate::ContextHistory, through: u64) -> Self {
        Self {
            through,
            events: context
                .events
                .iter()
                .zip(context.positions.iter())
                .filter(|(event, _)| policy_event(event))
                .map(|(event, position)| PolicyEvent {
                    position: position.clone(),
                    event: event.clone(),
                })
                .collect(),
        }
    }
    /// The facts' events without their positions.
    pub fn arcs(&self) -> Vec<Arc<AgentEvent>> {
        self.events.iter().map(|fact| fact.event.clone()).collect()
    }
}

/// The events a compaction cut needs for pinning: its anchor plus the turn's
/// input window. The caller supplies the cut, so no full history is decoded.
#[derive(Debug, Clone, Default)]
pub struct PolicyWindow {
    pub through: u64,
    pub cut: String,
    /// The cut's own chain position, when the window read found its row.
    pub cut_position: Option<TimelinePosition>,
    pub events: Vec<Arc<AgentEvent>>,
    /// The `step/tool-result` events from the previous cut on, each with the
    /// branch and sequence that place it before or after the new cut: a
    /// window merges ancestor and current-branch results.
    pub tool_results: Vec<PolicyEvent>,
}

/// The event ids and turn starts a compaction needs to enumerate legal cuts,
/// without decoding turn bodies.
#[derive(Debug, Clone, Default)]
pub struct PolicyOrder {
    pub through: u64,
    /// Branch chain from root to the current branch; positions compare by it.
    pub branches: Vec<String>,
    pub events: Vec<PolicyOrderEvent>,
}

#[derive(Debug, Clone)]
pub struct PolicyOrderEvent {
    pub id: String,
    pub branch_id: String,
    pub sequence: u64,
    pub turn_start: bool,
    pub compaction_start: Option<u64>,
    /// The branch's last event at its fixed cut: the cut that replaces everything.
    pub boundary: bool,
}

fn window_event(event: &AgentEvent) -> bool {
    matches!(
        &event.kind,
        EventKind::TurnStart(_) | EventKind::InputMessage(_) | EventKind::InputConsumed(_)
    )
}

fn order_event(event: &AgentEvent, key: &TimelineKey) -> PolicyOrderEvent {
    PolicyOrderEvent {
        id: event.id.clone(),
        branch_id: key.branch_id.clone(),
        sequence: event.sequence,
        turn_start: matches!(&event.kind, EventKind::TurnStart(_)),
        compaction_start: None,
        boundary: false,
    }
}

fn policy_event(event: &AgentEvent) -> bool {
    matches!(
        &event.kind,
        EventKind::ContextConfigured(_)
            | EventKind::ContextCompaction(_)
            | EventKind::ContextBudget(_)
            | EventKind::ModelUsage(_)
            | EventKind::ModelCall(_)
            | EventKind::TurnStart(_)
            | EventKind::InputMessage(_)
            | EventKind::InputConsumed(_)
    )
}

/// One stored event and its atomic publication boundary.
#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub event: Arc<AgentEvent>,
    pub trace: Option<TimelineTrace>,
    pub batch_end: u64,
}

#[derive(Debug, Clone)]
pub struct StoredBranch {
    pub info: BranchRecord,
    pub sequence: u64,
}

pub type PublishCallback = Box<dyn FnOnce(Result<()>) + Send + 'static>;

/// Agent-shared event storage. Append queues a complete atomic batch without
/// waiting for I/O; an accepted batch's callback runs once after durable commit
/// or failure.
/// Callbacks for one branch run in sequence order. A failed Session rejects
/// subsequent batches, while other Sessions continue. `head`, `read`, and
/// each `read_segments` segment require an existing Session and branch, even
/// for an empty range; missing keys return a storage error. `append` rejects
/// missing keys immediately or through its publish callback and never creates
/// a directory.
/// Each Session id has at most one live load per store; callers serialize
/// closing and restoring that id, including when Agents share a store.
pub trait TimelineStore: Send + Sync + 'static {
    fn create_session<'a>(
        &'a self,
        session_id: &'a str,
        config: &'a SessionConfig,
    ) -> BoxFuture<'a, Result<()>>;
    /// Removes a newly committed Session after initialization has failed.
    /// The caller first stops its producers; completion drains prior writes and
    /// removes its directory, ownership, events, branches, and content atomically.
    /// Never used for restore failures or ordinary Session close.
    fn rollback_session<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, Result<()>>;
    /// Releases runtime admission state after all Session producers stop.
    /// A running store drains prior writes and clears this load's failure state
    /// without deleting stored history, so a later restore can write again.
    /// A closing or stopped store treats release as already settled; shutdown
    /// remains responsible for pending writes and faults. Append failures remain
    /// observable through their own results.
    /// Idempotent: a Session with no admission state is already released.
    fn release_session<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, Result<()>>;
    fn session_config<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<SessionConfig>>>;
    /// Returns the Session's branch directory after checking the stored
    /// Timeline. An implementation may reuse a check it already ran for this
    /// Session in the same store while the stored directory and every branch
    /// head are unchanged. The first check of a Session in a process reads the
    /// whole Timeline.
    fn branches<'a>(&'a self, session_id: &'a str) -> BoxFuture<'a, Result<Vec<StoredBranch>>>;
    fn head<'a>(&'a self, key: &'a TimelineKey) -> BoxFuture<'a, Result<u64>>;
    fn append(
        &self,
        key: TimelineKey,
        events: Vec<StoredEvent>,
        publish: PublishCallback,
    ) -> Result<()>;
    /// Checks that `sequence` ends an atomic batch, without reading the event:
    /// whether a sequence closes a batch is the stored row's structure, not
    /// event content (ADR 020 D4). Zero, before the branch's first event, ends
    /// a batch. An interior sequence is invalid input; a sequence the branch
    /// does not store is a storage error.
    fn batch_end<'a>(&'a self, key: &'a TimelineKey, sequence: u64) -> BoxFuture<'a, Result<()>>;
    /// Range starts may be inside a batch; the end must be a batch boundary.
    fn read<'a>(
        &'a self,
        key: &'a TimelineKey,
        range: RangeInclusive<u64>,
    ) -> BoxFuture<'a, Result<Vec<StoredEvent>>> {
        Box::pin(async move {
            let mut result = self
                .read_segments(&key.session_id, vec![(key.branch_id.clone(), range)])
                .await?;
            Ok(result.pop().expect("one range returns one segment"))
        })
    }
    /// Reads complete batches after a batch-boundary cursor at a fixed head.
    /// The event limit is a target: the final batch may exceed it.
    fn read_page<'a>(
        &'a self,
        key: &'a TimelineKey,
        after: u64,
        limit: usize,
        through: u64,
    ) -> BoxFuture<'a, Result<Vec<StoredEvent>>> {
        Box::pin(async move {
            if after > through {
                return Err(crate::AgentError::InvalidInput(
                    "future Timeline cursor".into(),
                ));
            }
            let records = self.read(key, after.max(1)..=through).await?;
            if after > 0
                && records
                    .first()
                    .is_some_and(|record| record.batch_end != after)
            {
                return Err(crate::AgentError::InvalidInput(
                    "cursor is inside an atomic batch".into(),
                ));
            }
            if limit == 0 {
                return Ok(vec![]);
            }
            let mut records: Vec<_> = records
                .into_iter()
                .filter(|record| record.event.sequence > after)
                .collect();
            if let Some(record) = records.get(limit - 1) {
                let end = record.batch_end;
                records.retain(|record| record.event.sequence <= end);
            }
            Ok(records)
        })
    }
    /// The event at `sequence`, even when an atomic batch crosses it: the read
    /// runs to the branch head, which is a batch boundary, and picks the last
    /// event at or before the cut.
    fn last_event<'a>(
        &'a self,
        key: &'a TimelineKey,
        sequence: u64,
    ) -> BoxFuture<'a, Result<Option<Arc<AgentEvent>>>> {
        Box::pin(async move {
            let head = self.head(key).await?;
            if head == 0 {
                return Ok(None);
            }
            let records = self.read(key, 1..=head).await?;
            Ok(records
                .into_iter()
                .map(|record| record.event)
                .rfind(|event| event.sequence <= sequence))
        })
    }
    /// Reads complete batches after a batch-boundary cursor at a fixed head
    /// for a caller that does not use stored requests: a `context/prepare`
    /// or `model/call` record holds its request without its messages,
    /// `tools` and `system`, and the content rows behind them are neither
    /// loaded nor checked. Paging and projection reads discard these
    /// records or use only their other fields
    /// ([request delta storage §7.5](../../../docs/design/appserver/request-delta-storage.md#75-reads-that-do-not-use-request-bodies)).
    /// The default rebuilds them, exactly as [`Self::read_page`] does.
    fn read_page_without_request_bodies<'a>(
        &'a self,
        key: &'a TimelineKey,
        after: u64,
        limit: usize,
        through: u64,
    ) -> BoxFuture<'a, Result<Vec<StoredEvent>>> {
        self.read_page(key, after, limit, through)
    }
    /// One per-branch read for [`TimelineQuery`] (ADR 019, plugin contract
    /// §5.4): applies the read point, direction, kinds, flags, event id,
    /// predicate, limit and offset to `key`'s branch and returns up to `limit`
    /// matching facts, ordered by sequence in the query's direction, each with
    /// its position.
    ///
    /// `query.scope` must be `TimelineQueryScope::Branch`. The Timeline
    /// service composes the chain by reading each ancestor and then the
    /// queried branch with this method, carrying the offset the earlier
    /// segment did not use. Paging is skipping: `offset` matching facts are
    /// passed over before the page, so a page shorter than the limit means the
    /// selection is exhausted, and a page may end inside an atomic batch.
    /// `skipped` is the number of matching facts the read passed over, at most
    /// `offset`, so a caller can carry the rest into its next segment.
    ///
    /// A read point past the branch's published head and any other scope are
    /// invalid input. A kind, flag, id or data path the store does not know
    /// selects no rows; it is not an error.
    fn query<'a>(
        &'a self,
        key: &'a TimelineKey,
        query: &'a TimelineQuery,
        from: u64,
        through: u64,
    ) -> BoxFuture<'a, Result<TimelineQueryPage>>;
    /// Decodes all segments in one read scope, sharing content across branches.
    fn read_segments<'a>(
        &'a self,
        session_id: &'a str,
        segments: Vec<(String, RangeInclusive<u64>)>,
    ) -> BoxFuture<'a, Result<Vec<Vec<StoredEvent>>>>;
    /// The policy-relevant events at `through`. Stores may filter before
    /// decoding; the default reads the prefix and keeps the known kinds.
    fn policy_facts<'a>(
        &'a self,
        key: &'a TimelineKey,
        through: u64,
        turn_id: &'a str,
    ) -> BoxFuture<'a, Result<PolicyFacts>> {
        Box::pin(async move {
            if through == 0 {
                return Ok(PolicyFacts::default());
            }
            let records = self.read(key, 1..=through).await?;
            let latest = |predicate: &dyn Fn(&EventKind) -> bool| {
                records
                    .iter()
                    .rev()
                    .find(|record| predicate(&record.event.kind))
                    .map(|record| record.event.sequence)
            };
            let configured = latest(&|kind| matches!(kind, EventKind::ContextConfigured(_)));
            let compaction = records
                .iter()
                .rev()
                .find_map(|record| match &record.event.kind {
                    EventKind::ContextCompaction(data)
                        if matches!(
                            data.outcome,
                            dim_agent_types::ContextCompactionResult::Completed { .. }
                        ) =>
                    {
                        Some(record.event.sequence)
                    }
                    _ => None,
                });
            let usage = records
                .iter()
                .rev()
                .find_map(|record| match &record.event.kind {
                    EventKind::ModelUsage(data) => {
                        Some((record.event.sequence, data.request_id.clone()))
                    }
                    _ => None,
                });
            let call = usage.as_ref().and_then(|(_, request_id)| {
                records
                    .iter()
                    .rev()
                    .find_map(|record| match &record.event.kind {
                        EventKind::ModelCall(data) if data.request_id == *request_id => {
                            Some(record.event.sequence)
                        }
                        _ => None,
                    })
            });
            let turn = records
                .iter()
                .rev()
                .find_map(|record| match &record.event.kind {
                    EventKind::TurnStart(data) if data.turn_id == turn_id => {
                        Some(record.event.sequence)
                    }
                    _ => None,
                });
            let consumed: Vec<String> = turn.map_or_else(Vec::new, |turn| {
                records
                    .iter()
                    .filter(|record| record.event.sequence >= turn)
                    .filter_map(|record| match &record.event.kind {
                        EventKind::InputConsumed(data) => Some(data.input_id.clone()),
                        _ => None,
                    })
                    .collect()
            });
            Ok(PolicyFacts {
                through,
                events: records
                    .into_iter()
                    .filter(|record| {
                        let sequence = record.event.sequence;
                        Some(sequence) == configured
                            || Some(sequence) == compaction
                            || usage.as_ref().is_some_and(|(usage, _)| *usage == sequence)
                            || Some(sequence) == call
                            || Some(sequence) == turn
                            || turn.is_some_and(|turn| {
                                sequence >= turn
                                    && matches!(
                                        record.event.kind,
                                        EventKind::TurnStart(_)
                                            | EventKind::InputMessage(_)
                                            | EventKind::InputConsumed(_)
                                            | EventKind::ContextBudget(_)
                                    )
                            })
                            || consumed.iter().any(|id| record.event.id == *id)
                    })
                    .map(|record| PolicyEvent {
                        position: TimelinePosition {
                            branch_id: key.branch_id.clone(),
                            sequence: record.event.sequence,
                        },
                        event: record.event,
                    })
                    .collect(),
            })
        })
    }

    /// The event order a compaction derives its legal cuts from: this
    /// segment's last `turn/start` and last `context/compaction-start` at or
    /// before `through`, plus the `turn/start` events in `turns`. Events are
    /// ordered by sequence and each appears once.
    /// A store answers the turn starts without decoding their bodies; only
    /// the compaction start's `through` is read.
    fn policy_order<'a>(
        &'a self,
        key: &'a TimelineKey,
        turns: RangeInclusive<u64>,
        through: u64,
    ) -> BoxFuture<'a, Result<PolicyOrder>> {
        Box::pin(async move {
            let records = self.read(key, 1..=through).await?;
            let mut events = Vec::new();
            let mut last_turn = None;
            let mut last_start = None;
            for record in &records {
                match &record.event.kind {
                    EventKind::TurnStart(_) => {
                        last_turn = Some(record);
                        if turns.contains(&record.event.sequence) {
                            events.push(order_event(&record.event, key));
                        }
                    }
                    EventKind::ContextCompactionStart(data) => {
                        let mut event = order_event(&record.event, key);
                        event.compaction_start = Some(data.through);
                        last_start = Some(event);
                    }
                    _ => {}
                }
            }
            events.extend(
                last_turn
                    .filter(|record| !turns.contains(&record.event.sequence))
                    .map(|record| order_event(&record.event, key)),
            );
            events.extend(last_start);
            events.sort_by_key(|event| event.sequence);
            Ok(PolicyOrder {
                through,
                branches: vec![key.branch_id.clone()],
                events,
            })
        })
    }

    /// The anchor and turn window for a known compaction cut.
    fn policy_window<'a>(
        &'a self,
        key: &'a TimelineKey,
        through: u64,
        _turn_id: &'a str,
        cut: &'a str,
    ) -> BoxFuture<'a, Result<PolicyWindow>> {
        Box::pin(async move {
            let records = self.read(key, 1..=through).await?;
            let previous_cut = records
                .iter()
                .rev()
                .find_map(|record| match &record.event.kind {
                    EventKind::ContextCompaction(data) => match &data.outcome {
                        ContextCompactionResult::Completed { retain_from, .. } => {
                            Some(retain_from.as_str())
                        }
                        _ => None,
                    },
                    _ => None,
                });
            let previous_sequence = previous_cut.and_then(|cut| {
                records
                    .iter()
                    .find(|record| record.event.id == cut)
                    .map(|record| record.event.sequence)
            });
            let mut events = Vec::new();
            let mut tool_results = Vec::new();
            let mut cut_position = None;
            for record in records {
                let position = TimelinePosition {
                    branch_id: key.branch_id.clone(),
                    sequence: record.event.sequence,
                };
                if matches!(&record.event.kind, EventKind::ToolResult(_))
                    && previous_sequence.is_none_or(|cut| record.event.sequence >= cut)
                {
                    tool_results.push(PolicyEvent {
                        position: position.clone(),
                        event: record.event.clone(),
                    });
                }
                if record.event.id == cut {
                    cut_position = Some(position);
                }
                if record.event.id == cut || window_event(&record.event) {
                    events.push(record.event);
                }
            }
            Ok(PolicyWindow {
                through,
                cut: cut.to_owned(),
                cut_position,
                events,
                tool_results,
            })
        })
    }

    fn trace<'a>(
        &'a self,
        key: &'a TimelineKey,
        sequence: u64,
    ) -> BoxFuture<'a, Result<Option<TimelineTrace>>>;
    /// Optional acceleration only. Save after the anchoring fact commits;
    /// failures must not roll back or poison the fact stream.
    fn save_snapshot<'a>(
        &'a self,
        _key: &'a TimelineKey,
        _snapshot: RecoverySnapshot,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    /// Return the latest compatible candidate without interpreting its state.
    /// The caller validates the decoded resume position before trimming history.
    fn latest_snapshot<'a>(
        &'a self,
        _key: &'a TimelineKey,
        _through: u64,
        _interpreter: &'a str,
    ) -> BoxFuture<'a, Result<Option<RecoverySnapshot>>> {
        Box::pin(async { Ok(None) })
    }
}
