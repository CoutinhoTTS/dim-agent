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
    /// The turn's final reply: the last request of a turn that completed.
    Reply,
    /// Words before or between tool calls, or a request that failed or was
    /// cancelled: diagnostics, not the reply.
    Progress,
}

/// The current request's text while `exec` waits to learn whether the turn
/// delivers it as the reply.
struct HeldText {
    request: String,
    /// The turn the request's events named, when they named one.
    turn: Option<String>,
    text: String,
    route: Option<TextRoute>,
}

/// Which submitted input, and which turn consuming it, `exec`'s stdout may
/// carry the reply of.
#[derive(Clone, PartialEq)]
enum Reply {
    /// No submission of this invocation owns the stream.
    Closed,
    /// `session.submit` is in flight for this submission id.
    Submitted { submission_id: String },
    /// The branch wrote this input; the turn consuming it is not known yet.
    Accepted { input_id: String },
    /// The turn consuming the submitted input.
    ActiveTurn { input_id: String, turn_id: String },
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
    /// `exec`'s text mode: a request's text waits for the turn to end, so
    /// stdout carries the turn's final reply alone.
    pub final_reply_only: bool,
    /// The submitted input was deleted before its turn wrote a reply: the run
    /// has no answer and exits as a failure.
    input_deleted: bool,
    /// The submitted input and turn this invocation's stdout may carry.
    reply: Reply,
    /// The branch's running turn, from its `turn/start` to its `turn/end`.
    turn: Option<String>,
    /// Turns that ended before the submitted input's turn was known.
    ended: HashMap<String, TurnOutcome>,
    /// Inputs the branch consumed before the submission named them.
    consumed: HashMap<String, String>,
    /// Inputs the branch deleted before the submission named them.
    deleted: HashSet<String>,
    /// The turn that asked each request, from the events that name both.
    request_turn: HashMap<String, String>,
    /// First sighting of each request, so a late end of an earlier request is
    /// not mistaken for a newer one.
    request_order: HashMap<String, u64>,
    next_request: u64,
    /// Text held per request until its route is known.
    held: Vec<HeldText>,
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
            input_deleted: false,
            reply: Reply::Closed,
            turn: None,
            ended: HashMap::new(),
            consumed: HashMap::new(),
            deleted: HashSet::new(),
            request_turn: HashMap::new(),
            request_order: HashMap::new(),
            next_request: 0,
            held: Vec::new(),
            progress_text_open: false,
        }
    }
    pub fn record(&mut self, message: &str) -> Result<()> {
        if self.muted {
            return Ok(());
        }
        self.end_lines()?;
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
    /// Ends the text a run still holds: text the turn did not release as its
    /// reply is diagnostics.
    pub fn finish_text(&mut self) -> Result<()> {
        if self.final_reply_only {
            self.flush_held(TextRoute::Progress)?;
        }
        self.end_lines()
    }
    /// The CLI submits `submission_id`: stdout stays shut until the branch
    /// names the input this submission wrote.
    pub fn submitting(&mut self, submission_id: String) {
        if !self.final_reply_only {
            return;
        }
        self.reply = Reply::Submitted { submission_id };
        self.input_deleted = false;
        self.held.clear();
        self.turn = None;
        self.ended.clear();
        self.consumed.clear();
        self.deleted.clear();
        self.request_turn.clear();
        self.request_order.clear();
    }
    /// The branch wrote the submitted input: bind the turn that consumes it,
    /// or close when it was already deleted or has already ended.
    pub fn accepted(&mut self, input_id: String) -> Result<()> {
        if !self.final_reply_only
            || !matches!(self.reply, Reply::Submitted { .. } | Reply::Accepted { .. })
        {
            return Ok(());
        }
        if self.deleted.remove(&input_id) {
            self.close_reply()?;
            return Ok(());
        }
        self.reply = Reply::Accepted {
            input_id: input_id.clone(),
        };
        if let Some(turn) = self.consumed.remove(&input_id) {
            self.activate(input_id, turn)?;
        }
        Ok(())
    }
    /// Closes the stream for good: text no turn released is diagnostics.
    pub fn close_reply(&mut self) -> Result<()> {
        if !self.final_reply_only || self.reply == Reply::Closed {
            return Ok(());
        }
        self.reply = Reply::Closed;
        self.flush_held(TextRoute::Progress)?;
        self.turn = None;
        self.ended.clear();
        self.consumed.clear();
        self.deleted.clear();
        self.request_turn.clear();
        self.request_order.clear();
        Ok(())
    }
    /// Whether the submitted input was deleted before a turn wrote its reply.
    pub fn reply_input_deleted(&self) -> bool {
        self.input_deleted
    }
    fn activate(&mut self, input_id: String, turn_id: String) -> Result<()> {
        if let Some(outcome) = self.ended.remove(&turn_id) {
            self.reply = Reply::ActiveTurn {
                input_id,
                turn_id: turn_id.clone(),
            };
            return self.settle(&outcome);
        }
        self.reply = Reply::ActiveTurn {
            input_id,
            turn_id: turn_id.clone(),
        };
        self.bind_held(&turn_id)
    }
    /// Keeps the text whose request belongs to the bound turn; every other
    /// held request becomes diagnostics.
    fn bind_held(&mut self, turn_id: &str) -> Result<()> {
        for index in 0..self.held.len() {
            let turn = self.held[index]
                .turn
                .clone()
                .or_else(|| self.request_turn.get(&self.held[index].request).cloned());
            if turn.as_deref().is_some_and(|turn| turn != turn_id) {
                self.held[index].route = Some(TextRoute::Progress);
                self.flush(index)?;
            }
        }
        Ok(())
    }
    /// Ends the bound turn: the text of its last request is the reply when the
    /// turn completed, and every other held request is diagnostics.
    fn settle(&mut self, outcome: &TurnOutcome) -> Result<()> {
        let completed = matches!(outcome, TurnOutcome::Completed { .. });
        let turn = match &self.reply {
            Reply::ActiveTurn { turn_id, .. } => Some(turn_id.clone()),
            _ => None,
        };
        let mut last: Option<(String, u64)> = None;
        for held in &self.held {
            let ours = held
                .turn
                .as_deref()
                .is_none_or(|held| Some(held) == turn.as_deref())
                && self
                    .request_turn
                    .get(&held.request)
                    .is_none_or(|request| Some(request) == turn.as_ref());
            if ours {
                let order = self.request_order.get(&held.request).copied().unwrap_or(0);
                if last.as_ref().is_none_or(|(_, seen)| order > *seen) {
                    last = Some((held.request.clone(), order));
                }
            }
        }
        for (request, request_turn) in &self.request_turn {
            if Some(request_turn) != turn.as_ref() {
                continue;
            }
            let order = self.request_order.get(request).copied().unwrap_or(0);
            if last.as_ref().is_none_or(|(_, seen)| order > *seen) {
                last = Some((request.clone(), order));
            }
        }
        let last = last.map(|(request, _)| request);
        for index in 0..self.held.len() {
            if self.held[index].route.is_none() {
                self.held[index].route = Some(
                    if completed && last.as_deref() == Some(self.held[index].request.as_str()) {
                        TextRoute::Reply
                    } else {
                        TextRoute::Progress
                    },
                );
            }
            self.flush(index)?;
        }
        self.reply = Reply::Closed;
        self.held.clear();
        self.ended.clear();
        self.consumed.clear();
        self.deleted.clear();
        self.request_turn.clear();
        self.request_order.clear();
        Ok(())
    }
    /// Ends the lines this display has open, so the next one starts on its own.
    /// It does not decide where held text goes.
    fn end_lines(&mut self) -> Result<()> {
        self.finish_progress()?;
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
    /// Routes the text held for one request once its destination is known. A
    /// request keeps the route once it is decided.
    fn route_request(&mut self, request: &str, route: TextRoute) -> Result<()> {
        if let Some(index) = self.held.iter().position(|held| held.request == request)
            && self.held[index].route.is_none()
        {
            self.held[index].route = Some(route);
            self.flush(index)?;
        }
        Ok(())
    }
    /// Writes the text an entry holds and keeps its route.
    fn flush(&mut self, index: usize) -> Result<()> {
        let text = std::mem::take(&mut self.held[index].text);
        if text.is_empty() {
            return Ok(());
        }
        match self.held[index].route {
            Some(TextRoute::Reply) => self.show_text(&text),
            _ => self.show_progress(&text),
        }
    }
    fn flush_held(&mut self, route: TextRoute) -> Result<()> {
        for index in 0..self.held.len() {
            self.held[index].route = Some(self.held[index].route.unwrap_or(route));
            self.flush(index)?;
        }
        self.held.clear();
        Ok(())
    }
    /// The order a request first appeared in, so a late end of an earlier
    /// request is not mistaken for a newer request's end.
    fn note_request(&mut self, request: &str) -> u64 {
        if let Some(order) = self.request_order.get(request) {
            return *order;
        }
        let order = self.next_request;
        self.next_request += 1;
        self.request_order.insert(request.to_owned(), order);
        order
    }
    /// A request that first appears later makes the earlier held text
    /// diagnostics: the later request is where the turn's reply can come from.
    fn demote_before(&mut self, order: u64) -> Result<()> {
        for index in 0..self.held.len() {
            if self.held[index].route.is_none()
                && self.request_order.get(&self.held[index].request).copied() < Some(order)
            {
                self.held[index].route = Some(TextRoute::Progress);
                self.flush(index)?;
            }
        }
        Ok(())
    }
    /// Whether text of `request` may be `exec`'s reply; `None` while the
    /// submission is not bound to a turn yet.
    fn owns(&self, request: &str, turn: Option<&str>) -> Option<bool> {
        match &self.reply {
            Reply::Closed => Some(false),
            Reply::Submitted { .. } | Reply::Accepted { .. } => None,
            Reply::ActiveTurn { turn_id, .. } => Some(match turn {
                Some(turn) => turn == turn_id,
                None => self
                    .request_turn
                    .get(request)
                    .is_none_or(|request_turn| request_turn == turn_id),
            }),
        }
    }
    fn text(&mut self, turn: Option<&str>, request: &str, text: &str) -> Result<()> {
        if text.is_empty() || self.muted || self.json {
            return Ok(());
        }
        if !self.final_reply_only {
            return self.show_text(text);
        }
        let order = self.note_request(request);
        if let Some(turn) = turn {
            self.request_turn
                .insert(request.to_owned(), turn.to_owned());
        }
        match self.owns(request, turn) {
            Some(true) => self.hold(request, turn, text, order),
            Some(false) => {
                self.route_request(request, TextRoute::Progress)?;
                self.show_progress(text)
            }
            None => self.hold(request, turn, text, order),
        }
    }
    fn hold(&mut self, request: &str, turn: Option<&str>, text: &str, order: u64) -> Result<()> {
        if let Some(index) = self.held.iter().position(|held| held.request == request) {
            if let Some(route) = self.held[index].route {
                return match route {
                    TextRoute::Reply => self.show_text(text),
                    TextRoute::Progress => self.show_progress(text),
                };
            }
            self.held[index].text.push_str(text);
            return Ok(());
        }
        self.demote_before(order)?;
        self.held.push(HeldText {
            request: request.to_owned(),
            turn: turn.map(str::to_owned),
            text: text.to_owned(),
            route: None,
        });
        Ok(())
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
        let turn = self.request_turn.get(&request).cloned();
        self.text(
            turn.as_deref(),
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
        let event_id = event.id.clone();
        match event.content {
            EventContent::Input(data) => {
                if !history
                    && let Reply::Submitted { submission_id } = &self.reply
                    && data.submission_id.as_deref() == Some(submission_id.as_str())
                {
                    self.accepted(event_id)?;
                }
                if history {
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
            }
            EventContent::Text(data) => {
                self.showed.insert(data.request_id.clone());
                let request = data.request_id;
                let turn = data.turn_id;
                let position = self.requests.entry(request.clone()).or_default();
                let count = data.delta.chars().count() as u64;
                let skip = position
                    .printed
                    .saturating_sub(position.committed)
                    .min(count);
                position.committed += count;
                position.printed = position.printed.max(position.committed);
                self.text(
                    Some(&turn),
                    &request,
                    &data.delta.chars().skip(skip as usize).collect::<String>(),
                )?;
            }
            EventContent::ToolUse(data) => {
                if self.final_reply_only {
                    let order = self.note_request(&data.request_id);
                    self.request_turn
                        .insert(data.request_id.clone(), data.turn_id.clone());
                    self.demote_before(order)?;
                    self.route_request(&data.request_id, TextRoute::Progress)?;
                }
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
            EventContent::ModelStart(data) => {
                self.note_request(&data.request_id);
                self.request_turn.insert(data.request_id, data.turn_id);
            }
            EventContent::ModelEnd(data) => {
                self.note_request(&data.request_id);
                self.request_turn
                    .insert(data.request_id.clone(), data.turn_id.clone());
                self.finished.insert(data.request_id.clone());
                let incomplete = self.showed.remove(&data.request_id);
                self.retry_seen.remove(&data.request_id);
                if self.final_reply_only {
                    match &data.outcome {
                        ModelOutputOutcome::Completed {
                            finish_reason: FinishReason::ToolUse,
                        }
                        | ModelOutputOutcome::Failed { .. }
                        | ModelOutputOutcome::Cancelled { .. } => {
                            self.route_request(&data.request_id, TextRoute::Progress)?
                        }
                        // A request that ended normally is the candidate
                        // reply: the turn can still ask again, fail or be
                        // cancelled before it ends.
                        ModelOutputOutcome::Completed { .. } => {}
                    }
                }
                self.end_lines()?;
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
            EventContent::TurnStart(data) => {
                self.turn = Some(data.turn_id);
            }
            EventContent::InputConsumed(data) => {
                let input = data.input_id;
                if let Some(turn) = &self.turn {
                    self.consumed.insert(input.clone(), turn.clone());
                }
                let accepted = match &self.reply {
                    Reply::Accepted { input_id } if *input_id == input => Some(input_id.clone()),
                    _ => None,
                };
                if let (Some(input_id), Some(turn)) = (accepted, self.turn.clone()) {
                    self.activate(input_id, turn)?;
                }
            }
            EventContent::InputDeleted(data) => {
                if !self.final_reply_only {
                    return Ok(());
                }
                let input = data.input_id;
                let submitted = matches!(
                    &self.reply,
                    Reply::Accepted { input_id } | Reply::ActiveTurn { input_id, .. }
                        if *input_id == input
                );
                if submitted {
                    self.close_reply()?;
                    self.record("The submitted input was deleted before its turn ended")?;
                    self.input_deleted = true;
                } else if matches!(self.reply, Reply::Submitted { .. }) {
                    self.deleted.insert(input);
                }
            }
            EventContent::TurnEnd(data) => {
                let own = matches!(
                    &self.reply,
                    Reply::ActiveTurn { turn_id, .. } if *turn_id == data.turn_id
                );
                if self.final_reply_only {
                    if own {
                        self.settle(&data.outcome)?;
                    } else if matches!(self.reply, Reply::Submitted { .. } | Reply::Accepted { .. })
                    {
                        self.ended
                            .insert(data.turn_id.clone(), data.outcome.clone());
                    }
                }
                if self.turn.as_deref() == Some(data.turn_id.as_str()) {
                    self.turn = None;
                }
                if own || !matches!(self.reply, Reply::ActiveTurn { .. }) {
                    self.finish_text()?;
                }
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
                if own || !matches!(self.reply, Reply::ActiveTurn { .. }) {
                    self.requests.clear();
                }
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
