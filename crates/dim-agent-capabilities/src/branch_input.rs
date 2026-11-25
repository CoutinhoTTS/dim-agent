//! Automatic input a branch capability appends to its own branch. The
//! producer guarantees the input is valid; whether it starts a turn is
//! Loop's decision at dispatch.
use dim_agent_runtime::{
    AgentError, Dispatcher, QueryFilter, QueryOperand, QueryOutput, QueryPage, QueryPick,
    QuerySelect, QuerySpec, QueryValue, Result, Timeline, TimelineQuery, TimelineQueryScope,
};
use dim_agent_types::{AgentEvent, EventDraft, EventKind, InputMessageData};
use std::num::NonZeroU64;
use std::sync::Arc;

/// Appends `data` with `tags` to the capability's own Timeline, at load or
/// from its tasks afterwards.
pub(crate) fn append_own(
    timeline: &Timeline,
    data: InputMessageData,
    tags: Vec<String>,
) -> Result<Arc<AgentEvent>> {
    let mut draft = EventDraft::from(EventKind::InputMessage(data));
    if !tags.is_empty() {
        draft.tags = Some(tags);
    }
    timeline.append(draft)
}

/// [`append_own`] without tags, held until the branch dispatched the input:
/// the input queue has written its `input/applied` by then, so a branch
/// opening receipt that waits for this call seals after both.
pub(crate) async fn append_own_dispatched(
    timeline: &Timeline,
    dispatcher: &Dispatcher,
    data: InputMessageData,
) -> Result<Arc<AgentEvent>> {
    let event = append_own(timeline, data, Vec::new())?;
    let mut dispatched = dispatcher.watch();
    loop {
        {
            let state = dispatched.borrow_and_update();
            if let Some(error) = state.failure.clone() {
                return Err(error);
            }
            if state.sequence >= event.sequence {
                return Ok(event);
            }
        }
        dispatched
            .changed()
            .await
            .map_err(|_| AgentError::Closed("branch dispatch".into()))?;
    }
}

/// Whether this branch started under a parent line: TimelineTree writes that
/// line's identity input after the opening seals, so it is the line's first
/// input and a capability's opening receipt appends nothing ahead of it. The
/// published branch's first pass appends there instead.
///
/// The answer is this branch's own opening fact, the last `branch/start` or
/// `branch/resume` in its Timeline, so a receipt reads it when it writes and
/// does not depend on subscribing before the event was dispatched. The read
/// stops at the published head: an opening receipt runs once its branch
/// dispatched the opening fact, and dispatch advances only over published
/// events, so that fact is published then; a later batch that is committed
/// but not yet published does not turn the read into an error. A resumed line
/// has no identity input to wait for; only a start that names a parent opens
/// with one. `Err` is the caller's to handle: a receipt leaves its input to
/// the published branch rather than deciding on a fact it cannot read, and
/// the error must not fail the opening.
pub(crate) async fn opens_with_identity_input(timeline: &Timeline) -> Result<bool> {
    let reply = timeline
        .query(TimelineQuery {
            scope: TimelineQueryScope::Branch,
            from: None,
            through: None,
            include_publishing: false,
            query: QuerySpec {
                filter: Some(QueryFilter::In {
                    value: QueryOperand::Field {
                        path: "kind".into(),
                    },
                    values: vec!["branch/start".into(), "branch/resume".into()],
                }),
                select: QuerySelect::Fields {
                    fields: vec!["kind".into(), "data.parent".into()],
                },
                group_by: Vec::new(),
                pick: Some(QueryPick::Last),
                order_by: Vec::new(),
            },
            page: Some(QueryPage {
                limit: NonZeroU64::MIN,
                after: None,
            }),
        })
        .await?;
    let QueryOutput::Rows { rows } = reply.result else {
        unreachable!("the opening event selects fields")
    };
    Ok(rows.first().is_some_and(|row| {
        matches!(row.values.as_slice(),
            [
                QueryValue::Value {
                    value: serde_json::Value::String(kind),
                },
                parent,
            ] if kind == "branch/start" && !matches!(parent, QueryValue::Missing))
    }))
}
