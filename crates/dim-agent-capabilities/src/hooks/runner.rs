//! Hooks' parts: the command runner (Hooks A.3–A.5) and the six hook points
//! of §3.2 and §3.4.
//!
//! `HooksSession` reads the sources and runs commands; `HooksBranch` folds the
//! facts it consumes, decides each point and writes its inputs. A command
//! never runs in a Dispatcher handler or under the commit lock: the handler
//! advances the consumed view, starts the work and returns.
use crate::{
    approval::Approval,
    branch_input::append_own,
    context_block::{element, raw},
    hooks::{HookEvent, HookHandler, HookSourcesHost, HookSourcesSnapshot, OnFailure, handlers},
    host_platform::{HostPlatform, Os},
};
use dim_agent_runtime::{
    BranchLifecycle, Delivery, Dispatcher, Names, SessionConfig, SessionIdentity, Timeline,
    TimelineIdentity,
};
use dim_agent_types::{
    AgentEvent, ApprovalCancelledData, ApprovalDecision, ApprovalRequestData, ApprovalRespondData,
    BranchSource, EventKind, InputBlock, InputMessageData, InputMode, JsonObject, PermissionPreset,
    ToolCallResult, ToolContent, ToolDefinition, TurnOutcome,
};
use rutis::Ctx;
use rutis::host::{BoxFuture, CtxHost, DynPlugin, Effect, Key, TypeKey};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::mpsc,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// A hook's stdout is bounded; more than this is a failure (A.3).
const STDOUT_LIMIT: usize = 64 * 1024;
/// A blocking result's reason is the first 2 KiB of stderr (A.3).
const STDERR_LIMIT: usize = 2 * 1024;
/// Each hook's context is bounded (A.4).
const CONTEXT_LIMIT: usize = 16 * 1024;
/// Consecutive `Stop` follow-ups (A8).
const STOP_FOLLOW_UP_LIMIT: u32 = 3;
/// The root branch, where `SessionStart` is derived.
const MAIN_BRANCH: &str = "main";
/// The key prefix of a `SessionStart` block, which tells a started Session
/// from one that has not run it.
const SESSION_START_KEY: &str = "hooks:session-start/";
/// Variables the hook environment sets and never inherits.
const MANAGED_ENV: [&str; 6] = [
    "DIM_PROJECT_DIR",
    "DIM_HOOK_EVENT",
    "DIM_SESSION_ID",
    "DIMCODE_HOME",
    "CLAUDE_PLUGIN_ROOT",
    "GROK_PLUGIN_ROOT",
];

/// What one command produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookResult {
    /// Exit 0, with its bounded stdout.
    Success { stdout: String },
    /// Exit 2: the handler blocked, and this is the bounded stderr.
    Blocked { reason: String },
    /// Any other exit, a signal, a timeout, a cancellation or an oversized
    /// stdout.
    Failed {
        reason: String,
        /// The first [`STDERR_LIMIT`] bytes of its standard error, which the
        /// diagnostic carries (§7). Never standard output or the payload.
        stderr: String,
    },
}
/// One handler's run, at its catalog position.
#[derive(Clone, Debug)]
pub struct HookRun {
    pub result: HookResult,
    /// What the handler asks for when its command fails, times out or is
    /// cancelled (A.5).
    pub on_failure: OnFailure,
}

/// The Session service that reads the sources and runs commands.
pub struct HooksRuntime {
    sources: Arc<HookSourcesHost>,
    platform: Arc<HostPlatform>,
    config: Arc<SessionConfig>,
    identity: Arc<SessionIdentity>,
    cancel: CancellationToken,
    tasks: TaskTracker,
}

impl HooksRuntime {
    /// Every source for this Session's working directory. A failed read is a
    /// diagnostic, and the caller runs no hooks (appendix C).
    async fn snapshot(&self) -> Option<HookSourcesSnapshot> {
        match self
            .sources
            .read(Path::new(&self.config.cwd), &self.cancel)
            .await
        {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                tracing::warn!(%error, "hook sources could not be read");
                None
            }
        }
    }

    /// The common fields of every payload (A.3).
    fn payload(&self, event: HookEvent, branch: &str) -> JsonObject {
        let preset = match self.config.permission_policy.preset {
            PermissionPreset::ReadOnly => "read-only",
            PermissionPreset::WorkspaceWrite => "workspace-write",
            PermissionPreset::FullAccess => "full-access",
        };
        json!({
            "hook_event_name": event.name(),
            "session_id": self.identity.session_id,
            "cwd": self.config.cwd,
            "permission_mode": preset,
            "branch_id": branch,
        })
        .as_object()
        .cloned()
        .unwrap_or_default()
    }

    /// Runs `handlers` concurrently and combines their results in catalog
    /// order. An async handler is started and not waited for.
    async fn run(
        self: &Arc<Self>,
        handlers: &[HookHandler],
        payload: &JsonObject,
        cancel: &CancellationToken,
    ) -> Vec<HookRun> {
        let mut waiting = Vec::new();
        for handler in handlers {
            let mut fields = payload.clone();
            fields.insert(
                "dim_source".into(),
                Value::String(handler.source.as_str().into()),
            );
            let stdin = serde_json::to_vec(&Value::Object(fields)).unwrap_or_default();
            if handler.asynchronous {
                self.spawn_async(handler.clone(), stdin, cancel.clone());
                continue;
            }
            let handler = handler.clone();
            waiting.push(async move {
                HookRun {
                    result: self.run_one(&handler, stdin, cancel).await,
                    on_failure: handler.on_failure,
                }
            });
        }
        futures_util::future::join_all(waiting).await
    }

    fn spawn_async(
        self: &Arc<Self>,
        handler: HookHandler,
        stdin: Vec<u8>,
        cancel: CancellationToken,
    ) {
        let owner = self.clone();
        let generation = self.cancel.clone();
        self.tasks.spawn(async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {}
                _ = generation.cancelled() => {}
                _ = owner.run_one(&handler, stdin, &generation) => {}
            }
        });
    }

    async fn run_one(
        &self,
        handler: &HookHandler,
        stdin: Vec<u8>,
        cancel: &CancellationToken,
    ) -> HookResult {
        let command = handler.command(self.platform.os == Os::Windows);
        let started = Instant::now();
        let mut spec = match self
            .platform
            .build_process_spec(command, Path::new(&self.config.cwd))
            .await
        {
            Ok((spec, _)) => spec,
            Err(error) => {
                let result = failed(format!("{}: {}", error.code, error.reason), &[]);
                self.report(handler, &result, started.elapsed());
                return result;
            }
        };
        spec.env = hook_env(spec.env, &self.environment(handler));
        let mut tree = match self.platform.spawn_with_stdin(spec, stdin) {
            Ok(tree) => tree,
            Err(error) => {
                let result = failed(format!("spawn failed: {error}"), &[]);
                self.report(handler, &result, started.elapsed());
                return result;
            }
        };
        let mut stdout = tree.stdout();
        let mut stderr = tree.stderr();
        enum End {
            Done(
                (Vec<u8>, bool),
                (Vec<u8>, bool),
                std::io::Result<std::process::ExitStatus>,
            ),
            Cancelled,
            TimedOut,
        }
        let end = tokio::select! {
            biased;
            _ = cancel.cancelled() => End::Cancelled,
            _ = tokio::time::sleep(handler.timeout) => End::TimedOut,
            (out, err, status) = async {
                tokio::join!(
                    read_capped(&mut stdout, STDOUT_LIMIT),
                    read_capped(&mut stderr, STDERR_LIMIT),
                    tree.wait(),
                )
            } => End::Done(out, err, status),
        };
        let result = match end {
            End::Cancelled => {
                let _ = tree.terminate().await;
                failed("cancelled", &[])
            }
            End::TimedOut => {
                let _ = tree.terminate().await;
                failed(
                    format!("timed out after {}s", handler.timeout.as_secs()),
                    &[],
                )
            }
            End::Done((bytes, overflow), (stderr, _), status) => {
                let status = match status {
                    Ok(status) => status,
                    // A run whose end cannot be read still reports (§7).
                    Err(error) => {
                        let result = failed(format!("wait failed: {error}"), &stderr);
                        self.report(handler, &result, started.elapsed());
                        return result;
                    }
                };
                if overflow {
                    failed("stdout exceeded 64 KiB", &stderr)
                } else if status.code() == Some(2) {
                    exit_two(handler.event, &stderr)
                } else if status.success() {
                    HookResult::Success {
                        stdout: String::from_utf8_lossy(&bytes).into_owned(),
                    }
                } else {
                    failed(format!("exited with {status}"), &stderr)
                }
            }
        };
        self.report(handler, &result, started.elapsed());
        result
    }

    /// Diagnostics keep only source, event, exit status, duration and a
    /// bounded stderr excerpt (Hooks §7).
    fn report(&self, handler: &HookHandler, result: &HookResult, duration: Duration) {
        let HookResult::Failed { reason, stderr } = result else {
            return;
        };
        tracing::warn!(
            source = handler.source.as_str(),
            event = handler.event.name(),
            reason = %reason,
            stderr = %stderr,
            duration_ms = duration.as_millis() as u64,
            "hook command failed"
        );
    }

    fn environment(&self, handler: &HookHandler) -> Vec<(&'static str, OsString)> {
        let mut vars = vec![
            ("DIM_PROJECT_DIR", OsString::from(self.config.cwd.clone())),
            ("DIM_HOOK_EVENT", OsString::from(handler.event.name())),
            (
                "DIM_SESSION_ID",
                OsString::from(self.identity.session_id.clone()),
            ),
        ];
        if let Some(home) = self.sources.0.home() {
            vars.push(("DIMCODE_HOME", home.into_os_string()));
        }
        vars
    }
}

fn hook_env(
    env: Vec<(OsString, OsString)>,
    vars: &[(&'static str, OsString)],
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = env
        .into_iter()
        .filter(|(key, _)| !MANAGED_ENV.iter().any(|name| key == name))
        .collect();
    env.extend(
        vars.iter()
            .map(|(key, value)| (OsString::from(*key), value.clone())),
    );
    env
}

/// Reads at most `limit` bytes and drains the rest, so the writer never
/// blocks; the bool says more was written.
async fn read_capped<R: AsyncRead + Unpin>(reader: &mut R, limit: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut overflow = false;
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => return (kept, overflow),
            Ok(read) => {
                let room = limit.saturating_sub(kept.len());
                if read <= room {
                    kept.extend_from_slice(&buffer[..read]);
                } else {
                    kept.extend_from_slice(&buffer[..room]);
                    overflow = true;
                }
            }
        }
    }
}

/// A failed run: its reason and the first [`STDERR_LIMIT`] bytes of its
/// standard error, which its diagnostic carries (§7).
fn failed(reason: impl Into<String>, stderr: &[u8]) -> HookResult {
    HookResult::Failed {
        reason: reason.into(),
        stderr: String::from_utf8_lossy(&stderr[..stderr.len().min(STDERR_LIMIT)]).into_owned(),
    }
}

/// What exit 2 means at each event (A.4's Exit 2 column). Every event names
/// its own meaning, so adding one is a choice and never a fall-through to a
/// default; `UserPromptSubmit` blocks, as its A.4 row says.
fn exit_two(event: HookEvent, stderr: &[u8]) -> HookResult {
    match event {
        // A.4 marks these as failure: the run is the failure `report` logs,
        // with the stderr excerpt, and the point contributes nothing (A.5,
        // "diagnostic only" for observation hooks).
        HookEvent::SessionStart | HookEvent::Notification => failed("exited with 2", stderr),
        // Every other event keeps its blocking meaning (A.4's Exit 2 column).
        HookEvent::PreToolUse
        | HookEvent::UserPromptSubmit
        | HookEvent::PermissionRequest
        | HookEvent::PostToolUse
        | HookEvent::PostToolUseFailure
        | HookEvent::Stop => HookResult::Blocked {
            reason: String::from_utf8_lossy(stderr).into_owned(),
        },
    }
}

/// A hook point's own text, escaped for the runtime block it goes into. The
/// cap is applied to the hook's text before this (A.4 bounds the context and
/// the reason, not their escaped form), so escaping never eats into what a
/// hook may contribute.
fn escaped(parts: &[String]) -> String {
    crate::context_block::text(&parts.join("\n"))
}

/// Bounds one hook's context at 16 KiB and drops an empty one (A.4).
fn capped(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.len() <= CONTEXT_LIMIT {
        return Some(text.to_owned());
    }
    let mut end = CONTEXT_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(text[..end].to_owned())
}

/// The text one run contributes at an observation or context point, if any.
fn contribution_of(event: HookEvent, run: &HookRun) -> Option<String> {
    match &run.result {
        HookResult::Failed { .. } => None,
        // The observation hooks' exit 2 is a failure (`exit_two`), so a
        // blocking result here is always the event's own protocol (A.4).
        HookResult::Blocked { reason } => capped(reason),
        HookResult::Success { stdout } => {
            let trimmed = stdout.trim();
            if trimmed.is_empty() {
                return None;
            }
            match serde_json::from_str::<Value>(trimmed) {
                Ok(Value::Object(object)) => {
                    report_ignored_fields(&object);
                    let mut parts: Vec<&str> = Vec::new();
                    if object.get("decision").and_then(Value::as_str) == Some("block")
                        && let Some(reason) = object.get("reason").and_then(Value::as_str)
                    {
                        parts.push(reason);
                    }
                    if let Some(context) = object
                        .get("hookSpecificOutput")
                        .and_then(|output| output.get("additionalContext"))
                        .and_then(Value::as_str)
                    {
                        parts.push(context);
                    }
                    capped(&parts.join("\n"))
                }
                _ if event == HookEvent::SessionStart => capped(trimmed),
                _ => None,
            }
        }
    }
}

/// One `Stop` run's parts. Only a block writes a follow-up (§3.4); its
/// context rides along with a block from any handler (A.4).
#[derive(Default)]
struct StopOutput {
    /// The reason the handler blocks with: exit 2's stderr, or
    /// `decision: "block"` with its `reason`.
    reason: Option<String>,
    /// `hookSpecificOutput.additionalContext`.
    context: Option<String>,
}

/// The `Stop` parts of one run (A.4). A failure is a diagnostic only (A.5).
fn stop_output(run: &HookRun) -> StopOutput {
    match &run.result {
        HookResult::Failed { .. } => StopOutput::default(),
        HookResult::Blocked { reason } => StopOutput {
            reason: capped(reason),
            context: None,
        },
        HookResult::Success { stdout } => {
            let trimmed = stdout.trim();
            if trimmed.is_empty() {
                return StopOutput::default();
            }
            let Ok(Value::Object(object)) = serde_json::from_str::<Value>(trimmed) else {
                return StopOutput::default();
            };
            report_ignored_fields(&object);
            let reason = (object.get("decision").and_then(Value::as_str) == Some("block"))
                .then(|| object.get("reason").and_then(Value::as_str))
                .flatten()
                .and_then(capped);
            let context = object
                .get("hookSpecificOutput")
                .and_then(|output| output.get("additionalContext"))
                .and_then(Value::as_str)
                .and_then(capped);
            StopOutput { reason, context }
        }
    }
}

/// Output fields this build knows and ignores, at the top level or under
/// `hookSpecificOutput` (A.4).
const IGNORED_OUTPUT: [&str; 10] = [
    "updatedInput",
    "updatedPermissions",
    "updatedToolOutput",
    "message",
    "continue",
    "stopReason",
    "suppressOutput",
    "systemMessage",
    "initialUserMessage",
    "sessionTitle",
];

/// Names the ignored fields a handler's output carried, so a hook written for
/// another product can tell which parts did nothing (A.4, §7).
fn report_ignored_fields(object: &serde_json::Map<String, Value>) {
    let named_in = |object: &serde_json::Map<String, Value>| {
        IGNORED_OUTPUT
            .into_iter()
            .filter(|name| object.contains_key(*name))
            .collect::<Vec<_>>()
    };
    let mut named = named_in(object);
    if let Some(Value::Object(output)) = object.get("hookSpecificOutput") {
        named.extend(named_in(output));
    }
    if !named.is_empty() {
        tracing::warn!(
            fields = %named.join(","),
            "hook output fields ignored"
        );
    }
}

/// The decision a `PermissionRequest` run asks for (A.4, A.5).
fn permission_decision(run: &HookRun) -> Option<ApprovalDecision> {
    let stdout = match &run.result {
        HookResult::Success { stdout } => stdout,
        // Exit 2 is ignored for this event (A.4).
        HookResult::Blocked { .. } => return None,
        // A failure, timeout or cancellation is the handler's `onFailure`:
        // `block` denies and `continue` has no opinion (A.5).
        HookResult::Failed { .. } => {
            return (run.on_failure == OnFailure::Block).then_some(ApprovalDecision::Deny);
        }
    };
    let value: Value = serde_json::from_str(stdout.trim()).ok()?;
    if let Value::Object(object) = &value {
        report_ignored_fields(object);
    }
    match value
        .pointer("/hookSpecificOutput/decision/behavior")
        .and_then(Value::as_str)?
    {
        "allow" => Some(ApprovalDecision::Allow),
        "deny" => Some(ApprovalDecision::Deny),
        _ => None,
    }
}

pub struct HooksSessionPlugin {
    sources: Key<HookSourcesHost>,
    platform: Key<HostPlatform>,
    config: Key<SessionConfig>,
    identity: Key<SessionIdentity>,
    runtime: Key<HooksRuntime>,
}
impl HooksSessionPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            sources: names.key()?,
            platform: names.key()?,
            config: names.key()?,
            identity: names.key()?,
            runtime: names.key()?,
        })
    }
}
impl DynPlugin for HooksSessionPlugin {
    fn name(&self) -> &str {
        "HooksSession"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.sources.erase(),
            self.platform.erase(),
            self.config.erase(),
            self.identity.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let sources = ctx.require_at(&self.sources)?;
            let platform = ctx.require_at(&self.platform)?;
            let config = ctx.require_at(&self.config)?;
            let identity = ctx.require_at(&self.identity)?;
            let runtime = Arc::new(HooksRuntime {
                sources,
                platform,
                config,
                identity,
                cancel: ctx.cancellation_token(),
                tasks: TaskTracker::new(),
            });
            let closing = runtime.clone();
            ctx.effect(move || async move {
                closing.tasks.close();
                closing.tasks.wait().await;
                Ok(())
            })?;
            ctx.provide_at(self.runtime.clone(), runtime)?;
            Ok(Effect::Done)
        })
    }
}

/// One call of a `step/tool-result` batch, with what decides it.
struct ToolRun {
    event: HookEvent,
    call_id: String,
    turn_id: String,
    name: String,
    input: JsonObject,
    alias: Option<String>,
    content: ToolContent,
    is_error: bool,
}

enum Job {
    ToolResults { trigger: u64, calls: Vec<ToolRun> },
    Stop { trigger: u64, from_follow_up: bool },
}

/// What a written output must still hold after its commands ran (§3.4).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Validity {
    /// Any point but `SessionStart`: no interrupt or newer operator input.
    Branch,
    /// `SessionStart`: the Session has not started either.
    Unstarted,
}

/// What a branch consumed, folded from its events.
#[derive(Default)]
struct View {
    /// The sequence of the last consumed event.
    last: u64,
    /// The last consumed `stop/interrupt`.
    interrupted: u64,
    /// The last consumed input that was the operator's own (§3.4).
    spoken: u64,
    /// The ids of written operator inputs the branch has not consumed yet.
    operator_inputs: HashSet<String>,
    /// Tool calls by `call_id`, from `step/tool-call`.
    calls: HashMap<String, (String, JsonObject, String)>,
    /// Tool aliases for matchers, from the adopted `context/configured`.
    aliases: HashMap<String, String>,
    /// The id of the follow-up input Hooks wrote and no turn has consumed yet.
    follow_up_input: Option<String>,
    /// A follow-up input was consumed since the last `turn/end`.
    consuming: bool,
    /// Consecutive follow-ups that drove turns.
    follow_ups: u32,
    /// A `turn/start` or a Session-start block was seen, so SessionStart is
    /// decided.
    started: bool,
}

impl View {
    fn fold(&mut self, event: &AgentEvent) -> Option<Job> {
        self.last = event.sequence;
        match &event.kind {
            EventKind::StopInterrupt(_) => {
                self.interrupted = event.sequence;
            }
            EventKind::InputMessage(data) => {
                if operator_input(data) {
                    self.operator_inputs.insert(event.id.clone());
                }
                if session_started(event) {
                    self.started = true;
                }
                if follow_up_input(event) {
                    self.follow_up_input = Some(event.id.clone());
                    self.follow_ups += 1;
                }
            }
            EventKind::InputConsumed(data) => {
                if self.operator_inputs.remove(&data.input_id) {
                    self.spoken = event.sequence;
                }
                if self.follow_up_input.as_deref() == Some(data.input_id.as_str()) {
                    self.consuming = true;
                    self.follow_up_input = None;
                }
            }
            EventKind::InputDeleted(data) => {
                // A withdrawn input ends without being consumed: the queue
                // writes input/deleted for an input it still holds
                // (input_queue.rs). It must not stay an operator input.
                self.operator_inputs.remove(&data.input_id);
            }
            EventKind::TurnStart(_) => {
                self.started = true;
            }
            EventKind::TurnEnd(data) => {
                let from_follow_up = self.consuming;
                self.consuming = false;
                if !from_follow_up {
                    self.follow_ups = 0;
                }
                if matches!(data.outcome, TurnOutcome::Completed { .. }) {
                    return Some(Job::Stop {
                        trigger: event.sequence,
                        from_follow_up,
                    });
                }
            }
            EventKind::ToolCall(data) => {
                for call in &data.calls {
                    self.calls.insert(
                        call.id.clone(),
                        (call.name.clone(), call.input.clone(), data.turn_id.clone()),
                    );
                }
            }
            EventKind::ToolResult(data) => {
                let calls = data
                    .results
                    .iter()
                    .filter_map(|result| self.tool_run(result))
                    .collect();
                // A result settles its call; nothing keeps the call's input.
                for result in &data.results {
                    self.calls.remove(&result.call_id);
                }
                return Some(Job::ToolResults {
                    trigger: event.sequence,
                    calls,
                });
            }
            EventKind::ContextConfigured(data) => {
                self.aliases.clear();
                if let Some(tools) = &data.settings.tools {
                    for tool in tools {
                        if let Some(alias) = alias(tool) {
                            self.aliases.insert(tool.name.clone(), alias);
                        }
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn tool_run(&self, result: &ToolCallResult) -> Option<ToolRun> {
        let (name, input, turn_id) = self.calls.get(&result.call_id)?.clone();
        let is_error = result.result.is_error == Some(true);
        Some(ToolRun {
            event: if is_error {
                HookEvent::PostToolUseFailure
            } else {
                HookEvent::PostToolUse
            },
            alias: self.aliases.get(&name).cloned(),
            call_id: result.call_id.clone(),
            turn_id,
            name,
            input,
            content: result.result.content.clone(),
            is_error,
        })
    }
}

/// The migration alias a pasted Claude or Codex matcher selects (G.2).
fn alias(tool: &ToolDefinition) -> Option<String> {
    if let Some(metadata) = &tool.metadata
        && let (Some(server), Some(original)) =
            (&metadata.mcp_server_id, &metadata.mcp_original_tool_name)
    {
        return Some(format!("mcp__{server}__{original}"));
    }
    match (tool.name.as_str(), tool.risk.as_deref()) {
        (_, Some("execute_process")) => Some("Bash".into()),
        ("read", _) => Some("Read".into()),
        ("write", _) => Some("Write".into()),
        ("edit", _) => Some("Edit".into()),
        ("glob", _) => Some("Glob".into()),
        ("grep", _) => Some("Grep".into()),
        _ => None,
    }
}

/// The branch's live approval runs, by approval id. `fold` registers a run
/// before its task starts; the live answer or cancellation then cancels the
/// token here, and the task checks it before starting any command.
#[derive(Default)]
struct Pending {
    tokens: Mutex<HashMap<String, CancellationToken>>,
}

impl Pending {
    /// Registers a live run and returns its token.
    fn register(&self, approval_id: &str, parent: &CancellationToken) -> CancellationToken {
        let cancel = parent.child_token();
        self.tokens
            .lock()
            .unwrap()
            .insert(approval_id.to_owned(), cancel.clone());
        cancel
    }

    /// Ends a run; the token cannot cancel anything once its task is done.
    fn finish(&self, approval_id: &str) {
        self.tokens.lock().unwrap().remove(approval_id);
    }

    /// Cancels a live run when its answer or cancellation is folded.
    fn cancel(&self, approval_id: &str) {
        if let Some(cancel) = self.tokens.lock().unwrap().remove(approval_id) {
            cancel.cancel();
        }
    }
}

struct Branch {
    runtime: Arc<HooksRuntime>,
    timeline: Arc<Timeline>,
    approval: Arc<Approval>,
    branch_id: String,
    view: Mutex<View>,
    jobs: mpsc::UnboundedSender<Job>,
    /// Commands answering an approval, by approval id.
    pending: Pending,
    cancel: CancellationToken,
    /// The part's task tracker: the drain loop and each approval's commands.
    tasks: TaskTracker,
}

impl Branch {
    /// Advances the consumed view; a live event that needs work is spawned or
    /// enqueued, and the handler returns.
    fn fold(self: &Arc<Self>, event: &AgentEvent, live: bool) {
        let job = {
            let mut view = self.view.lock().unwrap();
            view.fold(event)
        };
        match &event.kind {
            EventKind::ApprovalRespond(ApprovalRespondData { approval_id, .. })
            | EventKind::ApprovalCancelled(ApprovalCancelledData { approval_id, .. }) => {
                // Cancelling never blocks, so the handler may do it.
                self.pending.cancel(approval_id);
            }
            _ => {}
        }
        if !live {
            return;
        }
        if let Some(job) = job {
            let _ = self.jobs.send(job);
        }
        if let EventKind::ApprovalRequest(data) = &event.kind {
            // An approval writes no input, so it needs no place in the queue:
            // its run is a task of its own, which starts at once instead of
            // behind a running `PostToolUse` or `Stop` point. The token is
            // registered before the task starts, so an answer or cancellation
            // folded first has already cancelled the run it would start.
            let cancel = self.pending.register(&data.approval_id, &self.cancel);
            let owner = self.clone();
            let data = data.clone();
            self.tasks
                .spawn(async move { owner.approval(data, cancel).await });
        }
    }

    async fn drain(self: Arc<Self>, mut jobs: mpsc::UnboundedReceiver<Job>) {
        loop {
            let job = tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return,
                job = jobs.recv() => match job {
                    Some(job) => job,
                    None => return,
                },
            };
            match job {
                Job::ToolResults { trigger, calls } => self.post_tools(trigger, calls).await,
                Job::Stop {
                    trigger,
                    from_follow_up,
                } => self.stop(trigger, from_follow_up).await,
            }
        }
    }

    /// `SessionStart`, derived on `main` from a fold with no turn and no
    /// Session-start block (§3.4).
    async fn start_session(self: &Arc<Self>) {
        if self.branch_id != MAIN_BRANCH {
            return;
        }
        // The branch as it stood when the point was decided; anything the
        // branch consumes later invalidates the output (§3.4).
        let trigger = {
            let view = self.view.lock().unwrap();
            if view.started {
                return;
            }
            view.last
        };
        let Some(snapshot) = self.runtime.snapshot().await else {
            return;
        };
        let matching = handlers(&snapshot, HookEvent::SessionStart, None, None);
        if matching.is_empty() {
            return;
        }
        let mut payload = self
            .runtime
            .payload(HookEvent::SessionStart, &self.branch_id);
        payload.insert("source".into(), Value::String("startup".into()));
        let runs = self.runtime.run(&matching, &payload, &self.cancel).await;
        let blocks: Vec<InputBlock> = runs
            .iter()
            .enumerate()
            .filter_map(|(index, run)| {
                let context = contribution_of(HookEvent::SessionStart, run)?;
                Some(raw(
                    element(
                        "dim-hooks",
                        &[("kind", "session-start")],
                        &escaped(&[context]),
                    ),
                    Some(format!("{SESSION_START_KEY}{}", index + 1)),
                ))
            })
            .collect();
        if blocks.is_empty() {
            return;
        }
        self.write(trigger, InputMode::Append, blocks, Validity::Unstarted);
    }

    async fn post_tools(self: &Arc<Self>, trigger: u64, calls: Vec<ToolRun>) {
        if calls.is_empty() {
            return;
        }
        let Some(snapshot) = self.runtime.snapshot().await else {
            return;
        };
        let mut blocks = Vec::new();
        for call in calls {
            let matching = handlers(
                &snapshot,
                call.event,
                Some(&call.name),
                call.alias.as_deref(),
            );
            if matching.is_empty() {
                continue;
            }
            let mut payload = self.runtime.payload(call.event, &self.branch_id);
            payload.insert("tool_name".into(), Value::String(call.name.clone()));
            payload.insert("tool_input".into(), Value::Object(call.input.clone()));
            payload.insert("tool_use_id".into(), Value::String(call.call_id.clone()));
            payload.insert("turn_id".into(), Value::String(call.turn_id.clone()));
            payload.insert(
                "tool_response".into(),
                json!({"content": call.content, "is_error": call.is_error}),
            );
            let runs = self.runtime.run(&matching, &payload, &self.cancel).await;
            let context: Vec<String> = runs
                .iter()
                .filter_map(|run| contribution_of(call.event, run))
                .collect();
            if context.is_empty() {
                continue;
            }
            blocks.push(raw(
                element(
                    "dim-hooks",
                    &[
                        ("kind", "post-tool"),
                        ("call_id", &call.call_id),
                        ("tool", &call.name),
                    ],
                    &escaped(&context),
                ),
                None,
            ));
        }
        if blocks.is_empty() {
            return;
        }
        self.write(trigger, InputMode::Append, blocks, Validity::Branch);
    }

    async fn stop(self: &Arc<Self>, trigger: u64, from_follow_up: bool) {
        if self.view.lock().unwrap().follow_ups >= STOP_FOLLOW_UP_LIMIT {
            tracing::warn!("Stop follow-up limit reached; no further follow-up");
            return;
        }
        let Some(snapshot) = self.runtime.snapshot().await else {
            return;
        };
        let matching = handlers(&snapshot, HookEvent::Stop, None, None);
        if matching.is_empty() {
            return;
        }
        let mut payload = self.runtime.payload(HookEvent::Stop, &self.branch_id);
        payload.insert("stop_hook_active".into(), Value::Bool(from_follow_up));
        let runs = self.runtime.run(&matching, &payload, &self.cancel).await;
        let outputs: Vec<StopOutput> = runs.iter().map(stop_output).collect();
        let mut parts: Vec<String> = outputs
            .iter()
            .filter_map(|out| out.reason.clone())
            .collect();
        if parts.is_empty() {
            // A Stop context alone does not drive a turn (§3.4, §5).
            if outputs.iter().any(|out| out.context.is_some()) {
                tracing::info!("Stop context ignored: no handler blocked");
            }
            return;
        }
        parts.extend(outputs.iter().filter_map(|out| out.context.clone()));
        // The follow-up is read back by its opening marker (`follow_up_input`),
        // which the escape leaves alone: it covers the body only.
        let follow_up = element("dim-hooks", &[("kind", "follow-up")], &escaped(&parts));
        self.write(
            trigger,
            InputMode::Queue,
            vec![raw(follow_up, None)],
            Validity::Branch,
        );
    }

    async fn approval(self: &Arc<Self>, data: ApprovalRequestData, cancel: CancellationToken) {
        let alias = self
            .view
            .lock()
            .unwrap()
            .aliases
            .get(&data.call.name)
            .cloned();
        let runs = approval_point(
            &self.runtime,
            &self.branch_id,
            alias.as_deref(),
            &data,
            &cancel,
        )
        .await;
        self.pending.finish(&data.approval_id);
        let Some(runs) = runs else {
            return;
        };
        // A cancelled point responds nothing: the request was answered or
        // `approval/cancelled` was consumed while its commands ran (A.5).
        if cancel.is_cancelled() {
            return;
        }
        let decision = runs.iter().filter_map(permission_decision).fold(
            None,
            |acc: Option<ApprovalDecision>, next| match (acc, next) {
                (_, ApprovalDecision::Deny) | (Some(ApprovalDecision::Deny), _) => {
                    Some(ApprovalDecision::Deny)
                }
                (Some(ApprovalDecision::Allow), _) | (_, ApprovalDecision::Allow) => {
                    Some(ApprovalDecision::Allow)
                }
            },
        );
        if let Some(decision) = decision {
            let _ = self.approval.respond(&data.approval_id, decision).await;
        }
    }

    /// Writes an input unless the branch moved on after the triggering fact:
    /// an interrupt or a newer operator input was consumed, or, for
    /// `SessionStart`, the Session started (§3.4).
    fn write(
        &self,
        trigger: u64,
        mode: InputMode,
        blocks: Vec<InputBlock>,
        validity: Validity,
    ) -> Option<Arc<AgentEvent>> {
        let view = self.view.lock().unwrap();
        let moved_on = view.interrupted > trigger || view.spoken > trigger;
        let started = validity == Validity::Unstarted && view.started;
        if moved_on || started {
            tracing::warn!("hook output dropped: the branch moved on");
            return None;
        }
        let event = match append_own(
            &self.timeline,
            InputMessageData {
                blocks,
                mode: Some(mode),
                ..InputMessageData::default()
            },
            Vec::new(),
        ) {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(%error, "hook output could not be written");
                return None;
            }
        };
        Some(event)
    }
}

/// The commands of a live `PermissionRequest` point. `None` when the
/// request's answer or cancellation was folded first: `fold` registers the
/// token before the task starts, and a cancelled token starts no command even
/// though the task was already spawned.
async fn approval_point(
    runtime: &Arc<HooksRuntime>,
    branch_id: &str,
    alias: Option<&str>,
    data: &ApprovalRequestData,
    cancel: &CancellationToken,
) -> Option<Vec<HookRun>> {
    if cancel.is_cancelled() {
        return None;
    }
    let snapshot = runtime.snapshot().await?;
    // The answer or cancellation can also land during the read, before the
    // process would start.
    if cancel.is_cancelled() {
        return None;
    }
    let name = data.call.name.clone();
    let mut fields = vec![
        ("tool_name".to_owned(), Value::String(name.clone())),
        (
            "tool_input".to_owned(),
            Value::Object(data.call.input.clone()),
        ),
        (
            "approval_id".to_owned(),
            Value::String(data.approval_id.clone()),
        ),
        ("reason".to_owned(), Value::String(data.reason.clone())),
    ];
    if let BranchSource::Tool {
        call_id, turn_id, ..
    } = &data.source
    {
        fields.push(("tool_use_id".to_owned(), Value::String(call_id.clone())));
        fields.push(("turn_id".to_owned(), Value::String(turn_id.clone())));
    }

    let noticed = handlers(&snapshot, HookEvent::Notification, Some(&name), alias);
    if !noticed.is_empty() {
        let mut payload = runtime.payload(HookEvent::Notification, branch_id);
        for (key, value) in &fields {
            payload.insert(key.clone(), value.clone());
        }
        payload.insert(
            "notification_type".into(),
            Value::String("permission_prompt".into()),
        );
        payload.insert("message".into(), Value::String(data.reason.clone()));
        let owner = runtime.clone();
        let cancel = runtime.cancel.clone();
        runtime.tasks.spawn(async move {
            let _ = owner.run(&noticed, &payload, &cancel).await;
        });
    }

    let asking = handlers(&snapshot, HookEvent::PermissionRequest, Some(&name), alias);
    if asking.is_empty() {
        return None;
    }
    let mut payload = runtime.payload(HookEvent::PermissionRequest, branch_id);
    for (key, value) in fields {
        payload.insert(key, value);
    }
    Some(runtime.run(&asking, &payload, cancel).await)
}

/// Whether `data` is the operator's own submission rather than a source
/// input. Every source input starts with a `<dim-…>` marker block (session
/// access A.1, A.2) and the operator's words arrive without one (the prompt
/// guide); only the first block decides, and nothing reads its content.
fn operator_input(data: &InputMessageData) -> bool {
    data.user.is_some()
        && !matches!(
            data.blocks.first(),
            Some(InputBlock::Raw { text, .. }) if text.starts_with("<dim-")
        )
}

/// Whether `event` is a follow-up input Hooks wrote: its block carries the
/// `follow-up` marker (§3.4). Only Hooks writes `dim-hooks` markers.
fn follow_up_input(event: &AgentEvent) -> bool {
    let EventKind::InputMessage(data) = &event.kind else {
        return false;
    };
    data.blocks.iter().any(|block| {
        matches!(
            block,
            InputBlock::Raw { text, .. } if text.starts_with("<dim-hooks kind=\"follow-up\">")
        )
    })
}

fn session_started(event: &AgentEvent) -> bool {
    let EventKind::InputMessage(data) = &event.kind else {
        return false;
    };
    data.blocks.iter().any(|block| {
        matches!(
            block,
            InputBlock::Raw { key: Some(key), .. } if key.starts_with(SESSION_START_KEY)
        )
    })
}

/// The events this branch folds, in the one list its live subscription uses.
/// Every arm [`View::fold`] and [`Branch::fold`] match names its key here: a
/// kind folded on replay but not delivered live would leave a live Session's
/// view disagreeing with a rebuilt one (AGENTS rule 4).
const FOLLOWED_EVENTS: [&str; 12] = [
    "stop/interrupt",
    "input/message",
    "input/consumed",
    "input/deleted",
    "turn/start",
    "turn/end",
    "step/tool-call",
    "step/tool-result",
    "context/configured",
    "approval/request",
    "approval/respond",
    "approval/cancelled",
];

pub struct HooksBranchPlugin {
    runtime: Key<HooksRuntime>,
    timeline: Key<Timeline>,
    dispatcher: Key<Dispatcher>,
    approval: Key<Approval>,
    identity: Key<TimelineIdentity>,
    lifecycle: Key<BranchLifecycle>,
}
impl HooksBranchPlugin {
    pub fn new(names: &Names<'_>) -> rutis::Result<Self> {
        Ok(Self {
            runtime: names.key()?,
            timeline: names.key()?,
            dispatcher: names.key()?,
            approval: names.key()?,
            identity: names.key()?,
            lifecycle: names.key()?,
        })
    }
}
impl DynPlugin for HooksBranchPlugin {
    fn name(&self) -> &str {
        "HooksBranch"
    }
    fn injects(&self) -> Vec<TypeKey> {
        vec![
            self.runtime.erase(),
            self.timeline.erase(),
            self.dispatcher.erase(),
            self.approval.erase(),
            self.identity.erase(),
            self.lifecycle.erase(),
        ]
    }
    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, rutis::Result<Effect>> {
        Box::pin(async move {
            let runtime = ctx.require_at(&self.runtime)?;
            let timeline = ctx.require_at(&self.timeline)?;
            let dispatcher = ctx.require_at(&self.dispatcher)?;
            let approval = ctx.require_at(&self.approval)?;
            let identity = ctx.require_at(&self.identity)?;
            let lifecycle = ctx.require_at(&self.lifecycle)?;
            let (sender, receiver) = mpsc::unbounded_channel();
            let tracker = TaskTracker::new();
            let branch = Arc::new(Branch {
                runtime,
                timeline,
                approval,
                branch_id: identity.branch_id.clone(),
                view: Mutex::new(View::default()),
                jobs: sender,
                pending: Pending::default(),
                cancel: ctx.cancellation_token(),
                tasks: tracker.clone(),
            });
            let owner = branch.clone();
            dispatcher
                .follow(ctx, "HooksBranch", &FOLLOWED_EVENTS, move |delivery| {
                    match delivery {
                        Delivery::Replay(events) => {
                            for event in &events {
                                owner.fold(event, false);
                            }
                        }
                        Delivery::Live(event) => owner.fold(&event, true),
                    }
                    Box::pin(async { Ok(()) })
                })
                .await?;
            let closing = tracker.clone();
            ctx.effect(move || async move {
                closing.close();
                closing.wait().await;
                Ok(())
            })?;
            let owner = branch.clone();
            tracker.spawn(async move {
                if !lifecycle.activated().await {
                    return;
                }
                owner.start_session().await;
                Branch::drain(owner, receiver).await;
            });
            Ok(Effect::Done)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{
        HookSourceId, HookSourceSnapshot, HookSourceState, HooksDocument, ReadHookSources,
    };
    use crate::host::HostFuture;
    use dim_agent_types::{
        ApprovalCallData, ContentBlock, ContextConfiguredData, InputConsumedData, InputDeletedData,
        InterruptScope, ModelConfiguration, StopInterruptData, ToolCall, ToolCallData, ToolResult,
        ToolResultData, TurnEndData, TurnStartData, UserInput,
    };
    use std::path::PathBuf;

    fn event(sequence: u64, kind: EventKind) -> AgentEvent {
        AgentEvent {
            id: format!("event-{sequence}"),
            sequence,
            create_at: 0,
            tags: None,
            kind,
        }
    }

    /// §7: a failed run's diagnostic carries a bounded standard-error
    /// excerpt and nothing of the standard output or the payload.
    #[test]
    fn a_failure_keeps_a_bounded_stderr_excerpt() {
        let long = vec![b'x'; STDERR_LIMIT * 2];
        let HookResult::Failed { reason, stderr } = failed("exited with 1", &long) else {
            panic!("a failure");
        };
        assert_eq!(reason, "exited with 1");
        assert_eq!(stderr.len(), STDERR_LIMIT);
        let HookResult::Failed { stderr, .. } = failed("exited with 1", b"denied\n") else {
            panic!("a failure");
        };
        assert_eq!(stderr, "denied\n");
    }

    /// A.4's Exit 2 column: at `SessionStart` and `Notification`, exit 2 is a
    /// failure, so the run is the one `report` logs, carrying the stderr
    /// excerpt, and contributes no block.
    #[cfg(unix)]
    #[tokio::test]
    async fn exit_two_is_a_failure_at_session_start_and_notification() {
        for event in [HookEvent::SessionStart, HookEvent::Notification] {
            let dir =
                std::env::temp_dir().join(format!("dim-hooks-exit2-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let runtime = test_runtime(
                &dir,
                Arc::new(DocumentSources {
                    document: exit_two_document(event.name()),
                }),
            );
            let snapshot = runtime.snapshot().await.expect("the sources");
            let matching = handlers(&snapshot, event, None, None);
            assert_eq!(matching.len(), 1, "{} matched no handler", event.name());
            let run = HookRun {
                result: runtime
                    .run_one(&matching[0], Vec::new(), &runtime.cancel)
                    .await,
                on_failure: matching[0].on_failure,
            };
            assert!(
                matches!(&run.result, HookResult::Failed { stderr, .. } if stderr.as_str() == "bad config\n"),
                "exit 2 at {} was not a failure: {:?}",
                event.name(),
                run.result
            );
            assert_eq!(
                contribution_of(event, &run),
                None,
                "{} wrote a block",
                event.name()
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// F4: a result settles its call, and the call's input (whole file
    /// contents for `write`/`edit`) does not stay in the folded view.
    #[test]
    fn a_settled_tool_call_leaves_nothing_behind_in_the_view() {
        let mut view = View::default();
        view.fold(&event(
            1,
            EventKind::ToolCall(ToolCallData {
                turn_id: "turn".into(),
                request_id: "request".into(),
                calls: vec![ToolCall {
                    id: "call".into(),
                    name: "read".into(),
                    input: JsonObject::new(),
                }],
            }),
        ));
        assert!(view.calls.contains_key("call"));
        view.fold(&event(
            2,
            EventKind::ToolResult(ToolResultData {
                turn_id: "turn".into(),
                request_id: "request".into(),
                call_event_id: None,
                results: vec![ToolCallResult {
                    call_id: "call".into(),
                    result: ToolResult {
                        content: ToolContent::Text("done".into()),
                        structured_content: None,
                        is_error: Some(false),
                        blob_refs: None,
                        background_task: None,
                        context_messages: None,
                        effects: None,
                    },
                }],
            }),
        ));
        assert!(
            view.calls.is_empty(),
            "a settled call stayed in the folded view"
        );
    }

    /// AGENTS rule 4: a live fold must see what a replay does. The dispatcher
    /// matches a live event with `event.kind.key()`, and the subscription uses
    /// `FOLLOWED_EVENTS` alone, so that list is exactly the keys of the kinds
    /// `View::fold` and `Branch::fold` match: a kind the folds handle and the
    /// list misses leaves a live Session's view behind a rebuilt one, and a
    /// listed key no fold matches is dead weight.
    #[test]
    fn the_live_subscription_follows_exactly_the_kinds_the_folds_match() {
        let folded = [
            // The nine arms of `View::fold`.
            event(
                1,
                EventKind::StopInterrupt(StopInterruptData {
                    reason: "stop".into(),
                    scope: InterruptScope::Branch,
                }),
            ),
            event(2, EventKind::InputMessage(InputMessageData::default())),
            event(
                3,
                EventKind::InputConsumed(InputConsumedData {
                    input_id: "input".into(),
                }),
            ),
            event(
                4,
                EventKind::InputDeleted(InputDeletedData {
                    input_id: "input".into(),
                }),
            ),
            event(
                5,
                EventKind::TurnStart(TurnStartData {
                    turn_id: "turn".into(),
                }),
            ),
            event(
                6,
                EventKind::TurnEnd(TurnEndData {
                    turn_id: "turn".into(),
                    outcome: TurnOutcome::Completed {
                        reason: "stop".into(),
                    },
                }),
            ),
            event(
                7,
                EventKind::ToolCall(ToolCallData {
                    turn_id: "turn".into(),
                    request_id: "request".into(),
                    calls: Vec::new(),
                }),
            ),
            event(
                8,
                EventKind::ToolResult(ToolResultData {
                    turn_id: "turn".into(),
                    request_id: "request".into(),
                    call_event_id: None,
                    results: Vec::new(),
                }),
            ),
            event(
                9,
                EventKind::ContextConfigured(ContextConfiguredData {
                    timeline: "main".into(),
                    revision: "configuration".into(),
                    previous: None,
                    prompt: None,
                    settings: Box::new(ModelConfiguration {
                        provider_id: "provider".into(),
                        model_id: "model".into(),
                        capability_snapshot: None,
                        tools: None,
                        tool_sources: Vec::new(),
                        config: None,
                        metadata: None,
                    }),
                    system: None,
                    system_source: None,
                }),
            ),
            // The three arms of `Branch::fold`.
            event(
                10,
                EventKind::ApprovalRequest(ApprovalRequestData {
                    approval_id: "approval".into(),
                    source: BranchSource::Host {
                        operation_id: "operation".into(),
                    },
                    call: ApprovalCallData {
                        name: "exec".into(),
                        input: JsonObject::new(),
                    },
                    reason: "needs permission".into(),
                }),
            ),
            event(
                11,
                EventKind::ApprovalRespond(ApprovalRespondData {
                    approval_id: "approval".into(),
                    decision: ApprovalDecision::Allow,
                }),
            ),
            event(
                12,
                EventKind::ApprovalCancelled(ApprovalCancelledData {
                    approval_id: "approval".into(),
                    reason: "cancelled".into(),
                }),
            ),
        ];
        let mut matched: Vec<&str> = folded.iter().map(|event| event.kind.key()).collect();
        matched.sort_unstable();
        let mut followed = FOLLOWED_EVENTS.to_vec();
        followed.sort_unstable();
        assert_eq!(
            followed, matched,
            "the live fold and the rebuilt fold disagree"
        );
    }

    /// §3.4 and AGENTS rule 4: a withdrawn operator input ends without being
    /// consumed, so it leaves the set, and a later consumption of that id
    /// does not move `spoken`.
    #[test]
    fn a_deleted_operator_input_is_forgotten() {
        let input = event(
            1,
            EventKind::InputMessage(InputMessageData {
                user: Some(UserInput {
                    content: vec![ContentBlock::Text {
                        text: "hello".into(),
                    }],
                }),
                mode: Some(InputMode::Queue),
                ..InputMessageData::default()
            }),
        );
        let mut view = View::default();
        view.fold(&input);
        assert!(view.operator_inputs.contains(&input.id));
        view.fold(&event(
            2,
            EventKind::InputDeleted(InputDeletedData {
                input_id: input.id.clone(),
            }),
        ));
        assert!(
            view.operator_inputs.is_empty(),
            "a deleted input stayed in the set"
        );
        view.fold(&event(
            3,
            EventKind::InputConsumed(InputConsumedData {
                input_id: input.id.clone(),
            }),
        ));
        assert_eq!(view.spoken, 0, "a deleted input moved spoken");
    }

    /// AGENTS rule 4: the follow-up identity and count are read from the
    /// branch, so a replay folds the same state a live fold held. A turn the
    /// follow-up drove keeps the count; an operator turn resets it.
    #[test]
    fn the_follow_up_state_is_rebuilt_from_the_branch() {
        let follow_up = event(
            1,
            EventKind::InputMessage(InputMessageData {
                blocks: vec![raw(
                    element("dim-hooks", &[("kind", "follow-up")], "reason"),
                    None,
                )],
                mode: Some(InputMode::Queue),
                ..InputMessageData::default()
            }),
        );
        let turn_end = |sequence| {
            event(
                sequence,
                EventKind::TurnEnd(TurnEndData {
                    turn_id: "turn".into(),
                    outcome: TurnOutcome::Completed {
                        reason: "stop".into(),
                    },
                }),
            )
        };
        let mut view = View::default();
        view.fold(&follow_up);
        assert_eq!(view.follow_ups, 1, "the follow-up was not counted");
        assert_eq!(view.follow_up_input.as_deref(), Some(follow_up.id.as_str()));
        view.fold(&event(
            2,
            EventKind::InputConsumed(InputConsumedData {
                input_id: follow_up.id.clone(),
            }),
        ));
        assert!(view.consuming, "the follow-up turn was not recognized");
        view.fold(&turn_end(3));
        assert_eq!(view.follow_ups, 1, "a follow-up turn reset the count");
        view.fold(&turn_end(4));
        assert_eq!(view.follow_ups, 0, "an operator turn did not reset it");
    }

    /// F10: `fold` registers the run's token before its task starts, so the
    /// answer that folds first cancels the registered one.
    #[test]
    fn a_live_request_registers_its_token_before_its_task_starts() {
        let pending = Pending::default();
        let cancel = pending.register("approval-1", &CancellationToken::new());
        assert!(!cancel.is_cancelled());
        pending.cancel("approval-1");
        assert!(
            cancel.is_cancelled(),
            "the registered token was not cancelled"
        );
        assert!(
            pending.tokens.lock().unwrap().is_empty(),
            "the cancelled run stayed registered"
        );
        pending.finish("approval-1");
    }

    #[cfg(unix)]
    fn approval_request() -> ApprovalRequestData {
        ApprovalRequestData {
            approval_id: "approval-1".into(),
            source: BranchSource::Host {
                operation_id: "operation-1".into(),
            },
            call: ApprovalCallData {
                name: "exec".into(),
                input: JsonObject::new(),
            },
            reason: "needs permission".into(),
        }
    }

    #[cfg(unix)]
    fn permission_document(marker: &Path) -> HooksDocument {
        serde_json::from_value(serde_json::json!({
            "hooks": {
                "PermissionRequest": [{
                    "hooks": [{
                        "type": "command",
                        "command": format!("printf ran > {}", marker.display()),
                    }],
                }],
            },
        }))
        .unwrap()
    }

    /// A document with one handler at `event` that writes `bad config` to its
    /// standard error and exits 2.
    #[cfg(unix)]
    fn exit_two_document(event: &str) -> HooksDocument {
        let mut hooks = serde_json::Map::new();
        hooks.insert(
            event.to_owned(),
            serde_json::json!([{
                "hooks": [{
                    "type": "command",
                    "command": "printf 'bad config\\n' >&2; exit 2",
                }],
            }]),
        );
        serde_json::from_value(serde_json::json!({ "hooks": hooks })).unwrap()
    }

    #[cfg(unix)]
    fn document_snapshot(document: HooksDocument) -> HookSourcesSnapshot {
        HookSourcesSnapshot {
            sources: vec![HookSourceSnapshot {
                id: HookSourceId::User,
                path: None,
                enabled: true,
                state: HookSourceState::Ok,
                document: Some(document),
            }],
        }
    }

    #[cfg(unix)]
    struct DocumentSources {
        document: HooksDocument,
    }
    #[cfg(unix)]
    impl ReadHookSources for DocumentSources {
        fn sources(&self, _: &Path, _: CancellationToken) -> HostFuture<'_, HookSourcesSnapshot> {
            let snapshot = document_snapshot(self.document.clone());
            Box::pin(async move { Ok(snapshot) })
        }
        fn home(&self) -> Option<PathBuf> {
            None
        }
    }

    /// A source read the test holds open, so an answer can be folded while it
    /// is in flight.
    #[cfg(unix)]
    struct GatedSources {
        document: HooksDocument,
        entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        gate: Arc<tokio::sync::Semaphore>,
    }
    #[cfg(unix)]
    impl ReadHookSources for GatedSources {
        fn sources(&self, _: &Path, _: CancellationToken) -> HostFuture<'_, HookSourcesSnapshot> {
            let snapshot = document_snapshot(self.document.clone());
            let entered = self.entered.lock().unwrap().take();
            let gate = self.gate.clone();
            Box::pin(async move {
                if let Some(entered) = entered {
                    let _ = entered.send(());
                }
                let _ = gate.acquire().await;
                Ok(snapshot)
            })
        }
        fn home(&self) -> Option<PathBuf> {
            None
        }
    }

    #[cfg(unix)]
    fn test_runtime(dir: &Path, sources: Arc<dyn ReadHookSources>) -> Arc<HooksRuntime> {
        use crate::host_platform::{
            ExecShellRuntime, HostPlatform, OutputEncoding, ShellEdition, ShellSource, ShellStatus,
        };
        std::fs::create_dir_all(dir.join("home")).unwrap();
        let mut platform = HostPlatform::current(ShellStatus::Available(ExecShellRuntime {
            executable_path: "/bin/sh".into(),
            edition: ShellEdition::Sh,
            source: ShellSource::Fallback,
            output_encoding: OutputEncoding::Utf8,
        }))
        .unwrap();
        platform.home = dir.join("home");
        Arc::new(HooksRuntime {
            sources: Arc::new(HookSourcesHost(sources)),
            platform: Arc::new(platform),
            config: Arc::new(SessionConfig {
                cwd: dir.to_string_lossy().into_owned(),
                ..SessionConfig::default()
            }),
            identity: Arc::new(SessionIdentity {
                unloading: CancellationToken::new(),
                session_id: "session".into(),
                restored: false,
            }),
            cancel: CancellationToken::new(),
            tasks: TaskTracker::new(),
        })
    }

    /// F10: the answer folds after the request and before `approval()` runs;
    /// the registered token is already cancelled when the task starts, and no
    /// `PermissionRequest` command may start.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_answer_folded_before_the_task_starts_runs_no_permission_command() {
        let dir = std::env::temp_dir().join(format!("dim-hooks-answer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("marker");
        let runtime = test_runtime(
            &dir,
            Arc::new(DocumentSources {
                document: permission_document(&marker),
            }),
        );
        let cancel = runtime.cancel.child_token();
        cancel.cancel();
        assert!(
            approval_point(&runtime, "main", None, &approval_request(), &cancel)
                .await
                .is_none(),
            "a cancelled request ran its commands"
        );
        assert!(!marker.exists(), "a cancelled request started its command");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// F10: the answer folds while the sources are being read, after the task
    /// started; the point still starts no command.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_answer_folded_during_the_read_runs_no_permission_command() {
        let dir = std::env::temp_dir().join(format!("dim-hooks-read-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("marker");
        let (entered, entered_rx) = tokio::sync::oneshot::channel();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let runtime = test_runtime(
            &dir,
            Arc::new(GatedSources {
                document: permission_document(&marker),
                entered: Mutex::new(Some(entered)),
                gate: gate.clone(),
            }),
        );
        let cancel = runtime.cancel.child_token();
        let task = {
            let runtime = runtime.clone();
            let cancel = cancel.clone();
            let data = approval_request();
            tokio::spawn(
                async move { approval_point(&runtime, "main", None, &data, &cancel).await },
            )
        };
        entered_rx.await.unwrap();
        cancel.cancel();
        gate.add_permits(1);
        assert!(
            task.await.unwrap().is_none(),
            "a request answered during the read ran its commands"
        );
        assert!(!marker.exists(), "a cancelled request started its command");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
