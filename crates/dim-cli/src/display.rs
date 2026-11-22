use crate::wire::{Configuration, Event, EventContent, Progress};
use crate::{Result, sanitize::safe};
use dim_agent_types::*;
use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
};

#[derive(Default)]
struct TextPosition {
    committed: u64,
    printed: u64,
}

/// Where a request's text goes in `exec`'s text mode ([`Display::final_reply_only`]).
#[derive(Clone, Copy, PartialEq)]
enum TextRoute {
    /// The turn's final reply.
    Reply,
    /// Words before or between tool calls, or a request that failed or was
    /// cancelled: diagnostics, not the reply.
    Progress,
}

/// The current request's text while `exec` waits to learn whether it is the
/// turn's final reply.
struct HeldText {
    request: String,
    text: String,
    route: Option<TextRoute>,
}

pub struct Display {
    pub mcp_auth_required: HashSet<String>,
    pub model: Option<Configuration>,
    pub model_through: u64,
    /// Local sequence of the last `permission/changed` of `main` already
    /// reported, by a record or by the command that wrote it.
    pub permission_seen: u64,
    pub json: bool,
    pub cursor: u64,
    pub history_before: Option<u64>,
    pub history_through: u64,
    pub failed: bool,
    pub muted: bool,
    interactive: bool,
    text_open: bool,
    requests: HashMap<String, TextPosition>,
    tools: HashMap<(String, String), String>,
    finished: HashSet<String>,
    completed_tools: HashSet<(String, String)>,
    /// Requests that already showed output, for the incomplete prefix.
    showed: HashSet<String>,
    /// The `error` of a model failure already shown this turn: the turn's
    /// repeat of it prints nothing.
    shown_failure: Option<String>,
    /// Whether retry waits are reported; exec keeps them quiet.
    pub retries: bool,
    /// The last attempt a retry line reported, by request id.
    retry_seen: HashMap<String, u8>,
    /// `exec`'s text mode: a request's text waits for the request's terminal,
    /// so stdout carries the turn's final reply alone.
    pub final_reply_only: bool,
    held: Option<HeldText>,
    /// A diagnostic text line is open on stderr.
    progress_text_open: bool,
}
impl Display {
    pub fn new(interactive: bool, json: bool) -> Self {
        Self {
            mcp_auth_required: HashSet::new(),
            model: None,
            model_through: 0,
            permission_seen: 0,
            json,
            cursor: 0,
            history_before: None,
            history_through: 0,
            failed: false,
            muted: false,
            interactive,
            text_open: false,
            requests: HashMap::new(),
            tools: HashMap::new(),
            finished: HashSet::new(),
            completed_tools: HashSet::new(),
            showed: HashSet::new(),
            shown_failure: None,
            retries: true,
            retry_seen: HashMap::new(),
            final_reply_only: false,
            held: None,
            progress_text_open: false,
        }
    }
    pub fn record(&mut self, message: &str) -> Result<()> {
        if self.muted {
            return Ok(());
        }
        self.finish_text()?;
        if self.interactive {
            println!("{}", safe(message));
        } else {
            eprintln!("{}", safe(message));
        }
        Ok(())
    }
    pub fn shell_progress(&mut self, text: &str) -> Result<()> {
        print!("{}", safe(text));
        self.text_open = !text.ends_with('\n');
        io::stdout().flush()?;
        Ok(())
    }
    pub fn finish_text(&mut self) -> Result<()> {
        if self.final_reply_only {
            self.route_text(TextRoute::Progress)?;
            self.finish_progress()?;
        }
        if self.text_open {
            println!();
            self.text_open = false;
        }
        io::stdout().flush()?;
        Ok(())
    }
    /// Ends the line an interruption leaves open. `exec` releases its held text
    /// to stderr; another mode writes the newline it has always written, even
    /// when no text is open.
    pub fn finish_interrupt(&mut self) -> Result<()> {
        if self.final_reply_only {
            return self.finish_text();
        }
        if !self.json {
            println!();
        }
        Ok(())
    }
    fn show_text(&mut self, text: &str) -> Result<()> {
        if !self.text_open && self.interactive {
            println!("Assistant:");
        }
        print!("{}", safe(text));
        io::stdout().flush()?;
        self.text_open = true;
        Ok(())
    }
    fn show_progress(&mut self, text: &str) -> Result<()> {
        eprint!("{}", safe(text));
        io::stderr().flush()?;
        self.progress_text_open = true;
        Ok(())
    }
    fn finish_progress(&mut self) -> Result<()> {
        if self.progress_text_open {
            eprintln!();
            self.progress_text_open = false;
        }
        io::stderr().flush()?;
        Ok(())
    }
    /// Routes the text held for the current request. A request keeps the route
    /// once it is decided.
    fn route_text(&mut self, route: TextRoute) -> Result<()> {
        let Some(mut held) = self.held.take() else {
            return Ok(());
        };
        held.route = Some(held.route.unwrap_or(route));
        if !held.text.is_empty() {
            let text = std::mem::take(&mut held.text);
            match held.route {
                Some(TextRoute::Reply) => self.show_text(&text)?,
                _ => self.show_progress(&text)?,
            }
        }
        self.held = Some(held);
        Ok(())
    }
    fn text(&mut self, request: &str, text: &str) -> Result<()> {
        if text.is_empty() || self.muted || self.json {
            return Ok(());
        }
        if !self.final_reply_only {
            return self.show_text(text);
        }
        if self
            .held
            .as_ref()
            .is_some_and(|held| held.request != request)
        {
            self.route_text(TextRoute::Progress)?;
            self.held = None;
        }
        let route = self
            .held
            .as_ref()
            .filter(|held| held.request == request)
            .and_then(|held| held.route);
        match route {
            Some(TextRoute::Reply) => self.show_text(text),
            Some(TextRoute::Progress) => self.show_progress(text),
            None => {
                let held = self.held.get_or_insert_with(|| HeldText {
                    request: request.to_owned(),
                    text: String::new(),
                    route: None,
                });
                held.text.push_str(text);
                Ok(())
            }
        }
    }
    pub fn progress(&mut self, progress: Progress) -> Result<()> {
        let Some(request) = progress.request_id else {
            return Ok(());
        };
        if self.retries {
            match &progress.retry {
                Some(retry) if self.retry_seen.get(&request) != Some(&retry.attempt) => {
                    self.retry_seen.insert(request.clone(), retry.attempt);
                    let line = crate::ui::i18n::tf(
                        "retrying {attempt}/{max} in {seconds} s: {reason}",
                        &[
                            ("attempt", &retry.attempt.to_string()),
                            ("max", &retry.max_attempts.to_string()),
                            (
                                "seconds",
                                &crate::projection::retry_seconds(retry.retry_at_ms).to_string(),
                            ),
                            (
                                "reason",
                                &crate::projection::failure_phrase(
                                    retry.kind,
                                    retry.reason.as_deref(),
                                ),
                            ),
                        ],
                    );
                    self.record(&line)?;
                }
                Some(_) => {}
                None => {
                    self.retry_seen.remove(&request);
                }
            }
        }
        if self.finished.contains(&request) {
            return Ok(());
        }
        let position = self.requests.entry(request.clone()).or_default();
        if progress.text_offset > position.printed {
            return Ok(());
        }
        let count = progress.text.chars().count() as u64;
        let skip = position.printed.saturating_sub(progress.text_offset);
        if skip >= count {
            return Ok(());
        }
        position.printed = progress.text_offset + count;
        self.showed.insert(request.clone());
        self.text(
            &request,
            &progress
                .text
                .chars()
                .skip(skip as usize)
                .collect::<String>(),
        )
    }
    pub fn event(&mut self, event: Event, history: bool) -> Result<()> {
        if let EventContent::Configuration(configuration) = &event.content
            && event.sequence > self.model_through
        {
            self.model = Some(configuration.clone());
            self.model_through = event.sequence;
        }
        if event.sequence <= self.cursor {
            return Ok(());
        }
        self.cursor = event.sequence;
        match event.content {
            EventContent::Input(data) if history => {
                if let Some(user) = data.user {
                    let text = crate::projection::user_text(&user.content);
                    self.record(&format!("> {text}"))?;
                    for line in crate::projection::context_lines(&user.content) {
                        self.record(&line)?;
                    }
                } else if let Some(line) = crate::projection::raw_input_line(&data.blocks) {
                    self.record(&line)?;
                }
            }
            EventContent::Text(data) => {
                self.showed.insert(data.request_id.clone());
                let request = data.request_id;
                let position = self.requests.entry(request.clone()).or_default();
                let count = data.delta.chars().count() as u64;
                let skip = position
                    .printed
                    .saturating_sub(position.committed)
                    .min(count);
                position.committed += count;
                position.printed = position.printed.max(position.committed);
                self.text(
                    &request,
                    &data.delta.chars().skip(skip as usize).collect::<String>(),
                )?;
            }
            EventContent::ToolUse(data) => {
                self.tools.insert(
                    (data.request_id.clone(), data.call.id.clone()),
                    data.call.name.clone(),
                );
                self.showed.insert(data.request_id.clone());
                self.record(&format!("[tool] {}", data.call.name))?;
            }
            EventContent::ToolCompleted(data) => {
                self.tool_result(&data.request_id, data.call_id, data.result.is_error)?;
            }
            EventContent::ToolResult(data) => {
                for result in data.results {
                    self.tool_result(&data.request_id, result.call_id, result.result.is_error)?;
                }
            }
            EventContent::ModelEnd(data) => {
                self.finished.insert(data.request_id.clone());
                let incomplete = self.showed.remove(&data.request_id);
                self.retry_seen.remove(&data.request_id);
                if self.final_reply_only {
                    let route = match &data.outcome {
                        ModelOutputOutcome::Completed {
                            finish_reason: FinishReason::ToolUse,
                        } => TextRoute::Progress,
                        ModelOutputOutcome::Completed { .. } => TextRoute::Reply,
                        ModelOutputOutcome::Failed { .. }
                        | ModelOutputOutcome::Cancelled { .. } => TextRoute::Progress,
                    };
                    self.route_text(route)?;
                }
                self.finish_text()?;
                if let ModelOutputOutcome::Failed {
                    error,
                    presentation,
                    failure,
                    ..
                } = data.outcome
                {
                    // A failed model terminal is reported, but it does not by
                    // itself make the run a failure: the turn that recovered
                    // from it, or ended Completed at its output limit, is what
                    // the caller acted on.
                    let line = match (presentation, failure) {
                        (Some(presentation), _) => Some(format!(
                            "[failed] {}: {}{}",
                            presentation.source,
                            presentation
                                .server_message
                                .as_deref()
                                .unwrap_or(&presentation.reason),
                            presentation
                                .action
                                .map(|action| format!(". {action}"))
                                .unwrap_or_default(),
                        )),
                        (None, Some(failure)) => Some(format!(
                            "[failed] {}",
                            crate::projection::failure_line(&failure, incomplete)
                        )),
                        (None, None) => None,
                    };
                    if let Some(line) = line {
                        self.record(&line)?;
                        self.shown_failure = Some(error);
                    }
                }
            }
            EventContent::CompactionStart(_) => self.record("[context] Compacting")?,
            EventContent::CompactionSummary(_) => self.record("[context] Compacted")?,
            EventContent::Compaction(data) => {
                self.record(match data.outcome {
                    ContextCompactionResult::Completed { .. } => "[context] Compacted",
                    ContextCompactionResult::Skipped { .. } => "[context] Compaction skipped",
                    ContextCompactionResult::Failed { .. } => "[context] Compaction failed",
                    ContextCompactionResult::Cancelled { .. } => "[context] Compaction cancelled",
                })?;
            }
            EventContent::TurnEnd(data) => {
                if self.final_reply_only {
                    let route = match &data.outcome {
                        TurnOutcome::Completed { .. } => TextRoute::Reply,
                        TurnOutcome::Failed { .. } | TurnOutcome::Cancelled { .. } => {
                            TextRoute::Progress
                        }
                    };
                    self.route_text(route)?;
                    self.held = None;
                }
                self.finish_text()?;
                let presented = self.shown_failure.take();
                match data.outcome {
                    TurnOutcome::Failed { error } => {
                        self.failed = true;
                        if presented.as_deref() != Some(error.as_str()) {
                            self.record(
                                "[failed] Model or turn failed; see the private daemon log for details",
                            )?;
                        }
                    }
                    TurnOutcome::Cancelled { .. } => self.record("[stopped]")?,
                    _ => {}
                }
                self.requests.clear();
            }
            _ => {}
        }
        Ok(())
    }
    fn tool_result(
        &mut self,
        request_id: &str,
        call_id: String,
        is_error: Option<bool>,
    ) -> Result<()> {
        let identity = (request_id.to_owned(), call_id.clone());
        if !self.completed_tools.insert(identity.clone()) {
            return Ok(());
        }
        let name = self.tools.remove(&identity).unwrap_or(call_id);
        self.record(&format!(
            "[{}] {name}",
            if is_error == Some(true) {
                "failed"
            } else {
                "done"
            }
        ))
    }
}
