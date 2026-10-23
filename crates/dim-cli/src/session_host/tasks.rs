//! Finite history pages and addressed stops owned by the Session worker.

use std::collections::{HashSet, VecDeque};

use dim_agent_types::BranchCreateFailedData;

use super::*;
use crate::model::view::TaskKind;

#[derive(Default)]
pub(super) struct BranchHistory {
    after: u64,
    through: Option<u64>,
    pub done: bool,
    pub error: Option<String>,
    live: Vec<Event>,
    catch_up: bool,
    pub running: Option<bool>,
}

pub(super) struct HistoryRequest {
    pub id: u64,
    session_id: String,
    branch_id: String,
    current: bool,
}

pub(super) struct StopBatch {
    session_id: String,
    targets: VecDeque<TaskKind>,
    pub request: Option<(u64, String)>,
    attempted: usize,
    errors: usize,
    unavailable: usize,
}

impl Worker {
    pub(super) fn invalidate_projection(&mut self) {
        self.watch_member(None);
        self.projection_id += 1;
        self.session_id.clear();
        self.projection = Projection::default();
        self.cursors.clear();
        self.task_history.clear();
        self.task_history_queue.clear();
        if let Some(request) = &mut self.task_history_request {
            request.current = false;
        }
        self.watch_task_output(None);
        self.publish();
    }

    pub(super) fn discover_branch(&mut self, branch_id: &str) {
        if !self.task_history.contains_key(branch_id) {
            self.task_history
                .insert(branch_id.to_owned(), BranchHistory::default());
            self.task_history_queue.push_back(branch_id.to_owned());
            self.changed();
        }
    }

    pub(super) fn discover_task_branches(&mut self, event: &Event) {
        match &event.content {
            EventContent::BranchCreate(data) => self.discover_branch(&data.branch),
            EventContent::BranchStart(data) => {
                if let Some(parent) = &data.parent {
                    self.discover_branch(parent);
                }
            }
            EventContent::BranchCreateFailed(data) => {
                let (branch, error) = match data {
                    BranchCreateFailedData::Failed { branch, error, .. } => (branch, error),
                    BranchCreateFailedData::Cancelled { branch, reason, .. } => (branch, reason),
                };
                let history = self.task_history.entry(branch.clone()).or_default();
                // Discovery and later adoption can observe the same creation failure.
                let newly_failed = !history.done;
                history.done = true;
                history.error = None;
                history.live.clear();
                history.catch_up = false;
                self.task_history_queue.retain(|id| id != branch);
                if let Some(request) = &mut self.task_history_request
                    && request.branch_id == *branch
                {
                    request.current = false;
                }
                if newly_failed {
                    self.send(Update::Notice(format!("Branch {branch}: {error}")));
                }
            }
            _ => {}
        }
    }

    pub(super) fn task_event(&mut self, branch_id: &str, event: Event) {
        self.discover_branch(branch_id);
        self.discover_task_branches(&event);
        let history = self
            .task_history
            .get_mut(branch_id)
            .expect("discovered branch");
        if history.done {
            self.apply(branch_id, event);
        } else if history.error.is_none() {
            history.live.push(event);
        }
    }

    pub(super) fn retry_task_history(&mut self) {
        for (id, history) in &mut self.task_history {
            if history.error.take().is_some() {
                self.task_history_queue.push_back(id.clone());
            }
        }
        self.changed();
    }

    pub(super) async fn poll_task_history(&mut self) -> crate::Result<()> {
        if self.task_history_request.is_some() || self.session_id.is_empty() {
            return Ok(());
        }
        let Some(branch_id) = self.task_history_queue.pop_front() else {
            return Ok(());
        };
        let history = self.task_history.get(&branch_id).expect("queued branch");
        let mut params = json!({
            "sessionId": self.session_id, "branchId": branch_id,
            "after": history.after, "limit": HISTORY_PAGE,
        });
        if let Some(through) = history.through {
            params["through"] = json!(through);
        }
        match self.client.send("session.history", params).await {
            Ok(id) => {
                self.task_history_request = Some(HistoryRequest {
                    id,
                    session_id: self.session_id.clone(),
                    branch_id,
                    current: true,
                })
            }
            Err(error) => {
                if branch_id == "main" {
                    return Err(format!("Main branch history: {error}").into());
                }
                let history = self
                    .task_history
                    .get_mut(&branch_id)
                    .expect("queued branch");
                history.error = Some(error.to_string());
                history.live.clear();
                history.catch_up = history.through.is_some();
                self.changed();
            }
        }
        Ok(())
    }

    pub(super) fn task_history_reply(
        &mut self,
        result: Result<serde_json::Value, client::RpcError>,
    ) -> crate::Result<()> {
        let request = self
            .task_history_request
            .take()
            .expect("matched history request");
        if !request.current || request.session_id != self.session_id {
            return Ok(());
        }
        let history = self
            .task_history
            .get(&request.branch_id)
            .expect("current history");
        let result = result
            .map_err(|error| error.to_string())
            .and_then(|value| History::decode(value, history.after, history.through));
        match result {
            Err(error) => {
                if request.branch_id == "main" {
                    return Err(format!("Main branch history: {error}").into());
                }
                let history = self
                    .task_history
                    .get_mut(&request.branch_id)
                    .expect("current history");
                history.error = Some(error);
                history.live.clear();
                history.catch_up = history.through.is_some();
            }
            Ok(page) => {
                let history = self
                    .task_history
                    .get_mut(&request.branch_id)
                    .expect("current history");
                history.after = page.next_after;
                history.through = Some(page.through);
                history.done = page.done && !history.catch_up;
                if page.done && history.catch_up {
                    // Re-read facts discarded during the failure after finishing its fixed window.
                    history.through = None;
                    history.catch_up = false;
                }
                let done = history.done;
                for event in page.events {
                    self.apply(&request.branch_id, event);
                }
                self.cursors
                    .insert(request.branch_id.clone(), page.next_after);
                if done {
                    let history = self
                        .task_history
                        .get_mut(&request.branch_id)
                        .expect("current history");
                    let mut live = std::mem::take(&mut history.live);
                    let running = history.running;
                    live.sort_by_key(|event| event.sequence);
                    for event in live {
                        self.apply(&request.branch_id, event);
                    }
                    if request.branch_id == "main"
                        && let Some(running) = running
                    {
                        self.projection.set_running(running);
                    }
                } else {
                    self.task_history_queue.push_back(request.branch_id);
                }
            }
        }
        self.changed();
        Ok(())
    }

    pub(super) fn stop_tasks(&mut self, session_id: String, projection_id: u64) {
        if self.task_stops.is_some() {
            self.send(Update::Notice(
                "Stop-all requests are already in progress".into(),
            ));
            return;
        }
        if self.session_id.is_empty()
            || session_id != self.session_id
            || projection_id != self.projection_id
        {
            self.send(Update::Notice(
                "Task attachment changed; reopen the panel before stopping work".into(),
            ));
            return;
        }
        if self.task_history.values().any(|history| !history.done) {
            self.send(Update::Notice(
                "Branch history is incomplete; stop-all unavailable".into(),
            ));
            return;
        }
        let mut members = HashSet::new();
        let mut targets = VecDeque::new();
        let mut unavailable = 0;
        for task in self
            .projection
            .tasks()
            .iter()
            .filter(|task| task.finished.is_none())
        {
            match &task.kind {
                TaskKind::Command(_) => targets.push_back(task.kind.clone()),
                TaskKind::Member {
                    session_id: Some(id),
                } if members.insert(id.clone()) => targets.push_back(task.kind.clone()),
                TaskKind::Member { session_id: None } => unavailable += 1,
                _ => {}
            }
        }
        self.task_stops = Some(StopBatch {
            session_id,
            targets,
            request: None,
            attempted: 0,
            errors: 0,
            unavailable,
        });
    }

    pub(super) async fn poll_task_stops(&mut self) {
        let Some(batch) = &mut self.task_stops else {
            return;
        };
        if batch.request.is_some() {
            return;
        }
        let Some(target) = batch.targets.pop_front() else {
            let batch = self.task_stops.take().expect("finished stop batch");
            self.report(format!(
                "Stop-all requests for {}: attempted {}, errors {}, unavailable {}.\nStarting members and new work may continue. If a member profile supplies background commands, they keep running and command ends may wake that member. Built-in member commands remain foreground. Teammate Steer messages may also wake members. Unsuppressed reports may wake or steer the leader, which may start new work during or after stop-all. Rows wait for real terminal facts.",
                batch.session_id, batch.attempted, batch.errors, batch.unavailable,
            ), batch.errors != 0);
            return;
        };
        let (label, method, params) = match target {
            TaskKind::Command(target) => (
                format!("command {}/{}", target.branch_id, target.task_id),
                "session.exec.stop",
                json!({
                    "sessionId": batch.session_id, "branchId": target.branch_id, "taskId": target.task_id,
                }),
            ),
            TaskKind::Member {
                session_id: Some(session_id),
            } => (
                format!("member {session_id}"),
                "session.stop",
                json!({
                    "sessionId": session_id, "branchId": "main", "reason": "stopped by the user",
                }),
            ),
            TaskKind::Member { session_id: None } => {
                unreachable!("admission excludes undelivered targets")
            }
        };
        batch.attempted += 1;
        match self.client.send(method, params).await {
            Ok(id) => batch.request = Some((id, label)),
            Err(error) => {
                batch.errors += 1;
                self.send(Update::Notice(format!("Stop-all {label}: {error}")));
                self.changed();
            }
        }
    }

    pub(super) fn task_stop_reply(&mut self, result: Result<serde_json::Value, client::RpcError>) {
        let batch = self.task_stops.as_mut().expect("matched stop reply");
        let (_, target) = batch.request.take().expect("matched stop target");
        if let Err(error) = result {
            batch.errors += 1;
            self.send(Update::Notice(format!("Stop-all {target}: {error}")));
        }
    }
}
