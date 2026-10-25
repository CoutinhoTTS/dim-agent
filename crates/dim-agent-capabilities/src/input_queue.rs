use crate::{
    methods::{Interruption, MethodDecl, SessionMethods, params, reply},
    timeline_tree::TreeCommands,
};
use dim_agent_runtime::{
    AgentError, Dispatcher, InputSnapshot, Names, Result, StateCell, Timeline,
};
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, DepKey, Effect, Keyed, TypedPlugin};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

/// What InputQueue writes for an input it decided on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Decision {
    Apply(String, InputMode),
    Delete(String),
}
impl Decision {
    fn input_id(&self) -> &str {
        match self {
            Self::Apply(id, _) | Self::Delete(id) => id,
        }
    }
}

/// InputQueue's answer to one `input/withdraw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Withdrawal {
    /// The input was held: `input/deleted` at this sequence, never applied.
    Deleted { sequence: u64 },
    /// Unknown, already applied or already deleted: nothing was written.
    NotHeld,
}

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
    /// Folds one event; returns the decisions to write now, in order.
    fn step(&mut self, event: &AgentEvent) -> Vec<Decision> {
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
            // Only an input still held unapplied can be withdrawn; once
            // applied it belongs to Loop.
            EventKind::InputWithdraw(data) => {
                match self
                    .entries
                    .iter()
                    .position(|entry| entry.id == data.input_id && !entry.released)
                {
                    Some(index) => {
                        self.entries.remove(index);
                        vec![Decision::Delete(data.input_id.clone())]
                    }
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }
    /// Releases one input. One that drives a turn while none runs makes a
    /// turn about to start.
    fn release(&mut self, id: &str) -> Vec<Decision> {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) else {
            return Vec::new();
        };
        entry.released = true;
        if !self.turn_open && entry.mode != InputMode::Append {
            self.starting = true;
        }
        vec![Decision::Apply(entry.id.clone(), entry.mode)]
    }
}
/// Consumes `input/message` and decides by its mode when Loop may consume
/// it, writing `input/applied` with the mode then: steer and append inputs
/// at once; a queue input when no turn runs or is about to start,
/// otherwise after the turn ends, one per turn. Knows of turns only through `turn/start`,
/// `turn/end` and `stop/interrupt`; Loop decides how to consume what is
/// applied. Consumes `input/withdraw` in order and writes `input/deleted`
/// for an input it still holds unapplied.
pub struct InputQueue {
    state: Mutex<QueueState>,
    observable: Arc<StateCell<InputSnapshot>>,
    timeline: Arc<Timeline>,
    /// Answers to `input/withdraw` commands by their sequence, until the
    /// withdrawal that wrote each one collects it.
    answers: Mutex<HashMap<u64, Withdrawal>>,
    answered: tokio::sync::Notify,
    /// Ends waits when this generation unloads.
    lifecycle: CancellationToken,
}
impl InputQueue {
    fn publish(&self, state: &QueueState) {
        self.observable.set(InputSnapshot {
            queued: state.entries.len(),
        });
    }
    /// Writes the decisions; returns the sequence of the last `input/deleted`.
    fn write(&self, decisions: Vec<Decision>) -> Result<Option<u64>> {
        let mut deleted = None;
        for decision in decisions {
            match decision {
                Decision::Apply(input_id, mode) => {
                    self.timeline.append(
                        EventKind::InputApplied(InputAppliedData {
                            input_id,
                            mode: Some(mode),
                        })
                        .into(),
                    )?;
                }
                Decision::Delete(input_id) => {
                    let event = self
                        .timeline
                        .append(EventKind::InputDeleted(InputDeletedData { input_id }).into())?;
                    deleted = Some(event.sequence);
                }
            }
        }
        Ok(deleted)
    }
    fn on_event(&self, event: Arc<AgentEvent>) -> Result<()> {
        let decisions = {
            let mut state = self.state.lock().unwrap();
            let decisions = state.step(&event);
            self.publish(&state);
            decisions
        };
        let deleted = self.write(decisions)?;
        if matches!(event.kind, EventKind::InputWithdraw(_)) {
            let answer = match deleted {
                Some(sequence) => Withdrawal::Deleted { sequence },
                None => Withdrawal::NotHeld,
            };
            self.answers.lock().unwrap().insert(event.sequence, answer);
            self.answered.notify_waiters();
        }
        Ok(())
    }
    /// Withdraws an input by its `input_id` and replies with this queue's
    /// decision for the command. With `steer`, the command and the new Steer
    /// input are one batch: the withdrawal takes effect only if the old
    /// input is still held, the new input is written either way.
    pub async fn withdraw(
        &self,
        command: InputWithdrawData,
        steer: Option<InputMessageData>,
    ) -> Result<(Withdrawal, Option<Arc<AgentEvent>>)> {
        command
            .validate_external()
            .map_err(|reason| AgentError::InvalidInput(reason.into()))?;
        let mut drafts = vec![EventKind::InputWithdraw(command).into()];
        if let Some(input) = steer {
            if input.mode != Some(InputMode::Steer) {
                return Err(AgentError::InvalidInput(
                    "a replacement input must be Steer".into(),
                ));
            }
            input
                .validate()
                .map_err(|reason| AgentError::InvalidInput(reason.into()))?;
            drafts.push(EventKind::InputMessage(input).into());
        }
        let mut events = self.timeline.append_batch_registered(drafts, |_| Ok(()))?;
        let replacement = (events.len() > 1).then(|| events.remove(1));
        let sequence = events[0].sequence;
        loop {
            let answered = self.answered.notified();
            if let Some(answer) = self.answers.lock().unwrap().remove(&sequence) {
                return Ok((answer, replacement));
            }
            tokio::select! {
                _ = answered => {}
                _ = self.lifecycle.cancelled() => {
                    return Err(AgentError::Reloading("InputQueue".into()));
                }
            }
        }
    }
}
pub struct InputQueuePlugin {
    state: DepKey<StateCell<InputSnapshot>>,
    queue: DepKey<InputQueue>,
}
impl InputQueuePlugin {
    pub fn new(names: &Names<'_>) -> std::result::Result<Self, CordisError> {
        Ok(Self {
            state: names.state_cell()?,
            queue: names.key()?,
        })
    }
}
impl TypedPlugin for InputQueuePlugin {
    type Deps = (Keyed<Timeline>, Keyed<Dispatcher>);
    fn name(&self) -> &str {
        "InputQueue"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(timeline), Keyed(dispatcher)): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let queue = Arc::new(InputQueue {
                state: Mutex::new(QueueState::default()),
                observable: Arc::new(StateCell::new(InputSnapshot::default())),
                timeline,
                answers: Mutex::new(HashMap::new()),
                answered: tokio::sync::Notify::new(),
                lifecycle: ctx.cancellation_token(),
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
                    "input/withdraw",
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
                let mut written = HashSet::new();
                for event in &events {
                    match &event.kind {
                        EventKind::InputApplied(data) => {
                            written.insert(data.input_id.clone());
                        }
                        EventKind::InputDeleted(data) => {
                            written.insert(data.input_id.clone());
                        }
                        _ => {}
                    }
                    decided.extend(state.step(event));
                }
                queue.publish(&state);
                decided
                    .into_iter()
                    .filter(|decision| !written.contains(decision.input_id()))
                    // An applied input still present was not consumed; Loop
                    // consumes only applied inputs, so this second check
                    // guards that invariant rather than filtering in practice.
                    .filter(|decision| match decision {
                        Decision::Apply(id, _) => state.entries.iter().any(|entry| entry.id == *id),
                        Decision::Delete(_) => true,
                    })
                    .collect::<Vec<_>>()
            };
            *rebuilding = events.last().map_or(0, |event| event.sequence);
            queue.write(owed)?;
            drop(rebuilding);
            ctx.provide_as(self.state.key().clone(), queue.observable.clone())?;
            ctx.provide_as(self.queue.key().clone(), queue)?;
            Ok(Effect::Done)
        })
    }
}

/// The Session method namespace InputQueue provides through `sessionMethods`.
pub const INPUT_METHODS: &str = "session.input";

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WithdrawParams {
    /// Read by Rpc to route the call; the provider already serves this Session.
    #[serde(rename = "sessionId")]
    _session_id: String,
    branch_id: String,
    input_id: String,
    #[serde(default)]
    reason: Option<String>,
    /// Re-submits the content as Steer in the same batch as the withdrawal.
    #[serde(default)]
    steer: Option<UserInput>,
}

/// Serves `session.input.withdraw` for every branch of its Session through
/// the branch's InputQueue, so Rpc names no owning service.
pub struct InputMethodsPlugin;
impl TypedPlugin for InputMethodsPlugin {
    type Deps = (Keyed<TreeCommands>, Keyed<SessionMethods>);
    fn name(&self) -> &str {
        "InputMethods"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(tree), Keyed(methods)): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            methods.register(
                ctx,
                INPUT_METHODS,
                vec![MethodDecl::new(
                    "session.input.withdraw",
                    true,
                    Interruption::Report,
                )],
                Arc::new(move |_, value, _cancel, _| {
                    let tree = tree.clone();
                    Box::pin(async move {
                        let p: WithdrawParams = params(value)?;
                        let queue = tree.get(&p.branch_id)?.service::<InputQueue>()?;
                        let steer = p.steer.map(|user| InputMessageData {
                            user: Some(user),
                            mode: Some(InputMode::Steer),
                            ..Default::default()
                        });
                        let (answer, replacement) = queue
                            .withdraw(
                                InputWithdrawData {
                                    input_id: p.input_id,
                                    reason: p.reason,
                                },
                                steer,
                            )
                            .await?;
                        let mut result = match answer {
                            Withdrawal::Deleted { sequence } => {
                                serde_json::json!({"status": "deleted", "sequence": sequence})
                            }
                            Withdrawal::NotHeld => serde_json::json!({"status": "notHeld"}),
                        };
                        if let Some(event) = replacement {
                            result["steer"] = serde_json::json!({"inputId": event.id, "sequence": event.sequence});
                        }
                        reply(result)
                    })
                }),
            )?;
            Ok(Effect::Done)
        })
    }
}
