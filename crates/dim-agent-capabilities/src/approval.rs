//! Branch approval: asks the user when Permission decides a call needs
//! approval. Writes `approval/*` events and waits for this branch's dispatched
//! `approval/respond`. Interrupts arrive through the caller's cancellation
//! token; branch close and reload settle what remains.
//!
//! Every path that ends a request removes it under the Approval lock and
//! writes `approval/cancelled` after releasing it, while [`Approval::replay`]
//! reads under the same lock: the end of a replayed request always follows
//! the replay, and a removed request is not replayed.
use crate::{
    host::HostError,
    permission::{Decision, Permission},
};
use dim_agent_runtime::{
    AgentError, BranchScope, CAPABILITY_RELOADING, Capability, Dispatcher, InBranch, KeysOf,
    Timeline, scoped,
};
use dim_agent_types::{
    AgentEvent, ApprovalCallData, ApprovalCancelledData, ApprovalDecision, ApprovalRequestData,
    ApprovalRespondData, BranchSource, EventKind,
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const CANCELLED: &str = "Tool execution cancelled";

pub struct AdmitCall {
    pub source: BranchSource,
    pub call: ApprovalCallData,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalRespondError {
    #[error("Approval is absent or already resolved")]
    Unknown,
    #[error("Approval was cancelled")]
    Cancelled,
    #[error("branch is closed")]
    Closed,
}

struct Pending {
    request: ApprovalRequestData,
    cancel: CancellationToken,
    /// `None`: a response arrived after the caller's batch was cancelled.
    /// A dropped sender means branch close or reload.
    response: oneshot::Sender<Option<ApprovalDecision>>,
}

#[derive(Default)]
struct Inner {
    closed: bool,
    /// Events up to this sequence were applied by the rebuild at load.
    rebuilt: u64,
    /// Committed requests not yet ended, in request order.
    pending: Vec<Pending>,
    /// `respond` waiters by event id, released when the event is dispatched.
    responses: HashMap<String, oneshot::Sender<Result<(), ApprovalRespondError>>>,
}

pub struct Approval {
    timeline: Arc<Timeline>,
    permission: Arc<Permission>,
    inner: Mutex<Inner>,
}
impl Approval {
    /// Calls `notify` with each pending request, in request order, under the
    /// Approval lock; see the module documentation for the ordering.
    pub fn replay(&self, mut notify: impl FnMut(&ApprovalRequestData)) {
        for item in &self.inner.lock().unwrap().pending {
            notify(&item.request);
        }
    }
    /// `Ok` means the call may start. `cancel` is the caller's batch token;
    /// the last cancellation check before the start happens here.
    pub async fn admit(
        &self,
        call: AdmitCall,
        risk: Option<&str>,
        cancel: CancellationToken,
    ) -> Result<(), HostError> {
        let reason = match self.permission.decide(&call.call, risk, &cancel).await? {
            Decision::Allow if cancel.is_cancelled() => {
                return Err(HostError::Cancelled(CANCELLED.into()));
            }
            Decision::Allow => return Ok(()),
            Decision::Deny { reason } => return Err(HostError::failed(reason)),
            Decision::Ask { reason } => reason,
        };
        if cancel.is_cancelled() {
            return Err(HostError::Cancelled(CANCELLED.into()));
        }
        // The call that runs is rechecked after an allow response.
        let checked = call.call.clone();
        let (response, received) = oneshot::channel();
        let registered = cancel.clone();
        let mut registered_id = None;
        // Registered from the final event under the commit lock: a replay after
        // this point includes the request, a listener before it receives it.
        let committed = self.timeline.append_registered(
            EventKind::ApprovalRequest(ApprovalRequestData {
                approval_id: uuid::Uuid::new_v4().to_string(),
                source: call.source,
                call: call.call,
                reason,
            })
            .into(),
            |event| {
                let EventKind::ApprovalRequest(data) = &event.kind else {
                    unreachable!("before transforms keep the event key");
                };
                let mut inner = self.inner.lock().unwrap();
                if inner.closed {
                    return Err(AgentError::Closed("Approval".into()));
                }
                inner.pending.push(Pending {
                    request: data.clone(),
                    cancel: registered,
                    response,
                });
                registered_id = Some(data.approval_id.clone());
                Ok(())
            },
        );
        let event = match committed {
            Ok(event) => event,
            Err(error) => {
                // A request that was not committed is not pending.
                if let Some(id) = registered_id {
                    take(&mut self.inner.lock().unwrap(), &id);
                }
                return Err(match error {
                    AgentError::Closed(_) | AgentError::Cancelled(_) => {
                        HostError::Cancelled(error.to_string())
                    }
                    error => HostError::failed(error.to_string()),
                });
            }
        };
        let EventKind::ApprovalRequest(data) = &event.kind else {
            unreachable!("before transforms keep the event key");
        };
        let approval_id = &data.approval_id;
        let decision = tokio::select! {
            biased;
            decision = received => decision,
            _ = cancel.cancelled() => {
                // A response consumed first has already ended the request.
                let taken = take(&mut self.inner.lock().unwrap(), approval_id);
                if taken.is_some() {
                    self.cancelled(approval_id, CANCELLED);
                }
                return Err(HostError::Cancelled(CANCELLED.into()));
            }
        };
        match decision {
            // Branch close or reload settled the request.
            Err(_) => Err(HostError::Cancelled(CAPABILITY_RELOADING.into())),
            // The response found the batch cancelled and ended the request.
            Ok(None) => Err(HostError::Cancelled(CANCELLED.into())),
            Ok(Some(ApprovalDecision::Deny)) => {
                Err(HostError::failed("Tool execution denied by user"))
            }
            // Re-check the policy that may have changed while waiting; a
            // repeated Ask counts as approved.
            Ok(Some(ApprovalDecision::Allow)) => {
                match self.permission.decide(&checked, risk, &cancel).await {
                    Ok(Decision::Deny { reason }) => {
                        self.cancelled(approval_id, &reason);
                        Err(HostError::failed(reason))
                    }
                    // Allowed, then stopped before the tool started.
                    Ok(_) if cancel.is_cancelled() => {
                        self.cancelled(approval_id, CANCELLED);
                        Err(HostError::Cancelled(CANCELLED.into()))
                    }
                    Ok(_) => Ok(()),
                    // The rules read was cancelled.
                    Err(error) => {
                        self.cancelled(approval_id, CANCELLED);
                        Err(error)
                    }
                }
            }
        }
    }
    /// `Ok` means the decision was dispatched and accepted, not that the tool
    /// started.
    pub async fn respond(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), ApprovalRespondError> {
        {
            let inner = self.inner.lock().unwrap();
            if inner.closed {
                return Err(ApprovalRespondError::Closed);
            }
            if !inner
                .pending
                .iter()
                .any(|item| item.request.approval_id == approval_id)
            {
                return Err(ApprovalRespondError::Unknown);
            }
        }
        let (waiter, result) = oneshot::channel();
        let mut registered_id = None;
        let committed = self.timeline.append_registered(
            EventKind::ApprovalRespond(ApprovalRespondData {
                approval_id: approval_id.to_owned(),
                decision,
            })
            .into(),
            |event| {
                let mut inner = self.inner.lock().unwrap();
                if inner.closed {
                    return Err(AgentError::Closed("Approval".into()));
                }
                // Registered inside the commit lock, before the event is
                // visible to the consuming subscription.
                inner.responses.insert(event.id.clone(), waiter);
                registered_id = Some(event.id.clone());
                Ok(())
            },
        );
        if committed.is_err() {
            if let Some(id) = registered_id {
                self.inner.lock().unwrap().responses.remove(&id);
            }
            return Err(ApprovalRespondError::Closed);
        }
        result.await.unwrap_or(Err(ApprovalRespondError::Closed))
    }
    fn consume(&self, event: &AgentEvent) {
        let EventKind::ApprovalRespond(data) = &event.kind else {
            return;
        };
        let mut inner = self.inner.lock().unwrap();
        if event.sequence <= inner.rebuilt {
            return;
        }
        let mut cancelled = false;
        let result = match take(&mut inner, &data.approval_id) {
            Some(item) if item.cancel.is_cancelled() => {
                let _ = item.response.send(None);
                cancelled = true;
                Err(ApprovalRespondError::Cancelled)
            }
            Some(item) => {
                let _ = item.response.send(Some(data.decision));
                Ok(())
            }
            None => Err(ApprovalRespondError::Unknown),
        };
        if let Some(waiter) = inner.responses.remove(&event.id) {
            let _ = waiter.send(result);
        }
        drop(inner);
        if cancelled {
            self.cancelled(&data.approval_id, CANCELLED);
        }
    }
    /// Ends a request. Call without the lock: Timeline registration callbacks
    /// take it under the commit lock. A closed branch no longer records it.
    fn cancelled(&self, approval_id: &str, reason: &str) {
        let _ = self.timeline.append(
            EventKind::ApprovalCancelled(ApprovalCancelledData {
                approval_id: approval_id.to_owned(),
                reason: reason.to_owned(),
            })
            .into(),
        );
    }
    /// Settles every pending request as the branch closes or the capability
    /// reloads.
    fn close(&self) {
        let pending: Vec<_> = {
            let mut inner = self.inner.lock().unwrap();
            inner.closed = true;
            // Dropped waiters report `Closed`.
            inner.responses.clear();
            std::mem::take(&mut inner.pending)
        };
        // Each waiter is released after its request is recorded as ended.
        for item in pending {
            self.cancelled(&item.request.approval_id, CAPABILITY_RELOADING);
        }
    }
}

fn take(inner: &mut Inner, approval_id: &str) -> Option<Pending> {
    let index = inner
        .pending
        .iter()
        .position(|item| item.request.approval_id == approval_id)?;
    Some(inner.pending.remove(index))
}

pub struct ApprovalPlugin {
    scope: BranchScope,
}
impl ApprovalPlugin {
    pub fn new(scope: &BranchScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for ApprovalPlugin {
    type Deps = (
        InBranch<Timeline>,
        InBranch<Dispatcher>,
        InBranch<Permission>,
    );
    fn name(&self) -> &str {
        "Approval"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InBranch(timeline), InBranch(dispatcher), InBranch(permission)): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Arc::new(Approval {
                timeline: timeline.clone(),
                permission,
                inner: Mutex::new(Inner::default()),
            });
            let closing = service.clone();
            ctx.effect(move || {
                Effect::Disposer(Box::new(move || {
                    closing.close();
                    Ok(())
                }))
            })?;
            // Subscribe, then rebuild while the gate holds handlers; see the
            // reload protocol. Requests of an earlier generation were settled
            // unless the process stopped.
            let gate = Arc::new(tokio::sync::Mutex::new(()));
            let rebuilding = gate.lock().await;
            let owner = service.clone();
            let handler_gate = gate.clone();
            dispatcher.subscribe(ctx, "Approval", &["approval/respond"], move |event| {
                let gate = handler_gate.clone();
                let owner = owner.clone();
                Box::pin(async move {
                    let _ready = gate.lock().await;
                    owner.consume(&event);
                    Ok(())
                })
            })?;
            let through = timeline.committed();
            let events = timeline.local(..=through).await?;
            let mut open = Vec::new();
            for event in &events {
                match &event.kind {
                    EventKind::ApprovalRequest(data) => open.push(data.approval_id.clone()),
                    EventKind::ApprovalRespond(ApprovalRespondData { approval_id, .. })
                    | EventKind::ApprovalCancelled(ApprovalCancelledData { approval_id, .. }) => {
                        open.retain(|id| id != approval_id)
                    }
                    _ => {}
                }
            }
            for approval_id in open {
                timeline.append(
                    EventKind::ApprovalCancelled(ApprovalCancelledData {
                        approval_id,
                        reason: "Approval cancelled: execution did not survive recovery".into(),
                    })
                    .into(),
                )?;
            }
            service.inner.lock().unwrap().rebuilt = events.last().map_or(0, |event| event.sequence);
            drop(rebuilding);
            ctx.provide_as(self.scope.key::<Approval>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for ApprovalPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}
