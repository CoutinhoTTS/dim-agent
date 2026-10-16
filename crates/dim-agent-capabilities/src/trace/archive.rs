//! Host-owned JSONL archive with explicit sensitive-content opt-in. Never used for recovery.
use dim_agent_runtime::{DiagnosticEvent, DiagnosticRecord, diagnostics_channel};
use dim_agent_types::{
    BranchCreateFailedData, BranchSource, EventKind, ModelOutputOutcome, TurnOutcome,
};
use rutis::{BoxFuture, CordisError, Ctx, Disposer, Listener};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt, BufWriter},
    sync::{mpsc, watch},
    task::JoinHandle,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveContent {
    MetadataOnly,
    /// May include secrets in free text. Only enable for trusted local inspection.
    SensitiveContent,
}
impl ArchiveContent {
    fn version(self) -> u8 {
        match self {
            Self::MetadataOnly => 1,
            Self::SensitiveContent => 2,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ArchiveStatus {
    pub records: u64,
    pub incomplete: bool,
}

struct ArchiveSink {
    sender: Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
    incomplete: Arc<AtomicBool>,
    content: ArchiveContent,
    sessions: Mutex<Sessions>,
    /// Number of listened Sessions whose `Closed` record has not arrived.
    open: watch::Sender<usize>,
}
#[derive(Default)]
struct Sessions {
    listeners: HashMap<String, Option<Disposer>>,
    /// A failed Session stops delivering; its archive is already incomplete.
    failed: HashSet<String>,
}
impl ArchiveSink {
    /// Enqueues without I/O or waiting: the listener runs on the rutis dispatch chain.
    /// Stops waiting for a Session. Without its end record, the archive
    /// may have missed records and is incomplete.
    fn end(&self, session_id: &str, ended: bool) {
        let listener = self.sessions.lock().unwrap().listeners.remove(session_id);
        if let Some(listener) = listener {
            if let Some(disposer) = listener {
                tokio::spawn(disposer.dispose());
            }
            if !ended {
                self.incomplete();
            }
            self.open.send_modify(|open| *open -= 1);
        }
    }
    fn deliver(self: &Arc<Self>, record: &DiagnosticRecord) {
        if matches!(record.event, DiagnosticEvent::Closed) {
            self.end(&record.session_id, true);
            return;
        }
        if self
            .sessions
            .lock()
            .unwrap()
            .failed
            .contains(&record.session_id)
        {
            return;
        }
        if self.record(record.clone()).is_err() {
            self.incomplete();
            self.sessions
                .lock()
                .unwrap()
                .failed
                .insert(record.session_id.clone());
        }
    }
    fn record(&self, record: DiagnosticRecord) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(&redact(record, self.content))?;
        bytes.push(b'\n');
        let sender = self.sender.lock().unwrap();
        let result = sender
            .as_ref()
            .ok_or_else(|| io::Error::other("trace archive closed"))
            .and_then(|sender| {
                sender
                    .send(bytes)
                    .map_err(|_| io::Error::other("trace archive writer stopped"))
            });
        if result.is_err() {
            self.incomplete();
        }
        result
    }
    fn incomplete(&self) {
        self.incomplete.store(true, Ordering::Relaxed);
    }
}

struct ArchiveListener {
    sink: Arc<ArchiveSink>,
    session_id: String,
}
impl Listener<DiagnosticRecord> for ArchiveListener {
    fn call<'a>(
        &'a self,
        _ctx: &'a Ctx,
        record: &'a DiagnosticRecord,
    ) -> BoxFuture<'a, Result<Option<()>, CordisError>> {
        self.sink.deliver(record);
        Box::pin(async { Ok(None) })
    }
}
/// A listener removed before its end record, e.g. because the host context
/// unloaded first, still releases `finish` and marks the archive incomplete.
impl Drop for ArchiveListener {
    fn drop(&mut self) {
        self.sink.end(&self.session_id, false);
    }
}

/// Registers Session listeners for one archive; handed to the Trace capability.
#[derive(Clone)]
pub struct TraceRecorder(Arc<ArchiveSink>);
impl TraceRecorder {
    /// Listens on the Session's diagnostics channel until its `Closed` record.
    /// `host` must outlive the Session so closing records still arrive.
    /// A rejected registration drops its listener, which ends the wait.
    pub fn listen(&self, host: &Ctx, session_id: &str) -> Result<(), CordisError> {
        {
            let mut sessions = self.0.sessions.lock().unwrap();
            if sessions.listeners.contains_key(session_id) {
                return Err(CordisError::PluginFailed(
                    format!("Session {session_id} is already traced").into(),
                ));
            }
            sessions.listeners.insert(session_id.to_owned(), None);
        }
        self.0.open.send_modify(|open| *open += 1);
        let disposer = host.events().on(
            host,
            &diagnostics_channel(session_id),
            ArchiveListener {
                sink: self.0.clone(),
                session_id: session_id.to_owned(),
            },
        )?;
        if let Some(slot) = self
            .0
            .sessions
            .lock()
            .unwrap()
            .listeners
            .get_mut(session_id)
        {
            *slot = Some(disposer);
        }
        Ok(())
    }
}

/// The host must finish Agent shutdown before finishing the archive.
/// Dropping this owner closes admission and lets the writer drain as incomplete.
pub struct TraceArchive {
    sink: Arc<ArchiveSink>,
    worker: Option<JoinHandle<io::Result<ArchiveStatus>>>,
}
impl TraceArchive {
    /// Creates a new private directory and trace.jsonl; never overwrites an archive.
    pub async fn create(directory: &Path) -> io::Result<Self> {
        Self::create_with_content(directory, ArchiveContent::MetadataOnly).await
    }
    pub async fn create_with_content(
        directory: &Path,
        content: ArchiveContent,
    ) -> io::Result<Self> {
        let directory_options = &mut tokio::fs::DirBuilder::new();
        #[cfg(unix)]
        directory_options.mode(0o700);
        directory_options.create(directory).await?;
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(directory.join("trace.jsonl")).await?;
        Ok(Self::with_content(file, content))
    }
    /// Supplies host storage without changing the runtime or requiring a global subscriber.
    pub fn new(writer: impl AsyncWrite + Unpin + Send + 'static) -> Self {
        Self::with_content(writer, ArchiveContent::MetadataOnly)
    }
    pub fn with_content(
        writer: impl AsyncWrite + Unpin + Send + 'static,
        content: ArchiveContent,
    ) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<Vec<u8>>();
        let incomplete = Arc::new(AtomicBool::new(false));
        let sink = Arc::new(ArchiveSink {
            sender: Mutex::new(Some(sender)),
            incomplete: incomplete.clone(),
            content,
            sessions: Mutex::default(),
            open: watch::channel(0).0,
        });
        let worker = tokio::spawn(async move {
            let result = async {
                let mut writer = BufWriter::new(writer);
                let mut header = serde_json::to_vec(&json!({"version":content.version(),"kind":"archive_start","content":if content == ArchiveContent::MetadataOnly { "metadata_only" } else { "sensitive_content" }}))?;
                header.push(b'\n');
                writer.write_all(&header).await?;
                let mut records = 0;
                while let Some(bytes) = receiver.recv().await {
                    writer.write_all(&bytes).await?;
                    records += 1;
                }
                let status = ArchiveStatus { records, incomplete: incomplete.load(Ordering::Relaxed) };
                let mut footer = serde_json::to_vec(&json!({"version":content.version(),"kind":"archive_end","records":records,"incomplete":status.incomplete}))?;
                footer.push(b'\n');
                writer.write_all(&footer).await?;
                writer.flush().await?;
                writer.shutdown().await?;
                Ok(status)
            }.await;
            if result.is_err() {
                incomplete.store(true, Ordering::Relaxed);
            }
            result
        });
        Self {
            sink,
            worker: Some(worker),
        }
    }
    pub fn recorder(&self) -> TraceRecorder {
        TraceRecorder(self.sink.clone())
    }
    pub fn is_incomplete(&self) -> bool {
        self.sink.incomplete.load(Ordering::Relaxed)
    }
    /// Waits until every listened Session has delivered its `Closed` record.
    pub async fn closed(&self) {
        let _ = self.sink.open.subscribe().wait_for(|open| *open == 0).await;
    }
    /// Waits for [`Self::closed`], then drains. Cancelling the waiter closes
    /// admission without aborting the already-owned writer task.
    pub async fn finish(mut self) -> io::Result<ArchiveStatus> {
        let worker = self.worker.take().expect("archive writer owned");
        self.closed().await;
        self.sink.sender.lock().unwrap().take();
        worker.await.map_err(io::Error::other)?
    }
}
impl Drop for TraceArchive {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.sink.incomplete();
        }
        self.sink.sender.lock().unwrap().take();
    }
}

fn redact(record: DiagnosticRecord, content: ArchiveContent) -> Value {
    let payload = match &record.event {
        DiagnosticEvent::Timeline { event, trace }
            if content == ArchiveContent::SensitiveContent =>
        {
            let mut value = json!({"event":event,"before":trace});
            mask_credentials(&mut value);
            Some(value)
        }
        _ => None,
    };
    let event = match record.event {
        DiagnosticEvent::Timeline { event, trace } => json!({
            "kind":"timeline", "event_id":event.id,"sequence":event.sequence,"create_at":event.create_at,
            "event": summary(&event.kind),
            "before": trace.map(|trace| json!({"original":summary(&trace.original.kind),"steps":trace.steps.iter().map(|step|
                json!({"handler":step.handler,"result":step.result.as_ref().map(|draft| summary(&draft.kind))})).collect::<Vec<_>>()})),
        }),
        DiagnosticEvent::Span {
            id,
            phase,
            mut fields,
            duration_ms,
        } => {
            fields.retain(|key, _| {
                matches!(
                    key.as_str(),
                    "operation"
                        | "branch_id"
                        | "turn_id"
                        | "request_id"
                        | "call_id"
                        | "prompt_version"
                        | "outcome"
                )
            });
            json!({"kind":"span","id":id.to_string(),"phase":phase,"fields":fields,"duration_ms":duration_ms})
        }
        DiagnosticEvent::Component { name, phase } => {
            json!({"kind":"component","name":name,"phase":phase})
        }
        DiagnosticEvent::Subscription {
            name,
            id,
            keys,
            phase,
        } => json!({"kind":"subscription","name":name,"id":id,"keys":keys,"phase":phase}),
        DiagnosticEvent::Closed => json!({"kind":"closed"}),
    };
    let mut value = json!({"version":content.version(),"session_id":record.session_id,"branch_id":record.branch_id,"timestamp":record.timestamp,"record":event});
    if let Some(payload) = payload {
        value["record"]["payload"] = payload;
    }
    value
}

/// Defense in depth only: free text remains sensitive, including embedded JSON and URLs.
fn mask_credentials(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(mask_credentials),
        Value::Object(fields) => {
            for (key, value) in fields {
                let normalized = key.replace(['_', '-'], "").to_ascii_lowercase();
                if matches!(
                    normalized.as_str(),
                    "headers"
                        | "customheaders"
                        | "authorization"
                        | "proxyauthorization"
                        | "apikey"
                        | "xapikey"
                        | "token"
                        | "accesstoken"
                        | "refreshtoken"
                        | "sessiontoken"
                        | "password"
                        | "passwd"
                        | "secret"
                        | "clientsecret"
                        | "secretaccesskey"
                        | "cookie"
                        | "setcookie"
                        | "credentials"
                ) {
                    *value = json!("[REDACTED]");
                } else {
                    mask_credentials(value);
                }
            }
        }
        _ => {}
    }
}

/// Explicit projection: free text, credentials, headers, tool input/output,
/// request config/metadata, paths and URLs never enter the archive.
fn summary(event: &EventKind) -> Value {
    let (turn, request, call) = match event {
        EventKind::TurnStart(d) => (Some(&d.turn_id), None, None),
        EventKind::TurnEnd(d) => (Some(&d.turn_id), None, None),
        EventKind::ModelStart(d) => (Some(&d.turn_id), Some(&d.request_id), None),
        EventKind::Thinking(d) | EventKind::Text(d) => {
            (Some(&d.turn_id), Some(&d.request_id), None)
        }
        EventKind::ContextPrepare(d) => (Some(&d.turn_id), Some(&d.request_id), None),
        EventKind::ContextCompactionStart(d) => (d.turn_id.as_ref(), d.request_id.as_ref(), None),
        EventKind::ContextCompaction(d) => (d.turn_id.as_ref(), d.request_id.as_ref(), None),
        EventKind::ContextBudget(d) => (Some(&d.turn_id), Some(&d.request_id), None),
        EventKind::ModelCall(d) => (Some(&d.turn_id), Some(&d.request_id), None),
        EventKind::ModelUsage(d) => (Some(&d.turn_id), Some(&d.request_id), None),
        EventKind::ModelOutputEnd(d) => (Some(&d.turn_id), Some(&d.request_id), None),
        EventKind::ToolCall(d) => (Some(&d.turn_id), Some(&d.request_id), None),
        EventKind::ToolResult(d) => (Some(&d.turn_id), Some(&d.request_id), None),
        EventKind::ToolUse(d) => (Some(&d.turn_id), Some(&d.request_id), Some(&d.call.id)),
        EventKind::ToolUseStart(d) => (Some(&d.turn_id), Some(&d.request_id), Some(&d.call_id)),
        EventKind::ToolStart(d) => (Some(&d.turn_id), Some(&d.request_id), Some(&d.call_id)),
        EventKind::ToolCompleted(d) => (Some(&d.turn_id), Some(&d.request_id), Some(&d.call_id)),
        EventKind::BranchCreate(d) => match &d.source {
            BranchSource::Tool {
                turn_id,
                request_id,
                call_id,
            } => (Some(turn_id), Some(request_id), Some(call_id)),
            BranchSource::Host { .. } => (None, None, None),
        },
        EventKind::BranchCreateFailed(_) => (None, None, None),
        EventKind::ApprovalRequest(d) => match &d.source {
            BranchSource::Tool {
                turn_id,
                request_id,
                call_id,
            } => (Some(turn_id), Some(request_id), Some(call_id)),
            BranchSource::Host { .. } => (None, None, None),
        },
        _ => (None, None, None),
    };
    let mut value = json!({"key":event.key(),"turn_id":turn,"request_id":request,"call_id":call});
    match event {
        EventKind::PromptChanged(d) => {
            value["timeline"] = json!(d.timeline);
            value["revision"] = json!(d.revision);
            value["previous"] = json!(d.previous);
            value["reason"] = json!(d.reason);
            value["changes"] = json!(d.changes.iter().map(|change| match change {
                dim_agent_types::PromptChange::Set { entry } => json!({
                    "op":"set", "version":entry.version, "scope":entry.scope,
                    "delivery":entry.delivery, "order":entry.order,
                }),
                dim_agent_types::PromptChange::Revoke { scope, .. } => {
                    json!({"op":"revoke", "scope":scope})
                }
            }).collect::<Vec<_>>());
        }
        EventKind::ContextConfigured(d) => {
            value["timeline"] = json!(d.timeline);
            value["revision"] = json!(d.revision);
            value["previous"] = json!(d.previous);
            value["prompt"] = json!(d.prompt);
            value["provider_id"] = json!(d.settings.provider_id);
            value["model_id"] = json!(d.settings.model_id);
        }
        EventKind::ContextScope(d) => {
            value["scope"] = json!(d.scope);
            value["status"] = json!(d.status);
        }
        EventKind::ContextPrepare(d) => {
            value["configuration"] = json!(d.configuration);
            value["prompt"] = json!(d.prompt);
            value["through"] = json!(d.through);
            value["model_id"] = json!(d.request.model_id);
            value["provider_id"] = json!(d.request.provider_id);
        }
        EventKind::ModelCall(d) => {
            value["model_id"] = json!(d.request.model_id);
            value["provider_id"] = json!(d.request.provider_id);
            value["configuration"] = json!(d.configuration);
            value["prompt"] = json!(d.prompt);
        }
        EventKind::ModelOutputEnd(d) => value["outcome"] = json!(match d.outcome {
            ModelOutputOutcome::Completed { .. } => "completed", ModelOutputOutcome::Cancelled { .. } => "cancelled", ModelOutputOutcome::Failed { .. } => "failed",
        }),
        EventKind::TurnEnd(d) => value["outcome"] = json!(match d.outcome {
            TurnOutcome::Completed { .. } => "completed", TurnOutcome::Cancelled { .. } => "cancelled", TurnOutcome::Failed { .. } => "failed",
        }),
        EventKind::ToolCall(d) => value["calls"] = json!(d.calls.iter().map(|call| json!({"id":call.id,"name":call.name})).collect::<Vec<_>>()),
        EventKind::ToolResult(d) => value["results"] = json!(d.results.iter().map(|call| json!({"call_id":call.call_id,"is_error":call.result.is_error,"blob_ids":call.result.blob_refs.as_ref().map(|refs| refs.iter().map(|blob| &blob.blob_id).collect::<Vec<_>>())})).collect::<Vec<_>>()),
        EventKind::BranchStart(d) => { value["parent"] = json!(d.parent); value["create_event"] = json!(d.create_event); }
        EventKind::BranchCreate(d) => {
            value["branch"] = json!(d.branch);
            value["source"] = json!(d.source);
            value["at"] = json!(d.at);
        }
        EventKind::BranchCreateFailed(d) => {
            let (branch, create_event, outcome) = match d {
                BranchCreateFailedData::Failed { branch, create_event, .. } => (branch, create_event, "failed"),
                BranchCreateFailedData::Cancelled { branch, create_event, .. } => (branch, create_event, "cancelled"),
            };
            value["branch"] = json!(branch);
            value["create_event"] = json!(create_event);
            value["outcome"] = json!(outcome);
        }
        EventKind::ApprovalRequest(d) => value["source"] = json!(d.source),
        EventKind::InputApplied(d) => value["input_id"] = json!(d.input_id),
        EventKind::InputConsumed(d) => value["input_id"] = json!(d.input_id),
        EventKind::StopInterrupt(d) => value["scope"] = json!(d.scope),
        _ => {}
    }
    value
}
