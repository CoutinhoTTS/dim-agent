//! BackgroundExec: this branch's detached `exec` commands, see
//! `docs/architecture/rust-agent/plugins/BackgroundExec.md` and the detached
//! work contract of `event-driven-flows.md` §3. The capability owns the task
//! table, writes `exec/started` / `exec/ended`, and wakes the branch at the
//! dispatch of every `exec/ended` that is not `cancelled`. Each running task
//! holds its Session (`SessionLifecycle::hold`) until its `exec/ended` is
//! appended, so an idle unload never ends it (idle unload §5, R2).
use super::exec::{ExecProcess, recovered_result};
use crate::context_block;
use dim_agent_runtime::{
    AgentError, BranchScope, Capability, Dispatcher, InBranch, InSession, KeysOf, SessionLifecycle,
    Timeline, scoped,
};
use dim_agent_types::*;
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub struct StartRequest {
    pub turn_id: String,
    pub request_id: String,
    pub call_id: String,
    /// Proposed id; a `before` hook may rewrite it, the final one is returned.
    pub task_id: String,
    pub description: String,
    pub output_path: String,
}

#[derive(Debug)]
pub enum StartError {
    /// The branch or Session is closing, or this instance is unloading.
    Closing,
    /// `exec/started` was not written.
    Timeline(AgentError),
    /// Rejected handover could not confirm process-tree termination.
    Termination(Box<ToolResult>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskError {
    /// Not a task of this branch.
    Unknown,
    /// This instance was unloaded.
    Closed,
}

struct Task {
    description: String,
    cancel: CancellationToken,
    /// The dispatched `exec/ended`, once its wake decision is done.
    ended: watch::Sender<Option<ExecEndedData>>,
}
impl Task {
    fn new(description: String, cancel: CancellationToken) -> Self {
        Self {
            description,
            cancel,
            ended: watch::Sender::new(None),
        }
    }
}

#[derive(Default)]
struct State {
    /// Rejects new starts.
    closing: bool,
    closed: bool,
    tasks: HashMap<String, Task>,
}

pub struct BackgroundExec {
    timeline: Arc<Timeline>,
    /// Gives each task its Session hold.
    session: Arc<SessionLifecycle>,
    /// Parent of every task's cancellation; cancelled on unload and branch close.
    lifecycle: CancellationToken,
    /// Supervising tasks, each holding one process until `exec/ended`.
    tasks: TaskTracker,
    state: Mutex<State>,
}

impl BackgroundExec {
    /// Records the task with `exec/started` and supervises `process`; on
    /// failure the process is terminated and reaped before returning.
    pub async fn start(
        self: &Arc<Self>,
        request: StartRequest,
        process: ExecProcess,
    ) -> Result<String, StartError> {
        #[cfg(feature = "testing")]
        testing::pause(&request.output_path).await;
        // Counted by the tracker, so unload waits for this handover.
        let _handover = self.tasks.token();
        // Taken while the tool call keeps the turn open, before `exec/started`.
        // `None` once the Session is closing: refused like a closing branch.
        let hold = if self.state.lock().unwrap().closing {
            None
        } else {
            self.session.hold()
        };
        let Some(hold) = hold else {
            process.abandon().await.map_err(StartError::Termination)?;
            return Err(StartError::Closing);
        };
        let cancel = self.lifecycle.child_token();
        let description = request.description.clone();
        let mut registered = None;
        let committed = self.timeline.append_registered(
            EventKind::ExecStarted(ExecStartedData {
                turn_id: request.turn_id,
                request_id: request.request_id,
                call_id: request.call_id,
                task_id: request.task_id,
                description: request.description,
                output_path: request.output_path,
            })
            .into(),
            |event| {
                let EventKind::ExecStarted(data) = &event.kind else {
                    unreachable!("exec/started kind is preserved")
                };
                let mut state = self.state.lock().unwrap();
                if state.closing {
                    return Err(AgentError::Closed("BackgroundExec".into()));
                }
                state.tasks.insert(
                    data.task_id.clone(),
                    Task::new(description.clone(), cancel.clone()),
                );
                registered = Some(data.task_id.clone());
                Ok(())
            },
        );
        let event = match committed {
            Ok(event) => event,
            Err(error) => {
                // Registered but not committed: no `exec/started` was written,
                // so the entry goes too. Dropping its sender ends a `stop`
                // that found it meanwhile as Closed.
                if let Some(task_id) = registered {
                    self.state.lock().unwrap().tasks.remove(&task_id);
                }
                process.abandon().await.map_err(StartError::Termination)?;
                return Err(if self.state.lock().unwrap().closing {
                    StartError::Closing
                } else {
                    StartError::Timeline(error)
                });
            }
        };
        let EventKind::ExecStarted(data) = &event.kind else {
            unreachable!("exec/started kind is preserved")
        };
        let task_id = data.task_id.clone();
        let owner = self.clone();
        let supervised = task_id.clone();
        self.tasks.spawn(async move {
            let (status, result) = process.finish_background(&supervised, cancel).await;
            let ended = owner.timeline.append(
                EventKind::ExecEnded(ExecEndedData {
                    task_id: supervised.clone(),
                    status,
                    result,
                })
                .into(),
            );
            // Released only once `exec/ended` is appended: the branch stays
            // busy until its dispatch has posted the wake, so no idle unload
            // comes between the end and the woken turn.
            drop(hold);
            // A later generation's rebuild writes the missing terminal.
            // Dropping the task's sender ends a waiting `stop` as Closed.
            if let Err(error) = ended {
                tracing::warn!(%error, task_id = supervised, "exec/ended was not written");
                owner.state.lock().unwrap().tasks.remove(&supervised);
            }
        });
        Ok(task_id)
    }

    /// Stops a task: cancels it while running, so it ends `cancelled` and
    /// does not wake. Returns once its `exec/ended` is dispatched; a task that
    /// already ended keeps its result and its wake.
    pub async fn stop(&self, task_id: &str) -> Result<ExecEndedData, TaskError> {
        let mut ended = {
            let mut state = self.state.lock().unwrap();
            if state.closed {
                return Err(TaskError::Closed);
            }
            let task = state.tasks.get_mut(task_id).ok_or(TaskError::Unknown)?;
            if let Some(done) = task.ended.borrow().clone() {
                return Ok(done);
            }
            task.cancel.cancel();
            task.ended.subscribe()
        };
        ended
            .wait_for(Option::is_some)
            .await
            .map(|done| done.clone().expect("waited for Some"))
            .map_err(|_| TaskError::Closed)
    }

    fn consume(
        self: &Arc<Self>,
        event: Arc<AgentEvent>,
    ) -> BoxFuture<'static, dim_agent_runtime::Result<()>> {
        match &event.kind {
            EventKind::StopInterrupt(data) if data.scope == InterruptScope::Branch => {
                // Branch close: every task ends cancelled, without a wake.
                self.state.lock().unwrap().closing = true;
                self.lifecycle.cancel();
            }
            EventKind::ExecEnded(data) => {
                let this = self.clone();
                let data = data.clone();
                return Box::pin(async move { this.ended(&event.id, data).await });
            }
            _ => {}
        }
        Box::pin(async { Ok(()) })
    }

    /// The wake of one dispatched `exec/ended`, decided by its status and
    /// position: every status but `cancelled` (a stop, branch close or
    /// unload) wakes, including the `failed` terminal a later generation
    /// wrote, unless the end comes after the branch-close interrupt.
    async fn ended(&self, event_id: &str, data: ExecEndedData) -> dim_agent_runtime::Result<()> {
        let (name, done, closing) = {
            let state = self.state.lock().unwrap();
            let task = state.tasks.get(&data.task_id);
            (
                task.map_or_else(|| data.task_id.clone(), |task| task.description.clone()),
                task.map(|task| task.ended.clone()),
                state.closing,
            )
        };
        // A lost wake leaves the result in the history and is posted again by
        // the next generation; it does not fail dispatch or keep `stop` waiting.
        // An end ordered after the branch-close interrupt does not wake, even
        // when the command exited on its own just before.
        if wakes(&data)
            && !closing
            && let Err(error) = self.wake(event_id, &data, name).await
        {
            tracing::warn!(%error, task_id = data.task_id, "exec wake was not posted");
        }
        if let Some(done) = done {
            done.send_replace(Some(data));
        }
        Ok(())
    }

    /// One attempt from the dispatch handler; a failure is reposted by the
    /// next generation. A closing branch takes no more input.
    async fn wake(
        &self,
        event_id: &str,
        data: &ExecEndedData,
        name: String,
    ) -> dim_agent_runtime::Result<()> {
        match self.timeline.append(wake_input(event_id, data, name)) {
            Err(AgentError::Closed(_)) => Ok(()),
            result => result.map(|_| ()),
        }
    }

    /// Pairs this branch's `exec/started` with `exec/ended`. An open task did
    /// not survive its owner: its `failed` terminal is written now and wakes
    /// when dispatched. Returns the waking `exec/ended` events that no input
    /// references yet: the previous generation stopped before posting them.
    fn rebuild(&self, events: &[Arc<AgentEvent>]) -> Vec<(String, ExecEndedData, String)> {
        let mut state = self.state.lock().unwrap();
        let mut open = vec![];
        let mut unposted: Vec<(String, ExecEndedData)> = vec![];
        let mut closing = false;
        for event in events {
            match &event.kind {
                EventKind::StopInterrupt(data) if data.scope == InterruptScope::Branch => {
                    closing = true;
                }
                EventKind::ExecStarted(data) => {
                    state.tasks.insert(
                        data.task_id.clone(),
                        Task::new(data.description.clone(), CancellationToken::new()),
                    );
                    open.push((data.task_id.clone(), data.output_path.clone()));
                }
                EventKind::ExecEnded(data) => {
                    open.retain(|(task_id, _)| *task_id != data.task_id);
                    if let Some(task) = state.tasks.get_mut(&data.task_id) {
                        task.ended.send_replace(Some(data.clone()));
                    }
                    if wakes(data) && !closing {
                        unposted.push((event.id.clone(), data.clone()));
                    }
                }
                EventKind::InputMessage(_) => {
                    if let Some(result) = tagged(event, WAKE_RESULT_TAG) {
                        unposted.retain(|(id, _)| id != result);
                    }
                }
                _ => {}
            }
        }
        let unposted = unposted
            .into_iter()
            .map(|(id, data)| {
                let name = state
                    .tasks
                    .get(&data.task_id)
                    .map_or_else(|| data.task_id.clone(), |task| task.description.clone());
                (id, data, name)
            })
            .collect();
        for (task_id, output_path) in open {
            let written = self.timeline.append(
                EventKind::ExecEnded(ExecEndedData {
                    result: recovered_result(&task_id, &output_path),
                    task_id: task_id.clone(),
                    status: ExecStatus::Failed,
                })
                .into(),
            );
            if let Err(error) = written {
                tracing::warn!(%error, task_id, "recovery exec/ended was not written");
            }
        }
        unposted
    }

    /// Posts the wakes the previous generation lost, at load: like every
    /// input this capability produces, they are valid by construction.
    fn repost(&self, unposted: Vec<(String, ExecEndedData, String)>) {
        for (event_id, data, name) in unposted {
            if let Err(error) = self.timeline.append(wake_input(&event_id, &data, name)) {
                tracing::warn!(%error, task_id = data.task_id, "lost exec wake was not posted");
            }
        }
    }

    /// Unload (a reload, or the close of its branch or Session; an idle
    /// unload waits for the tasks): running tasks end cancelled after their
    /// trees are released.
    async fn unload(&self) {
        self.state.lock().unwrap().closing = true;
        self.lifecycle.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        // Dropping the senders releases pending `stop` callers as Closed.
        state.tasks.clear();
    }
}

/// Tags of a wake input: the task it reports and the `exec/ended` it carries.
pub const WAKE_TASK_TAG: &str = "exec-wake-task:";
pub const WAKE_RESULT_TAG: &str = "exec-wake-result:";

pub(crate) fn tagged<'a>(event: &'a AgentEvent, prefix: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .flatten()
        .find_map(|tag| tag.strip_prefix(prefix))
}

/// A Steer input whose notification carries the result preview.
pub(crate) fn wake_input(event_id: &str, data: &ExecEndedData, name: String) -> EventDraft {
    let preview: Vec<&str> = match &data.result.content {
        ToolContent::Text(text) => vec![text.as_str()],
        ToolContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::Image { .. } => None,
            })
            .collect(),
    };
    let status = match data.status {
        ExecStatus::Completed => "completed",
        ExecStatus::Failed => "failed",
        ExecStatus::Cancelled => "cancelled",
    };
    let text = context_block::element(
        "dim-notification",
        &[("source", "exec"), ("task", &data.task_id)],
        &format!(
            "Background command \"{}\" {status}.\nOutput preview:\n{}\nFull result: read tool, using the output path in the command result.",
            context_block::text(&name),
            context_block::preview(&preview),
        ),
    );
    let mut draft = EventDraft::from(EventKind::InputMessage(InputMessageData {
        blocks: vec![context_block::raw(text, None)],
        mode: Some(InputMode::Steer),
        ..Default::default()
    }));
    draft.tags = Some(vec![
        format!("{WAKE_TASK_TAG}{}", data.task_id),
        format!("{WAKE_RESULT_TAG}{event_id}"),
    ]);
    draft
}

/// A stop, branch close or unload ends a task `cancelled`; every other end
/// is a result the model should see.
pub(crate) fn wakes(data: &ExecEndedData) -> bool {
    data.status != ExecStatus::Cancelled
}

pub struct BackgroundExecPlugin {
    scope: BranchScope,
}
impl BackgroundExecPlugin {
    pub fn new(scope: &BranchScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for BackgroundExecPlugin {
    type Deps = (
        InBranch<Timeline>,
        InBranch<Dispatcher>,
        InSession<SessionLifecycle>,
    );
    fn name(&self) -> &str {
        "BackgroundExec"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InBranch(timeline), InBranch(dispatcher), InSession(session)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Arc::new(BackgroundExec {
                timeline: timeline.clone(),
                session,
                lifecycle: CancellationToken::new(),
                tasks: TaskTracker::new(),
                state: Mutex::new(State::default()),
            });
            let owner = service.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        owner.unload().await;
                        Ok(())
                    })
                }))
            })?;
            // Subscribe, then rebuild under the gate: handlers skip the
            // events the rebuild covered.
            let gate = Arc::new(tokio::sync::Mutex::new(0u64));
            let mut rebuilding = gate.lock().await;
            let consumer = service.clone();
            let seen = gate.clone();
            dispatcher.subscribe(
                ctx,
                "BackgroundExec",
                &["exec/ended", "stop/interrupt"],
                move |event| {
                    let seen = seen.clone();
                    let consumer = consumer.clone();
                    Box::pin(async move {
                        if event.sequence > *seen.lock().await {
                            consumer.consume(event).await
                        } else {
                            Ok(())
                        }
                    })
                },
            )?;
            let through = timeline.committed();
            let events = timeline.local(..=through).await?;
            *rebuilding = events.last().map_or(0, |event| event.sequence);
            let unposted = service.rebuild(&events);
            drop(rebuilding);
            service.repost(unposted);
            ctx.provide_as(self.scope.key::<BackgroundExec>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for BackgroundExecPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

/// Holds a task's start open for race flow tests. Feature `testing` only;
/// production builds have no hook.
#[cfg(feature = "testing")]
pub mod testing {
    use std::{
        path::{Path, PathBuf},
        sync::Mutex,
    };
    use tokio::sync::oneshot;

    type Hold = (PathBuf, oneshot::Sender<()>, oneshot::Receiver<()>);
    static HOLDS: Mutex<Vec<Hold>> = Mutex::new(Vec::new());

    /// Holds the next start whose output file is in `dir` until this is
    /// dropped: before the unload counts it, checks the branch or takes its
    /// Session hold.
    pub struct HeldStart {
        arrived: oneshot::Receiver<()>,
        _release: oneshot::Sender<()>,
    }
    impl HeldStart {
        /// Resolves once the held start has arrived.
        pub async fn arrived(&mut self) {
            let _ = (&mut self.arrived).await;
        }
    }

    pub fn hold_start(dir: &Path) -> HeldStart {
        let (arrive, arrived) = oneshot::channel();
        let (release, released) = oneshot::channel();
        HOLDS
            .lock()
            .unwrap()
            .push((dir.to_owned(), arrive, released));
        HeldStart {
            arrived,
            _release: release,
        }
    }

    pub(super) async fn pause(output_path: &str) {
        let hold = {
            let mut holds = HOLDS.lock().unwrap();
            let index = holds
                .iter()
                .position(|(dir, ..)| Path::new(output_path).starts_with(dir));
            index.map(|index| holds.swap_remove(index))
        };
        if let Some((_, arrive, released)) = hold {
            let _ = arrive.send(());
            let _ = released.await;
        }
    }
}
