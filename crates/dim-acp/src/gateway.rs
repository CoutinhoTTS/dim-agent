use crate::{
    projection::Projection,
    rpc::{self, Native, Rpc},
};
use agent_client_protocol::{
    Client, ConnectionTo, Error, Responder, UntypedMessage, schema::v1 as acp,
};
use dim_agent_types::{ContentBlock, TurnOutcome, UserInput};
use dim_client::wire::{Event, EventContent, History, Progress};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, future::Future, path::Path};
use tokio::sync::mpsc;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub(crate) enum Incoming {
    Request(UntypedMessage, Responder),
    Notification(UntypedMessage),
    Work(Work),
}

pub(crate) enum Work {
    Created {
        reply: Responder,
        cwd: String,
        result: Result<Created, Error>,
    },
    Prepared {
        id: String,
        result: Result<Prepared, Error>,
    },
    Submitted {
        id: String,
        submission: String,
        result: Result<Submitted, Error>,
    },
    Cancelled {
        id: String,
        result: Result<Value, Error>,
    },
    Listed {
        reply: Responder,
        result: Result<Vec<SessionInfo>, Error>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Created {
    session_id: String,
}

#[derive(Deserialize)]
pub(crate) struct Submitted {
    event: Event,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionInfo {
    session_id: String,
    cwd: String,
    title: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventNotice {
    session_id: String,
    branch_id: String,
    event: Event,
}

pub(crate) struct Prepared {
    history: Vec<Event>,
    through: u64,
}

struct Waiter {
    reply: Responder,
    new: bool,
    replay: bool,
}
struct Loading {
    cwd: String,
    waiters: Vec<Waiter>,
    buffered: Vec<(String, Value)>,
    previous: Option<Box<Session>>,
}
struct Prompt {
    reply: Responder,
    submission: String,
    input: Option<String>,
    turn: Option<String>,
    cancelled: bool,
}
struct Session {
    cwd: String,
    projection: Projection,
    prompt: Option<Prompt>,
}
enum Slot {
    Loading(Loading),
    Ready(Box<Session>),
}

pub(crate) struct Gateway {
    rpc: Rpc,
    cx: ConnectionTo<Client>,
    incoming: mpsc::Sender<Incoming>,
    tasks: TaskTracker,
    stop: CancellationToken,
    agent_id: Option<String>,
    initialized: bool,
    creating: usize,
    sessions: HashMap<String, Slot>,
}

fn busy() -> Error {
    Error::new(-32001, "Session or gateway is busy").data(json!({"code":"session_busy"}))
}
fn params<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|_| Error::invalid_params())
}

fn controls(value: &Value) -> Result<(), Error> {
    if let Some(lody) = value.get("_meta").and_then(|meta| meta.get("lody")) {
        let lody = lody.as_object().ok_or_else(Error::invalid_params)?;
        for field in [
            "steer",
            "goalControl",
            "sessionConfig",
            "worktreeProject",
            "forkAtTurn",
        ] {
            if lody.contains_key(field) {
                return Err(Error::new(-32001, "Unsupported Lody control")
                    .data(json!({"code":"unsupported_control","field":field})));
            }
        }
    }
    Ok(())
}

fn establishment(cwd: &Path, servers: usize, roots: usize) -> Result<(), Error> {
    if !cwd.is_absolute() || servers != 0 || roots != 0 {
        return Err(Error::invalid_params());
    }
    Ok(())
}

impl Gateway {
    pub(crate) fn new(
        rpc: Rpc,
        cx: ConnectionTo<Client>,
        incoming: mpsc::Sender<Incoming>,
        tasks: TaskTracker,
        stop: CancellationToken,
        agent_id: Option<String>,
    ) -> Self {
        Self {
            rpc,
            cx,
            incoming,
            tasks,
            stop,
            agent_id,
            initialized: false,
            creating: 0,
            sessions: HashMap::new(),
        }
    }

    fn spawn(&self, future: impl Future<Output = Work> + Send + 'static) {
        let incoming = self.incoming.clone();
        let stop = self.stop.clone();
        self.tasks.spawn(async move {
            tokio::select! {
                () = stop.cancelled() => {},
                result = future => { let _ = incoming.send(Incoming::Work(result)).await; },
            }
        });
    }

    fn update(&self, id: &str, update: Value) -> Result<(), Error> {
        self.cx.send_notification(UntypedMessage::new(
            "session/update",
            json!({"sessionId":id,"update":update}),
        )?)
    }

    fn prepare(&mut self, id: String, cwd: String, waiter: Waiter, attach: bool) {
        let rpc = self.rpc.clone();
        let expected_cwd = (!waiter.new).then(|| cwd.clone());
        let previous = match self.sessions.remove(&id) {
            Some(Slot::Ready(session)) => Some(session),
            None => None,
            Some(Slot::Loading(_)) => unreachable!("preparations are coalesced at admission"),
        };
        self.sessions.insert(
            id.clone(),
            Slot::Loading(Loading {
                cwd,
                waiters: vec![waiter],
                buffered: Vec::new(),
                previous,
            }),
        );
        self.spawn(async move {
            let result = read_history(&rpc, &id, attach, expected_cwd).await;
            Work::Prepared { id, result }
        });
    }

    fn request(&mut self, request: UntypedMessage, reply: Responder) -> Result<(), Error> {
        if request.method == "initialize" {
            if self.initialized {
                return reply.respond_with_error(Error::invalid_request());
            }
            let _: acp::InitializeRequest = match params(request.params) {
                Ok(value) => value,
                Err(error) => return reply.respond_with_error(error),
            };
            if ![
                "session.create",
                "session.attach",
                "session.detach",
                "session.submit",
                "session.input_lookup",
                "session.history",
                "session.stop",
            ]
            .iter()
            .all(|method| self.rpc.supports(method))
            {
                return reply.respond_with_error(Error::new(
                    -32001,
                    "App Server lacks required ACP session methods",
                ));
            }
            self.initialized = true;
            return reply.respond(json!({"protocolVersion":1,"agentInfo":{"name":"dimcode","version":env!("CARGO_PKG_VERSION")},"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{},"list":if self.rpc.supports("session.list") { json!({}) } else { Value::Null }}}}));
        }
        if !self.initialized {
            return reply.respond_with_error(Error::invalid_request());
        }
        if let Err(error) = controls(&request.params) {
            return reply.respond_with_error(error);
        }
        let rpc = self.rpc.clone();
        match request.method.as_str() {
            "session/new" => {
                let request: acp::NewSessionRequest = match params(request.params) {
                    Ok(value) => value,
                    Err(error) => return reply.respond_with_error(error),
                };
                if let Err(error) = establishment(
                    &request.cwd,
                    request.mcp_servers.len(),
                    request.additional_directories.len(),
                ) {
                    return reply.respond_with_error(error);
                }
                if self.sessions.len() + self.creating >= 128 {
                    return reply.respond_with_error(busy());
                }
                self.creating += 1;
                let agent = self.agent_id.clone();
                self.spawn(async move {
                    let mut cwd = request.cwd.to_string_lossy().into_owned();
                    let result = async {
                        cwd = tokio::fs::canonicalize(&request.cwd)
                            .await
                            .map_err(|_| Error::invalid_params())?
                            .into_os_string()
                            .into_string()
                            .map_err(|_| Error::invalid_params())?;
                        let mut args = json!({"cwd":cwd});
                        if let Some(agent) = agent {
                            args["agentId"] = json!(agent);
                        }
                        rpc.typed("session.create", args).await
                    }
                    .await;
                    Work::Created { reply, cwd, result }
                });
            }
            "session/load" | "session/resume" => {
                let replay = request.method == "session/load";
                let parsed = if replay {
                    params::<acp::LoadSessionRequest>(request.params).map(|request| {
                        (
                            request.session_id,
                            request.cwd,
                            request.mcp_servers.len(),
                            request.additional_directories.len(),
                        )
                    })
                } else {
                    params::<acp::ResumeSessionRequest>(request.params).map(|request| {
                        (
                            request.session_id,
                            request.cwd,
                            request.mcp_servers.len(),
                            request.additional_directories.len(),
                        )
                    })
                };
                let (session_id, cwd, servers, roots) = match parsed {
                    Ok(value) => value,
                    Err(error) => return reply.respond_with_error(error),
                };
                if let Err(error) = establishment(&cwd, servers, roots) {
                    return reply.respond_with_error(error);
                }
                let id = session_id.to_string();
                let cwd = cwd.to_string_lossy().into_owned();
                match self.sessions.get_mut(&id) {
                    Some(Slot::Loading(loading)) => {
                        if loading.cwd != cwd {
                            return reply.respond_with_error(Error::invalid_params());
                        }
                        loading.waiters.push(Waiter {
                            reply,
                            new: false,
                            replay,
                        });
                    }
                    Some(Slot::Ready(session)) => {
                        if session.prompt.is_some() {
                            return reply.respond_with_error(busy());
                        }
                        if !replay && session.cwd == cwd {
                            return reply.respond(json!({}));
                        }
                        self.prepare(
                            id,
                            cwd,
                            Waiter {
                                reply,
                                new: false,
                                replay,
                            },
                            false,
                        );
                    }
                    None => {
                        if self.sessions.len() >= 128 {
                            return reply.respond_with_error(busy());
                        }
                        self.prepare(
                            id,
                            cwd,
                            Waiter {
                                reply,
                                new: false,
                                replay,
                            },
                            true,
                        );
                    }
                }
            }
            "session/prompt" => {
                let request: acp::PromptRequest = match params(request.params) {
                    Ok(value) => value,
                    Err(error) => return reply.respond_with_error(error),
                };
                let id = request.session_id.to_string();
                let Some(Slot::Ready(session)) = self.sessions.get_mut(&id) else {
                    return reply.respond_with_error(Error::resource_not_found(Some(id)));
                };
                if session.prompt.is_some() {
                    return reply.respond_with_error(busy());
                }
                let mut content = Vec::new();
                for block in request.prompt {
                    let text = match block {
                        acp::ContentBlock::Text(text) => text.text,
                        acp::ContentBlock::ResourceLink(link) => format!("@{}", link.uri),
                        _ => {
                            return reply.respond_with_error(
                                Error::new(-32001, "Unsupported prompt content")
                                    .data(json!({"code":"unsupported_content"})),
                            );
                        }
                    };
                    content.push(ContentBlock::Text { text });
                }
                if content.is_empty() {
                    return reply.respond_with_error(Error::invalid_params());
                }
                let submission = uuid::Uuid::new_v4().to_string();
                session.prompt = Some(Prompt {
                    reply,
                    submission: submission.clone(),
                    input: None,
                    turn: None,
                    cancelled: false,
                });
                self.spawn(async move {
                    let result = rpc.typed("session.submit", json!({"sessionId":id,"branchId":"main","submissionId":submission,"user":UserInput { content },"mode":"queue"})).await;
                    Work::Submitted { id, submission, result }
                });
            }
            "session/list" if self.rpc.supports("session.list") => {
                let request: acp::ListSessionsRequest = match params(request.params) {
                    Ok(value) => value,
                    Err(error) => return reply.respond_with_error(error),
                };
                if request
                    .cursor
                    .as_ref()
                    .is_some_and(|cursor| !cursor.is_empty())
                {
                    return reply.respond_with_error(Error::invalid_params());
                }
                let agent = self.agent_id.clone();
                self.spawn(async move {
                    let result = async {
                        let mut args = json!({});
                        if let Some(agent) = agent {
                            args["agentId"] = json!(agent);
                        }
                        let mut sessions: Vec<SessionInfo> =
                            rpc.typed("session.list", args).await?;
                        if let Some(cwd) = request.cwd {
                            sessions.retain(|session| Path::new(&session.cwd) == cwd);
                        }
                        Ok(sessions)
                    }
                    .await;
                    Work::Listed { reply, result }
                });
            }
            _ => return reply.respond_with_error(Error::method_not_found()),
        }
        Ok(())
    }

    fn cancel(&mut self, notification: UntypedMessage) -> Result<(), Error> {
        if notification.method != "session/cancel" {
            return Ok(());
        }
        let request: acp::CancelNotification = params(notification.params)?;
        let id = request.session_id.to_string();
        let Some(Slot::Ready(session)) = self.sessions.get_mut(&id) else {
            return Ok(());
        };
        if let Some(prompt) = &mut session.prompt {
            prompt.cancelled = true;
        }
        let rpc = self.rpc.clone();
        self.spawn(async move {
            let result = rpc
                .call("session.stop", json!({"sessionId":id,"branchId":"main"}))
                .await;
            Work::Cancelled { id, result }
        });
        Ok(())
    }

    fn work(&mut self, work: Work) -> Result<(), Error> {
        match work {
            Work::Created { reply, cwd, result } => {
                self.creating -= 1;
                match result {
                    Ok(created) => self.prepare(
                        created.session_id,
                        cwd,
                        Waiter {
                            reply,
                            new: true,
                            replay: false,
                        },
                        true,
                    ),
                    Err(error) => reply.respond_with_error(error)?,
                }
            }
            Work::Prepared { id, result } => {
                let Some(Slot::Loading(loading)) = self.sessions.remove(&id) else {
                    return Ok(());
                };
                match result {
                    Ok(prepared) => {
                        let replay = loading.waiters.iter().any(|waiter| waiter.replay);
                        let mut session = Session {
                            cwd: loading.cwd,
                            projection: Projection::default(),
                            prompt: None,
                        };
                        for event in prepared.history {
                            let updates = session.projection.event(&event);
                            if replay {
                                for update in updates {
                                    self.update(&id, update)?;
                                }
                            }
                        }
                        session.projection.through = prepared.through;
                        self.sessions
                            .insert(id.clone(), Slot::Ready(Box::new(session)));
                        for waiter in loading.waiters {
                            waiter.reply.respond(if waiter.new {
                                json!({"sessionId":id})
                            } else {
                                json!({})
                            })?;
                        }
                        for (method, params) in loading.buffered {
                            self.notice(method, params)?;
                        }
                    }
                    Err(error) => {
                        let buffered = loading.buffered;
                        if let Some(previous) = loading.previous {
                            self.sessions.insert(id.clone(), Slot::Ready(previous));
                        } else {
                            let rpc = self.rpc.clone();
                            let detached = id.clone();
                            self.tasks.spawn(async move {
                                let _ = rpc
                                    .call("session.detach", json!({"sessionId":detached}))
                                    .await;
                            });
                        }
                        for waiter in loading.waiters {
                            waiter.reply.respond_with_error(error.clone())?;
                        }
                        for (method, params) in buffered {
                            self.notice(method, params)?;
                        }
                    }
                }
            }
            Work::Submitted {
                id,
                submission,
                result,
            } => {
                let Some(Slot::Ready(session)) = self.sessions.get_mut(&id) else {
                    return Ok(());
                };
                if session
                    .prompt
                    .as_ref()
                    .is_some_and(|prompt| prompt.submission == submission)
                {
                    match result {
                        Ok(result) => {
                            session
                                .projection
                                .silent_inputs
                                .insert(result.event.id.clone());
                            session.prompt.as_mut().unwrap().input = Some(result.event.id);
                        }
                        Err(error) => session
                            .prompt
                            .take()
                            .unwrap()
                            .reply
                            .respond_with_error(error)?,
                    }
                }
            }
            Work::Cancelled { id, result } => {
                let Some(Slot::Ready(session)) = self.sessions.get_mut(&id) else {
                    return Ok(());
                };
                if session
                    .prompt
                    .as_ref()
                    .is_some_and(|prompt| prompt.cancelled && prompt.turn.is_none())
                {
                    session
                        .prompt
                        .take()
                        .unwrap()
                        .reply
                        .respond(json!({"stopReason":"cancelled"}))?;
                }
                result?;
            }
            Work::Listed { reply, result } => match result {
                Ok(sessions) => {
                    let sessions: Vec<Value> = sessions
                        .into_iter()
                        .map(|session| {
                            let mut row = json!({"sessionId":session.session_id,"cwd":session.cwd});
                            if let Some(title) = session.title {
                                row["title"] = json!(title);
                            }
                            row
                        })
                        .collect();
                    reply.respond(json!({"sessions":sessions}))?;
                }
                Err(error) => reply.respond_with_error(error)?,
            },
        }
        Ok(())
    }

    fn notice(&mut self, method: String, params: Value) -> Result<(), Error> {
        let id = params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(Slot::Loading(loading)) = id.as_ref().and_then(|id| self.sessions.get_mut(id)) {
            if loading.buffered.len() >= 256 {
                return Err(Error::new(-32001, "Native replay buffer overflow"));
            }
            loading.buffered.push((method, params));
            return Ok(());
        }
        match method.as_str() {
            "session.event" => {
                let notice: EventNotice = decode_native(params)?;
                if notice.branch_id != "main" {
                    return Ok(());
                }
                let Some(Slot::Ready(session)) = self.sessions.get_mut(&notice.session_id) else {
                    return Ok(());
                };
                if notice.event.sequence <= session.projection.through {
                    return Ok(());
                }
                let mut finish = None;
                if let Some(prompt) = &mut session.prompt {
                    match &notice.event.content {
                        EventContent::Input(data)
                            if data.submission_id.as_deref() == Some(&prompt.submission) =>
                        {
                            prompt.input = Some(notice.event.id.clone());
                            session
                                .projection
                                .silent_inputs
                                .insert(notice.event.id.clone());
                        }
                        EventContent::InputConsumed(data)
                            if prompt.input.as_deref() == Some(&data.input_id) =>
                        {
                            prompt.turn.clone_from(&session.projection.turn)
                        }
                        EventContent::InputDeleted(data)
                            if prompt.input.as_deref() == Some(&data.input_id) =>
                        {
                            finish = Some(Ok(json!({"stopReason":"cancelled"})))
                        }
                        EventContent::TurnEnd(data)
                            if prompt.turn.as_deref() == Some(&data.turn_id) =>
                        {
                            finish = Some(if prompt.cancelled {
                                Ok(json!({"stopReason":"cancelled"}))
                            } else {
                                match &data.outcome {
                                    TurnOutcome::Completed { .. } => {
                                        Ok(json!({"stopReason":"end_turn"}))
                                    }
                                    TurnOutcome::Cancelled { .. } => {
                                        Ok(json!({"stopReason":"cancelled"}))
                                    }
                                    TurnOutcome::Failed { error } => {
                                        Err(Error::new(-32001, error.clone())
                                            .data(json!({"code":"turn_failed"})))
                                    }
                                }
                            });
                        }
                        _ => {}
                    }
                }
                let updates = session.projection.event(&notice.event);
                let completed = finish
                    .and_then(|result| session.prompt.take().map(|prompt| (prompt.reply, result)));
                for update in updates {
                    self.update(&notice.session_id, update)?;
                }
                if let Some((reply, result)) = completed {
                    reply.respond_with_result(result)?;
                }
            }
            "session.model.progress" => {
                let progress: Progress = decode_native(params)?;
                if progress.branch_id != "main" {
                    return Ok(());
                }
                let id = progress.session_id.clone();
                if let Some(Slot::Ready(session)) = self.sessions.get_mut(&id) {
                    let updates = session.projection.progress(progress);
                    for update in updates {
                        self.update(&id, update)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) async fn run(
        mut self,
        mut incoming: mpsc::Receiver<Incoming>,
        mut native: mpsc::Receiver<Native>,
    ) -> Result<(), Error> {
        loop {
            tokio::select! {
                () = self.cx.incoming_closed() => return Ok(()),
                () = self.stop.cancelled() => return Ok(()),
                message = incoming.recv() => match message {
                    Some(Incoming::Request(request, reply)) => self.request(request, reply)?,
                    Some(Incoming::Notification(notification)) => self.cancel(notification)?,
                    Some(Incoming::Work(work)) => self.work(work)?,
                    None => return Ok(()),
                },
                message = native.recv() => match message {
                    Some(Native::Notice { method, params }) => self.notice(method, params)?,
                    Some(Native::Closed) | None => return Err(rpc::unavailable()),
                },
            }
        }
    }
}

fn decode_native<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value)
        .map_err(|_| Error::internal_error().data(json!({"code":"invalid_native_notification"})))
}

async fn read_history(
    rpc: &Rpc,
    id: &str,
    attach: bool,
    expected_cwd: Option<String>,
) -> Result<Prepared, Error> {
    if let Some(cwd) = expected_cwd {
        let method = if rpc.supports("session.summaries") {
            "session.summaries"
        } else {
            "session.list"
        };
        let sessions: Vec<SessionInfo> = rpc.typed(method, json!({})).await?;
        let session = sessions
            .into_iter()
            .find(|session| session.session_id == id)
            .ok_or_else(|| Error::resource_not_found(Some(id.to_owned())))?;
        let requested = tokio::fs::canonicalize(cwd)
            .await
            .map_err(|_| Error::invalid_params())?;
        if Path::new(&session.cwd) != requested {
            return Err(Error::invalid_params());
        }
    }
    if attach {
        rpc.call("session.attach", json!({"sessionId":id})).await?;
    }
    let mut history = Vec::new();
    let mut after = 0;
    let mut through = None;
    loop {
        let mut args = json!({"sessionId":id,"branchId":"main","after":after,"limit":256});
        if let Some(through) = through {
            args["through"] = json!(through);
        }
        let page: History = rpc.typed("session.history", args).await?;
        let next = page
            .next(after, through)
            .map_err(|_| Error::internal_error().data(json!({"code":"invalid_history_cursor"})))?;
        through = Some(page.through);
        for event in page.events {
            history.push(decode_native(event)?);
        }
        if let Some(next) = next {
            after = next;
        } else {
            return Ok(Prepared {
                history,
                through: page.through,
            });
        }
    }
}
