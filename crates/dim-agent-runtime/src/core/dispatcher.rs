use super::keys::KeyFilter;
use crate::{AgentError, BoxFuture, Result, Timeline, panic_message};
use dim_agent_types::AgentEvent;
use futures_util::FutureExt;
use rutis::host::{ErrorKind, InstanceId};
use rutis::service;
use rutis::{Ctx, Error, Event};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// The committed timeline event delivered through the rutis event bus, under
/// its branch's private name ([`committed_name`]).
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Committed {
    /// The name it is dispatched under, not part of the value.
    #[serde(skip)]
    name: String,
    event: Arc<AgentEvent>,
}
impl Event for Committed {
    const NAME: &'static str = "";
    type Value = ();
    fn name(&self) -> &str {
        &self.name
    }
}

/// The branch's private channel. The bus is flat, so the name carries the
/// branch instance; only the Dispatcher builds it.
fn committed_name(instance: InstanceId) -> String {
    format!("dim/timeline/{instance}/committed")
}

type Handler = dyn Fn(Arc<AgentEvent>) -> BoxFuture<'static, Result<()>> + Send + Sync;

/// Forwards committed events to one subscriber: unmatched keys are skipped,
/// handler errors become an explicit dispatcher failure, and handler panics
/// are attributed to the subscriber before the defect stops the branch.
struct SubscriptionListener {
    name: String,
    keys: KeyFilter,
    handler: Arc<Handler>,
    failures: Dispatcher,
}
impl SubscriptionListener {
    async fn on_event(
        &self,
        ctx: &Ctx,
        event: &Committed,
    ) -> std::result::Result<Option<()>, Error> {
        if !self.keys.matches(event.event.kind.key()) {
            return Ok(None);
        }
        let (name, failures) = (&self.name, &self.failures);
        let cancelled = ctx.cancelled();
        let future = match catch_unwind(AssertUnwindSafe(|| (self.handler)(event.event.clone()))) {
            Ok(future) => future,
            Err(panic) => return Err(self::panic(failures, name, panic)),
        };
        tokio::select! {
            _ = cancelled => {}
            result = AssertUnwindSafe(future).catch_unwind() => match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.fail(error),
                Err(panic) => return Err(self::panic(failures, name, panic)),
            },
        }
        Ok(None)
    }
}

/// Records an attributed fault and returns the defect that stops dispatch.
fn panic(failures: &Dispatcher, name: &str, payload: Box<dyn std::any::Any + Send>) -> Error {
    failures.fail(AgentError::fault(name, panic_message(payload)));
    rutis::Error::from(ErrorKind::PluginFailed(
        format!("{name} handler panicked").into(),
    ))
}

/// What an ordered consumer registered with [`Dispatcher::follow`] receives:
/// the replay once, then live events.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Delivery {
    /// The branch's own committed events through the cutoff, as
    /// [`Dispatcher::local`] reads them. They are not filtered by the keys:
    /// a rebuild also reads facts it does not consume live, such as the
    /// terminals of its own operations.
    Replay(Vec<Arc<AgentEvent>>),
    /// A committed event after the cutoff that matches the keys.
    Live(Arc<AgentEvent>),
}

#[derive(Clone, Debug, Default)]
pub struct DispatchState {
    pub sequence: u64,
    pub failure: Option<AgentError>,
}
/// Delivers one branch's committed events on that branch's private channel.
#[derive(Clone)]
pub struct Dispatcher {
    state: Arc<watch::Sender<DispatchState>>,
    instance: InstanceId,
    timeline: Timeline,
}
#[service]
impl Dispatcher {
    pub(crate) fn new(instance: InstanceId, sequence: u64, timeline: Timeline) -> Self {
        Self {
            state: Arc::new(
                watch::channel(DispatchState {
                    sequence,
                    failure: None,
                })
                .0,
            ),
            instance,
            timeline,
        }
    }
    /// Registers a bus listener under `name` for the given event keys: exact
    /// keys, `"*"` for every key, or `"domain/*"` for every key in a domain.
    ///
    /// Handler errors are infrastructure failures: they surface as an
    /// explicit dispatcher failure instead of propagating through dispatch.
    /// Business outcomes are written to the Timeline by the handler itself.
    /// Handler panics are attributed to `name` and stop the branch.
    ///
    /// The listener lives on the branch's private channel and is removed
    /// with the registering plugin.
    #[service(skip)]
    pub fn subscribe(
        &self,
        ctx: &Ctx,
        name: impl Into<String>,
        keys: &[&str],
        handler: impl Fn(Arc<AgentEvent>) -> BoxFuture<'static, Result<()>> + Send + Sync + 'static,
    ) -> Result<()> {
        let keys = KeyFilter::new(keys)?;
        let handler: Arc<Handler> = Arc::new(handler);
        let listener = Arc::new(SubscriptionListener {
            name: name.into(),
            keys,
            handler,
            failures: self.clone(),
        });
        ctx.listen::<Committed>()
            .name(committed_name(self.instance))
            .on(move |event, call| {
                let listener = listener.clone();
                async move { listener.on_event(call.ctx(), &event).await }
            })
            .map_err(AgentError::from)?;
        Ok(())
    }
    /// Registers an ordered consumer that rebuilds from history: the
    /// subscribe and rebuild steps of the capability reload protocol as one
    /// registration. It subscribes like [`Self::subscribe`], fixes the cutoff
    /// `S` = [`Self::committed`], hands the handler [`Delivery::Replay`] of
    /// `local(..=S)`, then [`Delivery::Live`] for each matching event after
    /// `S`, in sequence order. It returns once the replay is handled, and
    /// live events wait until then, so work the replay leaves to do, such as
    /// terminals for open operations, is done in the replay call. A replay
    /// failure is returned and lets no live event through; live failures and
    /// the lifetime are [`Self::subscribe`]'s.
    #[service(skip)]
    pub async fn follow(
        &self,
        ctx: &Ctx,
        name: impl Into<String>,
        keys: &[&str],
        handler: impl Fn(Delivery) -> BoxFuture<'static, Result<()>> + Send + Sync + 'static,
    ) -> Result<()> {
        let handler = Arc::new(handler);
        let cutoff = Arc::new(tokio::sync::Mutex::new(u64::MAX));
        let mut replayed = cutoff.lock().await;
        let live = handler.clone();
        let covered = cutoff.clone();
        self.subscribe(ctx, name, keys, move |event| {
            let live = live.clone();
            let covered = covered.clone();
            Box::pin(async move {
                if event.sequence <= *covered.lock().await {
                    return Ok(());
                }
                live(Delivery::Live(event)).await
            })
        })?;
        let through = self.committed();
        handler(Delivery::Replay(self.timeline.local(..=through).await?)).await?;
        *replayed = through;
        Ok(())
    }
    /// Registers a bus listener for every committed event of this branch;
    /// same failure and lifetime rules as [`Self::subscribe`].
    #[service(skip)]
    pub fn subscribe_all(
        &self,
        ctx: &Ctx,
        name: impl Into<String>,
        handler: impl Fn(Arc<AgentEvent>) -> BoxFuture<'static, Result<()>> + Send + Sync + 'static,
    ) -> Result<()> {
        self.subscribe(ctx, name, &["*"], handler)
    }
    /// [`Self::subscribe`] in data form, for parts in other languages: the
    /// calling row's `ctx`, owned keys, and a handler taking the committed
    /// event by value whose failure is the subscriber's handler failure.
    pub fn listen(
        &self,
        ctx: &Ctx,
        name: String,
        keys: Vec<String>,
        handler: Arc<
            dyn Fn(AgentEvent) -> BoxFuture<'static, std::result::Result<(), Error>> + Send + Sync,
        >,
    ) -> Result<()> {
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        self.subscribe(ctx, name, &keys, move |event| {
            let handler = handler.clone();
            Box::pin(async move { handler((*event).clone()).await.map_err(AgentError::from) })
        })
    }
    /// [`Self::follow`] in data form, for parts in other languages: the
    /// calling row's `ctx`, the keys, and a handler taking each delivery by
    /// value whose failure is the replay's or the subscriber's handler
    /// failure. It resolves once the replay is handled.
    pub async fn consume(
        &self,
        ctx: &Ctx,
        name: String,
        keys: Vec<String>,
        handler: Arc<
            dyn Fn(Delivery) -> BoxFuture<'static, std::result::Result<(), Error>> + Send + Sync,
        >,
    ) -> std::result::Result<(), AgentError> {
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        self.follow(ctx, name, &keys, move |delivery| {
            let handler = handler.clone();
            Box::pin(async move { handler(delivery).await.map_err(AgentError::from) })
        })
        .await
    }
    /// The branch's committed sequence: a subscriber's rebuild cutoff. It
    /// rebuilds from [`Self::local`] through it and skips live events at or
    /// below it.
    pub fn committed(&self) -> u64 {
        self.timeline.committed()
    }
    // The exported reads spell out `std::result::Result`: rutis' export
    // generator resolves an alias's item type where the alias is defined
    // (arcships/rutis#463).
    /// This branch's own committed events in `start..=through`.
    pub async fn local(
        &self,
        start: u64,
        through: u64,
    ) -> std::result::Result<Vec<Arc<AgentEvent>>, AgentError> {
        self.timeline.local(start..=through).await
    }
    /// The branch's committed events through `through`, ancestor prefix included.
    pub async fn history(
        &self,
        through: u64,
    ) -> std::result::Result<Vec<Arc<AgentEvent>>, AgentError> {
        self.timeline.history(Some(through)).await
    }
    pub fn notify(&self) {
        self.state.send_modify(|_| {});
    }
    #[service(skip)]
    pub fn fail(&self, error: AgentError) {
        self.state.send_modify(|s| {
            if s.failure.is_none() {
                s.failure = Some(error);
            }
        });
    }
    pub fn sequence(&self) -> u64 {
        self.state.borrow().sequence
    }
    #[service(skip)]
    pub fn failure(&self) -> Option<AgentError> {
        self.state.borrow().failure.clone()
    }
    #[service(skip)]
    pub fn watch(&self) -> watch::Receiver<DispatchState> {
        self.state.subscribe()
    }
    pub async fn changed(&self) {
        let _ = self.watch().changed().await;
    }
    pub(crate) async fn run(&self, timeline: Timeline, ctx: Ctx, stop: CancellationToken) {
        let mut reader = timeline.read(timeline.restored_sequence());
        let mut changed = self.watch();
        loop {
            if self.failure().is_some() {
                return;
            }
            let event = tokio::select! {
                _=stop.cancelled()=>return,
                _=changed.changed()=>continue,
                value=reader.next()=>match value{Ok(Some(e))=>e,Ok(None)=>return,Err(error)=>{self.fail(error);return;}},
            };
            let committed = Committed {
                name: committed_name(self.instance),
                event: event.clone(),
            };
            if let Err(error) = ctx.serial(committed).await {
                self.fail(AgentError::from(error));
                return;
            }
            timeline.dispatched(event.sequence);
            self.state.send_modify(|s| s.sequence = event.sequence);
        }
    }
}
