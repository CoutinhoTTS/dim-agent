//! Automatic input a branch capability appends to its own branch. The
//! producer guarantees the input is valid; whether it starts a turn is
//! Loop's decision at dispatch.
use dim_agent_runtime::{Result, Timeline};
use dim_agent_types::{AgentEvent, EventDraft, EventKind, InputMessageData};
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
