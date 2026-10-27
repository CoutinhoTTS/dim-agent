use crate::{
    host::{HostError, HostFuture, ToolExecutionContext},
    tool_registry::{ToolContribution, ToolHandler, ToolRegistry},
};
use dim_agent_runtime::{
    AgentError, Branch, BranchOptions, Dispatcher, Names, Result, SessionIdentity, StoredBranch,
    Timeline, TimelineContext, TimelineFactory, TimelineInfo, TimelineKey, TimelineStore,
    TimelineTreeControl,
};
use dim_agent_types::*;
use futures_util::{FutureExt, future::join_all};
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use rutis::service;
use rutis::{Ctx, Error};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    panic::AssertUnwindSafe,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::{oneshot, watch},
    task::JoinHandle,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// Events per page of a restored child's creation check; the events a
/// created child wrote while it opened fit in one.
const CREATION_PAGE: usize = 32;

const TAIL_EVENTS: u64 = 32;

/// How a loaded branch begins on its Timeline.
enum Opening {
    /// A new branch: the root of a new Session, or a child with its first
    /// input.
    Start {
        start: BranchStartData,
        first: Option<InputMessageData>,
    },
    /// A stored branch loaded again at its stored head. While it opens, its
    /// build reads its history up to that head, the history its capabilities
    /// rebuild from, and reports the facts it holds about its own lines: the
    /// creates it saw fail, and the source each create recorded. The restore
    /// checks its children against them.
    Resume {
        head: u64,
        lines: oneshot::Sender<LineFacts>,
    },
}

#[derive(Default)]
struct LineFacts {
    /// The `branch/create` sequences the line recorded as failed.
    failed_creates: BTreeSet<u64>,
    /// Each child it created, with the source that create recorded.
    sources: BTreeMap<String, BranchSource>,
    /// The children whose end it recorded.
    ended: BTreeSet<String>,
}

struct Node {
    info: TimelineInfo,
    /// The built branch, before and after publication.
    context: Option<Arc<TimelineContext>>,
    ready: watch::Sender<Option<Result<Branch>>>,
    started: bool,
}
/// A line whose `branch/end` is committed and whose close has not completed.
struct PendingEnd {
    /// The sequence of `branch/end` on the line: the result's `endEvent`.
    sequence: u64,
    data: BranchEndData,
}
struct TreeState {
    nodes: BTreeMap<String, Node>,
    ended: BTreeMap<String, u64>,
    /// Lines waiting for the turn end that a tool-sourced end belongs to.
    pending: BTreeMap<String, PendingEnd>,
    closing: bool,
    cleanup_errors: Vec<AgentError>,
}
pub struct TimelineTree {
    weak: Weak<Self>,
    state: Mutex<TreeState>,
    factory: Arc<TimelineFactory>,
    store: Arc<dyn TimelineStore>,
    session_id: String,
    cancel: CancellationToken,
    builds: TaskTracker,
    close_result: watch::Sender<Option<Result<()>>>,
    close_task: Mutex<Option<JoinHandle<()>>>,
    close_started: AtomicBool,
    /// Wakes the callers waiting for an end to complete or to wait for a turn.
    end_notify: tokio::sync::Notify,
}
/// Shared with Timeline roots; never owns the tree or its child roots.
pub struct TreeCommands {
    tree: Weak<TimelineTree>,
}
#[service]
impl TreeCommands {
    #[service(skip)]
    pub fn get(&self, id: &str) -> Result<Branch> {
        let tree = self
            .tree
            .upgrade()
            .ok_or_else(|| AgentError::Closed("TimelineTree".into()))?;
        if tree.ended_head(id).is_some() {
            return Err(AgentError::BranchEnded(id.into()));
        }
        tree.owned_context(id)
            .map(|context| context.branch())
            .ok_or_else(|| AgentError::UnknownBranch(id.into()))
    }
    pub fn list(&self) -> Vec<TimelineInfo> {
        self.tree
            .upgrade()
            .map(|tree| tree.list())
            .unwrap_or_default()
    }
    #[service(skip)]
    pub async fn branch(
        &self,
        parent: String,
        at: u64,
        options: BranchOptions,
        admission: CancellationToken,
    ) -> Result<Branch> {
        self.tree
            .upgrade()
            .ok_or_else(|| AgentError::Closed("TimelineTree".into()))?
            .branch(parent, at, options, admission)
            .await
    }
    // The data methods below spell out `std::result::Result` for rutis'
    // export generator, which resolves an alias's item type where the alias
    // is defined (arcships/rutis#463).
    /// The directory record of branch `id`.
    pub fn info(&self, id: String) -> std::result::Result<TimelineInfo, AgentError> {
        self.list()
            .into_iter()
            .find(|info| info.id == id)
            .ok_or(AgentError::UnknownBranch(id))
    }
    /// Commits `input` to branch `id` through its validating `Branch::input`
    /// and returns the committed event, or the input already committed with
    /// its submission id.
    pub async fn input(
        &self,
        id: String,
        input: InputMessageData,
    ) -> std::result::Result<Arc<AgentEvent>, AgentError> {
        self.get(&id)?.input(input).await
    }
    /// Ends the line `id` with `source`; only the target is checked (§10).
    pub async fn end(
        &self,
        id: String,
        source: BranchSource,
    ) -> std::result::Result<(), AgentError> {
        self.tree
            .upgrade()
            .ok_or_else(|| AgentError::Closed("TimelineTree".into()))?
            .end(&id, source)
            .await
    }
    /// One page of `id`'s committed batches; an ended line reads at its
    /// fixed head.
    pub async fn page(
        &self,
        id: String,
        after: u64,
        limit: usize,
    ) -> std::result::Result<Vec<Arc<AgentEvent>>, AgentError> {
        self.tree
            .upgrade()
            .ok_or_else(|| AgentError::Closed("TimelineTree".into()))?
            .page(&id, after, limit)
            .await
    }
    /// Creates a child of `parent` cut at `at` and returns its id.
    /// `cancelled` is the caller's cancellation as an explicit argument, the
    /// form every cross-language contract uses for it (C4's `cancelled`): the
    /// creation's admission is cancelled, as [`Self::branch`]'s token does,
    /// once the future it returns completes, or fails because the caller is
    /// gone. Dropping the call cancels it too. TimelineTree records the outcome.
    /// Cancellation is observed through that future, so it ends only a
    /// creation still open when the future completes: a child already built
    /// returns its id, however early the caller's cancellation completed on
    /// its own side.
    pub async fn create(
        &self,
        parent: String,
        at: u64,
        options: BranchOptions,
        cancelled: Arc<
            dyn Fn() -> BoxFuture<'static, std::result::Result<(), Error>> + Send + Sync,
        >,
    ) -> Result<String> {
        let admission = CancellationToken::new();
        let cancel_on_drop = admission.clone().drop_guard();
        let caller = admission.clone();
        let created = self.branch(parent, at, options, admission);
        tokio::pin!(created);
        let created = tokio::select! {
            result = &mut created => result,
            _ = cancelled() => {
                caller.cancel();
                created.await
            }
        };
        cancel_on_drop.disarm();
        Ok(created?.id().to_owned())
    }
}
impl TimelineTree {
    fn create_failed(info: &TimelineInfo, error: &AgentError) -> BranchCreateFailedData {
        let branch = info.id.clone();
        let create_event = info.create_anchor.expect("child creation anchor");
        match error {
            AgentError::Cancelled(_) => BranchCreateFailedData::Cancelled {
                branch,
                create_event,
                reason: "branch creation cancelled".into(),
            },
            _ => BranchCreateFailedData::Failed {
                branch,
                create_event,
                error: "branch creation failed".into(),
            },
        }
    }
    fn record_create_failed(&self, info: &TimelineInfo, error: &AgentError) -> Result<()> {
        let parent = info.parent.as_deref().expect("failed child has parent");
        let parent = self
            .owned_context(parent)
            .ok_or_else(|| AgentError::UnknownBranch(parent.into()))?;
        parent
            .timeline()
            .append(EventKind::BranchCreateFailed(Self::create_failed(info, error)).into())?;
        Ok(())
    }
    fn complete_end(&self, info: &TimelineInfo, end_event: u64, data: BranchEndData) -> Result<()> {
        let Some(parent) = info.parent.as_deref() else {
            return Ok(());
        };
        let Some(parent) = self.owned_context(parent) else {
            return Ok(());
        };
        parent
            .timeline()
            .append(
                EventKind::BranchEnded(BranchEndedData {
                    branch: info.id.clone(),
                    end_event,
                    source: data.source,
                    ancestor: data.ancestor,
                })
                .into(),
            )
            .map(|_| ())
    }
    fn settle_failed(&self, info: &TimelineInfo, error: AgentError) -> AgentError {
        match self.record_create_failed(info, &error) {
            Ok(()) => error,
            Err(record_error) => {
                self.state
                    .lock()
                    .unwrap()
                    .cleanup_errors
                    .push(record_error.clone());
                error.with_cleanup(record_error)
            }
        }
    }
    /// Whether a stored child's own facts settle its creation: `branch/start`
    /// followed by the first input, which is Queue. What Prompt writes while
    /// `branch/start` dispatches is Append and does not count. Reads from the
    /// start a page at a time and stops at that input, so a created child
    /// reads only the events of its opening, not its whole history.
    async fn created(&self, branch: &str, head: u64) -> Result<bool> {
        let key = TimelineKey {
            session_id: self.session_id.clone(),
            branch_id: branch.into(),
        };
        let (mut after, mut started) = (0, false);
        while after < head {
            let page = self
                .store
                .read_page(&key, after, CREATION_PAGE, head)
                .await?;
            for record in &page {
                match &record.event.kind {
                    EventKind::BranchStart(_) => started = true,
                    EventKind::InputMessage(data)
                        if started && data.effective_mode() == InputMode::Queue =>
                    {
                        return Ok(true);
                    }
                    _ => {}
                }
            }
            after = page.last().map_or(head, |record| record.event.sequence);
        }
        Ok(false)
    }
    fn new(
        factory: Arc<TimelineFactory>,
        store: Arc<dyn TimelineStore>,
        session_id: String,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            weak: weak.clone(),
            state: Mutex::new(TreeState {
                nodes: BTreeMap::new(),
                ended: BTreeMap::new(),
                pending: BTreeMap::new(),
                closing: false,
                cleanup_errors: Vec::new(),
            }),
            factory,
            store,
            session_id,
            cancel: CancellationToken::new(),
            builds: TaskTracker::new(),
            close_result: watch::channel(None).0,
            close_task: Mutex::new(None),
            close_started: AtomicBool::new(false),
            end_notify: tokio::sync::Notify::new(),
        })
    }
    fn owned_context(&self, id: &str) -> Option<Arc<TimelineContext>> {
        self.state
            .lock()
            .unwrap()
            .nodes
            .get(id)
            .and_then(|node| node.context.clone())
    }
    fn reserve(
        &self,
        state: &mut TreeState,
        info: TimelineInfo,
    ) -> Result<watch::Receiver<Option<Result<Branch>>>> {
        if state.closing {
            return Err(AgentError::Closed("TimelineTree".into()));
        }
        if state.nodes.contains_key(&info.id) {
            return Err(AgentError::InvalidInput(
                "branch identity is already occupied".into(),
            ));
        }
        let (ready, receiver) = watch::channel(None);
        state.nodes.insert(
            info.id.clone(),
            Node {
                info,
                context: None,
                ready,
                started: false,
            },
        );
        Ok(receiver)
    }
    fn launch(
        &self,
        state: &mut TreeState,
        id: String,
        ancestors: Vec<(String, u64)>,
        validate: Option<(Timeline, u64)>,
        opening: Opening,
    ) {
        let node = state.nodes.get_mut(&id).expect("reserved branch");
        node.started = true;
        let ready = node.ready.clone();
        let info = node.info.clone();
        let parent_ready = info.parent.as_ref().map(|parent| {
            state
                .nodes
                .get(parent)
                .expect("owned parent")
                .ready
                .subscribe()
        });
        let owner = self.weak.upgrade().expect("tree owns branch creation");
        self.builds.spawn(async move {
            let deadline = tokio::time::Instant::now() + owner.factory.startup_timeout();
            let mut acquired = None;
            let mut cleanup_failure = None;
            let mut started = false;
            let mut outcome = AssertUnwindSafe(async {
                if let Some(parent_ready) = parent_ready {
                    tokio::select! {
                        result = Self::await_branch(parent_ready) => { result?; }
                        _ = owner.cancel.cancelled() => {
                            return Err(AgentError::Cancelled("branch creation".into()));
                        }
                        _ = tokio::time::sleep_until(deadline) => {
                            return Err(AgentError::Build("branch opening timed out".into()));
                        }
                    }
                }
                if let Some((timeline, through)) = validate {
                    let context = tokio::select! {
                        result = crate::history::read_context(&timeline, through) => result?,
                        _ = owner.cancel.cancelled() => return Err(AgentError::Cancelled("branch creation".into())),
                        _ = tokio::time::sleep_until(deadline) => return Err(AgentError::Build("branch opening timed out".into())),
                    };
                    crate::history::validate_context_boundary(&context)
                        .map_err(AgentError::InvalidInput)?;
                }
                let built = owner
                    .factory
                    .build_until(id.clone(), ancestors, owner.cancel.child_token(), deadline)
                    .await;
                let context = match built {
                    Err(error) => {
                        cleanup_failure = error.cleanup.clone();
                        return Err(error.into());
                    }
                    Ok(context) => context,
                };
                acquired = Some(context.clone());
                owner
                    .state
                    .lock()
                    .unwrap()
                    .nodes
                    .get_mut(&id)
                    .expect("building branch")
                    .context = Some(context.clone());
                let (opening, first) = match opening {
                    Opening::Start { start, first } => (EventKind::BranchStart(start), first),
                    Opening::Resume { head, lines } => {
                        let history = tokio::select! {
                            result = context.timeline().local(..=head) => result?,
                            _ = owner.cancel.cancelled() => return Err(AgentError::Cancelled("branch creation".into())),
                            _ = tokio::time::sleep_until(deadline) => return Err(AgentError::Build("branch opening timed out".into())),
                        };
                        let mut facts = LineFacts::default();
                        for event in &history {
                            match &event.kind {
                                EventKind::BranchCreateFailed(
                                    BranchCreateFailedData::Failed { create_event, .. }
                                    | BranchCreateFailedData::Cancelled { create_event, .. },
                                ) => {
                                    facts.failed_creates.insert(*create_event);
                                }
                                EventKind::BranchCreate(data) => {
                                    facts.sources.insert(data.branch.clone(), data.source.clone());
                                }
                                EventKind::BranchEnded(data) => {
                                    facts.ended.insert(data.branch.clone());
                                }
                                _ => {}
                            }
                        }
                        let _ = lines.send(facts);
                        (EventKind::BranchResume(BranchResumeData {}), None)
                    }
                };
                let opened = context.timeline().append(opening.into())?;
                let mut dispatch = context.branch().dispatch_watch();
                loop {
                    let state = dispatch.borrow_and_update().clone();
                    if let Some(error) = state.failure {
                        return Err(error);
                    }
                    if state.sequence >= opened.sequence {
                        break;
                    }
                    tokio::select! {
                        changed = dispatch.changed() => if changed.is_err() {
                            return Err(AgentError::Closed("branch dispatch".into()));
                        },
                        _ = owner.cancel.cancelled() => {
                            return Err(AgentError::Cancelled("branch creation".into()));
                        }
                        _ = tokio::time::sleep_until(deadline) => {
                            return Err(AgentError::Build("branch opening timed out".into()));
                        }
                    }
                }
                tokio::select! {
                    result = context.finish_opening() => result?,
                    _ = owner.cancel.cancelled() => return Err(AgentError::Cancelled("branch creation".into())),
                    _ = tokio::time::sleep_until(deadline) => return Err(AgentError::Build("branch opening timed out".into())),
                }
                if let Some(first) = first {
                    context.branch().input(first).await?;
                    started = true;
                }
                if owner.cancel.is_cancelled() {
                    return Err(AgentError::Cancelled("branch creation".into()));
                }
                Ok(context.branch())
            })
            .catch_unwind()
            .await
            .unwrap_or_else(|panic| {
                Err(AgentError::fault(
                    "TimelineTree build",
                    crate::host::panic_message(panic),
                ))
            });
            if let Ok(branch) = &outcome {
                let state = owner.state.lock().unwrap();
                if !state.closing {
                    if let Some(context) = &acquired {
                        context.activate();
                    }
                    ready.send_replace(Some(Ok(branch.clone())));
                    return;
                }
                drop(state);
                outcome = Err(AgentError::Cancelled("branch creation".into()));
            }
            if let (Err(error), Some(context)) = (&outcome, acquired) {
                let cleanup = AssertUnwindSafe(async { context.close().await })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|panic| {
                        Err(AgentError::fault(
                            "TimelineTree rollback",
                            crate::host::panic_message(panic),
                        ))
                    });
                if let Err(cleanup) = cleanup {
                    cleanup_failure = Some(cleanup.clone());
                    outcome = Err(error.clone().with_cleanup(cleanup));
                }
            }
            if info.parent.is_some()
                && !started
                && let Err(error) = &outcome
            {
                outcome = Err(owner.settle_failed(&info, error.clone()));
            }
            let mut state = owner.state.lock().unwrap();
            if let Some(error) = cleanup_failure {
                state.cleanup_errors.push(error);
            }
            state.nodes.remove(&id);
            ready.send_replace(Some(outcome));
        });
    }
    async fn page(&self, id: &str, after: u64, limit: usize) -> Result<Vec<Arc<AgentEvent>>> {
        let ended = self.state.lock().unwrap().ended.get(id).copied();
        let through = match ended {
            Some(head) => head,
            None => self
                .owned_context(id)
                .ok_or_else(|| AgentError::UnknownBranch(id.into()))?
                .timeline()
                .sequence(),
        };
        if through <= after {
            return Ok(Vec::new());
        }
        let key = TimelineKey {
            session_id: self.session_id.clone(),
            branch_id: id.into(),
        };
        Ok(self
            .store
            .read_page(&key, after, limit, through)
            .await?
            .into_iter()
            .map(|record| record.event)
            .collect())
    }
    async fn own_end(&self, branch: &str, head: u64) -> Result<Option<(u64, BranchEndData)>> {
        if head == 0 {
            return Ok(None);
        }
        let key = TimelineKey {
            session_id: self.session_id.clone(),
            branch_id: branch.into(),
        };
        let mut window = TAIL_EVENTS;
        loop {
            let from = head.saturating_sub(window.saturating_sub(1)).max(1);
            let records = self.store.read(&key, from..=head).await?;
            if let Some(found) = records
                .iter()
                .rev()
                .find_map(|record| match &record.event.kind {
                    EventKind::BranchEnd(data) => Some((record.event.sequence, data.clone())),
                    _ => None,
                })
            {
                return Ok(Some(found));
            }
            if from == 1 {
                return Ok(None);
            }
            window = window.saturating_mul(8);
        }
    }
    async fn creation_source(
        &self,
        parent: &str,
        child: &str,
        anchor: u64,
    ) -> Result<Option<BranchSource>> {
        let key = TimelineKey {
            session_id: self.session_id.clone(),
            branch_id: parent.into(),
        };
        Ok(self
            .store
            .read(&key, anchor..=anchor)
            .await?
            .into_iter()
            .find_map(|record| match &record.event.kind {
                EventKind::BranchCreate(data) if data.branch == child => Some(data.source.clone()),
                _ => None,
            }))
    }
    /// The stored head of an ended line, `None` for an open or unknown one.
    fn ended_head(&self, id: &str) -> Option<u64> {
        self.state.lock().unwrap().ended.get(id).copied()
    }
    async fn end(&self, id: &str, source: BranchSource) -> Result<()> {
        let context = {
            let state = self.state.lock().unwrap();
            if state.closing {
                return Err(AgentError::Closed("TimelineTree".into()));
            }
            if id == "main" {
                return Err(AgentError::InvalidInput(
                    "`main` cannot end; closing the Session is not an end".into(),
                ));
            }
            if state.ended.contains_key(id) {
                return Err(AgentError::BranchEnded(id.into()));
            }
            let node = state
                .nodes
                .get(id)
                .ok_or_else(|| AgentError::UnknownBranch(id.into()))?;
            if !matches!(&*node.ready.borrow(), Some(Ok(_))) {
                return Err(AgentError::UnknownBranch(id.into()));
            }
            node.context
                .clone()
                .ok_or_else(|| AgentError::UnknownBranch(id.into()))?
        };
        let data = BranchEndData {
            source,
            ancestor: None,
        };
        context
            .timeline()
            .append_registered(EventKind::BranchEnd(data.clone()).into(), |_| {
                if context.ended() {
                    return Err(AgentError::BranchEnded(id.into()));
                }
                context.mark_ended();
                Ok(())
            })?;
        loop {
            let notified = self.end_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.state.lock().unwrap();
                if state.ended.contains_key(id) || state.pending.contains_key(id) {
                    return Ok(());
                }
                if state.closing {
                    return Err(AgentError::Closed("TimelineTree".into()));
                }
            }
            tokio::select! {
                _ = notified => {}
                _ = self.cancel.cancelled() => {
                    return Err(AgentError::Closed("TimelineTree".into()));
                }
            }
        }
    }
    fn on_end(self: &Arc<Self>, id: &str, sequence: u64, data: BranchEndData) {
        if data.ancestor.is_some() {
            return;
        }
        let context = self
            .state
            .lock()
            .unwrap()
            .nodes
            .get(id)
            .and_then(|node| node.context.clone());
        let running = context
            .and_then(|context| context.branch().state().ok())
            .is_some_and(|status| status.running);
        if running && matches!(data.source, BranchSource::Tool { .. }) {
            self.state
                .lock()
                .unwrap()
                .pending
                .insert(id.into(), PendingEnd { sequence, data });
            self.end_notify.notify_waiters();
            return;
        }
        self.close_ended(id, sequence, data);
    }
    /// Closes a line whose end command is part of the turn that just ended.
    fn on_turn_end(self: &Arc<Self>, id: &str) {
        let pending = self.state.lock().unwrap().pending.remove(id);
        if let Some(pending) = pending {
            self.close_ended(id, pending.sequence, pending.data);
        }
    }
    fn close_ended(self: &Arc<Self>, id: &str, sequence: u64, data: BranchEndData) {
        let owner = self.clone();
        let id = id.to_owned();
        {
            let state = self.state.lock().unwrap();
            if state.closing || state.ended.contains_key(&id) {
                return;
            }
            self.builds.spawn(async move {
                if let Err(error) = owner.finish_end(&id, sequence, data).await {
                    owner.state.lock().unwrap().cleanup_errors.push(error);
                }
                owner.end_notify.notify_waiters();
            });
        }
    }
    fn descendants(&self, id: &str) -> Vec<(String, String)> {
        let state = self.state.lock().unwrap();
        let mut found = Vec::new();
        let mut pending = vec![(id.to_owned(), 0usize)];
        while let Some((parent, depth)) = pending.pop() {
            for node in state
                .nodes
                .values()
                .filter(|node| node.info.parent.as_deref() == Some(parent.as_str()))
            {
                found.push((node.info.id.clone(), parent.clone(), depth + 1));
                pending.push((node.info.id.clone(), depth + 1));
            }
        }
        found.sort_by_key(|found| std::cmp::Reverse(found.2));
        found
            .into_iter()
            .map(|(id, parent, _)| (id, parent))
            .collect()
    }
    async fn finish_end(&self, id: &str, end_event: u64, data: BranchEndData) -> Result<()> {
        let mut errors = Vec::new();
        for (child, ancestor) in self.descendants(id) {
            if let Err(error) = self.end_descendant(&child, &ancestor, &data.source).await {
                errors.push(error);
            }
        }
        if let Err(error) = self.close_and_record(id, end_event, data).await {
            errors.push(error);
        }
        AgentError::aggregate(errors)
    }
    async fn end_descendant(&self, id: &str, ancestor: &str, source: &BranchSource) -> Result<()> {
        let ready = {
            let state = self.state.lock().unwrap();
            if state.ended.contains_key(id) {
                return Ok(());
            }
            let node = state
                .nodes
                .get(id)
                .ok_or_else(|| AgentError::UnknownBranch(id.into()))?;
            (!matches!(&*node.ready.borrow(), Some(Ok(_)))).then(|| node.ready.subscribe())
        };
        if let Some(ready) = ready {
            tokio::select! {
                result = Self::await_branch(ready) => {
                    if result.is_err() {
                        return Ok(());
                    }
                }
                _ = self.cancel.cancelled() => {
                    return Err(AgentError::Closed("TimelineTree".into()));
                }
            }
        }
        let context = {
            let state = self.state.lock().unwrap();
            if state.ended.contains_key(id) {
                return Ok(());
            }
            state
                .nodes
                .get(id)
                .and_then(|node| node.context.clone())
                .ok_or_else(|| AgentError::UnknownBranch(id.into()))?
        };
        let data = BranchEndData {
            source: source.clone(),
            ancestor: Some(ancestor.into()),
        };
        let sequence =
            context
                .timeline()
                .append_registered(EventKind::BranchEnd(data.clone()).into(), |_| {
                    if context.ended() {
                        return Err(AgentError::BranchEnded(id.into()));
                    }
                    context.mark_ended();
                    Ok(())
                });
        let sequence = match sequence {
            Ok(event) => event.sequence,
            Err(AgentError::BranchEnded(_)) => return self.wait_ended(id).await,
            Err(error) => return Err(error),
        };
        self.close_and_record(id, sequence, data).await
    }
    /// Waits until the line is recorded as ended.
    async fn wait_ended(&self, id: &str) -> Result<()> {
        loop {
            let notified = self.end_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.state.lock().unwrap();
                if state.ended.contains_key(id) {
                    return Ok(());
                }
                if state.closing {
                    return Err(AgentError::Closed("TimelineTree".into()));
                }
            }
            tokio::select! {
                _ = notified => {}
                _ = self.cancel.cancelled() => {
                    return Err(AgentError::Closed("TimelineTree".into()));
                }
            }
        }
    }
    async fn close_and_record(&self, id: &str, end_event: u64, data: BranchEndData) -> Result<()> {
        let (context, parent) = {
            let state = self.state.lock().unwrap();
            let node = state
                .nodes
                .get(id)
                .ok_or_else(|| AgentError::UnknownBranch(id.into()))?;
            (
                node.context
                    .clone()
                    .ok_or_else(|| AgentError::UnknownBranch(id.into()))?,
                node.info.parent.clone(),
            )
        };
        let mut errors = Vec::new();
        if let Err(error) = context.branch().close().await {
            errors.push(error);
        }
        let head = context.timeline().sequence();
        if let Some(parent) = parent {
            let recorded = self
                .owned_context(&parent)
                .ok_or_else(|| AgentError::UnknownBranch(parent.clone()))
                .and_then(|parent| {
                    parent
                        .timeline()
                        .append(
                            EventKind::BranchEnded(BranchEndedData {
                                branch: id.into(),
                                end_event,
                                source: data.source,
                                ancestor: data.ancestor,
                            })
                            .into(),
                        )
                        .map(|_| ())
                });
            if let Err(error) = recorded {
                errors.push(AgentError::Storage(format!(
                    "ending line {id}: its result could not be recorded: {error}"
                )));
            }
        }
        {
            let mut state = self.state.lock().unwrap();
            state.pending.remove(id);
            state.nodes.remove(id);
            state.ended.insert(id.into(), head);
        }
        AgentError::aggregate(errors)
    }
    fn on_create(&self, parent: &str, timeline: &Timeline, event: &AgentEvent) -> Result<()> {
        let EventKind::BranchCreate(data) = &event.kind else {
            unreachable!("branch subscription")
        };
        let point = match data.at {
            BranchCreateAt::Sequence { sequence } => sequence,
            BranchCreateAt::Create {} => event.sequence,
        };
        let mut state = self.state.lock().unwrap();
        if state.closing {
            return Ok(());
        }
        if !state
            .nodes
            .get(&data.branch)
            .is_some_and(|node| !node.started)
        {
            return Ok(());
        }
        let mut ancestors = timeline.ancestors();
        ancestors.push((parent.into(), point));
        self.launch(
            &mut state,
            data.branch.clone(),
            ancestors,
            Some((timeline.clone(), point)),
            Opening::Start {
                start: BranchStartData {
                    parent: Some(parent.into()),
                    create_event: Some(event.sequence),
                },
                first: Some(first_input(parent, data)),
            },
        );
        Ok(())
    }
    async fn await_branch(mut ready: watch::Receiver<Option<Result<Branch>>>) -> Result<Branch> {
        loop {
            if let Some(result) = ready.borrow().clone() {
                return result;
            }
            ready
                .changed()
                .await
                .map_err(|_| AgentError::fault("TimelineTree", "build result channel closed"))?;
        }
    }
    fn cancel_pending(
        state: &mut TreeState,
        parent: Option<&str>,
    ) -> Vec<(TimelineInfo, watch::Sender<Option<Result<Branch>>>)> {
        let mut cancelled = Vec::new();
        state.nodes.retain(|_, node| {
            if !node.started
                && parent.is_none_or(|parent| node.info.parent.as_deref() == Some(parent))
            {
                cancelled.push((node.info.clone(), node.ready.clone()));
                false
            } else {
                true
            }
        });
        cancelled
    }
    fn settle_cancelled(
        &self,
        cancelled: Vec<(TimelineInfo, watch::Sender<Option<Result<Branch>>>)>,
    ) {
        for (info, ready) in cancelled {
            let error = AgentError::Cancelled("branch creation closed before dispatch".into());
            let outcome = self.settle_failed(&info, error);
            ready.send_replace(Some(Err(outcome)));
        }
    }
    fn start_close(&self) {
        self.begin_close();
        if self.close_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let owner = self.weak.upgrade().expect("owned close");
        let task = tokio::spawn(async move {
            owner.builds.close();
            owner.builds.wait().await;
            let (contexts, failures): (Vec<_>, Vec<_>) = {
                let mut state = owner.state.lock().unwrap();
                (
                    state
                        .nodes
                        .values()
                        .filter_map(|node| node.context.clone())
                        .collect(),
                    std::mem::take(&mut state.cleanup_errors),
                )
            };
            let mut errors = failures;
            errors.extend(
                join_all(contexts.iter().map(|context| context.close()))
                    .await
                    .into_iter()
                    .filter_map(|result| result.err()),
            );
            owner
                .close_result
                .send_replace(Some(AgentError::aggregate(errors)));
        });
        *self.close_task.lock().unwrap() = Some(task);
    }
}
impl TimelineTreeControl for TimelineTree {
    fn begin_close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closing = true;
        let cancelled = Self::cancel_pending(&mut state, None);
        drop(state);
        self.settle_cancelled(cancelled);
        self.cancel.cancel();
    }
    fn create_main(&self) -> BoxFuture<'_, Result<Branch>> {
        Box::pin(async move {
            let ready = {
                let mut state = self.state.lock().unwrap();
                let ready = self.reserve(
                    &mut state,
                    TimelineInfo {
                        id: "main".into(),
                        parent: None,
                        branch_point: None,
                        create_anchor: None,
                        source: None,
                    },
                )?;
                self.launch(
                    &mut state,
                    "main".into(),
                    vec![],
                    None,
                    Opening::Start {
                        start: BranchStartData {
                            parent: None,
                            create_event: None,
                        },
                        first: None,
                    },
                );
                ready
            };
            Self::await_branch(ready).await
        })
    }
    fn restore(&self, branches: Vec<StoredBranch>) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if branches.is_empty() {
                self.create_main().await?;
                return Ok(());
            }
            let mut remaining: BTreeMap<_, _> = branches
                .into_iter()
                .map(|branch| (branch.info.id.clone(), branch))
                .collect();
            let mut failed_creates = BTreeSet::new();
            let mut sources = BTreeMap::new();
            let mut recorded_ends = BTreeSet::new();
            let mut inactive = BTreeSet::new();
            while !remaining.is_empty() {
                let stored = remaining
                    .values()
                    .find(|branch| {
                        branch.info.parent.as_ref().is_none_or(|parent| {
                            self.get(parent).is_some() || inactive.contains(parent)
                        })
                    })
                    .cloned()
                    .ok_or_else(|| {
                        AgentError::Storage(
                            "branch directory contains missing parents or a cycle".into(),
                        )
                    })?;
                let mut next: TimelineInfo = stored.info.into();
                next.source = sources.get(&next.id).cloned();
                if recorded_ends.contains(&next.id) {
                    self.state
                        .lock()
                        .unwrap()
                        .ended
                        .insert(next.id.clone(), stored.sequence);
                    remaining.remove(&next.id);
                    inactive.insert(next.id);
                    continue;
                }
                if next.parent.as_ref().is_some_and(|parent| {
                    inactive.contains(parent)
                        || failed_creates.contains(&(
                            parent.clone(),
                            next.create_anchor.expect("validated creation anchor"),
                        ))
                }) {
                    let ended_parent = next
                        .parent
                        .as_deref()
                        .filter(|parent| self.ended_head(parent).is_some());
                    if let Some(parent) = ended_parent
                        && self.created(&next.id, stored.sequence).await?
                    {
                        if next.source.is_none()
                            && let Some(anchor) = next.create_anchor
                        {
                            next.source = self.creation_source(parent, &next.id, anchor).await?;
                        }
                        self.state
                            .lock()
                            .unwrap()
                            .ended
                            .insert(next.id.clone(), stored.sequence);
                    }
                    remaining.remove(&next.id);
                    inactive.insert(next.id);
                    continue;
                }
                let own_end = if next.parent.is_some() {
                    self.own_end(&next.id, stored.sequence).await?
                } else {
                    None
                };
                if let Some((end_event, data)) = own_end {
                    self.complete_end(&next, end_event, data)?;
                    self.state
                        .lock()
                        .unwrap()
                        .ended
                        .insert(next.id.clone(), stored.sequence);
                    remaining.remove(&next.id);
                    inactive.insert(next.id);
                    continue;
                }
                if next.parent.is_some() && !self.created(&next.id, stored.sequence).await? {
                    let error =
                        AgentError::Build("branch creation interrupted by process restart".into());
                    self.record_create_failed(&next, &error)?;
                    remaining.remove(&next.id);
                    inactive.insert(next.id);
                    continue;
                }
                let ancestors = match &next.parent {
                    Some(parent) => {
                        let parent_context = self.owned_context(parent).expect("restored parent");
                        let timeline = parent_context.timeline();
                        let cut = next.branch_point.expect("validated child cut");
                        let context =
                            crate::history::read_context(timeline, cut)
                                .await
                                .map_err(|error| {
                                    AgentError::Storage(format!(
                                        "branch {} cannot be restored: {error}",
                                        next.id
                                    ))
                                })?;
                        crate::history::validate_context_boundary(&context)
                            .map_err(AgentError::Storage)?;
                        let mut ancestors = timeline.ancestors();
                        ancestors.push((parent.clone(), cut));
                        ancestors
                    }
                    None => vec![],
                };
                let (ready, reported) = {
                    let mut state = self.state.lock().unwrap();
                    let ready = self.reserve(&mut state, next.clone())?;
                    let (opening, reported) = if next.parent.is_none() && stored.sequence == 0 {
                        let start = BranchStartData {
                            parent: None,
                            create_event: None,
                        };
                        (Opening::Start { start, first: None }, None)
                    } else {
                        let (lines, reported) = oneshot::channel();
                        let head = stored.sequence;
                        (Opening::Resume { head, lines }, Some(reported))
                    };
                    self.launch(&mut state, next.id.clone(), ancestors, None, opening);
                    (ready, reported)
                };
                Self::await_branch(ready).await?;
                if let Some(reported) = reported {
                    let facts = reported.await.expect(
                        "a resumed branch reports its own lines before it is published",
                    );
                    failed_creates.extend(
                        facts
                            .failed_creates
                            .into_iter()
                            .map(|create| (next.id.clone(), create)),
                    );
                    sources.extend(facts.sources);
                    recorded_ends.extend(facts.ended);
                }
                remaining.remove(&next.id);
            }
            Ok(())
        })
    }
    fn branch(
        &self,
        parent: String,
        at: u64,
        options: BranchOptions,
        admission: CancellationToken,
    ) -> BoxFuture<'_, Result<Branch>> {
        Box::pin(async move {
            if self.state.lock().unwrap().ended.contains_key(&parent) {
                return Err(AgentError::BranchEnded(parent.clone()));
            }
            let parent_context = self
                .owned_context(&parent)
                .ok_or_else(|| AgentError::UnknownBranch(parent.clone()))?;
            let timeline = parent_context.timeline();
            let parent_length = timeline.sequence();
            if at > parent_length {
                return Err(AgentError::InvalidInput(
                    "branch position is beyond committed history".into(),
                ));
            }
            let requested = crate::history::read_context(timeline, at).await?;
            crate::history::validate_context_boundary(&requested)
                .map_err(AgentError::InvalidInput)?;
            let original = BranchCreateData {
                at: if at == parent_length {
                    BranchCreateAt::Create {}
                } else {
                    BranchCreateAt::Sequence { sequence: at }
                },
                source: options.source,
                branch: uuid::Uuid::new_v4().to_string(),
                prompt: options.prompt,
            };
            if parent_context.is_closing() {
                return Err(AgentError::Closed(parent.clone()));
            }
            let mut ready = None;
            let _ =
                timeline.append_registered(EventKind::BranchCreate(original).into(), |event| {
                    let EventKind::BranchCreate(final_data) = &event.kind else {
                        unreachable!("before preserves kind")
                    };
                    let branch_point = match final_data.at {
                        BranchCreateAt::Create {} => event.sequence,
                        BranchCreateAt::Sequence { sequence } if sequence < event.sequence => {
                            sequence
                        }
                        BranchCreateAt::Sequence { .. } => {
                            return Err(AgentError::InvalidInput(
                                "branch position is beyond committed history".into(),
                            ));
                        }
                    };
                    if parent_context.is_closing() {
                        return Err(AgentError::Closed(parent.clone()));
                    }
                    if parent_context.ended() {
                        return Err(AgentError::BranchEnded(parent.clone()));
                    }
                    if admission.is_cancelled() {
                        return Err(AgentError::Cancelled("branch admission".into()));
                    }
                    let mut state = self.state.lock().unwrap();
                    let mut above = Some(parent.clone());
                    while let Some(id) = above {
                        if state.ended.contains_key(&id) {
                            return Err(AgentError::BranchEnded(id));
                        }
                        let node = state
                            .nodes
                            .get(&id)
                            .ok_or_else(|| AgentError::UnknownBranch(id.clone()))?;
                        if node
                            .context
                            .as_ref()
                            .is_some_and(|context| context.ended())
                        {
                            return Err(AgentError::BranchEnded(id));
                        }
                        above = node.info.parent.clone();
                    }
                    ready = Some(self.reserve(
                        &mut state,
                        TimelineInfo {
                            id: final_data.branch.clone(),
                            parent: Some(parent.clone()),
                            branch_point: Some(branch_point),
                            create_anchor: Some(event.sequence),
                            source: Some(final_data.source.clone()),
                        },
                    )?);
                    Ok(())
                })?;
            tokio::select! {
                result = Self::await_branch(ready.expect("committed branch reservation")) => result,
                _ = admission.cancelled() => Err(AgentError::Cancelled(
                    "stopped waiting; the branch creation continues".into(),
                )),
            }
        })
    }
    fn get(&self, id: &str) -> Option<Branch> {
        self.state.lock().unwrap().nodes.get(id).and_then(|node| {
            node.ready
                .borrow()
                .as_ref()
                .and_then(|result| result.as_ref().ok().cloned())
        })
    }
    fn end(&self, id: String, source: BranchSource) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { TimelineTree::end(self, &id, source).await })
    }
    fn ended_head(&self, id: &str) -> Option<u64> {
        TimelineTree::ended_head(self, id)
    }
    fn list(&self) -> Vec<TimelineInfo> {
        self.state
            .lock()
            .unwrap()
            .nodes
            .values()
            .filter(|node| matches!(&*node.ready.borrow(), Some(Ok(_))))
            .map(|node| node.info.clone())
            .collect()
    }
    fn close(&self) -> BoxFuture<'_, Result<()>> {
        self.start_close();
        Box::pin(async move {
            let mut receiver = self.close_result.subscribe();
            let result = loop {
                if let Some(result) = receiver.borrow().clone() {
                    break result;
                }
                receiver.changed().await.map_err(|_| {
                    AgentError::fault("TimelineTree", "close result channel closed")
                })?;
            };
            let task = self.close_task.lock().unwrap().take();
            if let Some(task) = task {
                task.await
                    .map_err(|error| AgentError::fault("TimelineTree close", error.to_string()))?;
            }
            result
        })
    }
}

/// Starts the tree's build task for each final `branch/create` of its branch.
pub struct TreeTimelinePlugin {
    timeline: Key<Timeline>,
    dispatcher: Key<Dispatcher>,
    commands: Key<TreeCommands>,
    identity: Key<dim_agent_runtime::TimelineIdentity>,
}
impl TreeTimelinePlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            timeline: names.key()?,
            dispatcher: names.key()?,
            commands: names.key()?,
            identity: names.key()?,
        })
    }
}
impl DynPlugin for TreeTimelinePlugin {
    fn name(&self) -> &str {
        "TreeTimeline"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.timeline.erase(),
            self.dispatcher.erase(),
            self.commands.erase(),
            self.identity.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let timeline = ctx.require_at(&self.timeline)?;
            let dispatcher = ctx.require_at(&self.dispatcher)?;
            let commands = ctx.require_at(&self.commands)?;
            let identity = ctx.require_at(&self.identity)?;
            let cleanup = commands.clone();
            let parent = identity.branch_id.clone();
            ctx.effect(move || async move {
                if let Some(tree) = cleanup.tree.upgrade() {
                    let cancelled = TimelineTree::cancel_pending(
                        &mut tree.state.lock().unwrap(),
                        Some(&parent),
                    );
                    tree.settle_cancelled(cancelled);
                }
                Ok(())
            })?;
            dispatcher.subscribe(
                ctx,
                "TreeTimeline",
                &["branch/create", "branch/end", "turn/end"],
                move |event| {
                    let branch = &identity.branch_id;
                    let result = match &event.kind {
                        EventKind::BranchCreate(_) => commands
                            .tree
                            .upgrade()
                            .ok_or_else(|| AgentError::Closed("TimelineTree".into()))
                            .and_then(|tree| tree.on_create(branch, &timeline, &event)),
                        EventKind::BranchEnd(data) => commands
                            .tree
                            .upgrade()
                            .ok_or_else(|| AgentError::Closed("TimelineTree".into()))
                            .map(|tree| tree.on_end(branch, event.sequence, data.clone())),
                        EventKind::TurnEnd(_) => commands
                            .tree
                            .upgrade()
                            .ok_or_else(|| AgentError::Closed("TimelineTree".into()))
                            .map(|tree| tree.on_turn_end(branch)),
                        _ => Ok(()),
                    };
                    Box::pin(async move { result })
                },
            )?;
            Ok(Effect::Done)
        })
    }
}
pub struct TimelineTreePlugin {
    commands: Key<TreeCommands>,
    tree: Key<TimelineTree>,
    control: Key<dyn TimelineTreeControl>,
    factory: Key<TimelineFactory>,
    store: Key<dyn TimelineStore>,
    identity: Key<SessionIdentity>,
}
impl TimelineTreePlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            commands: names.key()?,
            tree: names.key()?,
            control: names.key()?,
            factory: names.key()?,
            store: names.key()?,
            identity: names.key()?,
        })
    }
}
impl DynPlugin for TimelineTreePlugin {
    fn name(&self) -> &str {
        "TimelineTree"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.factory.erase(),
            self.store.erase(),
            self.identity.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let factory = ctx.require_at(&self.factory)?;
            let store = ctx.require_at(&self.store)?;
            let identity = ctx.require_at(&self.identity)?;
            let tree = TimelineTree::new(factory, store, identity.session_id.clone());
            let owner = tree.clone();
            ctx.effect(move || async move { owner.close().await.map_err(Into::into) })?;
            let commands = Arc::new(TreeCommands {
                tree: Arc::downgrade(&tree),
            });
            ctx.provide_at(self.commands.clone(), commands)?;
            ctx.provide_at(self.tree.clone(), tree.clone())?;
            ctx.provide_at(self.control.clone(), tree as Arc<dyn TimelineTreeControl>)?;
            Ok(Effect::Done)
        })
    }
}
/// Session part: registers the `branch` tool, which forks the calling
/// branch through the tree.
pub struct BranchToolPlugin {
    tree: Key<TreeCommands>,
    tools: Key<ToolRegistry>,
}
impl BranchToolPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            tree: names.key()?,
            tools: names.key()?,
        })
    }
}
impl DynPlugin for BranchToolPlugin {
    fn name(&self) -> &str {
        "BranchTool"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![self.tree.erase(), self.tools.erase()]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let tree = ctx.require_at(&self.tree)?;
            let tools = ctx.require_at(&self.tools)?;
            tools.register(
                ctx,
                ToolContribution {
                    definition: branch_definition(),
                    handler: Arc::new(BranchTool { tree }),
                },
            )?;
            Ok(Effect::Done)
        })
    }
}
struct BranchTool {
    tree: Arc<TreeCommands>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BranchInput {
    prompt: String,
    at: Option<u64>,
}
impl ToolHandler for BranchTool {
    fn execute(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            let input: BranchInput = serde_json::from_value(serde_json::Value::Object(call.input))
                .map_err(|error| HostError::failed(format!("Invalid branch arguments: {error}")))?;
            if input.prompt.is_empty() || input.at.is_some_and(|at| at > 9_007_199_254_740_991) {
                return Err(HostError::failed(
                    "branch requires a nonempty prompt and a safe integer position",
                ));
            }
            crate::context_policy::check_cancel(&cancel)?;
            let parent = self
                .tree
                .get(&context.branch_id)
                .map_err(|e| HostError::failed(e.to_string()))?;
            let at = match input.at {
                Some(at) => at,
                None => {
                    let events = parent
                        .local(..)
                        .await
                        .map_err(|e| HostError::failed(e.to_string()))?;
                    let first_use = events
                        .iter()
                        .position(|event| {
                            matches!(&event.kind, EventKind::ToolUse(data)
                            if data.turn_id == context.turn_id
                                && data.request_id == context.request_id)
                        })
                        .ok_or_else(|| {
                            HostError::failed("branch tool call is absent from its owning timeline")
                        })?;
                    events[..first_use]
                        .iter()
                        .rev()
                        .find_map(|event| match &event.kind {
                            EventKind::ModelStart(data)
                                if data.turn_id == context.turn_id
                                    && data.request_id == context.request_id =>
                            {
                                Some(event.sequence - 1)
                            }
                            _ => None,
                        })
                        .ok_or_else(|| {
                            HostError::failed("branch tool batch has no owning model start")
                        })?
                }
            };
            let branch = self
                .tree
                .branch(
                    context.branch_id,
                    at,
                    BranchOptions {
                        prompt: input.prompt,
                        source: BranchSource::Tool {
                            turn_id: context.turn_id,
                            request_id: context.request_id,
                            call_id: call.id,
                        },
                    },
                    cancel,
                )
                .await
                .map_err(|error| match error {
                    AgentError::Cancelled(reason) => HostError::Cancelled(reason),
                    error => HostError::failed(error.to_string()),
                })?;
            Ok(ToolResult::text(format!("Branch created: {}", branch.id())))
        })
    }
}
fn line_key() -> String {
    "branch:line".into()
}

fn first_input(parent: &str, data: &BranchCreateData) -> InputMessageData {
    let (blocks, user) = match data.source {
        BranchSource::Tool { .. } => (
            vec![tool_line_identity(&data.branch, parent, &data.prompt)],
            None,
        ),
        BranchSource::Host { .. } => (
            vec![host_line_identity(&data.branch, parent)],
            Some(UserInput {
                content: vec![ContentBlock::Text {
                    text: data.prompt.clone(),
                }],
            }),
        ),
    };
    InputMessageData {
        blocks,
        user,
        mode: Some(InputMode::Queue),
        ..InputMessageData::default()
    }
}

fn tool_line_identity(line: &str, parent: &str, prompt: &str) -> InputBlock {
    let prompt = crate::context_block::text(prompt);
    let body = [
        format!("You are line {line} of this session. Line {parent} opened you at this point of its history to work in parallel with it. You are the same agent as {parent}: everything above is your own history, and you continue from here on your own line. {parent} keeps working on its line; you do not see its work after this point, and it sees yours only through what you send it. Messages above that were addressed to {parent}, such as its list of open lines, are {parent}'s, not yours."),
        String::new(),
        format!("Your purpose, as {parent} wrote it:"),
        prompt,
        String::new(),
        format!("How you work with {parent}:"),
        format!("- Work on this purpose only. You cannot open lines; if more parallel work is needed, tell {parent}."),
        format!("- Tell {parent} with the branch tool (message, to \"{parent}\"). Use mode \"steer\" for what {parent} must act on now: a finding that changes its work or another line's, a blocking question, a conflict. Use \"append\" for progress {parent} should see at its next step. Do not repeat what {parent} already knows."),
        format!("- You share the workspace with {parent} and its other lines. Change only the files {parent} gave you; before changing any other file, tell {parent} which. If edit or write fails with file_modified_since_read, another line may have changed the file: read it again and tell {parent} before you continue."),
        "- The user can talk to you directly. Answer the user, and ask with ask_user when only the user can decide.".to_string(),
        format!("- When your purpose is done, or cannot be done, call the branch tool with action \"end\" and content saying what you found, did, changed and left undone. That is your last action; {parent} receives it."),
    ]
    .join("\n");
    crate::context_block::raw(
        crate::context_block::element(
            "dim-branch-line",
            &[("branch", line), ("parent", parent), ("source", "tool")],
            &body,
        ),
        Some(line_key()),
    )
}

fn host_line_identity(line: &str, parent: &str) -> InputBlock {
    crate::context_block::raw(
        crate::context_block::element(
            "dim-branch-line",
            &[("branch", line), ("parent", parent), ("source", "host")],
            &format!("You are line {line} of this session, opened from line {parent} at this point of its history. You are the same agent as {parent}: everything above is your own history, and you continue from here on your own line, in parallel with {parent}. You do not see {parent}'s work after this point. Messages above that were addressed to {parent} are {parent}'s, not yours. The message after this one says what this line is for."),
        ),
        Some(line_key()),
    )
}

fn branch_definition() -> ToolDefinition {
    ToolDefinition {
        risk: Some("session".into()),
        ..crate::tool_registry::definition(
            "branch",
            "Create an independent branch that continues from your timeline and works on prompt as a task you delegate. The result contains its branch ID.",
            serde_json::json!({"type":"object","required":["prompt"],"additionalProperties":false,"properties":{"prompt":{"type":"string","description":"The task the branch works on"},"at":{"type":"number","minimum":0}}}),
        )
    }
}
