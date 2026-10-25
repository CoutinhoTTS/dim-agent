//! Broadcasts every dispatched event of one branch on its Session channel,
//! which also carries owners' notices about the Session's process-level state.
use dim_agent_runtime::{Dispatcher, SessionIdentity, TimelineIdentity};
use dim_agent_types::AgentEvent;
use rutis::{BoxFuture, CordisError, Ctx, Effect, Event, EventKey, Keyed, TypedPlugin};
use serde::{Deserialize, Serialize};
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
    // Dispatched only under a Session's name ([`session_channel`]).
    const NAME: &'static str = "";
    type Value = ();
}

/// The public serde form of a [`SessionBroadcastEvent`], which every runtime
/// listening to `session/{id}` receives: `{kind: "event", branchId, event}`,
/// `{kind: "notice", notice}`, `{kind: "fence", recipient, token}` or
/// `{kind: "closed"}`. A notice keeps its own `kind` field inside `notice`.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum WireRef<'a> {
    Event {
        #[serde(rename = "branchId")]
        branch_id: &'a str,
        event: &'a AgentEvent,
    },
    Notice {
        notice: &'a SessionNotice,
    },
    Fence {
        recipient: &'a str,
        token: u64,
    },
    Closed,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Wire {
    Event {
        #[serde(rename = "branchId")]
        branch_id: String,
        event: Arc<AgentEvent>,
    },
    Notice {
        notice: Arc<SessionNotice>,
    },
    Fence {
        recipient: String,
        token: u64,
    },
    Closed,
}
impl Serialize for SessionBroadcastEvent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Event { branch_id, event } => WireRef::Event { branch_id, event },
            Self::Notice(notice) => WireRef::Notice { notice },
            Self::Fence { recipient, token } => WireRef::Fence {
                recipient,
                token: *token,
            },
            Self::Closed => WireRef::Closed,
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for SessionBroadcastEvent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Event { branch_id, event } => Self::Event { branch_id, event },
            Wire::Notice { notice } => Self::Notice(notice),
            Wire::Fence { recipient, token } => Self::Fence { recipient, token },
            Wire::Closed => Self::Closed,
        })
    }
}

/// The rutis channel shared by the broadcaster and every listener.
pub fn session_channel(session_id: &str) -> EventKey<SessionBroadcastEvent> {
    EventKey::dynamic(format!("session/{session_id}"))
}

/// An owner's announcement that its process-level state about a Session
/// changed. The broadcast and Rpc never interpret the fields; it has no
/// sequence and is never replayed.
#[derive(Debug, Serialize, Deserialize)]
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

pub struct SessionBroadcastPlugin;
impl TypedPlugin for SessionBroadcastPlugin {
    type Deps = (Keyed<Dispatcher>, Keyed<TimelineIdentity>);
    fn name(&self) -> &str {
        "SessionBroadcast"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(dispatcher), Keyed(identity)): Self::Deps,
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
/// Ends the Session channel. Session capabilities unload after every branch
/// closed, so this record follows every broadcast event.
pub struct SessionBroadcastEndPlugin;
impl TypedPlugin for SessionBroadcastEndPlugin {
    type Deps = (Keyed<SessionIdentity>,);
    fn name(&self) -> &str {
        "SessionBroadcastEnd"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (Keyed(identity),): Self::Deps,
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
