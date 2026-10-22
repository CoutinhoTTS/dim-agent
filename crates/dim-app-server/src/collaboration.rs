//! Team collaboration, see `docs/design/appserver/collaboration.md`.
//!
//! `CollaborationDirectoryPlugin` (process) owns member relations and temporary
//! Agent records across reloads. `CollaborationCapability` (Session) provides the `agent` tool and
//! registers each branch; `CollaborationTimeline` (branch) binds the branch's
//! Dispatcher to the business that writes its collaboration facts. Peers are
//! only reached through their public Session interface, on their main branch.
//! An open request holds its leader (`SessionLifecycle::hold`) until its
//! `result` is written, so an idle unload never ends it.
use crate::agents::{AgentIndex, AgentProfile, SessionRestored};
use dim_agent_capabilities::{
    context_block,
    host::{HostError, HostFuture, ToolExecutionContext},
    timeline_tree::TreeCommands,
    tool_registry::{ToolContribution, ToolHandler, definition},
};
use dim_agent_runtime::{
    AgentBuilder, AgentError, BranchLifecycle, BranchScope, Capability, CollaborationRole,
    Dispatcher, InBranch, InSession, KeysOf, Session, SessionConfig, SessionHold, SessionIdentity,
    SessionLifecycle, SessionScope, SessionService, Timeline, TimelineIdentity, scoped,
};
use dim_agent_types::*;
use futures_util::{
    FutureExt,
    future::{BoxFuture as WorkFuture, Shared},
};
use rutis::{BoxFuture, CordisError, Ctx, Effect, Listener, TypeKey, TypedPlugin};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, Mutex},
};
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

const AGENT_TOOL: &str = "agent";
const READ_PAGE_LIMIT: usize = 200;
const INTERRUPTED: &str = "interrupted by leader";
const CANCELLED: &str = "cancelled by leader";
const LEADER_CLOSED: &str = "leader closed";
const MEMBER_CLOSED: &str = "member closed";
const RELOADING: &str = dim_agent_runtime::CAPABILITY_RELOADING;

type Refusal = CollaborationErrorCode;
/// Waits for an operation's `delivered` and the cause of a refusal.
type Waiter = watch::Sender<Option<(CollaborationDelivery, Option<String>)>>;

/// A refusal and, when known, its concrete cause for the model.
struct Failure {
    code: Refusal,
    detail: Option<String>,
}
impl From<Refusal> for Failure {
    fn from(code: Refusal) -> Self {
        Self { code, detail: None }
    }
}
fn member_create_failed(detail: String) -> Failure {
    Failure {
        code: Refusal::MemberCreateFailed,
        detail: Some(detail),
    }
}

/// Registers the Session and branch halves; `serve` provides the process half.
pub fn install(builder: &mut AgentBuilder) {
    builder
        .session_tool(AGENT_TOOL)
        .session_plugin(CollaborationCapabilityPlugin::new)
        .timeline_plugin(CollaborationTimelinePlugin::new);
}

// ---------------------------------------------------------------- process

struct Relation {
    leader: String,
    member_agent: String,
}

struct WatchedSession {
    agent_id: String,
    session: Session,
}

/// Process-lifetime records, independent of reloadable AgentIndex logic.
#[derive(Default)]
pub(crate) struct CollaborationDirectory {
    /// Keyed by member Session id: a member belongs to exactly one leader.
    relations: Mutex<BTreeMap<String, Relation>>,
    /// Temporary Agent id -> the leader Session whose Team it serves; each
    /// closes with its last Session.
    temporary: Mutex<BTreeMap<String, String>>,
    /// Watched Session ids and their Agents: closure (treated as deletion)
    /// removes their relations and may close a temporary Agent.
    watched: Mutex<BTreeMap<String, Arc<WatchedSession>>>,
}

/// Provides collaboration records without depending on reloadable services.
pub(crate) struct CollaborationDirectoryPlugin;
impl TypedPlugin for CollaborationDirectoryPlugin {
    type Deps = ();
    fn name(&self) -> &str {
        "CollaborationDirectory"
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx, _: ()) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide(CollaborationDirectory::default())?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for CollaborationDirectoryPlugin {
    fn keys(&self) -> KeysOf<Self> {}
}

/// Member management over stable records. Calls no Session capability and
/// writes no Timeline; observes Sessions only via `closed()`.
pub struct Collaboration {
    index: Arc<AgentIndex>,
    directory: Arc<CollaborationDirectory>,
    tasks: TaskTracker,
    stop: CancellationToken,
}
struct Peer {
    usage: crate::agents::SessionUse,
    agent_id: String,
    session: Session,
}
enum NewMember {
    Agent(String),
    Temporary(AgentProfile),
}
struct Participant {
    role: CollaborationRole,
    agent_id: String,
    session_id: String,
    name: String,
    status: &'static str,
}
impl Collaboration {
    /// Creates a member Session under the target Agent (created first when
    /// temporary) and records the relation.
    async fn establish(
        self: &Arc<Self>,
        leader_id: &str,
        target: NewMember,
    ) -> Result<Peer, Failure> {
        let (leader_host, leader) = self.index.session(leader_id).ok_or(Refusal::Closed)?;
        let (host, owned) = match target {
            // A temporary Agent belongs to the Team of the leader that created it.
            NewMember::Agent(id) if !self.visible(&id, leader_id) => {
                return Err(Refusal::AgentNotFound.into());
            }
            NewMember::Agent(id) => (
                self.index.host(Some(&id)).ok_or(Refusal::AgentNotFound)?,
                false,
            ),
            NewMember::Temporary(profile) => {
                let id = uuid::Uuid::new_v4();
                // Establish privacy before the Agent can enter the live index.
                self.directory
                    .temporary
                    .lock()
                    .unwrap()
                    .insert(id.to_string(), leader_id.to_owned());
                let host = match self.index.create(id, Some(profile)).await {
                    Ok(host) => host,
                    Err(error) => {
                        self.directory
                            .temporary
                            .lock()
                            .unwrap()
                            .remove(&id.to_string());
                        return Err(member_create_failed(error.to_string()));
                    }
                };
                (host, true)
            }
        };
        let mut config =
            crate::agents::session_config(&host.profile, self.index.session_defaults());
        // A member works in its leader's workspace under the leader's policy.
        config.cwd = leader.config().cwd.clone();
        config.permission_policy = leader.config().permission_policy.clone();
        config.collaboration_role = CollaborationRole::Member;
        if self.index.selects_models() {
            let selected = match self.index.model_selection() {
                Some(selection) => selection
                    .0
                    .resolve(
                        host.profile.model.clone(),
                        Some(config.cwd.clone().into()),
                        self.stop.clone(),
                    )
                    .await
                    .map_err(|error| error.to_string()),
                None => Err("model selection is unavailable while its provider reloads".into()),
            };
            config.model = match selected {
                Ok(model) => model,
                Err(error) => {
                    if owned {
                        let _ = self.index.close(&host.agent_id).await;
                        self.directory
                            .temporary
                            .lock()
                            .unwrap()
                            .remove(&host.agent_id);
                    }
                    return Err(member_create_failed(error));
                }
            };
        }
        if !config.tools.iter().any(|tool| tool == AGENT_TOOL) {
            config.tools.push(AGENT_TOOL.into());
        }
        // Clients of the leader's protocol view and address its members.
        let version = self
            .index
            .version_of(leader_id)
            .ok_or_else(|| member_create_failed("leader protocol version is absent".into()))?;
        let session = match self.index.create_session(&host, config, version).await {
            Ok(session) => session,
            Err(error) => {
                if owned {
                    let _ = self.index.close(&host.agent_id).await;
                    self.directory
                        .temporary
                        .lock()
                        .unwrap()
                        .remove(&host.agent_id);
                }
                return Err(member_create_failed(error.to_string()));
            }
        };
        self.directory.relations.lock().unwrap().insert(
            session.id().to_owned(),
            Relation {
                leader: leader_id.to_owned(),
                member_agent: host.agent_id.clone(),
            },
        );
        self.watch(&leader_host.agent_id, leader);
        self.watch(&host.agent_id, session.clone());
        let usage = self
            .index
            .acquire(session.id())
            .await
            .map_err(|_| Refusal::MemberClosed)?;
        Ok(Peer {
            agent_id: host.agent_id.clone(),
            session,
            usage,
        })
    }
    /// A temporary Agent is visible only within its creating leader's Team.
    fn visible(&self, agent_id: &str, leader_id: &str) -> bool {
        self.directory
            .temporary
            .lock()
            .unwrap()
            .get(agent_id)
            .is_none_or(|creator| creator == leader_id)
    }
    fn watch(self: &Arc<Self>, agent_id: &str, session: Session) {
        let watched = {
            let mut entries = self.directory.watched.lock().unwrap();
            if entries
                .get(session.id())
                .is_some_and(|entry| !entry.session.is_closing())
            {
                return;
            }
            let watched = Arc::new(WatchedSession {
                agent_id: agent_id.to_owned(),
                session,
            });
            entries.insert(watched.session.id().to_owned(), watched.clone());
            watched
        };
        self.observe(watched);
    }
    fn observe(self: &Arc<Self>, watched: Arc<WatchedSession>) {
        let this = self.clone();
        let stop = self.stop.clone();
        self.tasks.spawn(async move {
            tokio::select! {
                _ = stop.cancelled() => {}
                _ = watched.session.closed() => this.session_closed(watched).await,
            }
        });
    }
    /// A business close ends the Session's relations: Agents loads a Session
    /// that `session.close` or `session.delete` closed no more in this
    /// process, whether or not a delete removed its rows, and relations are
    /// process memory. An unloaded Session keeps them.
    async fn session_closed(self: &Arc<Self>, watched: Arc<WatchedSession>) {
        let id = watched.session.id();
        {
            let mut entries = self.directory.watched.lock().unwrap();
            if !entries
                .get(id)
                .is_some_and(|entry| Arc::ptr_eq(entry, &watched))
            {
                return;
            }
            entries.remove(id);
        }
        if watched.session.is_unloading() {
            return;
        }
        self.directory
            .relations
            .lock()
            .unwrap()
            .retain(|member, relation| member != id && relation.leader != id);
        let agent_id = &watched.agent_id;
        if !self
            .directory
            .temporary
            .lock()
            .unwrap()
            .contains_key(agent_id)
        {
            return;
        }
        let remaining: Vec<Session> = self
            .index
            .host(Some(agent_id))
            .map(|host| host.agent.sessions())
            .unwrap_or_default()
            .into_iter()
            .filter(|session| session.id() != id && !session.is_closing())
            .collect();
        if remaining.is_empty() {
            // A temporary Agent closes with its last Session.
            let _ = self.index.close(agent_id).await;
            self.directory.temporary.lock().unwrap().remove(agent_id);
        } else {
            for session in remaining {
                self.watch(agent_id, session);
            }
        }
    }
    /// Removes a relation; the member Session and its Agent stay.
    fn remove(&self, member: &str) {
        self.directory.relations.lock().unwrap().remove(member);
    }
    /// Checks the persisted identity without loading a participant.
    fn authorize_peer(&self, from: &str, agent_id: &str, session_id: &str) -> Result<(), Refusal> {
        let relations = self.directory.relations.lock().unwrap();
        let leader = relations.get(from).map(|relation| relation.leader.as_str());
        let team = leader.unwrap_or(from);
        let member = relations
            .get(session_id)
            .is_some_and(|relation| relation.leader == team && relation.member_agent == agent_id);
        if session_id == from
            || !(member || leader == Some(session_id))
            || self
                .index
                .host_by_session(session_id)
                .is_none_or(|host| host.agent_id != agent_id)
        {
            return Err(Refusal::TeamMemberNotFound);
        }
        Ok(())
    }
    async fn resolve(
        self: &Arc<Self>,
        from: &str,
        agent_id: &str,
        session_id: &str,
    ) -> Result<Peer, Refusal> {
        self.authorize_peer(from, agent_id, session_id)?;
        self.lookup(agent_id, session_id).await
    }
    async fn member(
        self: &Arc<Self>,
        leader: &str,
        agent_id: &str,
        session_id: &str,
    ) -> Result<Peer, Refusal> {
        if !self
            .directory
            .relations
            .lock()
            .unwrap()
            .get(session_id)
            .is_some_and(|relation| relation.leader == leader && relation.member_agent == agent_id)
        {
            return Err(Refusal::TeamMemberNotFound);
        }
        self.lookup(agent_id, session_id).await
    }
    async fn lookup(self: &Arc<Self>, agent_id: &str, session_id: &str) -> Result<Peer, Refusal> {
        let usage = self
            .index
            .acquire(session_id)
            .await
            .map_err(|_| Refusal::MemberClosed)?;
        if usage.host.agent_id != agent_id {
            return Err(Refusal::TeamMemberNotFound);
        }
        Ok(Peer {
            agent_id: agent_id.into(),
            session: usage.session.clone(),
            usage,
        })
    }
    /// Every participant of the caller's Team; empty outside a Team.
    fn team(&self, from: &str) -> Vec<Participant> {
        let relations = self.directory.relations.lock().unwrap();
        let leader = relations
            .get(from)
            .map_or(from, |relation| relation.leader.as_str())
            .to_owned();
        let mut members: Vec<(CollaborationRole, String, String)> = relations
            .iter()
            .filter(|(_, relation)| relation.leader == leader)
            .map(|(member, relation)| {
                (
                    CollaborationRole::Member,
                    member.clone(),
                    relation.member_agent.clone(),
                )
            })
            .collect();
        drop(relations);
        if members.is_empty() {
            return vec![];
        }
        let leader_agent = self
            .index
            .host_by_session(&leader)
            .map(|host| host.agent_id.clone())
            .unwrap_or_default();
        members.insert(0, (CollaborationRole::Leader, leader, leader_agent));
        members
            .into_iter()
            .map(|(role, session_id, agent_id)| {
                let (name, status) = match self.index.session(&session_id) {
                    Some((host, session)) if !session.is_closing() => (
                        host.profile.name.clone(),
                        if session.state("main").is_ok_and(|state| state.running) {
                            "working"
                        } else {
                            "idle"
                        },
                    ),
                    _ => (
                        self.index
                            .host(Some(&agent_id))
                            .map(|host| host.profile.name.clone())
                            .unwrap_or_default(),
                        if self.index.session_available(&session_id) {
                            "idle"
                        } else {
                            "closed"
                        },
                    ),
                };
                Participant {
                    role,
                    agent_id,
                    session_id,
                    name,
                    status,
                }
            })
            .collect()
    }
}

impl Listener<SessionRestored> for Arc<Collaboration> {
    fn call<'a>(
        &'a self,
        _: &'a Ctx,
        event: &'a SessionRestored,
    ) -> BoxFuture<'a, Result<Option<()>, CordisError>> {
        let participates = {
            let relations = self.directory.relations.lock().unwrap();
            relations.contains_key(event.session.id())
                || relations
                    .values()
                    .any(|relation| relation.leader == event.session.id())
        };
        if participates
            || self
                .directory
                .temporary
                .lock()
                .unwrap()
                .contains_key(&event.agent_id)
        {
            self.watch(&event.agent_id, event.session.clone());
        }
        Box::pin(async { Ok(None) })
    }
}

/// Process plugin; loads after `Agents` and reloads with it.
pub(crate) struct CollaborationPlugin {}
impl CollaborationPlugin {
    pub fn new() -> Self {
        Self {}
    }
}
impl TypedPlugin for CollaborationPlugin {
    type Deps = (Arc<AgentIndex>, Arc<CollaborationDirectory>);
    fn name(&self) -> &str {
        "Collaboration"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (index, directory): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Arc::new(Collaboration {
                index,
                directory,
                tasks: TaskTracker::new(),
                stop: CancellationToken::new(),
            });
            let owner = service.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        owner.stop.cancel();
                        owner.tasks.close();
                        owner.tasks.wait().await;
                        Ok(())
                    })
                }))
            })?;
            ctx.events()
                .on(ctx, &service.index.session_restored, service.clone())?;
            // Retain handles across the reload gap: even a Session that closed
            // while no observer ran must still clean up its relation/Agent.
            let watched: Vec<_> = service
                .directory
                .watched
                .lock()
                .unwrap()
                .values()
                .cloned()
                .collect();
            for watched in watched {
                service.observe(watched);
            }
            ctx.provide_as(TypeKey::of::<Collaboration>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for CollaborationPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&())
    }
}

// ---------------------------------------------------------------- Session

/// An operation event awaiting its `delivered`, or a delivered request
/// awaiting its `result`.
struct Op {
    turn_id: String,
    request: bool,
    /// Covered by a `stop/interrupt`: no result input.
    suppressed: bool,
    delivered: bool,
    /// Cause of a refusal, handed to the waiting tool call with `delivered`.
    detail: Option<String>,
    waiter: Option<Waiter>,
}
struct BranchRecord {
    timeline: Arc<Timeline>,
    tasks: TaskTracker,
    /// Cancels waiting for members on unload.
    abort: CancellationToken,
    current_turn: Option<String>,
    interrupted: BTreeSet<String>,
    closing: bool,
    ops: BTreeMap<String, Op>,
    /// Member Session id -> this branch's requests to it that are neither
    /// refused nor have a handled `result`. Updated only by handlers, in
    /// sequence order.
    open: BTreeMap<String, BTreeSet<String>>,
    /// Positions only; full result bodies remain in the Timeline store.
    results: BTreeMap<String, u64>,
    /// Delivery work per peer runs in operation event order.
    chains: BTreeMap<String, (u64, Shared<WorkFuture<'static, ()>>)>,
}
/// A delivered or delivering request, until its `result` is written.
struct Request {
    _usage: crate::agents::SessionUse,
    /// Keeps the leader from an idle release while the request is open
    /// (idle unload R4).
    _hold: SessionHold,
    branch: String,
    op: String,
    turn_id: String,
    agent_id: String,
    session: Session,
    input_event_id: Option<String>,
    /// Settlement decided by the leader side instead of the member's work.
    settle: Option<CollaborationOutcome>,
    done: watch::Sender<bool>,
}
#[derive(Default)]
struct State {
    branches: BTreeMap<String, BranchRecord>,
    /// Keyed by member Session id: one request per member.
    requests: BTreeMap<String, Request>,
}

/// Per-Session collaboration: operations this Session starts and the
/// collaboration facts of its Timeline.
pub struct CollaborationCapability {
    collaboration: Arc<Collaboration>,
    session_id: String,
    role: CollaborationRole,
    tree: Arc<TreeCommands>,
    /// Gives each request its hold on this Session.
    lifecycle: Arc<SessionLifecycle>,
    state: Mutex<State>,
}
impl SessionService for CollaborationCapability {}

fn op_turn(kind: &EventKind) -> Option<(&str, bool)> {
    match kind {
        EventKind::CollaborationRequest(data) => Some((&data.turn_id, true)),
        EventKind::CollaborationCancel(data) | EventKind::CollaborationClose(data) => {
            Some((&data.turn_id, false))
        }
        EventKind::CollaborationMessage(data) => Some((&data.turn_id, false)),
        _ => None,
    }
}

impl CollaborationCapability {
    // ------------------------------------------------------------ tool entry

    /// Writes the operation event and waits for its `delivered`; cancelling
    /// abandons only the wait.
    async fn operate(
        &self,
        branch_id: &str,
        kind: EventKind,
        cancel: &CancellationToken,
    ) -> Result<(CollaborationDelivery, Option<String>), HostError> {
        let closed = (
            CollaborationDelivery::Error {
                error: Refusal::Closed,
            },
            None,
        );
        let Some(timeline) = self
            .state
            .lock()
            .unwrap()
            .branches
            .get(branch_id)
            .map(|branch| branch.timeline.clone())
        else {
            return Ok(closed);
        };
        let (sender, mut receiver) = watch::channel(None);
        let committed = timeline.append_registered(kind.into(), |event| {
            // `before` callbacks transform the data but never the kind.
            let Some((turn_id, request)) = op_turn(&event.kind) else {
                return Err(AgentError::fault(
                    "collaboration",
                    "the committed event is not an operation",
                ));
            };
            let mut state = self.state.lock().unwrap();
            let branch = state
                .branches
                .get_mut(branch_id)
                .filter(|branch| !branch.closing)
                .ok_or_else(|| AgentError::Closed("collaboration".into()))?;
            branch.ops.insert(
                event.id.clone(),
                Op {
                    turn_id: turn_id.to_owned(),
                    request,
                    suppressed: false,
                    delivered: false,
                    detail: None,
                    waiter: Some(sender),
                },
            );
            Ok(())
        });
        // Not recorded: the runtime's own error is the tool result.
        if let Err(error) = committed {
            return Err(HostError::failed(error.to_string()));
        }
        loop {
            if let Some(delivery) = receiver.borrow_and_update().clone() {
                return Ok(delivery);
            }
            tokio::select! {
                changed = receiver.changed() => if changed.is_err() { return Ok(closed); },
                _ = cancel.cancelled() => {
                    return Err(HostError::Cancelled("agent tool cancelled".into()));
                }
            }
        }
    }

    async fn run(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> Result<ToolResult, HostError> {
        let input: Input = match serde_json::from_value(Value::Object(call.input)) {
            Ok(input) => input,
            Err(error) => return Ok(refusal(Refusal::InvalidInput, &error.to_string())),
        };
        let leader_only = matches!(
            input,
            Input::Request { .. }
                | Input::Cancel { .. }
                | Input::Close { .. }
                | Input::Inspect { .. }
                | Input::Result { .. }
        );
        if leader_only && self.role == CollaborationRole::Member {
            return Ok(refusal(
                Refusal::InvalidInput,
                "members can only message, read and list",
            ));
        }
        let turn_id = context.turn_id.clone();
        let request_id = context.request_id.clone();
        let call_id = call.id;
        let peer = |agent_id: String, session_id: String| CollaborationPeerData {
            turn_id: turn_id.clone(),
            request_id: request_id.clone(),
            call_id: call_id.clone(),
            agent_id,
            session_id,
        };
        let (kind, action) = match input {
            Input::List {} => return Ok(self.list()),
            Input::Inspect { agent_id } => return Ok(self.inspect(&agent_id)),
            Input::Result { result_id } => {
                return Ok(self.result(&context.branch_id, &result_id).await);
            }
            Input::Read {
                agent_id,
                session_id,
                after,
                limit,
            } => {
                let limit = limit.unwrap_or(READ_PAGE_LIMIT);
                if !(1..=READ_PAGE_LIMIT).contains(&limit) {
                    return Ok(refusal(
                        Refusal::InvalidInput,
                        &format!(
                            "limit must be between 1 and {READ_PAGE_LIMIT}; read more pages by passing nextCursor as after"
                        ),
                    ));
                }
                return Ok(self.read(&agent_id, &session_id, after, limit).await);
            }
            Input::Request {
                prompt,
                agent_id,
                session_id,
                profile,
            } => {
                let target = match (agent_id, session_id, profile) {
                    (Some(agent_id), None, None) => CollaborationTarget::Agent { agent_id },
                    (Some(agent_id), Some(session_id), None) => CollaborationTarget::Member {
                        agent_id,
                        session_id,
                    },
                    (None, None, Some(profile)) => {
                        if let Err(error) = self.collaboration.index.check_profile(&profile) {
                            return Ok(refusal(
                                Refusal::InvalidInput,
                                &format!("invalid profile: {error}"),
                            ));
                        }
                        CollaborationTarget::Temporary { profile }
                    }
                    _ => {
                        return Ok(refusal(
                            Refusal::InvalidInput,
                            "request takes agentId, profile, or agentId + sessionId",
                        ));
                    }
                };
                (
                    EventKind::CollaborationRequest(CollaborationRequestData {
                        turn_id,
                        request_id,
                        call_id,
                        target,
                        prompt,
                    }),
                    "request",
                )
            }
            Input::Cancel {
                agent_id,
                session_id,
            } => (
                EventKind::CollaborationCancel(peer(agent_id, session_id)),
                "cancel",
            ),
            Input::Close {
                agent_id,
                session_id,
            } => (
                EventKind::CollaborationClose(peer(agent_id, session_id)),
                "close",
            ),
            Input::Message {
                agent_id,
                session_id,
                content,
                mode,
            } => (
                EventKind::CollaborationMessage(CollaborationMessageData {
                    turn_id,
                    request_id,
                    call_id,
                    agent_id,
                    session_id,
                    mode,
                    content,
                }),
                "message",
            ),
        };
        let mode = match &kind {
            EventKind::CollaborationMessage(data) => Some(data.mode),
            _ => None,
        };
        Ok(
            match self.operate(&context.branch_id, kind, &cancel).await? {
                (CollaborationDelivery::Error { error }, detail) => {
                    refusal(error, detail.as_deref().unwrap_or(explain(error)))
                }
                (
                    CollaborationDelivery::Ok {
                        agent_id,
                        session_id,
                        outcome,
                        ..
                    },
                    _,
                ) => {
                    let status = match (action, outcome) {
                        ("request", _) => json!("running"),
                        ("cancel", outcome) => json!(outcome),
                        ("close", _) => json!("closed"),
                        _ => json!("delivered"),
                    };
                    let mut result =
                        json!({"status": status, "agentId": agent_id, "sessionId": session_id});
                    if let Some(mode) = mode {
                        result["mode"] = json!(mode);
                    }
                    success(result)
                }
            },
        )
    }

    /// The full profile of one Agent this leader may address.
    fn inspect(&self, agent_id: &str) -> ToolResult {
        let host = self
            .collaboration
            .visible(agent_id, &self.session_id)
            .then(|| self.collaboration.index.host(Some(agent_id)))
            .flatten();
        match host {
            Some(host) => success(json!({"agentId": agent_id, "profile": host.profile})),
            None => refusal(Refusal::AgentNotFound, "no such agent is available to you"),
        }
    }
    fn list(&self) -> ToolResult {
        let participants: Vec<Value> = self
            .collaboration
            .team(&self.session_id)
            .into_iter()
            .map(|participant| {
                json!({"role": participant.role, "agentId": participant.agent_id,
                    "sessionId": participant.session_id, "name": participant.name,
                    "status": participant.status})
            })
            .collect();
        let mut result = json!({"role": self.role, "participants": participants});
        if self.role == CollaborationRole::Leader {
            // Keep policy stable while taking and filtering the directory
            // snapshot; a concurrent close must not declassify a stale row.
            let temporary = self.collaboration.directory.temporary.lock().unwrap();
            result["agents"] = self
                .collaboration
                .index
                .list()
                .into_iter()
                .filter(|agent| {
                    temporary
                        .get(&agent.agent_id)
                        .is_none_or(|creator| creator == &self.session_id)
                })
                .map(|agent| {
                    json!({"agentId": agent.agent_id, "name": agent.name,
                        "description": agent.description})
                })
                .collect();
        }
        success(result)
    }

    async fn result(&self, branch_id: &str, result_id: &str) -> ToolResult {
        let position = {
            let state = self.state.lock().unwrap();
            state.branches.get(branch_id).and_then(|branch| {
                branch
                    .results
                    .get(result_id)
                    .map(|sequence| (branch.timeline.clone(), *sequence))
            })
        };
        let Some((timeline, sequence)) = position else {
            return ToolResult::error("No collaboration result with this resultId.");
        };
        let local = match timeline.local(sequence..=sequence).await {
            Ok(local) => local,
            Err(error) => return ToolResult::error(error.to_string()),
        };
        let Some(event) = local.iter().find(|event| event.id == result_id) else {
            return ToolResult::error("No collaboration result with this resultId.");
        };
        match &event.kind {
            EventKind::CollaborationResult(data) => {
                success(json!({"resultId": event.id, "result": data}))
            }
            _ => ToolResult::error("The resultId does not identify a collaboration result."),
        }
    }

    async fn read(&self, agent_id: &str, session_id: &str, after: u64, limit: usize) -> ToolResult {
        if let Err(error) =
            self.collaboration
                .authorize_peer(&self.session_id, agent_id, session_id)
        {
            return refusal(error, explain(error));
        }
        let (_, events) = match self
            .collaboration
            .index
            .read_page(session_id, after, limit, None)
            .await
        {
            Ok(page) => page,
            Err(error) => return ToolResult::error(error.to_string()),
        };
        let mut items: Vec<Value> = vec![];
        let mut text_request = None;
        let mut next = after;
        // Started call names by (request_id, call_id), for undispatched results.
        let mut started = HashMap::<(String, String), String>::new();
        for event in &events {
            next = event.sequence;
            let mut merged = None;
            match &event.kind {
                EventKind::InputMessage(data) => items.push(json!({"sequence": event.sequence,
                    "type": "input", "mode": data.effective_mode(), "user": data.user,
                    "blocks": data.blocks})),
                EventKind::Text(data) => {
                    if text_request.as_deref() == Some(data.request_id.as_str())
                        && let Some(text) = items.last_mut().and_then(|item| item.get_mut("text"))
                    {
                        *text = json!(format!("{}{}", text.as_str().unwrap_or(""), data.delta));
                    } else {
                        items.push(json!({"sequence": event.sequence, "type": "text",
                            "text": data.delta}));
                    }
                    merged = Some(data.request_id.clone());
                }
                EventKind::ToolCall(data) => {
                    for call in &data.calls {
                        items.push(json!({"sequence": event.sequence, "type": "tool_call",
                            "callId": call.id, "name": call.name}));
                    }
                }
                EventKind::ToolUseStart(data) => {
                    started.insert(
                        (data.request_id.clone(), data.call_id.clone()),
                        data.name.clone(),
                    );
                }
                EventKind::ToolResult(data) => {
                    let call = match &data.call_event_id {
                        Some(call_event_id) => match self
                            .collaboration
                            .index
                            .read_event(session_id, call_event_id)
                            .await
                        {
                            Ok(call) => call,
                            Err(error) => return ToolResult::error(error.to_string()),
                        },
                        // Results closing undispatched calls answer no dispatch;
                        // their names come from the calls' starts, which may
                        // precede this page.
                        None => {
                            for result in &data.results {
                                let key = (data.request_id.clone(), result.call_id.clone());
                                if started.contains_key(&key) {
                                    continue;
                                }
                                match self
                                    .collaboration
                                    .index
                                    .read_tool_use_start(session_id, &key.0, &key.1)
                                    .await
                                {
                                    Ok(Some(event)) => {
                                        if let EventKind::ToolUseStart(start) = &event.kind {
                                            started.insert(key, start.name.clone());
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(error) => return ToolResult::error(error.to_string()),
                                }
                            }
                            None
                        }
                    };
                    let calls = call.as_ref().and_then(|call| match &call.kind {
                        EventKind::ToolCall(batch) => Some(&batch.calls),
                        _ => None,
                    });
                    for result in &data.results {
                        let name = match calls {
                            Some(calls) => calls
                                .iter()
                                .find(|call| call.id == result.call_id)
                                .map(|call| &call.name),
                            None => started.get(&(data.request_id.clone(), result.call_id.clone())),
                        };
                        items.push(json!({"sequence": event.sequence, "type": "tool_result",
                            "callId": result.call_id, "name": name,
                            "status": if result.result.is_error == Some(true) { "error" } else { "ok" }}));
                    }
                }
                EventKind::TurnEnd(data) => {
                    let mut item = json!({"sequence": event.sequence, "type": "turn_end"});
                    match &data.outcome {
                        TurnOutcome::Completed { .. } => item["status"] = json!("completed"),
                        TurnOutcome::Failed { error } => {
                            item["status"] = json!("failed");
                            item["reason"] = json!(error);
                        }
                        TurnOutcome::Cancelled { reason } => {
                            item["status"] = json!("cancelled");
                            item["reason"] = json!(reason);
                        }
                    }
                    items.push(item);
                }
                _ => {}
            }
            text_request = merged;
        }
        success(
            json!({"agentId": agent_id, "sessionId": session_id, "events": items,
            "nextCursor": next}),
        )
    }

    // ------------------------------------------------------------ dispatch

    fn consume(
        self: &Arc<Self>,
        branch_id: &str,
        event: Arc<AgentEvent>,
    ) -> BoxFuture<'static, dim_agent_runtime::Result<()>> {
        match &event.kind {
            EventKind::TurnStart(data) => {
                self.with_branch(branch_id, |branch| {
                    branch.current_turn = Some(data.turn_id.clone())
                });
            }
            EventKind::TurnEnd(_) => {
                self.with_branch(branch_id, |branch| branch.current_turn = None);
            }
            EventKind::StopInterrupt(data) => self.interrupt(branch_id, data.scope),
            EventKind::CollaborationDelivered(data) => self.delivered(branch_id, data),
            EventKind::CollaborationResult(data) => {
                self.with_branch(branch_id, |branch| {
                    branch.results.insert(event.id.clone(), event.sequence);
                    close_request(&mut branch.open, &data.request_event_id);
                });
                let this = self.clone();
                let branch_id = branch_id.to_owned();
                let data = data.clone();
                let result_id = event.id.clone();
                return Box::pin(async move { this.adopt(&branch_id, &result_id, &data).await });
            }
            _ => {
                if let Some((turn_id, request)) = op_turn(&event.kind) {
                    let turn_id = turn_id.to_owned();
                    self.dispatch(branch_id, event, turn_id, request);
                }
            }
        }
        Box::pin(async { Ok(()) })
    }

    fn with_branch(&self, branch_id: &str, apply: impl FnOnce(&mut BranchRecord)) {
        if let Some(branch) = self.state.lock().unwrap().branches.get_mut(branch_id) {
            apply(branch);
        }
    }

    /// `scope: turn` covers requests of the current turn; `scope: branch`
    /// (branch close) covers every request of this branch.
    fn interrupt(self: &Arc<Self>, branch_id: &str, scope: InterruptScope) {
        let mut state = self.state.lock().unwrap();
        let State { branches, requests } = &mut *state;
        let Some(branch) = branches.get_mut(branch_id) else {
            return;
        };
        let turn = match scope {
            InterruptScope::Turn => match &branch.current_turn {
                Some(turn) => Some(turn.clone()),
                None => return,
            },
            InterruptScope::Branch => {
                branch.closing = true;
                None
            }
        };
        let covers = |turn_id: &str| turn.as_deref().is_none_or(|turn| turn == turn_id);
        if let Some(turn) = &turn {
            branch.interrupted.insert(turn.clone());
        }
        // A delivered request is detached work: a turn stop leaves it running
        // and its result wakes the branch later. Only a branch stop ends it.
        let mut undelivered = BTreeSet::new();
        for (id, op) in branch.ops.iter_mut() {
            if covers(&op.turn_id) && (turn.is_none() || !op.delivered) {
                op.suppressed = true;
                undelivered.insert(id.clone());
            }
        }
        let members: Vec<_> = requests
            .values_mut()
            .filter(|request| {
                request.branch == branch_id
                    && covers(&request.turn_id)
                    && (turn.is_none() || undelivered.contains(&request.op))
            })
            .map(|request| {
                if turn.is_none() {
                    request.settle = Some(CollaborationOutcome::Cancelled {
                        reason: LEADER_CLOSED.into(),
                    });
                }
                (request.session.clone(), request.done.subscribe())
            })
            .collect();
        if !members.is_empty() {
            let abort = branch.abort.clone();
            branch.tasks.spawn(async move {
                for (member, mut done) in members {
                    let _ = member.stop("main", INTERRUPTED.into()).await;
                    if scope == InterruptScope::Branch {
                        // The branch close also owns the request's result read
                        // and publication, not just the member's native stop.
                        tokio::select! {
                            _ = done.wait_for(|done| *done) => {}
                            _ = abort.cancelled() => break,
                        }
                    }
                }
            });
        }
    }

    /// Starts delivery work, or refuses an operation of an interrupted turn
    /// or a closing branch.
    fn dispatch(
        self: &Arc<Self>,
        branch_id: &str,
        event: Arc<AgentEvent>,
        turn_id: String,
        request: bool,
    ) {
        let mut state = self.state.lock().unwrap();
        let Some(branch) = state.branches.get_mut(branch_id) else {
            return;
        };
        let refused = if branch.closing {
            Some(Refusal::Closed)
        } else if branch.interrupted.contains(&turn_id) {
            Some(Refusal::Interrupted)
        } else {
            None
        };
        branch.ops.entry(event.id.clone()).or_insert_with(|| Op {
            turn_id,
            request,
            suppressed: false,
            delivered: false,
            detail: None,
            waiter: None,
        });
        // A cancel's outcome is decided at its own position: whether a
        // request of this branch to the member has no handled `result` yet.
        let mut cancel = CollaborationCancelOutcome::Idle;
        match &event.kind {
            EventKind::CollaborationRequest(CollaborationRequestData {
                target: CollaborationTarget::Member { session_id, .. },
                ..
            }) => {
                branch
                    .open
                    .entry(session_id.clone())
                    .or_default()
                    .insert(event.id.clone());
            }
            EventKind::CollaborationCancel(data) if branch.open.contains_key(&data.session_id) => {
                cancel = CollaborationCancelOutcome::Cancelled;
            }
            _ => {}
        }
        let peer = match &event.kind {
            EventKind::CollaborationRequest(CollaborationRequestData {
                target: CollaborationTarget::Member { session_id, .. },
                ..
            })
            | EventKind::CollaborationCancel(CollaborationPeerData { session_id, .. })
            | EventKind::CollaborationClose(CollaborationPeerData { session_id, .. })
            | EventKind::CollaborationMessage(CollaborationMessageData { session_id, .. }) => {
                session_id.clone()
            }
            // A new member is a peer of its own.
            _ => event.id.clone(),
        };
        let previous = branch.chains.get(&peer).map(|(_, work)| work.clone());
        let this = self.clone();
        let branch_key = branch_id.to_owned();
        let chain_key = peer.clone();
        let sequence = event.sequence;
        let work = async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let delivered = match refused {
                Some(code) => Err(Failure::from(code)),
                None => this.deliver(&branch_key, &event, cancel).await,
            };
            let (delivery, detail) = match delivered {
                Ok(delivery) => (delivery, None),
                Err(Failure { code, detail }) => {
                    (CollaborationDelivery::Error { error: code }, detail)
                }
            };
            this.write_delivered(&branch_key, &event.id, delivery, detail);
            let mut state = this.state.lock().unwrap();
            if let Some(branch) = state.branches.get_mut(&branch_key)
                && branch
                    .chains
                    .get(&chain_key)
                    .is_some_and(|(owner, _)| *owner == sequence)
            {
                branch.chains.remove(&chain_key);
            }
        }
        .boxed()
        .shared();
        branch.chains.insert(peer, (sequence, work.clone()));
        branch.tasks.spawn(work);
    }

    fn write_delivered(
        &self,
        branch_id: &str,
        op: &str,
        delivery: CollaborationDelivery,
        detail: Option<String>,
    ) {
        // The cause is recorded before the fact its handler consumes.
        let timeline = self
            .state
            .lock()
            .unwrap()
            .branches
            .get_mut(branch_id)
            .map(|branch| {
                if let Some(op) = branch.ops.get_mut(op) {
                    op.detail = detail;
                }
                branch.timeline.clone()
            });
        let written = timeline.is_some_and(|timeline| {
            timeline
                .append(
                    EventKind::CollaborationDelivered(CollaborationDeliveredData {
                        op_event_id: op.to_owned(),
                        delivery: delivery.clone(),
                    })
                    .into(),
                )
                .is_ok()
        });
        let mut state = self.state.lock().unwrap();
        let Some(branch) = state.branches.get_mut(branch_id) else {
            return;
        };
        if !written && let Some(mut op) = branch.ops.remove(op) {
            // No fact was written: release the waiting tool call directly.
            if let Some(waiter) = op.waiter.take() {
                waiter.send_replace(Some((
                    CollaborationDelivery::Error {
                        error: Refusal::Closed,
                    },
                    None,
                )));
            }
        }
    }

    async fn deliver(
        self: &Arc<Self>,
        branch_id: &str,
        event: &AgentEvent,
        cancel: CollaborationCancelOutcome,
    ) -> Result<CollaborationDelivery, Failure> {
        let delivered = |peer: &Peer, input: Option<String>, outcome| CollaborationDelivery::Ok {
            agent_id: peer.agent_id.clone(),
            session_id: peer.session.id().to_owned(),
            input_event_id: input,
            outcome,
        };
        match &event.kind {
            EventKind::CollaborationRequest(data) => self.request(branch_id, event, data).await,
            EventKind::CollaborationMessage(data) => {
                let sender = self.sender()?;
                let peer = self
                    .collaboration
                    .resolve(&self.session_id, &data.agent_id, &data.session_id)
                    .await?;
                let mode = match data.mode {
                    CollaborationMessageMode::Append => InputMode::Append,
                    CollaborationMessageMode::Steer => InputMode::Steer,
                };
                let input = peer
                    .session
                    .input(
                        self.message_input(&sender, data.content.clone(), mode),
                        "main",
                    )
                    .await
                    .map_err(|_| Refusal::MemberClosed)?;
                Ok(delivered(&peer, Some(input.id.clone()), None))
            }
            EventKind::CollaborationCancel(data) => {
                let peer = self
                    .collaboration
                    .member(&self.session_id, &data.agent_id, &data.session_id)
                    .await?;
                let _ = peer.session.stop("main", CANCELLED.into()).await;
                Ok(delivered(&peer, None, Some(cancel)))
            }
            EventKind::CollaborationClose(data) => {
                let peer = self
                    .collaboration
                    .member(&self.session_id, &data.agent_id, &data.session_id)
                    .await?;
                let working = self.in_flight(&data.session_id);
                let _ = peer.session.stop("main", CANCELLED.into()).await;
                if let Some(mut done) = working {
                    let abort = self.with_abort(branch_id);
                    // The relation outlives the request: its result comes first.
                    tokio::select! {
                        _ = done.wait_for(|done| *done) => {}
                        _ = abort.cancelled() => return Err(Refusal::Closed.into()),
                    }
                }
                self.collaboration.remove(&data.session_id);
                Ok(delivered(&peer, None, None))
            }
            _ => unreachable!("operation event"),
        }
    }

    fn in_flight(&self, member: &str) -> Option<watch::Receiver<bool>> {
        self.state
            .lock()
            .unwrap()
            .requests
            .get(member)
            .map(|request| request.done.subscribe())
    }

    fn with_abort(&self, branch_id: &str) -> CancellationToken {
        self.state
            .lock()
            .unwrap()
            .branches
            .get(branch_id)
            .map_or_else(
                || {
                    let token = CancellationToken::new();
                    token.cancel();
                    token
                },
                |branch| branch.abort.clone(),
            )
    }

    async fn request(
        self: &Arc<Self>,
        branch_id: &str,
        event: &AgentEvent,
        data: &CollaborationRequestData,
    ) -> Result<CollaborationDelivery, Failure> {
        let sender = self.sender()?;
        let (peer, new) = match &data.target {
            CollaborationTarget::Agent { agent_id } => (
                self.collaboration
                    .establish(&self.session_id, NewMember::Agent(agent_id.clone()))
                    .await?,
                true,
            ),
            CollaborationTarget::Temporary { profile } => (
                self.collaboration
                    .establish(&self.session_id, NewMember::Temporary(profile.clone()))
                    .await?,
                true,
            ),
            CollaborationTarget::Member {
                agent_id,
                session_id,
            } => (
                self.collaboration
                    .member(&self.session_id, agent_id, session_id)
                    .await?,
                false,
            ),
        };
        let member = peer.session.id().to_owned();
        let covered = {
            let state = self.state.lock().unwrap();
            // Recheck before writing to the member: a stop handled since this
            // operation was dispatched covers it.
            state.branches.get(branch_id).and_then(|branch| {
                let op = branch.ops.get(&event.id)?;
                op.suppressed.then_some(if branch.closing {
                    Refusal::Closed
                } else {
                    Refusal::Interrupted
                })
            })
        };
        // The request holds the leader while it is open. Only a closing
        // leader has no hold to give: refused as a closing branch is.
        let hold = covered.is_none().then(|| self.lifecycle.hold()).flatten();
        let Some(hold) = hold else {
            if new {
                self.collaboration.remove(&member);
            }
            return Err(covered.unwrap_or(Refusal::Closed).into());
        };
        {
            let mut state = self.state.lock().unwrap();
            if !new
                && (state.requests.contains_key(&member)
                    || peer.session.state("main").is_ok_and(|state| state.running))
            {
                return Err(Refusal::Busy.into());
            }
            state.requests.insert(
                member.clone(),
                Request {
                    _usage: peer.usage.clone(),
                    _hold: hold,
                    branch: branch_id.to_owned(),
                    op: event.id.clone(),
                    turn_id: data.turn_id.clone(),
                    agent_id: peer.agent_id.clone(),
                    session: peer.session.clone(),
                    input_event_id: None,
                    settle: None,
                    done: watch::channel(false).0,
                },
            );
        }
        let input = match peer
            .session
            .input(
                self.message_input(&sender, data.prompt.clone(), InputMode::Queue),
                "main",
            )
            .await
        {
            Ok(input) => input,
            Err(error) => {
                // Refused before delivery: its hold drops here, and the
                // `delivered` that answers the waiting tool call follows
                // without an await.
                self.state.lock().unwrap().requests.remove(&member);
                if new {
                    self.collaboration.remove(&member);
                    return Err(member_create_failed(error.to_string()));
                }
                return Err(Refusal::MemberClosed.into());
            }
        };
        let suppressed = {
            let mut state = self.state.lock().unwrap();
            if let Some(request) = state.requests.get_mut(&member) {
                request.input_event_id = Some(input.id.clone());
            }
            state
                .branches
                .get(branch_id)
                .and_then(|branch| branch.ops.get(&event.id))
                .is_none_or(|op| op.suppressed)
        };
        // Covered by a stop before its delivery: the result does not return.
        if suppressed {
            let _ = peer.session.stop("main", INTERRUPTED.into()).await;
        }
        Ok(CollaborationDelivery::Ok {
            agent_id: peer.agent_id,
            session_id: member,
            input_event_id: Some(input.id.clone()),
            outcome: None,
        })
    }

    /// Releases the waiting tool call; a delivered request starts its
    /// completion work.
    fn delivered(self: &Arc<Self>, branch_id: &str, data: &CollaborationDeliveredData) {
        let mut state = self.state.lock().unwrap();
        let State { branches, requests } = &mut *state;
        let Some(branch) = branches.get_mut(branch_id) else {
            return;
        };
        let Some(op) = branch.ops.get_mut(&data.op_event_id) else {
            return;
        };
        op.delivered = true;
        if let Some(waiter) = op.waiter.take() {
            waiter.send_replace(Some((data.delivery.clone(), op.detail.take())));
        }
        let member = match &data.delivery {
            CollaborationDelivery::Ok { session_id, .. } if op.request => {
                branch
                    .open
                    .entry(session_id.clone())
                    .or_default()
                    .insert(data.op_event_id.clone());
                session_id
            }
            _ => {
                if op.request {
                    close_request(&mut branch.open, &data.op_event_id);
                }
                branch.ops.remove(&data.op_event_id);
                return;
            }
        };
        let Some(request) = requests
            .get(member)
            .filter(|request| request.op == data.op_event_id)
        else {
            return;
        };
        let session = request.session.clone();
        let member = member.clone();
        // Leader stop and idle do not wait for the member; unload cancels
        // the wait and settles.
        let abort = branch.abort.clone();
        let this = self.clone();
        let branch_id = branch_id.to_owned();
        branch.tasks.spawn(async move {
            let idle = tokio::select! {
                result = session.idle("main") => result,
                // Unload writes the result of an aborted wait.
                _ = abort.cancelled() => return,
            };
            this.settle(&branch_id, &member, Some(idle)).await;
        });
    }

    /// Writes the request's `result` once and ends it.
    async fn settle(
        &self,
        branch_id: &str,
        member: &str,
        idle: Option<dim_agent_runtime::Result<u64>>,
    ) {
        let Some((request, timeline)) = ({
            let state = self.state.lock().unwrap();
            state
                .requests
                .get(member)
                .filter(|request| request.branch == branch_id)
                .map(|request| {
                    (
                        (
                            request.op.clone(),
                            request.agent_id.clone(),
                            request.session.clone(),
                            request.input_event_id.clone(),
                        ),
                        state
                            .branches
                            .get(branch_id)
                            .map(|branch| branch.timeline.clone()),
                    )
                })
        }) else {
            return;
        };
        let (op, agent_id, session, input) = request;
        // The work range ends where the member's idle point was reached;
        // later turns of the member are not this request's work.
        let through = match &idle {
            Some(Ok(cutoff)) => Some(*cutoff),
            _ => None,
        };
        let (last_turn_id, text, derived) = outcome(
            &self.collaboration.index,
            &session,
            input.as_deref(),
            through,
        )
        .await;
        let settle = self
            .state
            .lock()
            .unwrap()
            .requests
            .get(member)
            .and_then(|request| request.settle.clone());
        let outcome = settle.unwrap_or_else(|| match idle {
            _ if session.is_closing() => CollaborationOutcome::Failed {
                reason: MEMBER_CLOSED.into(),
            },
            Some(Err(error)) => CollaborationOutcome::Failed {
                reason: error.to_string(),
            },
            _ => derived,
        });
        if let Some(timeline) = timeline {
            let _ = timeline.append(
                EventKind::CollaborationResult(CollaborationResultData {
                    request_event_id: op,
                    agent_id,
                    session_id: member.to_owned(),
                    last_turn_id,
                    text,
                    outcome,
                })
                .into(),
            );
        }
        // Removed after its last fact: this drops the leader's hold.
        if let Some(request) = self.state.lock().unwrap().requests.remove(member) {
            request.done.send_replace(true);
        }
    }

    /// Posts the result input unless a stop covered the request or the
    /// branch is closing.
    async fn adopt(
        &self,
        branch_id: &str,
        result_id: &str,
        data: &CollaborationResultData,
    ) -> dim_agent_runtime::Result<()> {
        let suppressed = {
            let mut state = self.state.lock().unwrap();
            let Some(branch) = state.branches.get_mut(branch_id) else {
                return Ok(());
            };
            branch
                .ops
                .remove(&data.request_event_id)
                .is_none_or(|op| op.suppressed || branch.closing)
        };
        if suppressed {
            return Ok(());
        }
        let branch = self.tree.get(branch_id)?;
        if branch.is_closing() {
            return Ok(());
        }
        let status = match &data.outcome {
            CollaborationOutcome::Completed => "completed",
            CollaborationOutcome::Failed { .. } => "failed",
            CollaborationOutcome::Cancelled { .. } => "cancelled",
        };
        let preview = match &data.outcome {
            CollaborationOutcome::Completed => context_block::preview(&[&data.text]),
            CollaborationOutcome::Failed { reason }
            | CollaborationOutcome::Cancelled { reason } => {
                context_block::preview(&[reason, &data.text])
            }
        };
        let text = context_block::element(
            "dim-agent-report",
            &[("agent", &data.agent_id), ("session", &data.session_id)],
            &format!(
                "Status: {status}.\nResult preview:\n{preview}\nFull result: agent tool, action \"result\", resultId \"{}\".",
                context_block::text(result_id)
            ),
        );
        let input = InputMessageData {
            blocks: vec![context_block::raw(text, None)],
            mode: Some(InputMode::Steer),
            ..Default::default()
        };
        match branch.input(input) {
            Ok(_) => Ok(()),
            Err(AgentError::Closed(_)) if branch.is_closing() => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// The Agent of this Session, which sends its requests and messages. A
    /// Session its Agent no longer hosts is closing: refused as a closing
    /// branch is.
    fn sender(&self) -> Result<String, Refusal> {
        self.collaboration
            .index
            .host_by_session(&self.session_id)
            .map(|host| host.agent_id.clone())
            .ok_or(Refusal::Closed)
    }

    fn message_input(&self, agent_id: &str, content: String, mode: InputMode) -> InputMessageData {
        let relation = match self.role {
            CollaborationRole::Leader => "your team leader, assigned you this work",
            CollaborationRole::Member => "your teammate, sent you this message",
        };
        let text = context_block::element(
            "dim-agent-message",
            &[("agent", agent_id)],
            &format!(
                "Agent {}, {relation}:\n{}",
                context_block::text(agent_id),
                context_block::text(&content)
            ),
        );
        InputMessageData {
            blocks: vec![context_block::raw(text, None)],
            mode: Some(mode),
            ..Default::default()
        }
    }

    // ------------------------------------------------------------ lifecycle

    /// Unload: pending work stops; operations without `delivered` and
    /// requests without `result` are settled on this Timeline.
    async fn unload(&self, branch_id: &str) {
        let tasks = {
            let mut state = self.state.lock().unwrap();
            let Some(branch) = state.branches.get_mut(branch_id) else {
                return;
            };
            branch.closing = true;
            branch.abort.cancel();
            branch.tasks.clone()
        };
        tasks.close();
        tasks.wait().await;
        let closing = self
            .tree
            .get(branch_id)
            .map_or(true, |branch| branch.is_closing());
        let (ops, members): (Vec<String>, Vec<String>) = {
            let state = self.state.lock().unwrap();
            let ops = state
                .branches
                .get(branch_id)
                .map_or_else(Vec::new, |branch| {
                    branch
                        .ops
                        .iter()
                        .filter(|(_, op)| !op.delivered)
                        .map(|(id, _)| id.clone())
                        .collect()
                });
            let members = state
                .requests
                .iter()
                .filter(|(_, request)| request.branch == branch_id)
                .map(|(member, _)| member.clone())
                .collect();
            (ops, members)
        };
        for op in ops {
            self.write_delivered(
                branch_id,
                &op,
                CollaborationDelivery::Error {
                    error: Refusal::Closed,
                },
                None,
            );
        }
        for member in members {
            let session = {
                let mut state = self.state.lock().unwrap();
                let Some(request) = state.requests.get_mut(&member) else {
                    continue;
                };
                request.settle = Some(if closing {
                    CollaborationOutcome::Cancelled {
                        reason: LEADER_CLOSED.into(),
                    }
                } else {
                    CollaborationOutcome::Failed {
                        reason: RELOADING.into(),
                    }
                });
                request.session.clone()
            };
            if closing {
                let _ = session.stop("main", INTERRUPTED.into()).await;
            } else {
                // A reload does not wait for the member to settle: its Loop
                // may be reloading in the same cascade and return only after
                // this unload. Commit before releasing the request so the
                // interrupt cannot land after a later member input.
                let _ = session.interrupt("main", INTERRUPTED.into());
            }
            self.settle(branch_id, &member, None).await;
        }
        self.state.lock().unwrap().branches.remove(branch_id);
    }

    /// Reload: operations of an earlier generation without `delivered` are
    /// refused, its delivered requests without `result` fail. Settlement
    /// acts only on what this Session holds: a member is stopped only while
    /// its relation names this Session as its leader. A fork's copied
    /// request names its source's member, which keeps working for the
    /// source (#127); after a daemon restart no relation exists, so nothing
    /// is stopped.
    fn rebuild(
        &self,
        timeline: &Arc<Timeline>,
        phase: &BranchLifecycle,
        events: &[Arc<AgentEvent>],
        tasks: &TaskTracker,
    ) -> (Option<String>, BTreeSet<String>) {
        let mut current_turn = None;
        let mut interrupted = BTreeSet::new();
        // Undelivered operations, whether each is a request.
        let mut ops = BTreeMap::new();
        let mut open: BTreeMap<String, (String, String)> = BTreeMap::new();
        for event in events {
            match &event.kind {
                EventKind::TurnStart(data) => current_turn = Some(data.turn_id.clone()),
                EventKind::TurnEnd(_) => current_turn = None,
                EventKind::StopInterrupt(data) if data.scope == InterruptScope::Turn => {
                    interrupted.extend(current_turn.clone());
                }
                EventKind::CollaborationDelivered(data) => {
                    if let (
                        Some(true),
                        CollaborationDelivery::Ok {
                            agent_id,
                            session_id,
                            ..
                        },
                    ) = (ops.remove(&data.op_event_id), &data.delivery)
                    {
                        open.insert(
                            data.op_event_id.clone(),
                            (agent_id.clone(), session_id.clone()),
                        );
                    }
                }
                EventKind::CollaborationResult(data) => {
                    open.remove(&data.request_event_id);
                }
                kind => {
                    if let Some((_, request)) = op_turn(kind) {
                        ops.insert(event.id.clone(), request);
                    }
                }
            }
        }
        for op in ops.into_keys() {
            let _ = timeline.append(
                EventKind::CollaborationDelivered(CollaborationDeliveredData {
                    op_event_id: op,
                    delivery: CollaborationDelivery::Error {
                        error: Refusal::Closed,
                    },
                })
                .into(),
            );
        }
        // A request left by the previous generation ends once the branch is
        // Active: a member this Session leads stops first, then the result
        // is written. A branch that never starts leaves it to the next
        // generation.
        for (op, (agent_id, session_id)) in open {
            let leads = self
                .collaboration
                .directory
                .relations
                .lock()
                .unwrap()
                .get(&session_id)
                .is_some_and(|relation| relation.leader == self.session_id);
            let member = leads
                .then(|| self.collaboration.index.session(&session_id))
                .flatten()
                .map(|(_, member)| member);
            let (timeline, phase) = (timeline.clone(), phase.clone());
            tasks.spawn(async move {
                if !phase.activated().await {
                    return;
                }
                if let Some(member) = member {
                    let _ = member.stop("main", INTERRUPTED.into()).await;
                }
                let _ = timeline.append(
                    EventKind::CollaborationResult(CollaborationResultData {
                        request_event_id: op,
                        agent_id,
                        session_id,
                        last_turn_id: None,
                        text: String::new(),
                        outcome: CollaborationOutcome::Failed {
                            reason: RELOADING.into(),
                        },
                    })
                    .into(),
                );
            });
        }
        (current_turn, interrupted)
    }
}

fn close_request(open: &mut BTreeMap<String, BTreeSet<String>>, request: &str) {
    open.retain(|_, requests| {
        requests.remove(request);
        !requests.is_empty()
    });
}

/// Result of a request's work range: from the member's `input/consumed` of
/// the request input to its idle point; the last `turn/end` decides.
async fn outcome(
    index: &crate::agents::AgentIndex,
    session: &Session,
    input: Option<&str>,
    cutoff: Option<u64>,
) -> (Option<String>, String, CollaborationOutcome) {
    let through = match cutoff.map_or_else(|| session.sequence("main"), Ok) {
        Ok(sequence) => sequence,
        Err(error) => {
            return (
                None,
                String::new(),
                CollaborationOutcome::Failed {
                    reason: error.to_string(),
                },
            );
        }
    };
    let mut after = 0;
    let mut started = false;
    let mut current_turn = None;
    let mut text = String::new();
    let mut last = None;
    while after < through {
        let events = match index
            .read_page(session.id(), after, 256, Some(through))
            .await
        {
            Ok((_, events)) => events,
            Err(error) => {
                return (
                    None,
                    String::new(),
                    CollaborationOutcome::Failed {
                        reason: error.to_string(),
                    },
                );
            }
        };
        if events.is_empty() {
            break;
        }
        for event in events {
            after = event.sequence;
            if matches!(&event.kind, EventKind::InputConsumed(data) if Some(data.input_id.as_str()) == input)
            {
                started = true;
            }
            if !started {
                continue;
            }
            match &event.kind {
                EventKind::ModelStart(data) => {
                    current_turn = Some(data.turn_id.clone());
                    text.clear();
                }
                EventKind::Text(data) if current_turn.as_ref() == Some(&data.turn_id) => {
                    text.push_str(&data.delta)
                }
                EventKind::TurnEnd(data) => {
                    let outcome = match &data.outcome {
                        TurnOutcome::Completed { .. } => CollaborationOutcome::Completed,
                        TurnOutcome::Failed { error } => CollaborationOutcome::Failed {
                            reason: error.clone(),
                        },
                        TurnOutcome::Cancelled { reason } => CollaborationOutcome::Cancelled {
                            reason: reason.clone(),
                        },
                    };
                    let text = if current_turn.as_ref() == Some(&data.turn_id) {
                        text.clone()
                    } else {
                        String::new()
                    };
                    last = Some((Some(data.turn_id.clone()), text, outcome));
                }
                _ => {}
            }
        }
    }
    last.unwrap_or_else(|| {
        (
            None,
            String::new(),
            CollaborationOutcome::Cancelled {
                reason: "not started".into(),
            },
        )
    })
}

fn success(result: Value) -> ToolResult {
    let mut output = ToolResult::text(result.to_string());
    output.structured_content = result.as_object().cloned();
    output
}
fn refusal(error: Refusal, explanation: &str) -> ToolResult {
    let code = serde_json::to_value(error).expect("error code serializes");
    let mut output = ToolResult::error(format!(
        "{}: {explanation}",
        code.as_str().expect("error code is a string")
    ));
    output.structured_content = json!({"error": code, "message": explanation})
        .as_object()
        .cloned();
    output
}
fn explain(error: Refusal) -> &'static str {
    match error {
        Refusal::InvalidInput => "the arguments do not match the agent tool schema",
        Refusal::AgentNotFound => "no Agent with this agentId exists",
        Refusal::MemberCreateFailed => "the Agent could not create a member Session",
        Refusal::TeamMemberNotFound => {
            "the participant is not in your team or does not belong to this agentId; use list"
        }
        Refusal::Busy => "the member is working; wait for its result or cancel it",
        Refusal::MemberClosed => "the participant's Session is closed",
        Refusal::Interrupted => "the operation came from a stopped turn and was not delivered",
        Refusal::Closed => "this Session is closing or its collaboration is reloading",
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum Input {
    Result {
        result_id: String,
    },
    Request {
        prompt: String,
        agent_id: Option<String>,
        session_id: Option<String>,
        profile: Option<AgentProfile>,
    },
    Cancel {
        agent_id: String,
        session_id: String,
    },
    Close {
        agent_id: String,
        session_id: String,
    },
    Message {
        agent_id: String,
        session_id: String,
        content: String,
        #[serde(default)]
        mode: CollaborationMessageMode,
    },
    Read {
        agent_id: String,
        session_id: String,
        #[serde(default)]
        after: u64,
        limit: Option<usize>,
    },
    List {},
    Inspect {
        agent_id: String,
    },
}

const LEADER_DESCRIPTION: &str = "Work with other agents as a team you lead.
- request: give a task to a new member (agentId or profile) or to one of your members
  (agentId + sessionId). Returns immediately; the member's final result arrives later
  as an input. One task per member at a time.
- cancel / close: stop a member's current task / remove the member from your team.
- message: send information to a team member. Default mode \"append\" adds context the
  receiver sees in its next step without interrupting it. Use mode \"steer\" only when the
  receiver must change course now.
- read: view a team member's recent activity. list: show your team and the agents you
  can use. inspect: show one agent's full profile (instructions, tools, model).
- result: read a complete result by resultId from a report, even after closing the member.
Address members with agentId + sessionId from earlier results.";

const MEMBER_DESCRIPTION: &str = "You are a member of a team. You cannot delegate work.
- message: send information to your leader or another member. Default mode \"append\"
  adds context the receiver sees in its next step without interrupting it. Use mode
  \"steer\" only when the receiver must act on it now, such as a blocking question or a
  finding that invalidates its current work.
- read: view a team member's recent activity. list: show your team.
Your final answer to the task is returned to the leader automatically; do not send it
as a message.";

fn agent_definition(role: CollaborationRole) -> ToolDefinition {
    let mut properties = json!({
        "agentId": {"type": "string"},
        "sessionId": {"type": "string"},
        "content": {"type": "string"},
        "mode": {"type": "string", "enum": ["append", "steer"]},
        "after": {"type": "integer", "minimum": 0},
        "limit": {"type": "integer", "minimum": 1, "maximum": READ_PAGE_LIMIT,
            "default": READ_PAGE_LIMIT,
            "description": "Target events per page. The final atomic batch is returned whole and may exceed the target. Pass nextCursor as after to continue."},
    });
    let (actions, description) = match role {
        CollaborationRole::Leader => {
            properties["resultId"] = json!({"type": "string"});
            properties["prompt"] = json!({"type": "string"});
            properties["profile"] = json!({
                "type": "object",
                "required": ["name", "description", "systemPrompt", "tools"],
                "additionalProperties": false,
                "properties": {
                    "name": {"type": "string"},
                    "description": {"type": "string"},
                    "systemPrompt": {"type": "string"},
                    "tools": {"type": "array", "items": {"type": "string"}},
                    "model": {"type": "object", "required": ["providerId", "modelId"],
                        "additionalProperties": false,
                        "properties": {"providerId": {"type": "string"}, "modelId": {"type": "string"}}},
                },
            });
            (
                json!([
                    "request", "cancel", "close", "message", "read", "list", "inspect", "result"
                ]),
                LEADER_DESCRIPTION,
            )
        }
        CollaborationRole::Member => (json!(["message", "read", "list"]), MEMBER_DESCRIPTION),
    };
    properties["action"] = json!({"type": "string", "enum": actions});
    definition(
        AGENT_TOOL,
        description,
        json!({"type": "object", "required": ["action"], "additionalProperties": false,
            "properties": properties}),
    )
}

struct AgentTool {
    capability: Arc<CollaborationCapability>,
    stopping: CancellationToken,
}
impl ToolHandler for AgentTool {
    fn execute(
        &self,
        call: ToolCall,
        context: ToolExecutionContext,
        cancel: CancellationToken,
    ) -> HostFuture<'_, ToolResult> {
        Box::pin(async move {
            if self.stopping.is_cancelled() {
                return Err(HostError::Cancelled(RELOADING.into()));
            }
            self.capability.run(call, context, cancel).await
        })
    }
}

struct CollaborationCapabilityPlugin {
    scope: SessionScope,
}
impl CollaborationCapabilityPlugin {
    fn new(scope: &SessionScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for CollaborationCapabilityPlugin {
    type Deps = (
        Arc<Collaboration>,
        InSession<SessionIdentity>,
        InSession<SessionConfig>,
        InSession<TreeCommands>,
        InSession<SessionLifecycle>,
    );
    fn name(&self) -> &str {
        "CollaborationCapability"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (
            collaboration,
            InSession(identity),
            InSession(config),
            InSession(tree),
            InSession(lifecycle),
        ): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let service = Arc::new(CollaborationCapability {
                collaboration,
                session_id: identity.session_id.clone(),
                role: config.collaboration_role,
                tree,
                lifecycle,
                state: Mutex::new(State::default()),
            });
            ctx.provide_as(
                self.scope.named_key::<ToolContribution>(AGENT_TOOL),
                Arc::new(ToolContribution {
                    definition: agent_definition(config.collaboration_role),
                    handler: Arc::new(AgentTool {
                        capability: service.clone(),
                        stopping: ctx.cancellation_token(),
                    }),
                }),
            )?;
            ctx.provide_as(self.scope.key::<CollaborationCapability>(), service)?;
            Ok(Effect::Done)
        })
    }
}
impl Capability for CollaborationCapabilityPlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}

/// Binds one branch's collaboration events to the Session capability.
struct CollaborationTimelinePlugin {
    scope: BranchScope,
}
impl CollaborationTimelinePlugin {
    fn new(scope: &BranchScope) -> Self {
        Self { scope: *scope }
    }
}
impl TypedPlugin for CollaborationTimelinePlugin {
    type Deps = (
        InSession<CollaborationCapability>,
        InBranch<Timeline>,
        InBranch<Dispatcher>,
        InBranch<TimelineIdentity>,
        InBranch<BranchLifecycle>,
    );
    fn name(&self) -> &str {
        "CollaborationTimeline"
    }
    fn apply<'a>(
        &'a self,
        ctx: &'a Ctx,
        (
            InSession(service),
            InBranch(timeline),
            InBranch(dispatcher),
            InBranch(identity),
            InBranch(phase),
        ): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let branch_id = identity.branch_id.clone();
            let owner = service.clone();
            let unloading = branch_id.clone();
            ctx.effect(move || {
                Effect::AsyncDisposer(Box::new(move || {
                    Box::pin(async move {
                        owner.unload(&unloading).await;
                        Ok(())
                    })
                }))
            })?;
            // Subscribe, then rebuild under the gate: see the capability
            // reload protocol. Handlers skip events the rebuild covered.
            let rebuilt = Arc::new(tokio::sync::Mutex::new(0u64));
            let gate = rebuilt.clone();
            let mut rebuilding = rebuilt.lock().await;
            let consumer = service.clone();
            let consumed = branch_id.clone();
            dispatcher.subscribe(
                ctx,
                "CollaborationTimeline",
                &[
                    "turn/start",
                    "turn/end",
                    "stop/interrupt",
                    "collaboration/request",
                    "collaboration/cancel",
                    "collaboration/close",
                    "collaboration/message",
                    "collaboration/delivered",
                    "collaboration/result",
                ],
                move |event| {
                    let gate = gate.clone();
                    let consumer = consumer.clone();
                    let consumed = consumed.clone();
                    Box::pin(async move {
                        if event.sequence > *gate.lock().await {
                            consumer.consume(&consumed, event).await
                        } else {
                            Ok(())
                        }
                    })
                },
            )?;
            let through = timeline.committed();
            let events = timeline.local(..=through).await?;
            *rebuilding = events.last().map_or(0, |event| event.sequence);
            // Unload waits for the rebuild's member stops with other work.
            let tasks = TaskTracker::new();
            let (current_turn, interrupted) = service.rebuild(&timeline, &phase, &events, &tasks);
            service.state.lock().unwrap().branches.insert(
                branch_id,
                BranchRecord {
                    timeline,
                    tasks,
                    abort: CancellationToken::new(),
                    current_turn,
                    interrupted,
                    closing: false,
                    ops: BTreeMap::new(),
                    open: BTreeMap::new(),
                    results: events
                        .iter()
                        .filter(|&event| matches!(event.kind, EventKind::CollaborationResult(_)))
                        .map(|event| (event.id.clone(), event.sequence))
                        .collect(),
                    chains: BTreeMap::new(),
                },
            );
            drop(rebuilding);
            Ok(Effect::Done)
        })
    }
}
impl Capability for CollaborationTimelinePlugin {
    fn keys(&self) -> KeysOf<Self> {
        scoped::<Self::Deps, _>(&self.scope)
    }
}
