//! Follows one branch the way a host does: listen on the Session broadcast
//! first, then read the Timeline history and drop what the broadcast repeats.
#![allow(dead_code)]
use dim_agent_capabilities::session_broadcast::{SessionBroadcastEvent, session_channel};
use dim_agent_runtime::Session;
use dim_agent_types::AgentEvent;
use rutis::{BoxFuture, CordisError, Ctx, Disposer, Listener};
use std::{collections::VecDeque, sync::Arc};
use tokio::sync::mpsc;

/// A root the test owns, so it can listen on the Agent's rutis bus.
pub fn root() -> Ctx {
    Ctx::root_with(tokio::runtime::Handle::current())
}

struct Forward {
    branch: String,
    sender: mpsc::UnboundedSender<Option<Arc<AgentEvent>>>,
}
impl Listener<SessionBroadcastEvent> for Forward {
    fn call<'a>(
        &'a self,
        _ctx: &'a Ctx,
        record: &'a SessionBroadcastEvent,
    ) -> BoxFuture<'a, Result<Option<()>, CordisError>> {
        match record {
            SessionBroadcastEvent::Event { branch_id, event } if *branch_id == self.branch => {
                let _ = self.sender.send(Some(event.clone()));
            }
            SessionBroadcastEvent::Event { .. } => {}
            SessionBroadcastEvent::Notice(_) | SessionBroadcastEvent::Fence { .. } => {}
            SessionBroadcastEvent::Closed => {
                let _ = self.sender.send(None);
            }
        }
        Box::pin(async { Ok(None) })
    }
}

pub struct Events {
    history: VecDeque<Arc<AgentEvent>>,
    live: mpsc::UnboundedReceiver<Option<Arc<AgentEvent>>>,
    last: u64,
    ended: bool,
    _listener: Disposer,
}
impl Events {
    /// Events of `branch` after `after`, then each one as it is dispatched.
    pub async fn follow(ctx: &Ctx, session: &Session, branch: &str, after: u64) -> Self {
        let (sender, live) = mpsc::unbounded_channel();
        let listener = ctx
            .events()
            .on(
                ctx,
                &session_channel(session.id()),
                Forward {
                    branch: branch.to_owned(),
                    sender,
                },
            )
            .unwrap();
        let history: VecDeque<_> = session
            .local(branch, ..)
            .await
            .unwrap()
            .into_iter()
            .filter(|event| event.sequence > after)
            .collect();
        Self {
            history,
            live,
            last: after,
            ended: false,
            _listener: listener,
        }
    }
    /// `None` once the Session channel has ended.
    pub async fn next(&mut self) -> Option<Arc<AgentEvent>> {
        loop {
            let event = match self.history.pop_front() {
                Some(event) => event,
                None if self.ended => return None,
                None => match self.live.recv().await.flatten() {
                    Some(event) => event,
                    None => {
                        self.ended = true;
                        return None;
                    }
                },
            };
            if event.sequence > self.last {
                self.last = event.sequence;
                return Some(event);
            }
        }
    }
}

/// Reads committed events while dispatch may still be held by a test handler.
pub struct Committed {
    session: Session,
    branch: String,
    last: u64,
}
impl Committed {
    pub fn after(session: &Session, branch: &str, after: u64) -> Self {
        Self {
            session: session.clone(),
            branch: branch.to_owned(),
            last: after,
        }
    }
    pub async fn next(&mut self) -> Arc<AgentEvent> {
        loop {
            let next = self
                .session
                .local(&self.branch, ..)
                .await
                .unwrap()
                .into_iter()
                .find(|event| event.sequence > self.last);
            if let Some(event) = next {
                self.last = event.sequence;
                return event;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }
}

/// Every record of one Session channel; `None` is the end record.
pub type Record = Option<(String, Arc<AgentEvent>)>;
struct Collect(Arc<std::sync::Mutex<Vec<Record>>>, Arc<tokio::sync::Notify>);
impl Listener<SessionBroadcastEvent> for Collect {
    fn call<'a>(
        &'a self,
        _ctx: &'a Ctx,
        record: &'a SessionBroadcastEvent,
    ) -> BoxFuture<'a, Result<Option<()>, CordisError>> {
        self.0.lock().unwrap().push(match record {
            SessionBroadcastEvent::Event { branch_id, event } => {
                Some((branch_id.clone(), event.clone()))
            }
            SessionBroadcastEvent::Closed => None,
            SessionBroadcastEvent::Notice(_) | SessionBroadcastEvent::Fence { .. } => {
                return Box::pin(async { Ok(None) });
            }
        });
        self.1.notify_waiters();
        Box::pin(async { Ok(None) })
    }
}
pub struct Broadcast {
    records: Arc<std::sync::Mutex<Vec<Record>>>,
    changed: Arc<tokio::sync::Notify>,
    _listener: Disposer,
}
impl Broadcast {
    pub fn listen(ctx: &Ctx, session_id: &str) -> Self {
        let records = Arc::default();
        let changed = Arc::new(tokio::sync::Notify::new());
        let listener = ctx
            .events()
            .on(
                ctx,
                &session_channel(session_id),
                Collect(Arc::clone(&records), changed.clone()),
            )
            .unwrap();
        Self {
            records,
            changed,
            _listener: listener,
        }
    }
    /// Every record up to and including the end record.
    pub async fn ended(&self) -> Vec<Record> {
        loop {
            let notified = self.changed.notified();
            {
                let records = self.records.lock().unwrap();
                if records.last().is_some_and(Option::is_none) {
                    return records.clone();
                }
            }
            notified.await;
        }
    }
}
