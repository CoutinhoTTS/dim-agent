use crate::ServerError;
use dim_agent_runtime::{Capability, KeysOf};
use rutis::{BoxFuture, CordisError, Ctx, Effect, TypedPlugin};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// Process admission: the readiness gate, shutdown intent and listener faults.
pub struct ServerControl {
    ready: watch::Sender<bool>,
    shutdown: CancellationToken,
    force: AtomicBool,
    fault: Mutex<Option<ServerError>>,
}
impl ServerControl {
    fn new() -> Self {
        Self {
            ready: watch::channel(false).0,
            shutdown: CancellationToken::new(),
            force: AtomicBool::new(false),
            fault: Mutex::new(None),
        }
    }
    pub(crate) fn open(&self) {
        self.ready.send_replace(true);
    }
    /// Resolves once every selected plugin is ready; never after shutdown began.
    pub(crate) async fn opened(&self) -> bool {
        let mut ready = self.ready.subscribe();
        tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => false,
            result = ready.wait_for(|ready| *ready) => result.is_ok(),
        }
    }
    pub(crate) fn is_ready(&self) -> bool {
        *self.ready.borrow() && !self.shutdown.is_cancelled()
    }
    pub(crate) fn shutdown(&self) -> &CancellationToken {
        &self.shutdown
    }
    pub(crate) fn force(&self) {
        self.force.store(true, Ordering::Release);
    }
    pub(crate) fn forced(&self) -> bool {
        self.force.load(Ordering::Acquire)
    }
    /// A listener exit is a process failure; the first one is kept.
    pub(crate) fn fail(&self, error: ServerError) {
        self.fault
            .lock()
            .expect("server fault")
            .get_or_insert(error);
        self.shutdown.cancel();
    }
    pub(crate) fn take_fault(&self) -> Option<ServerError> {
        self.fault.lock().expect("server fault").take()
    }
}

#[derive(Clone)]
pub(crate) struct ServerControlPlugin(pub Arc<ServerControl>);
impl ServerControlPlugin {
    pub fn new() -> Self {
        Self(Arc::new(ServerControl::new()))
    }
}
impl TypedPlugin for ServerControlPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "ServerControl"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide_as(rutis::TypeKey::of::<ServerControl>(), self.0.clone())?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for ServerControlPlugin {
    fn keys(&self) -> KeysOf<Self> {}
}
