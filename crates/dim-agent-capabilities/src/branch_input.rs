//! Automatic input a branch capability appends to its own branch. The
//! producer guarantees the input is valid; whether it starts a turn is
//! Loop's decision at dispatch.
use dim_agent_runtime::{AgentError, Dispatcher, Result, Timeline};
use dim_agent_types::{AgentEvent, EventDraft, EventKind, InputMessageData};
use rutis::Ctx;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

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
pub(crate) fn opens_with_identity_input(
    dispatcher: &Dispatcher,
    ctx: &Ctx,
    name: &str,
) -> Result<Arc<AtomicBool>> {
    let opened = Arc::new(AtomicBool::new(false));
    let seen = opened.clone();
    dispatcher.subscribe(ctx, name, &["branch/start"], move |event| {
        if matches!(&event.kind, EventKind::BranchStart(data) if data.parent.is_some()) {
            seen.store(true, Ordering::Release);
        }
        Box::pin(async { Ok(()) })
    })?;
    Ok(opened)
}
