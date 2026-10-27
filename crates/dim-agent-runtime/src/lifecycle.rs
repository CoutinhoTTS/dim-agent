use crate::{AgentError, Result};
use futures_util::FutureExt;
use std::{
    future::Future,
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::watch;

/// Memoizes one owned close task. Cancelling a waiter never cancels cleanup.
#[derive(Default)]
pub(crate) struct CloseOnce<T = ()> {
    started: AtomicBool,
    result: watch::Sender<Option<Result<T>>>,
}
impl<T: Clone + Send + Sync + 'static> CloseOnce<T> {
    pub fn finished(&self) -> bool {
        self.result.borrow().is_some()
    }
    pub fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }
    pub fn start(
        &self,
        handle: &tokio::runtime::Handle,
        work: impl Future<Output = Result<T>> + Send + 'static,
    ) -> rutis::host::BoxFuture<'static, Result<T>> {
        if !self.started.swap(true, Ordering::AcqRel) {
            let send = self.result.clone();
            handle.spawn(async move {
                let result = AssertUnwindSafe(work)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| Err(AgentError::fault("close", "cleanup task panicked")));
                send.send_replace(Some(result));
            });
        }
        self.wait()
    }
    pub fn wait(&self) -> rutis::host::BoxFuture<'static, Result<T>> {
        let mut result = self.result.subscribe();
        Box::pin(async move {
            loop {
                if let Some(done) = result.borrow().clone() {
                    return done;
                }
                result
                    .changed()
                    .await
                    .map_err(|_| AgentError::fault("close", "close result channel ended"))?;
            }
        })
    }
}
/// Reason recorded for work a capability reload left unfinished.
pub const CAPABILITY_RELOADING: &str = "capability reloading";

#[derive(Clone, Default)]
pub struct FaultReporter(Arc<watch::Sender<Option<AgentError>>>);
impl FaultReporter {
    pub fn report(&self, error: AgentError) {
        self.0.send_modify(|v| {
            if v.is_none()
                || matches!(&error, AgentError::Storage(_))
                    && !matches!(v, Some(AgentError::Storage(_)))
            {
                *v = Some(error)
            }
        });
    }
    pub fn watch(&self) -> watch::Receiver<Option<AgentError>> {
        self.0.subscribe()
    }
    pub fn failure(&self) -> Option<AgentError> {
        self.0.borrow().clone()
    }
}

pub(crate) async fn guarded<T>(
    component: &'static str,
    work: impl Future<Output = Result<T>> + Send,
) -> Result<T> {
    AssertUnwindSafe(work)
        .catch_unwind()
        .await
        .unwrap_or_else(|_| Err(AgentError::fault(component, "operation panicked")))
}
