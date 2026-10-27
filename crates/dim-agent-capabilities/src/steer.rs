use dim_agent_types::*;

/// Reason of the Cancelled terminal written for a request that a steer input
/// replaced before it had any model output (ADR agent-029 decision 5).
pub const REPLACED_BY_STEER: &str = "replaced by steer";

/// Folded by the owners of a request's preparation and model call from the
/// events they consume: the latest `step/model-start` and whether an input
/// that drives the Loop was applied after it. Such an input is never part of
/// that request: inputs a request consumes are applied before its
/// `step/model-start`. A request handed over after such an input was
/// replaced while its previous owner still held it.
#[derive(Default)]
pub(crate) struct SteerMark(Option<(String, bool)>);
impl SteerMark {
    pub fn apply(&mut self, event: &AgentEvent) {
        match &event.kind {
            EventKind::ModelStart(data) => self.0 = Some((data.request_id.clone(), false)),
            EventKind::InputApplied(data) if data.mode.unwrap_or_default() != InputMode::Append => {
                if let Some((_, steered)) = self.0.as_mut() {
                    *steered = true;
                }
            }
            _ => {}
        }
    }
    /// A driving input was applied after `request_id` started.
    pub fn steered(&self, request_id: &str) -> bool {
        matches!(&self.0, Some((request, true)) if request == request_id)
    }
}
