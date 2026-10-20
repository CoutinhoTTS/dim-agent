//! Runs the App Server session for the interactive interface on a worker thread.
//!
//! The terminal loop is synchronous; the client is async. The worker owns the
//! connection and the projection, receives commands, and publishes view
//! snapshots: one a frame interval after the first change, however fast the
//! stream runs, so text arrives steadily and a burst of events costs one
//! snapshot. Snapshot rows are shared with the projection, not copied.
//!
//! Features the protocol v3 App Server does not offer yet (provider
//! management, Codex / Grok sign-in, usage, approval preferences) answer with
//! an explicit "not available" update instead of failing silently.

use std::sync::mpsc;
use std::time::Duration;

use dim_agent_types::{ApprovalDecision, ModelId};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::args::Options;
use crate::client::{self, Client, Message};
use crate::model::session::{
    Command, DraftPart, ModelItem, ModelList, OauthEvent, OauthStart, SessionItem, SessionPort,
    SwitchKind, Update,
};
use crate::projection::Projection;
use crate::wire::{Configuration, Event, EventContent, Progress};
use crate::{configure, mcp};

const PUBLISH_INTERVAL: Duration = Duration::from_millis(16);
const HISTORY_PAGE: u64 = 256;
const UNAVAILABLE: &str = "not available in this build yet";
const LOGIN_POLL: Duration = Duration::from_secs(1);
const NO_SESSION: &str = "Sign in with /login or choose a model with /models to start a session.";
const PLUGIN_MISSING: &str = "The DimAgent account plugin is not installed in this build";

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
    mcp_callbacks: (UnboundedSender<McpCallback>, UnboundedReceiver<McpCallback>),
    /// Servers already reported as needing sign-in.
    mcp_required: std::collections::HashSet<String>,
    /// The managed-provider sign-in of the `/login` panel.
    login: Option<Login>,
    /// Browser callbacks its local listener captured.
    login_callbacks: (UnboundedSender<McpCallback>, UnboundedReceiver<McpCallback>),
}

struct Login {
    /// The panel's attempt tag for its updates.
    tag: u64,
    provider_id: String,
    attempt_id: String,
    /// Device login: when its status is asked next.
    poll_at: Option<tokio::time::Instant>,
    /// Browser login: the local listener on the redirect address, when the
    /// port was free. A pasted callback works either way.
    listener: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Login {
    fn drop(&mut self) {
        if let Some(listener) = self.listener.take() {
            listener.abort();
        }
    }
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
        login: None,
        login_callbacks: unbounded_channel(),
    };
    if let Err(error) = worker.open().await {
        if worker.options.resume.is_some() {
            let _ = updates.send(Update::Failed(error.to_string()));
            return;
        }
        // A new session can still start once `/login` or `/models` gives
        // it a usable model; until then the interface stays up.
        let _ = updates.send(Update::Notice(format!("{error}. {NO_SESSION}")));
    }
    loop {
        let login_poll = worker.login.as_ref().and_then(|login| login.poll_at);
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
            Some(callback) = worker.login_callbacks.1.recv() => worker.login_callback(callback).await,
            _ = tokio::time::sleep_until(login_poll.unwrap_or_else(tokio::time::Instant::now)),
                if login_poll.is_some() => worker.poll_login().await,
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
        let accepts_images = self.accepts_images(&model).await;
        self.send(Update::Ready {
            provider: model.provider_id,
            model: model.model_id,
        });
        // Unknown capabilities leave images as they are: sent as images.
        self.send(Update::AcceptsImages(accepts_images.unwrap_or(true)));
        self.send(Update::Switched { session_id, kind });
        self.publish();
        Ok(())
    }

    /// Whether the model takes image input, from its catalog entry:
    /// `vision`, or `image` among its modalities. `None` when the server
    /// has no catalog or it does not answer in time.
    async fn accepts_images(&mut self, model: &ModelId) -> Option<bool> {
        if !self.client.offers("provider.catalog") {
            return None;
        }
        #[derive(Deserialize)]
        struct Snapshot {
            catalog: Option<Catalog>,
        }
        #[derive(Deserialize)]
        struct Catalog {
            models: Vec<CatalogModel>,
        }
        #[derive(Deserialize)]
        struct CatalogModel {
            id: String,
            capabilities: dim_agent_types::ModelCapabilities,
        }
        let request = self
            .client
            .typed::<Snapshot>("provider.catalog", json!({"providerId": model.provider_id}));
        let snapshot = tokio::time::timeout(Duration::from_secs(5), request)
            .await
            .ok()?
            .ok()?;
        let capabilities = snapshot
            .catalog?
            .models
            .into_iter()
            .find(|entry| entry.id == model.model_id)?
            .capabilities;
        Some(capabilities.vision || capabilities.modalities.iter().any(|m| m == "image"))
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
                    if value.len() <= crate::attachments::INLINE_PASTE {
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
                        Ok(stored) => text
                            .push_str(&format!("[Pasted text: {} ({lines} lines)]", stored.path)),
                        Err(error) => {
                            self.send(Update::Notice(format!("Paste {pastes}: {error}; not sent")));
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
        if self.session_id.is_empty()
            && matches!(command, Command::Submit(_) | Command::SubmitDraft(_))
        {
            self.send(Update::Notice(NO_SESSION.into()));
            return Ok(());
        }
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
                let models = self.models().await.map_err(|error| error.to_string());
                self.send(Update::Models(models));
            }
            Command::SetApprovals(_) => self.send(Update::Approvals(Err(format!(
                "Approval preferences are {UNAVAILABLE}"
            )))),
            Command::ListProviders => self.send(Update::Providers(Err(format!(
                "Providers are {UNAVAILABLE}"
            )))),
            Command::Provider(_) => self.send(Update::ProviderResult(Err(format!(
                "Providers are {UNAVAILABLE}"
            )))),
            // Codex and Grok keep their native sign-in, not offered here yet.
            Command::OauthStart(start)
                if !matches!(
                    start.provider_id.as_str(),
                    "openai-codex" | "xai-grok-build"
                ) =>
            {
                self.login_start(start).await
            }
            Command::OauthStart(start) => self.send(Update::Oauth {
                attempt: start.attempt,
                event: OauthEvent::Done(Err(Some(format!("Sign-in is {UNAVAILABLE}")))),
            }),
            Command::OauthCancel => self.login_cancel().await,
            Command::OauthPaste(url) => self.login_complete(url, None).await,
            Command::LoginStatus(provider_id) => {
                let account = self
                    .account(&provider_id)
                    .await
                    .map_err(|error| error.to_string());
                self.send(Update::Login(account));
            }
            Command::Logout(provider_id) => {
                let result = self
                    .client
                    .rpc("account.logout", json!({"providerId": provider_id}))
                    .await;
                self.send(Update::Notice(match result {
                    Ok(_) => "Signed out.".into(),
                    Err(error) => format!("Sign-out failed: {error}"),
                }));
            }
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
            Command::SelectModel {
                provider_id,
                model_id,
            } => {
                if let Err(error) = self.select_model(provider_id, model_id).await {
                    self.send(Update::Notice(format!("Model switch failed: {error}")));
                }
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
        const USAGE: &str = "Usage: /mcp [list|login [--no-browser] ID|callback URL|logout ID]";
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
            format!("Resource: {}", crate::sanitize::safe(&started.resource)),
            format!(
                "Authorization server: {}",
                crate::sanitize::safe(
                    issuer
                        .host_str()
                        .ok_or("Authorization server has no host")?
                )
            ),
            format!(
                "Open: {}",
                crate::sanitize::safe(&started.authorization_url)
            ),
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
            crate::browser::open(&started.authorization_url);
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
        if let Some(stream) = browser {
            answer_browser(stream, result.is_ok()).await;
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
                if error.downcast_ref::<client::RpcError>().and_then(app_error)
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

    /// The selectable models: every enabled static provider with
    /// credentials and every signed-in managed provider (`provider.list`
    /// lists static ones only), less unavailable and disabled models. A
    /// provider whose catalog cannot be read is left out.
    async fn models(&mut self) -> crate::Result<ModelList> {
        #[derive(Deserialize)]
        struct Providers {
            providers: Vec<Provider>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Provider {
            provider_id: String,
            display_name: String,
            enabled: bool,
            credential_configured: bool,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Managed {
            provider_id: String,
            display_name: String,
        }
        #[derive(Deserialize)]
        struct Account {
            account: Option<serde_json::Value>,
        }
        #[derive(Deserialize)]
        struct Snapshot {
            catalog: Option<Catalog>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Catalog {
            models: Vec<CatalogModel>,
            disabled_model_ids: Vec<String>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CatalogModel {
            id: String,
            name: String,
            unavailable_reason: Option<String>,
        }
        let listed: Providers = self.client.typed("provider.list", json!({})).await?;
        let mut usable: Vec<(String, String)> = listed
            .providers
            .into_iter()
            .filter(|provider| provider.enabled && provider.credential_configured)
            .map(|provider| (provider.provider_id, provider.display_name))
            .collect();
        // Without an account plugin there are no managed providers.
        let managed: Vec<Managed> = self
            .client
            .typed("account.providers", json!({}))
            .await
            .unwrap_or_default();
        for provider in managed {
            let status = self
                .client
                .typed::<Account>(
                    "account.status",
                    json!({"providerId": provider.provider_id}),
                )
                .await;
            if status.is_ok_and(|status| status.account.is_some()) {
                usable.push((provider.provider_id, provider.display_name));
            }
        }
        let mut models = Vec::new();
        for (provider_id, provider_name) in usable {
            let snapshot = self
                .client
                .typed::<Snapshot>("provider.catalog", json!({"providerId": provider_id}))
                .await;
            let Some(catalog) = snapshot.ok().and_then(|snapshot| snapshot.catalog) else {
                continue;
            };
            models.extend(
                catalog
                    .models
                    .into_iter()
                    .filter(|model| {
                        model.unavailable_reason.is_none()
                            && !catalog.disabled_model_ids.contains(&model.id)
                    })
                    .map(|model| ModelItem {
                        provider_id: provider_id.clone(),
                        provider_name: provider_name.clone(),
                        model_id: model.id,
                        display_name: model.name,
                    }),
            );
        }
        let current = self
            .model
            .as_ref()
            .map(|model| (model.provider_id.clone(), model.model_id.clone()));
        Ok(ModelList { models, current })
    }

    /// Switches the session's model, or starts a session on it when there is
    /// none yet, then makes it the default for new ones.
    async fn select_model(&mut self, provider_id: String, model_id: String) -> crate::Result<()> {
        #[derive(Deserialize)]
        struct Updated {
            model: ModelId,
        }
        let model = if self.session_id.is_empty() {
            let params = json!({
                "cwd": self.options.cwd,
                "model": {"provider": provider_id, "modelId": model_id},
            });
            let created: Created = self.client.typed("session.create", params).await?;
            self.enter(created.session_id, created.model.clone(), SwitchKind::New)
                .await?;
            created.model
        } else {
            let updated: Updated = self
                .client
                .typed(
                    "session.model.update",
                    json!({
                        "sessionId": self.session_id,
                        "branchId": "main",
                        "providerId": provider_id,
                        "modelId": model_id,
                    }),
                )
                .await?;
            let accepts_images = self.accepts_images(&updated.model).await;
            self.send(Update::Ready {
                provider: updated.model.provider_id.clone(),
                model: updated.model.model_id.clone(),
            });
            self.send(Update::AcceptsImages(accepts_images.unwrap_or(true)));
            updated.model
        };
        // The switch stands even when the default cannot be saved.
        if let Err(error) = self.make_default(&model).await {
            self.send(Update::Notice(format!(
                "The model was not saved as the default: {error}"
            )));
        }
        Ok(())
    }

    /// Saves the global default model over the selection read just before,
    /// as `provider.select` requires its current version.
    async fn make_default(&mut self, model: &ModelId) -> crate::Result<()> {
        #[derive(Deserialize)]
        struct Current {
            selection: Option<Selection>,
        }
        #[derive(Deserialize)]
        struct Selection {
            version: u32,
        }
        let current: Current = self.client.typed("provider.selection", json!({})).await?;
        let mut params = json!({"providerId": model.provider_id, "modelId": model.model_id});
        // The first default has no version; the field takes no null.
        if let Some(selection) = current.selection {
            params["expectedVersion"] = json!(selection.version);
        }
        self.client.rpc("provider.select", params).await?;
        Ok(())
    }

    /// The signed-in subject of a managed provider, or none when signed out.
    async fn account(&mut self, provider_id: &str) -> crate::Result<Option<String>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Provider {
            provider_id: String,
        }
        #[derive(Deserialize)]
        struct Snapshot {
            account: Option<Identity>,
        }
        #[derive(Deserialize)]
        struct Identity {
            subject: String,
        }
        let providers: Vec<Provider> = match self.client.typed("account.providers", json!({})).await
        {
            Err(error)
                if error.downcast_ref::<client::RpcError>().and_then(app_error)
                    == Some("unknown_method") =>
            {
                return Err(PLUGIN_MISSING.into());
            }
            result => result?,
        };
        if !providers
            .iter()
            .any(|provider| provider.provider_id == provider_id)
        {
            return Err(PLUGIN_MISSING.into());
        }
        let snapshot: Snapshot = self
            .client
            .typed("account.status", json!({"providerId": provider_id}))
            .await?;
        Ok(snapshot
            .account
            .map(|account| crate::sanitize::safe(&account.subject)))
    }

    fn login_event(&self, tag: u64, event: OauthEvent) {
        self.send(Update::Oauth {
            attempt: tag,
            event,
        });
    }

    /// Begins a browser or device sign-in, replacing one still pending.
    async fn login_start(&mut self, start: OauthStart) {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Browser {
            attempt_id: String,
            authorization_url: String,
            expires_at: u64,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Device {
            attempt_id: String,
            verification_uri: String,
            verification_uri_complete: Option<String>,
            user_code: String,
        }
        self.login_cancel().await;
        let mut params = json!({
            "providerId": start.provider_id,
            "directory": crate::login::account_directory(),
        });
        if let Some(email) = &start.email {
            params["email"] = json!(email);
        }
        let mut listener = None;
        let attempt_id = if start.device {
            match self
                .client
                .typed::<Device>("account.login.begin", params)
                .await
            {
                Ok(device) => {
                    self.login_event(
                        start.attempt,
                        OauthEvent::Device {
                            verification_uri: crate::sanitize::safe(&device.verification_uri),
                            verification_uri_complete: device
                                .verification_uri_complete
                                .map(|uri| crate::sanitize::safe(&uri)),
                            user_code: crate::sanitize::safe(&device.user_code),
                        },
                    );
                    Ok(device.attempt_id)
                }
                Err(error) => Err(error),
            }
        } else {
            params["redirectUri"] = json!(self.options.redirect_uri);
            match self
                .client
                .typed::<Browser>("account.login.browser.begin", params)
                .await
            {
                Ok(browser) => {
                    let local = std::env::var_os("SSH_CONNECTION").is_none()
                        && std::env::var_os("SSH_TTY").is_none();
                    let capture = if local {
                        mcp::Capture::bind(&self.options.redirect_uri).await
                    } else {
                        None
                    };
                    listener = capture.map(|capture| {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |elapsed| elapsed.as_millis() as u64);
                        let deadline = tokio::time::Instant::now()
                            + Duration::from_millis(browser.expires_at.saturating_sub(now));
                        let callbacks = self.login_callbacks.0.clone();
                        let attempt_id = browser.attempt_id.clone();
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
                    let opened = local && crate::browser::open(&browser.authorization_url);
                    self.login_event(
                        start.attempt,
                        OauthEvent::Url {
                            url: crate::sanitize::safe(&browser.authorization_url),
                            opened,
                        },
                    );
                    Ok(browser.attempt_id)
                }
                Err(error) => Err(error),
            }
        };
        match attempt_id {
            Ok(attempt_id) => {
                self.login = Some(Login {
                    tag: start.attempt,
                    provider_id: start.provider_id,
                    attempt_id,
                    poll_at: start
                        .device
                        .then(|| tokio::time::Instant::now() + LOGIN_POLL),
                    listener,
                });
            }
            Err(error) => self.login_event(
                start.attempt,
                OauthEvent::Done(Err(Some(error.to_string()))),
            ),
        }
    }

    /// Abandons the pending sign-in. An exchange the plugin already accepted
    /// stays its own.
    async fn login_cancel(&mut self) {
        if let Some(login) = self.login.take() {
            let _ = self
                .client
                .rpc(
                    "account.login.cancel",
                    json!({"providerId": login.provider_id, "attemptId": login.attempt_id}),
                )
                .await;
        }
    }

    /// The browser reached the local listener: complete with its callback.
    /// A failed listener leaves the pasted callback to finish sign-in.
    async fn login_callback(&mut self, callback: McpCallback) {
        let Some(login) = self.login.as_mut() else {
            return;
        };
        if login.attempt_id != callback.attempt_id {
            return;
        }
        match callback.callback {
            Ok((url, stream)) => self.login_complete(url, Some(stream)).await,
            Err(_) => login.listener = None,
        }
    }

    /// Completes a browser sign-in with its callback URL, pasted or caught
    /// by the listener (whose browser tab is then answered).
    async fn login_complete(&mut self, callback: String, browser: Option<tokio::net::TcpStream>) {
        let Some(login) = self.login.take() else {
            return;
        };
        self.login_event(login.tag, OauthEvent::Exchanging);
        let result = self
            .client
            .rpc(
                "account.login.browser.complete",
                json!({
                    "providerId": login.provider_id,
                    "attemptId": login.attempt_id,
                    "callbackUrl": callback,
                }),
            )
            .await;
        if let Some(stream) = browser {
            answer_browser(stream, result.is_ok()).await;
        }
        match result {
            Ok(_) => self.login_done(login).await,
            Err(error) => {
                self.login_event(login.tag, OauthEvent::Done(Err(Some(error.to_string()))))
            }
        }
    }

    /// Asks how the device sign-in is going.
    async fn poll_login(&mut self) {
        #[derive(Deserialize)]
        #[serde(tag = "status", rename_all = "lowercase")]
        enum State {
            Pending,
            Completed,
            Failed { error: String },
            Cancelled,
            Expired,
        }
        let Some(login) = self.login.take() else {
            return;
        };
        let status = self
            .client
            .typed::<State>(
                "account.login.status",
                json!({"providerId": login.provider_id, "attemptId": login.attempt_id}),
            )
            .await;
        let failed = |error: String| OauthEvent::Done(Err(Some(error)));
        match status {
            Ok(State::Pending) => {
                let mut login = login;
                login.poll_at = Some(tokio::time::Instant::now() + LOGIN_POLL);
                self.login = Some(login);
            }
            Ok(State::Completed) => self.login_done(login).await,
            Ok(State::Failed { error }) => self.login_event(
                login.tag,
                failed(format!("Login failed: {}", crate::sanitize::safe(&error))),
            ),
            Ok(State::Cancelled) => self.login_event(login.tag, OauthEvent::Done(Err(None))),
            Ok(State::Expired) => {
                self.login_event(login.tag, failed("Login expired; start login again".into()))
            }
            Err(error) => self.login_event(login.tag, failed(error.to_string())),
        }
    }

    /// Refreshes the provider's catalog, so `/models` lists its models at
    /// once.
    async fn login_done(&mut self, login: Login) {
        if let Err(error) = self
            .client
            .rpc("provider.refresh", json!({"providerId": login.provider_id}))
            .await
        {
            self.send(Update::Notice(format!(
                "Model catalog refresh failed: {error}"
            )));
        }
        self.login_event(login.tag, OauthEvent::Done(Ok(login.provider_id.clone())));
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
        if self
            .shell
            .as_ref()
            .is_some_and(|(request, _)| *request == id)
        {
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

/// Answers the browser tab the local listener caught a callback from.
async fn answer_browser(mut stream: tokio::net::TcpStream, completed: bool) {
    use tokio::io::AsyncWriteExt;
    let page = if completed {
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
