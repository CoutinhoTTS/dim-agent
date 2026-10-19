//! Runs the App Server session for the interactive interface on a worker thread.
//!
//! The terminal loop is synchronous; the client is async. The worker owns the
//! connection and the projection, receives commands, and publishes view
//! snapshots: one a frame interval after the first change, however fast the
//! stream runs, so text arrives steadily and a burst of events costs one
//! snapshot. Snapshot rows are shared with the projection, not copied.
//!
//! Features the protocol v3 App Server does not offer yet (model and provider
//! management, sign-in, usage, approval preferences) answer with an explicit
//! "not available" update instead of failing silently.

use std::sync::mpsc;
use std::time::Duration;

use dim_agent_types::{ApprovalDecision, ModelId};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::args::Options;
use crate::client::{self, Client, Message};
use crate::model::session::{
    Command, DraftPart, OauthEvent, SessionItem, SessionPort, SwitchKind, Update,
};
use crate::projection::Projection;
use crate::wire::{Configuration, Event, EventContent, Progress};
use crate::{configure, mcp};

const PUBLISH_INTERVAL: Duration = Duration::from_millis(16);
const HISTORY_PAGE: u64 = 256;
const UNAVAILABLE: &str = "not available in this build yet";

pub struct SessionHost {
    commands: UnboundedSender<Command>,
    updates: mpsc::Receiver<Update>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SessionPort for SessionHost {
    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
    fn drain(&self) -> Vec<Update> {
        self.updates.try_iter().collect()
    }
    fn shutdown(self: Box<Self>) {
        SessionHost::shutdown(*self);
    }
}

impl SessionHost {
    pub fn start(options: Options) -> Self {
        let (commands, command_rx) = unbounded_channel();
        let (update_tx, updates) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("dim-session".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = update_tx.send(Update::Failed(format!("runtime: {error}")));
                        return;
                    }
                };
                runtime.block_on(run(options, command_rx, update_tx));
            })
            .expect("spawn session thread");
        Self {
            commands,
            updates,
            thread: Some(thread),
        }
    }

    /// Closes the command channel. The worker is not joined: it may still be
    /// inside the App Server start (seconds), and leaving the interface must
    /// not wait for that. The process exit reaps the thread.
    pub fn shutdown(mut self) {
        drop(std::mem::replace(&mut self.commands, unbounded_channel().0));
        if let Some(thread) = self.thread.take() {
            let deadline = std::time::Instant::now() + Duration::from_millis(150);
            while !thread.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
        }
    }
}

/// The session the worker is attached to and what the screen shows of it.
struct Worker {
    options: Options,
    client: Client,
    updates: mpsc::Sender<Update>,
    session_id: String,
    projection: Projection,
    /// Highest event sequence applied; replayed or duplicate events are skipped.
    cursor: u64,
    /// When the next snapshot goes out; set by the first unpublished change.
    publish_at: Option<tokio::time::Instant>,
    /// The `shell.run` in flight: its request id and run id.
    shell: Option<(u64, String)>,
    /// `shell.stop` requests awaiting their replies.
    shell_stops: Vec<u64>,
    /// The model the session adopted, for the preference commands.
    model: Option<Configuration>,
    /// The MCP sign-in waiting for its callback.
    mcp_attempt: Option<McpAttempt>,
    /// Callbacks the local listener captured.
    mcp_callbacks: (
        UnboundedSender<McpCallback>,
        UnboundedReceiver<McpCallback>,
    ),
    /// Servers already reported as needing sign-in.
    mcp_required: std::collections::HashSet<String>,
}

struct McpAttempt {
    server_id: String,
    attempt_id: String,
    deadline: tokio::time::Instant,
    listener: Option<tokio::task::JoinHandle<()>>,
}

/// What the local callback listener heard for `attempt_id`.
struct McpCallback {
    attempt_id: String,
    callback: crate::Result<(String, tokio::net::TcpStream)>,
}

async fn run(
    options: Options,
    mut commands: UnboundedReceiver<Command>,
    updates: mpsc::Sender<Update>,
) {
    let client = match client::connect(&options).await {
        Ok(client) => client,
        Err(error) => {
            let _ = updates.send(Update::Failed(format!(
                "Cannot start the App Server: {error}"
            )));
            return;
        }
    };
    let mut worker = Worker {
        options,
        client,
        updates: updates.clone(),
        session_id: String::new(),
        projection: Projection::default(),
        cursor: 0,
        publish_at: None,
        shell: None,
        shell_stops: Vec::new(),
        model: None,
        mcp_attempt: None,
        mcp_callbacks: unbounded_channel(),
        mcp_required: Default::default(),
    };
    if let Err(error) = worker.open().await {
        let _ = updates.send(Update::Failed(error.to_string()));
        return;
    }
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                if let Err(error) = worker.command(command).await {
                    let _ = updates.send(Update::Failed(error.to_string()));
                    break;
                }
            }
            message = worker.client.next() => {
                let message = match message {
                    Ok(message) => message,
                    Err(error) => {
                        let _ = updates.send(Update::Failed(error.to_string()));
                        break;
                    }
                };
                if let Err(error) = worker.message(message) {
                    let _ = updates.send(Update::Failed(error.to_string()));
                    break;
                }
            }
            Some(callback) = worker.mcp_callbacks.1.recv() => worker.mcp_callback(callback).await,
            // The deadline is set by the first change and kept: a sleep made
            // afresh on every pass would restart with each event and hold the
            // snapshot back until the stream paused.
            _ = tokio::time::sleep_until(worker.publish_at.unwrap_or_else(tokio::time::Instant::now)),
                if worker.publish_at.is_some() => worker.publish(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    session_id: String,
    model: ModelId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct History {
    events: Vec<Event>,
    through: u64,
    next_after: u64,
    done: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventNotice {
    session_id: String,
    event: Event,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateNotice {
    session_id: String,
    running: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalNotice {
    session_id: String,
    request_id: String,
    call: ApprovalCall,
    reason: String,
}

#[derive(Deserialize)]
struct ApprovalCall {
    name: String,
    input: dim_agent_types::JsonObject,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelNotice {
    request_id: String,
}

impl Worker {
    /// Creates or resumes the session the process was started for.
    async fn open(&mut self) -> crate::Result<()> {
        if let Some(id) = self.options.resume.clone() {
            return self.attach(id, SwitchKind::Resumed).await;
        }
        let mut params = json!({"cwd": self.options.cwd});
        if let (Some(provider), Some(model)) = (&self.options.provider, &self.options.model) {
            params["model"] = json!({"provider": provider, "modelId": model});
        }
        let created: Created = self.client.typed("session.create", params).await?;
        self.enter(created.session_id, created.model, SwitchKind::New)
            .await
    }

    async fn enter(
        &mut self,
        session_id: String,
        model: ModelId,
        kind: SwitchKind,
    ) -> crate::Result<()> {
        self.client
            .rpc("session.attach", json!({"sessionId": session_id}))
            .await?;
        self.session_id = session_id.clone();
        self.projection = Projection::default();
        self.cursor = 0;
        self.mcp_required.clear();
        self.model = Some(Configuration {
            provider_id: model.provider_id.clone(),
            model_id: model.model_id.clone(),
        });
        self.load_history().await?;
        self.send(Update::Ready {
            provider: model.provider_id,
            model: model.model_id,
        });
        self.send(Update::Switched { session_id, kind });
        self.publish();
        Ok(())
    }

    async fn attach(&mut self, session_id: String, kind: SwitchKind) -> crate::Result<()> {
        let listed = self.list().await?;
        let session = listed
            .into_iter()
            .find(|session| session.session_id == session_id)
            .ok_or_else(|| format!("Session {session_id} was not found"))?;
        self.enter(session_id, session.model, kind).await
    }

    async fn list(&mut self) -> crate::Result<Vec<SessionSummary>> {
        // Only the summaries carry the preview and update time the picker
        // shows; the reply is the bare list.
        self.client.typed("session.summaries", json!({})).await
    }

    async fn load_history(&mut self) -> crate::Result<()> {
        let mut through = None;
        let mut after = self.cursor;
        loop {
            let mut params = json!({
                "sessionId": self.session_id,
                "branchId": "main",
                "after": after,
                "limit": HISTORY_PAGE,
            });
            if let Some(through) = through {
                params["through"] = json!(through);
            }
            let page: History = self.client.typed("session.history", params).await?;
            if through.is_some_and(|through| through != page.through)
                || page.next_after < after
                || page.next_after > page.through
                || !page.done && page.next_after == after
            {
                return Err("Invalid history page cursor".into());
            }
            through = Some(page.through);
            for event in &page.events {
                if event.sequence <= after || event.sequence > page.next_after {
                    return Err("History event is outside its page".into());
                }
            }
            for event in page.events {
                self.apply(event);
            }
            after = page.next_after;
            if page.done {
                if after != page.through {
                    return Err("History ended before its fixed watermark".into());
                }
                return Ok(());
            }
        }
    }

    async fn submit(&mut self, text: String, mode: &str) {
        let params = json!({
            "sessionId": self.session_id,
            "branchId": "main",
            "submissionId": uuid::Uuid::new_v4().to_string(),
            "text": text,
            "mode": mode,
        });
        let result = self.client.rpc("session.submit", params).await;
        self.report_failure(result);
    }

    /// Stores long pastes as attachments, then submits the draft's text
    /// and blocks in order.
    async fn submit_draft(&mut self, parts: Vec<DraftPart>) {
        use dim_agent_types::ContentBlock;
        let mut content = Vec::new();
        let mut text = String::new();
        let mut pastes = 0;
        for part in parts {
            match part {
                DraftPart::Text(value) => text.push_str(&value),
                DraftPart::Paste(value) => {
                    pastes += 1;
                    if value.len() <= crate::ui::composer::INLINE_PASTE {
                        text.push_str(&value);
                        continue;
                    }
                    #[derive(Deserialize)]
                    struct Stored {
                        path: String,
                    }
                    let lines = value.lines().count();
                    let stored: crate::Result<Stored> = self
                        .client
                        .typed(
                            "attachment.put",
                            json!({"sessionId": self.session_id, "text": value}),
                        )
                        .await;
                    match stored {
                        Ok(stored) => text.push_str(&format!(
                            "[Pasted text: {} ({lines} lines)]",
                            stored.path
                        )),
                        Err(error) => {
                            self.send(Update::Notice(format!(
                                "Paste {pastes}: {error}; not sent"
                            )));
                            return;
                        }
                    }
                }
                DraftPart::Block(block) => {
                    if !text.is_empty() {
                        content.push(ContentBlock::Text {
                            text: std::mem::take(&mut text),
                        });
                    }
                    content.push(block);
                }
            }
        }
        if !text.is_empty() {
            content.push(ContentBlock::Text { text });
        }
        let mut params = json!({
            "sessionId": self.session_id,
            "branchId": "main",
            "submissionId": uuid::Uuid::new_v4().to_string(),
            "mode": "queue",
        });
        match content.as_slice() {
            [ContentBlock::Text { text }] => params["text"] = json!(text),
            _ => params["user"] = json!(dim_agent_types::UserInput { content }),
        }
        let result = self.client.rpc("session.submit", params).await;
        self.report_failure(result);
    }

    async fn command(&mut self, command: Command) -> crate::Result<()> {
        match command {
            Command::Submit(text) => self.submit(text, "queue").await,
            Command::SubmitDraft(parts) => self.submit_draft(parts).await,
            Command::Stop => {
                let params = json!({
                    "sessionId": self.session_id,
                    "branchId": "main",
                    "reason": "TUI interrupted",
                });
                let result = self.client.rpc("session.stop", params).await;
                self.report_failure(result);
            }
            Command::Approval {
                request_id,
                branch_id,
                allow,
                remember: _,
            } => {
                let decision = if allow {
                    ApprovalDecision::Allow
                } else {
                    ApprovalDecision::Deny
                };
                let params = json!({
                    "sessionId": self.session_id,
                    "branchId": branch_id,
                    "requestId": request_id,
                    "decision": decision,
                });
                let result = self.client.rpc("approval.respond", params).await;
                if self.report_failure(result) {
                    self.projection.remove_approval(&request_id);
                    self.changed();
                }
            }
            Command::NewSession => {
                let created: Created = self
                    .client
                    .typed("session.create", json!({"cwd": self.options.cwd}))
                    .await?;
                self.enter(created.session_id, created.model, SwitchKind::New)
                    .await?;
            }
            Command::Resume(session_id) => {
                self.attach(session_id, SwitchKind::Resumed).await?;
            }
            Command::ListSessions => {
                let sessions = self.list().await.map(|mut sessions| {
                    // The daemon lists in no particular order; the picker
                    // shows the most recent first.
                    sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
                    sessions
                        .into_iter()
                        .map(|session| SessionItem {
                            id: session.session_id,
                            title: None,
                            preview: session.preview,
                            cwd: session.cwd,
                            updated_at: session.updated_at,
                            model: session.model.model_id,
                        })
                        .collect()
                });
                self.send(Update::Sessions(
                    sessions.map_err(|error| error.to_string()),
                ));
            }
            Command::Usage => self.send(Update::Report {
                text: format!("Usage reporting is {UNAVAILABLE}."),
                warning: false,
            }),
            Command::ListModels => {
                self.send(Update::Models(Err(format!("Models are {UNAVAILABLE}"))))
            }
            Command::SetApprovals(_) => self.send(Update::Approvals(Err(
                format!("Approval preferences are {UNAVAILABLE}"),
            ))),
            Command::ListProviders => self.send(Update::Providers(Err(format!(
                "Providers are {UNAVAILABLE}"
            )))),
            Command::Provider(_) => self.send(Update::ProviderResult(Err(format!(
                "Providers are {UNAVAILABLE}"
            )))),
            Command::OauthStart(start) => self.send(Update::Oauth {
                attempt: start.attempt,
                event: OauthEvent::Done(Err(Some(format!("Sign-in is {UNAVAILABLE}")))),
            }),
            Command::OauthCancel | Command::OauthPaste(_) => {}
            Command::ShellRun {
                run_id,
                command,
                private,
            } => {
                let params = json!({
                    "sessionId": self.session_id,
                    "branchId": "main",
                    "runId": run_id,
                    "command": command,
                    "mode": if private { "private" } else { "record" },
                });
                // Not awaited: the reply comes once the command ends, and
                // other commands keep going meanwhile.
                match self.client.send("shell.run", params).await {
                    Ok(id) => self.shell = Some((id, run_id)),
                    Err(error) => self.send(Update::ShellDone {
                        run_id,
                        result: Err(error.to_string()),
                    }),
                }
            }
            Command::ShellStop { run_id } => {
                let params = json!({"sessionId": self.session_id, "runId": run_id});
                match self.client.send("shell.stop", params).await {
                    Ok(id) => self.shell_stops.push(id),
                    Err(error) => self.send(Update::Notice(error.to_string())),
                }
            }
            Command::Mcp(args) => {
                let result = self.mcp(&args).await;
                if let Err(error) = result {
                    self.report(mcp::error(error).to_string(), true);
                }
            }
            Command::Configure(text) => {
                let model = self.model.clone();
                match configure::run(&mut self.client, &self.session_id, model.as_ref(), &text)
                    .await
                {
                    Ok(report) => self.report(report.lines.join("\n"), false),
                    Err(error) => self.report(error.to_string(), true),
                }
            }
            Command::SelectModel { .. } => {
                self.send(Update::Notice(format!("Model selection is {UNAVAILABLE}")))
            }
            Command::AskAnswers { .. } | Command::AskCancel { .. } => {
                self.send(Update::Notice(format!("Questions are {UNAVAILABLE}")))
            }
            Command::InputControl { .. } => {
                self.send(Update::Notice(format!("Queue control is {UNAVAILABLE}")))
            }
            Command::Reconnect => self.send(Update::Notice(format!("Reconnect is {UNAVAILABLE}"))),
        }
        Ok(())
    }

    /// Reports a failed command as a notice; the session itself keeps going.
    fn report_failure<T>(&self, result: crate::Result<T>) -> bool {
        match result {
            Ok(_) => true,
            Err(error) => {
                self.send(Update::Notice(error.to_string()));
                false
            }
        }
    }

    fn message(&mut self, message: Message) -> crate::Result<()> {
        let (method, params) = match message {
            Message::Notice { method, params } => (method, params),
            Message::Response { id, result } => {
                self.reply(id, result);
                return Ok(());
            }
        };
        match method.as_str() {
            "management.progress"
                if params["method"] == "shell.run"
                    && self
                        .shell
                        .as_ref()
                        .is_some_and(|(request, _)| params["id"].as_u64() == Some(*request)) =>
            {
                let run_id = self.shell.as_ref().expect("shell run").1.clone();
                let data = &params["data"];
                if let Some(text) = data["text"].as_str() {
                    self.send(Update::ShellProgress {
                        run_id,
                        text: text.to_owned(),
                        skipped: data["skipped"].as_u64().unwrap_or(0),
                    });
                }
            }
            "session.event" => {
                let notice: EventNotice = serde_json::from_value(params)?;
                if notice.session_id == self.session_id {
                    self.apply(notice.event);
                }
            }
            "session.model.progress" => {
                let notice: Progress = serde_json::from_value(params)?;
                if notice.session_id == self.session_id {
                    self.projection.progress(&notice);
                    self.changed();
                }
            }
            "session.state" => {
                let notice: StateNotice = serde_json::from_value(params)?;
                if notice.session_id == self.session_id {
                    self.projection.set_running(notice.running);
                    self.changed();
                }
            }
            "approval.request" => {
                let notice: ApprovalNotice = serde_json::from_value(params)?;
                if notice.session_id == self.session_id {
                    self.projection.add_approval(
                        notice.request_id,
                        notice.call.name,
                        notice.call.input,
                        notice.reason,
                    );
                    self.changed();
                }
            }
            "approval.cancelled" => {
                let notice: CancelNotice = serde_json::from_value(params)?;
                self.projection.remove_approval(&notice.request_id);
                self.changed();
            }
            "session.mcp.state" => {
                let state: mcp::State = serde_json::from_value(params)?;
                if state.session_id == self.session_id {
                    let required: std::collections::HashSet<_> = state
                        .servers
                        .into_iter()
                        .filter(|server| server.state == "auth_required")
                        .map(|server| server.id)
                        .collect();
                    for id in required.difference(&self.mcp_required) {
                        self.send(Update::Notice(format!(
                            "MCP {id:?}: sign-in required. Run /mcp login {id:?}."
                        )));
                    }
                    self.mcp_required = required;
                }
            }
            "session.closed" => return Err("Session closed while attached".into()),
            _ => {}
        }
        Ok(())
    }

    fn report(&self, text: String, warning: bool) {
        self.send(Update::Report { text, warning });
    }

    /// `/mcp [list]`, `/mcp login [--no-browser] ID`, `/mcp callback URL`,
    /// `/mcp logout ID`.
    async fn mcp(&mut self, args: &str) -> crate::Result<()> {
        const USAGE: &str =
            "Usage: /mcp [list|login [--no-browser] ID|callback URL|logout ID]";
        let words: Vec<&str> = args.split_whitespace().collect();
        match words.as_slice() {
            [] | ["list"] => {
                let (rows, notes) = mcp::catalog_report(&mut self.client, &self.options).await?;
                let mut lines = rows;
                lines.extend(notes);
                lines.extend(mcp::session_report(&mut self.client, &self.session_id).await?);
                self.report(lines.join("\n"), false);
            }
            ["login", rest @ ..] => {
                let no_browser = rest.contains(&"--no-browser");
                if rest.contains(&"--client-secret") {
                    return Err("A client secret needs hidden input; run dimcode mcp login ID --client-secret in a terminal".into());
                }
                let ids: Vec<_> = rest.iter().filter(|word| !word.starts_with("--")).collect();
                let [id] = ids.as_slice() else {
                    return Err(USAGE.into());
                };
                self.mcp_login(id, no_browser).await?;
            }
            ["callback", url] => {
                let Some(attempt) = &self.mcp_attempt else {
                    return Err("No MCP sign-in is waiting; run /mcp login ID".into());
                };
                let attempt_id = attempt.attempt_id.clone();
                self.mcp_complete(attempt_id, url.to_string(), None).await;
            }
            ["logout", id] => {
                #[derive(Deserialize)]
                struct LoggedOut {
                    revoked: bool,
                }
                let params = mcp::target(&mut self.client, &self.options, id).await?;
                let result: LoggedOut = self.client.typed("mcp.auth.logout", params).await?;
                self.report(
                    format!("Signed out of {id:?}. Revoked: {}.", result.revoked),
                    false,
                );
            }
            _ => return Err(USAGE.into()),
        }
        Ok(())
    }

    async fn mcp_login(&mut self, id: &str, no_browser: bool) -> crate::Result<()> {
        self.end_mcp_attempt();
        let params = mcp::target(&mut self.client, &self.options, id).await?;
        let started: mcp::Started = self.client.typed("mcp.auth.start", params).await?;
        let issuer = url::Url::parse(&started.issuer)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        let deadline = tokio::time::Instant::now()
            + Duration::from_millis(started.expires_at.saturating_sub(now));
        let local_browser = !no_browser
            && std::env::var_os("SSH_CONNECTION").is_none()
            && std::env::var_os("SSH_TTY").is_none();
        let capture = if local_browser {
            mcp::Capture::bind(&started.redirect_uri).await
        } else {
            None
        };
        let mut lines = vec![
            format!("MCP server: {id:?}"),
            format!("Resource: {}", crate::display::safe(&started.resource)),
            format!(
                "Authorization server: {}",
                crate::display::safe(issuer.host_str().ok_or("Authorization server has no host")?)
            ),
            format!("Open: {}", crate::display::safe(&started.authorization_url)),
        ];
        let listener = capture.map(|capture| {
            let callbacks = self.mcp_callbacks.0.clone();
            let attempt_id = started.attempt_id.clone();
            tokio::spawn(async move {
                let callback = tokio::select! {
                    callback = capture.receive() => callback,
                    _ = tokio::time::sleep_until(deadline) => Err("Sign-in expired".into()),
                };
                let _ = callbacks.send(McpCallback {
                    attempt_id,
                    callback,
                });
            })
        });
        lines.push(if listener.is_some() {
            "Waiting for the browser callback; you can also paste it with /mcp callback URL.".into()
        } else {
            "Copy the full URL from the browser address bar and paste it with /mcp callback URL."
                .into()
        });
        if local_browser {
            crate::login::open_browser(&started.authorization_url);
        }
        self.report(lines.join("\n"), false);
        self.mcp_attempt = Some(McpAttempt {
            server_id: id.to_owned(),
            attempt_id: started.attempt_id,
            deadline,
            listener,
        });
        Ok(())
    }

    /// Forgets the sign-in, closing its listener.
    fn end_mcp_attempt(&mut self) {
        if let Some(listener) = self.mcp_attempt.take().and_then(|attempt| attempt.listener) {
            listener.abort();
        }
    }

    async fn mcp_callback(&mut self, callback: McpCallback) {
        if self
            .mcp_attempt
            .as_ref()
            .is_none_or(|attempt| attempt.attempt_id != callback.attempt_id)
        {
            return;
        }
        match callback.callback {
            Ok((url, stream)) => {
                self.mcp_complete(callback.attempt_id, url, Some(stream))
                    .await
            }
            Err(error) => {
                if let Some(attempt) = self.mcp_attempt.as_mut() {
                    attempt.listener = None;
                }
                self.report(
                    format!("{error}; paste the callback with /mcp callback URL, or run /mcp login ID again."),
                    true,
                );
            }
        }
    }

    async fn mcp_complete(
        &mut self,
        attempt_id: String,
        url: String,
        browser: Option<tokio::net::TcpStream>,
    ) {
        let attempt = self.mcp_attempt.as_ref().expect("MCP attempt");
        if tokio::time::Instant::now() >= attempt.deadline {
            let id = attempt.server_id.clone();
            self.end_mcp_attempt();
            self.report(
                format!("Sign-in expired. Run /mcp login {id:?} to start again."),
                true,
            );
            return;
        }
        let result = self
            .client
            .rpc(
                "mcp.auth.complete",
                json!({"attemptId": attempt_id, "callbackUrl": url}),
            )
            .await;
        if let Some(mut stream) = browser {
            use tokio::io::AsyncWriteExt;
            let page = if result.is_ok() {
                "Sign-in completed. You can close this window."
            } else {
                "Return to the terminal to continue sign-in."
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{page}",
                page.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
        let id = self
            .mcp_attempt
            .as_ref()
            .map(|attempt| attempt.server_id.clone())
            .unwrap_or_default();
        match result {
            Ok(_) => {
                self.end_mcp_attempt();
                self.report(format!("Signed in to {id:?}."), false);
            }
            Err(error)
                if error
                    .downcast_ref::<client::RpcError>()
                    .and_then(app_error)
                    == Some("mcp_auth_callback_invalid") =>
            {
                self.report(
                    "Callback did not match this attempt; paste it again with /mcp callback URL."
                        .into(),
                    true,
                );
            }
            Err(error) => {
                self.end_mcp_attempt();
                self.report(mcp::error(error).to_string(), true);
            }
        }
    }

    /// A reply to a request sent without waiting for it.
    fn reply(&mut self, id: u64, result: Result<serde_json::Value, client::RpcError>) {
        if let Some(index) = self.shell_stops.iter().position(|stop| *stop == id) {
            self.shell_stops.remove(index);
            if let Err(error) = result {
                self.send(Update::Notice(format!("Shell stop: {error}")));
            }
            return;
        }
        if self.shell.as_ref().is_some_and(|(request, _)| *request == id) {
            let (_, run_id) = self.shell.take().expect("shell run");
            let result = result
                .map_err(|error| match app_error(&error) {
                    // The wait ended, not necessarily the run.
                    Some("operation_interrupted") => {
                        format!("{error}; the command may still be running")
                    }
                    _ => error.to_string(),
                })
                .and_then(|value| {
                    serde_json::from_value(value).map_err(|_| "Invalid shell.run reply".to_owned())
                });
            self.send(Update::ShellDone { run_id, result });
        }
    }

    fn apply(&mut self, event: Event) {
        if event.sequence <= self.cursor {
            return;
        }
        self.cursor = event.sequence;
        if let EventContent::Configuration(configuration) = &event.content {
            self.model = Some(configuration.clone());
        }
        self.projection.apply(&event);
        self.changed();
    }

    /// Schedules a snapshot a frame interval after the first change.
    fn changed(&mut self) {
        self.publish_at
            .get_or_insert_with(|| tokio::time::Instant::now() + PUBLISH_INTERVAL);
    }

    fn publish(&mut self) {
        self.publish_at = None;
        self.send(Update::Snapshot(Box::new(self.projection.snapshot())));
    }

    fn send(&self, update: Update) {
        let _ = self.updates.send(update);
    }
}

fn app_error(error: &client::RpcError) -> Option<&str> {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("appError"))
        .and_then(serde_json::Value::as_str)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionSummary {
    session_id: String,
    cwd: String,
    model: ModelId,
    updated_at: u64,
    #[serde(default)]
    preview: Option<String>,
}
