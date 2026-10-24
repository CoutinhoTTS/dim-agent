//! Questions on the client side (Questions design A.5): which `ask_user`
//! calls of the attached Session wait for an answer, what they ask, the
//! `session.question.respond` params of an answer, and what a reply means.
//! Nothing here reads the terminal or prints, so the line mode in [`line`]
//! and a full-screen client share it.
//!
//! A call waits from its `step/tool-call` until its batch's `step/tool-result`
//! (the pending-question rule). Events of one branch arrive live and from
//! `session.history`, in either order and possibly twice, so the projection
//! reads each event once by its identity:
//!
//! ```text
//! step/tool-call {id: e41, request_id: r1, calls: [ask_user c1]}
//!     ──> pending (branch, r1, c1)
//! step/tool-result {call_event_id: e41, request_id: r1, results: [c1]}
//!     ──> settled: answered or cancelled
//! the same events again, or the result first ──> no change
//! ```
//!
//! A call id may come back in a later model request, so a call is named by
//! its branch, the `request_id` of its batch and its call id, both when it
//! settles and when it is answered.
pub mod line;

use crate::client::RpcError;
use dim_agent_types::{ToolCallData, ToolResultData};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::{HashSet, VecDeque};

pub const TOOL: &str = "ask_user";
pub const RESPOND: &str = "session.question.respond";
const MAX_QUESTIONS: usize = 4;
const OPTIONS: std::ops::RangeInclusive<usize> = 2..=6;
const MAX_HEADER_CHARS: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub header: Option<String>,
    pub question: String,
    pub options: Vec<Choice>,
    pub multi_select: bool,
    pub allow_custom: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub label: String,
    pub description: Option<String>,
}

/// A waiting `ask_user` call: answered with its branch, request and call id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub branch_id: String,
    /// The `request_id` of the call's `step/tool-call`.
    pub request_id: String,
    pub call_id: String,
    pub questions: Vec<Question>,
}

/// One question's answer: offered labels and, where allowed, typed text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub question_id: String,
    pub selected: Vec<String>,
    pub custom: Option<String>,
}

/// Reads an `ask_user` input as the handler does (A.1). `None`: the handler
/// rejects it at once, so nothing is asked.
pub fn parse(input: &Map<String, Value>) -> Option<Vec<Question>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        questions: Vec<Raw>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Raw {
        id: Option<String>,
        header: Option<String>,
        question: String,
        options: Vec<Choice>,
        #[serde(default)]
        multi_select: bool,
        #[serde(default = "allowed")]
        allow_custom: bool,
    }
    fn allowed() -> bool {
        true
    }
    let input: Input = serde_json::from_value(Value::Object(input.clone())).ok()?;
    if !(1..=MAX_QUESTIONS).contains(&input.questions.len()) {
        return None;
    }
    let mut ids = HashSet::new();
    input
        .questions
        .into_iter()
        .enumerate()
        .map(|(index, raw)| {
            let id = raw.id.unwrap_or_else(|| (index + 1).to_string());
            let mut labels = HashSet::new();
            let valid = !id.is_empty()
                && ids.insert(id.clone())
                && raw
                    .header
                    .as_ref()
                    .is_none_or(|header| header.chars().count() <= MAX_HEADER_CHARS)
                && !raw.question.trim().is_empty()
                && OPTIONS.contains(&raw.options.len())
                && raw.options.iter().all(|option| {
                    !option.label.trim().is_empty() && labels.insert(option.label.as_str())
                });
            valid.then_some(Question {
                id,
                header: raw.header,
                question: raw.question,
                options: raw.options,
                multi_select: raw.multi_select,
                allow_custom: raw.allow_custom,
            })
        })
        .collect()
}

/// `session.question.respond` params: the answers in question order, or a
/// decline when `answers` is `None`.
pub fn respond(session_id: &str, call: &Call, answers: Option<&[Answer]>) -> Value {
    let mut params = json!({"sessionId":session_id,"branchId":call.branch_id,
        "requestId":call.request_id,"callId":call.call_id});
    match answers {
        Some(answers) => {
            params["answers"] = answers
                .iter()
                .map(|answer| {
                    let mut value =
                        json!({"questionId":answer.question_id,"selected":answer.selected});
                    if let Some(custom) = &answer.custom {
                        value["custom"] = json!(custom);
                    }
                    value
                })
                .collect();
        }
        None => params["declined"] = json!(true),
    }
    params
}

/// What a `session.question.respond` reply means for the prompt (A.3).
#[derive(Debug, PartialEq)]
pub enum Reply {
    /// The waiting call took the answer.
    Accepted,
    /// Nothing waits under that key: not registered yet, or already settled.
    /// The batch's `step/tool-result` tells which.
    NotPending,
    /// The answers do not fit the call's questions; the call keeps waiting.
    Rejected(String),
    /// Any other failure; the answer may be sent again.
    Failed(String),
}

pub fn reply(result: &std::result::Result<Value, RpcError>) -> Reply {
    match result {
        Ok(_) => Reply::Accepted,
        Err(error) => match error
            .data
            .as_ref()
            .and_then(|data| data.get("appError"))
            .and_then(Value::as_str)
        {
            Some("question_not_pending") => Reply::NotPending,
            Some("invalid_params") => Reply::Rejected(error.message.clone()),
            _ => Reply::Failed(error.to_string()),
        },
    }
}

/// A call whose batch settled while it was pending.
#[derive(Debug, PartialEq)]
pub struct Settled {
    pub call: Call,
    /// Answered or declined; otherwise cancelled, reloaded or not executed.
    pub answered: bool,
}

/// The waiting `ask_user` calls of one Session's branches.
#[derive(Default)]
pub struct Pending {
    calls: VecDeque<Call>,
    /// `step/tool-call` events already read, by branch and event id.
    seen: HashSet<(String, String)>,
    /// Batches whose `step/tool-result` was read, by branch and call event.
    settled: HashSet<(String, String)>,
}

impl Pending {
    /// Waiting calls in arrival order.
    pub fn calls(&self) -> impl Iterator<Item = &Call> {
        self.calls.iter()
    }

    /// Reads one event of `branch_id`, live or from history.
    pub fn event(&mut self, branch_id: &str, event: &Value) -> Vec<Settled> {
        if event["type"] != "step" {
            return Vec::new();
        }
        match event["name"].as_str() {
            Some("tool-call") => {
                if let Some(id) = event["id"].as_str()
                    && let Ok(data) = serde_json::from_value(event["data"].clone())
                {
                    self.asked(branch_id, id, &data);
                }
                Vec::new()
            }
            Some("tool-result") => serde_json::from_value(event["data"].clone())
                .map(|data| self.answered(branch_id, &data))
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// The TUI's wire boundary already decoded the batch.
    pub fn asked(&mut self, branch_id: &str, event_id: &str, data: &ToolCallData) {
        let key = (branch_id.to_owned(), event_id.to_owned());
        if self.settled.contains(&key) || !self.seen.insert(key) {
            return;
        }
        for call in data.calls.iter().filter(|call| call.name == TOOL) {
            if let Some(questions) = parse(&call.input) {
                self.calls.push_back(Call {
                    branch_id: branch_id.to_owned(),
                    request_id: data.request_id.clone(),
                    call_id: call.id.clone(),
                    questions,
                });
            }
        }
    }

    pub fn answered(&mut self, branch_id: &str, data: &ToolResultData) -> Vec<Settled> {
        // Results without a call event close calls that were never dispatched.
        let Some(event_id) = &data.call_event_id else {
            return Vec::new();
        };
        self.settled
            .insert((branch_id.to_owned(), event_id.clone()));
        let mut settled = Vec::new();
        for result in &data.results {
            let position = self.calls.iter().position(|call| {
                call.branch_id == branch_id
                    && call.request_id == data.request_id
                    && call.call_id == result.call_id
            });
            if let Some(call) = position.and_then(|index| self.calls.remove(index)) {
                settled.push(Settled {
                    call,
                    answered: result.result.is_error != Some(true),
                });
            }
        }
        settled
    }

    /// A successful respond closes this client's prompt before the batch settles.
    pub fn remove(&mut self, call: &Call) {
        if let Some(index) = self.calls.iter().position(|pending| pending == call) {
            self.calls.remove(index);
        }
    }
}
