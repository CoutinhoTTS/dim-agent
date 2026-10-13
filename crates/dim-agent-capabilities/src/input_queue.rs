use dim_agent_runtime::{
    BranchScope, Capability, Dispatcher, InBranch, InputSnapshot, KeysOf, Result, StateCell,
    Timeline, scoped,
};
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::sync::{Arc, Mutex};

/// One committed input until Loop consumes it.
struct Entry {
    id: String,
    mode: InputMode,
    /// This queue decided to write its `input/applied`.
    released: bool,
}
/// What InputQueue derives from the events it consumes.
#[derive(Default)]
struct QueueState {
    entries: Vec<Entry>,
    /// Between a dispatched `turn/start` and its `turn/end`.
    turn_open: bool,
    /// An input released while no turn ran is about to start one: until
    /// its `turn/start`, queue inputs wait as if the turn ran. A
    /// `stop/interrupt` ends the wait, since inputs applied before it no
    /// longer start a turn.
    starting: bool,
}
impl QueueState {
    /// Folds one event; returns the inputs to apply now, in order.
    fn step(&mut self, event: &AgentEvent) -> Vec<(String, InputMode)> {
        match &event.kind {
            EventKind::InputMessage(data) => {
                let mode = data.effective_mode();
                self.entries.push(Entry {
                    id: event.id.clone(),
                    mode,
                    released: false,
                });
                let held_queue = self.entries.iter().any(|entry| {
                    !entry.released && entry.mode == InputMode::Queue && entry.id != event.id
                });
                match mode {
                    // A queue input waits for the running or starting turn
                    // to end, and behind any queue input still waiting.
                    InputMode::Queue if self.turn_open || self.starting || held_queue => Vec::new(),
                    _ => self.release(&event.id),
                }
            }
            EventKind::TurnStart(_) => {
                self.turn_open = true;
                self.starting = false;
                Vec::new()
            }
            EventKind::StopInterrupt(_) => {
                self.starting = false;
                Vec::new()
            }
            EventKind::TurnEnd(_) => {
                self.turn_open = false;
                // The next queue input runs after the turn, however it ended.
                match self
                    .entries
                    .iter()
                    .find(|entry| !entry.released && entry.mode == InputMode::Queue)
                {
                    Some(next) => {
                        let next = next.id.clone();
                        self.release(&next)
                    }
                    None => Vec::new(),
                }
            }
            EventKind::InputConsumed(data) => {
                self.entries.retain(|entry| entry.id != data.input_id);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
    /// Releases one input. One that drives a turn while none runs makes a
    /// turn about to start.
    fn release(&mut self, id: &str) -> Vec<(String, InputMode)> {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) else {
            return Vec::new();
        };
        entry.released = true;
        if !self.turn_open && entry.mode != InputMode::Append {
            self.starting = true;
        }
        vec![(entry.id.clone(), entry.mode)]
    }
}
/// Consumes `input/message` and decides by its mode when Loop may consume
/// it, writing `input/applied` with the mode then: steer and append inputs
/// at once; a queue input when no turn runs or is about to start,
/// otherwise after the turn ends, one per turn. Knows of turns only through `turn/start`,
/// `turn/end` and `stop/interrupt`; Loop decides how to consume what is
/// applied.
pub struct InputQueue {
    state: Mutex<QueueState>,
    observable: Arc<StateCell<InputSnapshot>>,
    timeline: Arc<Timeline>,
}
impl InputQueue {
    fn publish(&self, state: &QueueState) {
        self.observable.set(InputSnapshot {
            queued: state.entries.len(),
        });
    }
    fn apply(&self, inputs: Vec<(String, InputMode)>) -> Result<()> {
        for (input_id, mode) in inputs {
            self.timeline.append(
                EventKind::InputApplied(InputAppliedData {
                    input_id,
                    mode: Some(mode),
                })
                .into(),
            )?;
        }
        Ok(())
    }
    fn on_event(&self, event: Arc<AgentEvent>) -> Result<()> {
        let apply = {
            let mut state = self.state.lock().unwrap();
            let apply = state.step(&event);
            self.publish(&state);
            apply
        };
        self.apply(apply)
    }
}
pub struct InputQueuePlugin {
    scope: BranchScope,
}
impl InputQueuePlugin {
    pub fn new(scope: &BranchScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for InputQueuePlugin {
    type Deps = (InBranch<Timeline>, InBranch<Dispatcher>);
    fn name(&self) -> &str {
        "InputQueue"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InBranch(timeline), InBranch(dispatcher)): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let queue = Arc::new(InputQueue {
                state: Mutex::new(QueueState::default()),
                observable: Arc::new(StateCell::new(InputSnapshot::default())),
                timeline,
            });
            let gate = Arc::new(tokio::sync::Mutex::new(0u64));
            let mut rebuilding = gate.lock().await;
            let handler_gate = gate.clone();
            let owner = queue.clone();
            dispatcher.subscribe(
                ctx,
                "InputQueue",
                &[
                    "input/message",
                    "input/consumed",
                    "turn/start",
                    "turn/end",
                    "stop/interrupt",
                ],
                move |event| {
                    let owner = owner.clone();
                    let gate = handler_gate.clone();
                    Box::pin(async move {
                        if event.sequence > *gate.lock().await {
                            owner.on_event(event)?;
                        }
                        Ok(())
                    })
                },
            )?;
            let through = queue.timeline.committed();
            let events = queue.timeline.local(..=through).await?;
            // Replays the same decisions; those the previous generation made
            // but did not write before it stopped are written now.
            let owed = {
                let mut state = queue.state.lock().unwrap();
                let mut decided = Vec::new();
                let mut written = std::collections::HashSet::new();
                for event in &events {
                    if let EventKind::InputApplied(data) = &event.kind {
                        written.insert(data.input_id.clone());
                    }
                    decided.extend(state.step(event));
                }
                queue.publish(&state);
                decided
                    .into_iter()
                    // An input still present was not consumed; Loop consumes
                    // only applied inputs, so this second check guards that
                    // invariant rather than filtering in practice.
                    .filter(|(id, _)| {
                        !written.contains(id) && state.entries.iter().any(|entry| entry.id == *id)
                    })
                    .collect::<Vec<_>>()
            };
            *rebuilding = events.last().map_or(0, |event| event.sequence);
            queue.apply(owed)?;
            drop(rebuilding);
            ctx.provide_as(
                self.scope.key::<StateCell<InputSnapshot>>(),
                queue.observable.clone(),
            )?;
            ctx.provide_as(self.scope.key::<InputQueue>(), queue)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for InputQueuePlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}
