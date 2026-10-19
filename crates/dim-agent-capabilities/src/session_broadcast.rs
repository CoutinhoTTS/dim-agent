//! Broadcasts every dispatched event of one branch on its Session channel,
//! which also carries owners' notices about the Session's process-level state.
use dim_agent_runtime::{
    BranchScope, Capability, Dispatcher, InBranch, InSession, KeysOf, SessionIdentity,
    SessionScope, TimelineIdentity, scoped,
};
use dim_agent_types::AgentEvent;
use rutis::{BoxFuture, CordisError, Ctx, Effect, Event, EventKey, TypedPlugin};
use serde::Serialize;
use std::sync::Arc;

/// A record on one Session's channel; the channel name identifies the Session.
pub enum SessionBroadcastEvent {
    /// One event dispatched by its branch.
    Event {
        branch_id: String,
        event: Arc<AgentEvent>,
    },
    /// A committed change of process-level state about this Session.
    Notice(Arc<SessionNotice>),
    /// A host observation fence. Tokens have no Agent or wire semantics.
    Fence { recipient: String, token: u64 },
    /// The last event record: every branch has stopped dispatching. Host
    /// fences and notices racing the close may follow; listeners can settle
    /// fences from this end record.
    Closed,
}
impl Event for SessionBroadcastEvent {
    const NAME: &'static str = "session.broadcast";
    type Value = ();
}

/// The rutis channel shared by the broadcaster and every listener.
pub fn session_channel(session_id: &str) -> EventKey<SessionBroadcastEvent> {
    EventKey::dynamic(format!("session/{session_id}"))
}

/// An owner's announcement that its process-level state about a Session
/// changed. The broadcast and Rpc never interpret the fields; it has no
/// sequence and is never replayed.
#[derive(Debug, Serialize)]
pub struct SessionNotice {
    /// The owner, such as `SessionStore`.
    pub source: String,
    /// The owner's kind of change, such as `title`.
    pub kind: String,
    /// The owner's data for that kind.
    pub data: serde_json::Value,
}

/// Announces a committed change on the Session's channel, from the owner's
/// own context after the commit. It does not wait for listeners: without one
/// (the Session is not loaded or not observed) the notice is dropped.
pub fn notify(ctx: &Ctx, session_id: &str, notice: SessionNotice) {
    if let Err(error) = ctx.events().emit(
        ctx,
        &session_channel(session_id),
        Arc::new(SessionBroadcastEvent::Notice(Arc::new(notice))),
    ) {
        tracing::warn!(%error, session_id, "failed to emit a Session notice");
    }
}

pub struct SessionBroadcastPlugin {
    scope: BranchScope,
}
impl SessionBroadcastPlugin {
    pub fn new(scope: &BranchScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for SessionBroadcastPlugin {
    type Deps = (InBranch<Dispatcher>, InBranch<TimelineIdentity>);
    fn name(&self) -> &str {
        "SessionBroadcast"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InBranch(dispatcher), InBranch(identity)): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let channel = session_channel(&identity.session_id);
            let branch_id = identity.branch_id.clone();
            let emitter = ctx.clone();
            dispatcher.subscribe_all(ctx, "SessionBroadcast", move |event| {
                let _ = emitter.events().emit(
                    &emitter,
                    &channel,
                    Arc::new(SessionBroadcastEvent::Event {
                        branch_id: branch_id.clone(),
                        event,
                    }),
                );
                Box::pin(async { Ok(()) })
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for SessionBroadcastPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

/// Ends the Session channel. Session capabilities unload after every branch
/// closed, so this record follows every broadcast event.
pub struct SessionBroadcastEndPlugin {
    scope: SessionScope,
}
impl SessionBroadcastEndPlugin {
    pub fn new(scope: &SessionScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for SessionBroadcastEndPlugin {
    type Deps = (InSession<SessionIdentity>,);
    fn name(&self) -> &str {
        "SessionBroadcastEnd"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (InSession(identity),): Self::Deps,
    ) -> BoxFuture<'a, std::result::Result<Effect, CordisError>> {
        Box::pin(async move {
            let channel = session_channel(&identity.session_id);
            let emitter = ctx.clone();
            ctx.effect(move || {
                Effect::Disposer(Box::new(move || {
                    if !identity.unloading.is_cancelled() {
                        let _ = emitter.events().emit(
                            &emitter,
                            &channel,
                            Arc::new(SessionBroadcastEvent::Closed),
                        );
                    }
                    Ok(())
                }))
            })?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for SessionBroadcastEndPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}
