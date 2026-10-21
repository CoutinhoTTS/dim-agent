use crate::{AgentError, Timeline};
use dim_agent_types::AgentEvent;
use dim_agent_types::EventKind;
use dim_agent_types::ExtensionEvent;
use std::sync::Arc;

/// A branch's write port for extension events (ADR 001, issue #2): parts in
/// other languages append events of their own domains through it without
/// receiving the branch's whole [`Timeline`]. The event's envelope was checked
/// when it was decoded; its payload is the producer's contract.
pub struct ExtensionEvents(Timeline);
impl ExtensionEvents {
    pub(crate) fn new(timeline: Timeline) -> Self {
        Self(timeline)
    }
    /// Appends one extension event to this branch and returns it with its
    /// sequence; it is published once its batch commits, like any event.
    pub fn append(&self, event: ExtensionEvent) -> Result<Arc<AgentEvent>, AgentError> {
        self.0.append(EventKind::Extension(event).into())
    }
}
