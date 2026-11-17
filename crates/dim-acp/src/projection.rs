//! Presentation derived only from committed product facts and native progress.

use agent_client_protocol::schema::v1 as acp;
use dim_agent_types::{
    CompactionTrigger, ContentBlock, ContextCompactionResult, ExecStatus, ImageSource, ToolContent,
    UserInput, markup,
};
use dim_client::wire::{Event, EventContent, Progress};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
struct TextCursor {
    committed: u64,
    emitted: u64,
    pending: Option<(u64, String)>,
}

impl TextCursor {
    fn committed(&mut self, text: &str) -> String {
        let start = self.committed;
        self.committed += text.chars().count() as u64;
        let mut output = self.append(start, text);
        if let Some((start, text)) = self.pending.take() {
            if start <= self.emitted {
                output.push_str(&self.append(start, &text));
            } else {
                self.pending = Some((start, text));
            }
        }
        output
    }

    fn progress(&mut self, start: u64, text: String) -> String {
        if start > self.emitted {
            self.pending = Some((start, text));
            String::new()
        } else {
            self.append(start, &text)
        }
    }

    fn append(&mut self, start: u64, text: &str) -> String {
        let end = start + text.chars().count() as u64;
        if end <= self.emitted {
            return String::new();
        }
        let suffix: String = text.chars().skip((self.emitted - start) as usize).collect();
        self.emitted = end;
        suffix
    }
}

#[derive(Default)]
pub(crate) struct Projection {
    pub(crate) through: u64,
    pub(crate) turn: Option<String>,
    pub(crate) silent_inputs: HashSet<String>,
    inputs: HashMap<String, UserInput>,
    texts: HashMap<String, TextCursor>,
    thoughts: HashMap<String, TextCursor>,
    tools: HashSet<String>,
    compactions: HashMap<u64, bool>,
    closed_requests: HashSet<String>,
}

pub(crate) fn tool_id(branch: &str, request: &str, call: &str) -> String {
    json!([branch, request, call]).to_string()
}

/// A message's `_meta`: the `lody` extension it carries, with the turn that
/// produced it.
fn meta(lody: Option<Value>, turn: Option<&str>) -> Option<acp::Meta> {
    let mut lody = match (lody, turn) {
        (Some(Value::Object(lody)), _) => lody,
        (Some(_), _) => unreachable!("a lody extension is an object"),
        (None, None) => return None,
        (None, Some(_)) => acp::Meta::new(),
    };
    if let Some(turn) = turn {
        lody.insert("turnId".into(), json!(turn));
    }
    let mut meta = acp::Meta::new();
    meta.insert("lody".into(), Value::Object(lody));
    Some(meta)
}

/// One text block as tool call content.
fn text_content(text: &str) -> acp::ToolCallContent {
    acp::ToolCallContent::from(acp::ContentBlock::Text(acp::TextContent::new(text)))
}

/// An image block. The session may store the image as an attachment, whose id
/// rides in `_meta` and whose bytes the gateway sends in `data`; the block
/// carries an empty `data` until then, and an image naming neither is dropped.
fn image(source: &ImageSource) -> Option<acp::ContentBlock> {
    let image = match (&source.file_id, &source.data) {
        (Some(file_id), _) => {
            let mut meta = acp::Meta::new();
            meta.insert("fileId".into(), json!(file_id));
            acp::ImageContent::new("", &source.media_type).meta(meta)
        }
        (None, Some(data)) => acp::ImageContent::new(data, &source.media_type),
        (None, None) => return None,
    };
    Some(acp::ContentBlock::Image(image))
}

/// What a tool message says besides its status: the content it produced, the
/// output it returns as data, and the `lody` extension it carries.
#[derive(Default)]
struct ToolFields {
    content: Option<Vec<acp::ToolCallContent>>,
    raw_output: Option<Value>,
    lody: Option<Value>,
}

impl ToolFields {
    fn content(mut self, content: Vec<acp::ToolCallContent>) -> Self {
        self.content = Some(content);
        self
    }

    fn raw_output(mut self, raw_output: Value) -> Self {
        self.raw_output = Some(raw_output);
        self
    }

    fn lody(mut self, lody: Value) -> Self {
        self.lody = Some(lody);
        self
    }
}

/// The messages one fact produces, each serialized once with the turn that
/// produced it already in `_meta`.
#[derive(Default)]
struct Messages {
    turn: Option<String>,
    updates: Vec<Value>,
}

impl Messages {
    fn new(turn: Option<String>) -> Self {
        Self {
            turn,
            updates: Vec::new(),
        }
    }

    fn push(&mut self, update: acp::SessionUpdate) {
        self.updates
            .push(serde_json::to_value(update).expect("an ACP session update is JSON"));
    }

    /// A new tool call: the crate omits a field holding that field's own
    /// default while the literal always wrote both, so `call_message` spells
    /// them out. It is the only write a serialized message still takes.
    fn push_call(&mut self, call: acp::ToolCall) {
        self.updates.push(call_message(call));
    }

    fn meta(&self, lody: Option<Value>) -> Option<acp::Meta> {
        meta(lody, self.turn.as_deref())
    }

    /// A streamed chunk of text: one of the three chunk variants.
    fn chunk(&mut self, kind: fn(acp::ContentChunk) -> acp::SessionUpdate, text: String) {
        if !text.is_empty() {
            let meta = self.meta(None);
            let block = acp::ContentBlock::Text(acp::TextContent::new(text));
            self.push(kind(acp::ContentChunk::new(block).meta(meta)));
        }
    }

    fn image(&mut self, kind: fn(acp::ContentChunk) -> acp::SessionUpdate, source: &ImageSource) {
        if let Some(block) = image(source) {
            let meta = self.meta(None);
            self.push(kind(acp::ContentChunk::new(block).meta(meta)));
        }
    }
}

/// The marker a loaded Skill's block starts with, after the operator's own
/// text: the gateway shows that block as one line, as every client does.
const SKILL_BLOCK: &str = "<dim-skill name=\"";

/// `text`, a chunk of the operator's own words, with each `/skill:<name>`
/// token delimited so an editor draws it as one unit rather than as prose.
/// The text itself is unchanged: the gateway repeats what the operator sent.
fn with_tokens(text: &str) -> String {
    let tokens: Vec<_> = dim_agent_types::skill_token::tokens(text).collect();
    if tokens.is_empty() {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut at = 0;
    for token in tokens {
        out.push_str(&text[at..token.start]);
        out.push('`');
        out.push_str(&text[token.start..token.end]);
        out.push('`');
        at = token.end;
    }
    out.push_str(&text[at..]);
    out
}

fn loaded_skill(index: usize, text: &str) -> Option<String> {
    if markup::runtime_element(index, text) != Some("dim-skill") {
        return None;
    }
    let rest = text.strip_prefix(SKILL_BLOCK)?;
    let name = rest.split('"').next()?;
    (!name.is_empty()).then(|| {
        name.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
    })
}

/// A tool call's own content, as the protocol's own blocks: the tool's output
/// with an image it names still to be read from the session's attachment.
fn tool_content(content: &ToolContent) -> Vec<acp::ToolCallContent> {
    match content {
        ToolContent::Text(_) => vec![text_content(&preview(content))],
        ToolContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => {
                    Some(text_content(&preview(&ToolContent::Text(text.clone()))))
                }
                ContentBlock::Image { source } => image(source).map(acp::ToolCallContent::from),
            })
            .collect(),
    }
}

fn preview(content: &ToolContent) -> String {
    let text = match content {
        ToolContent::Text(text) => text.clone(),
        ToolContent::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::Image { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    };
    let mut result = String::new();
    for ch in text.chars() {
        if result.len() + ch.len_utf8() > 16 * 1024 {
            result.push_str("\n[truncated]");
            break;
        }
        result.push(ch);
    }
    result
}

/// A tool call message: the crate omits a field holding that field's own
/// default, and the gateway spells out both so a client reads the same
/// message it read before the projection was typed.
fn call_message(call: acp::ToolCall) -> Value {
    let mut message = serde_json::to_value(acp::SessionUpdate::ToolCall(call))
        .expect("an ACP session update is JSON");
    if message.get("kind").is_none() {
        message["kind"] = json!("other");
    }
    if message.get("status").is_none() {
        message["status"] = json!("pending");
    }
    message
}

impl Projection {
    pub(crate) fn active_output(&mut self, output: &dim_agent_types::ActiveOutput) {
        for (cursors, count) in [
            (&mut self.texts, output.text_characters),
            (&mut self.thoughts, output.thinking_characters),
        ] {
            let cursor = cursors.entry(output.request_id.clone()).or_default();
            cursor.committed = count;
            cursor.emitted = count;
        }
    }

    pub(crate) fn pending_input(&mut self, input: dim_agent_types::PendingInput) {
        if !input.applied
            && let Some(user) = input.user
        {
            self.inputs.insert(input.input_id, user);
        }
    }

    pub(crate) fn pending_exec(&mut self, task: &dim_agent_types::PendingExec) -> Value {
        let id = json!(["background", task.task_id]).to_string();
        let mut messages = Messages::default();
        self.tool(
            id,
            &task.description,
            Value::Null,
            acp::ToolCallStatus::InProgress,
            ToolFields::default().lody(json!({"task":{"version":1,"taskId":task.task_id,"kind":"background","status":"in_progress","description":task.description,"parentToolCallId":tool_id("main", &task.request_id, &task.call_id),"startedAtEpochSeconds":task.started_at as f64 / 1000.}})),
            &mut messages,
        );
        messages.updates.pop().expect("task update")
    }

    /// Announces `id`, or updates it when it was already announced.
    fn tool(
        &mut self,
        id: String,
        name: &str,
        input: Value,
        status: acp::ToolCallStatus,
        fields: ToolFields,
        messages: &mut Messages,
    ) {
        let meta = messages.meta(fields.lody);
        if self.tools.insert(id.clone()) {
            messages.push_call(
                acp::ToolCall::new(id, name)
                    .name(name)
                    .status(status)
                    .content(fields.content.unwrap_or_default())
                    .raw_input(input)
                    .raw_output(fields.raw_output)
                    .meta(meta),
            );
            return;
        }
        messages.push(acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(
                id,
                acp::ToolCallUpdateFields::new()
                    .status(status)
                    .raw_input(input)
                    .content(fields.content)
                    .raw_output(fields.raw_output),
            )
            .meta(meta),
        ));
    }

    fn tool_update(
        id: String,
        status: acp::ToolCallStatus,
        fields: ToolFields,
        messages: &mut Messages,
    ) {
        let meta = messages.meta(fields.lody);
        messages.push(acp::SessionUpdate::ToolCallUpdate(
            acp::ToolCallUpdate::new(
                id,
                acp::ToolCallUpdateFields::new()
                    .status(status)
                    .content(fields.content)
                    .raw_output(fields.raw_output),
            )
            .meta(meta),
        ));
    }

    pub(crate) fn event(&mut self, event: &Event) -> Vec<Value> {
        let turn = match &event.content {
            EventContent::TurnStart(data) => Some(data.turn_id.clone()),
            EventContent::Text(data) => Some(data.turn_id.clone()),
            EventContent::Thinking(data) => Some(data.turn_id.clone()),
            EventContent::ToolUseStart(data) => Some(data.turn_id.clone()),
            EventContent::ToolUse(data) => Some(data.turn_id.clone()),
            EventContent::ToolCall(data) => Some(data.turn_id.clone()),
            EventContent::ToolStart(data) => Some(data.turn_id.clone()),
            EventContent::ToolCompleted(data) => Some(data.turn_id.clone()),
            EventContent::ToolResult(data) => Some(data.turn_id.clone()),
            EventContent::ExecStarted(data) => Some(data.turn_id.clone()),
            EventContent::CompactionStart(data) => data.turn_id.clone(),
            EventContent::Compaction(data) => data.turn_id.clone(),
            _ => self.turn.clone(),
        };
        let mut messages = Messages::new(turn);
        match &event.content {
            EventContent::TurnStart(data) => self.turn = Some(data.turn_id.clone()),
            EventContent::Input(data) => {
                if let Some(user) = &data.user {
                    self.inputs.insert(event.id.clone(), user.clone());
                }
            }
            EventContent::InputConsumed(data) => {
                let user = self.inputs.remove(&data.input_id);
                let silent = self.silent_inputs.remove(&data.input_id);
                if !silent && let Some(user) = user {
                    for (index, content) in user.content.into_iter().enumerate() {
                        match content {
                            // The operator's words, and a loaded Skill as one line
                            // naming it. Context a hook appended is the model's, not
                            // theirs, so the editor's message does not carry it.
                            ContentBlock::Text { text } => match loaded_skill(index, &text) {
                                Some(name) => messages.chunk(
                                    acp::SessionUpdate::UserMessageChunk,
                                    format!("\nSkill loaded: {name}"),
                                ),
                                None if markup::runtime_context(index, &text) => {}
                                None => messages.chunk(
                                    acp::SessionUpdate::UserMessageChunk,
                                    with_tokens(&text),
                                ),
                            },
                            ContentBlock::Image { source } => {
                                messages.image(acp::SessionUpdate::UserMessageChunk, &source)
                            }
                        }
                    }
                }
            }
            EventContent::InputDeleted(data) => {
                self.inputs.remove(&data.input_id);
                self.silent_inputs.remove(&data.input_id);
            }
            EventContent::Text(data) => messages.chunk(
                acp::SessionUpdate::AgentMessageChunk,
                self.texts
                    .entry(data.request_id.clone())
                    .or_default()
                    .committed(&data.delta),
            ),
            EventContent::Thinking(data) => messages.chunk(
                acp::SessionUpdate::AgentThoughtChunk,
                self.thoughts
                    .entry(data.request_id.clone())
                    .or_default()
                    .committed(&data.delta),
            ),
            EventContent::ToolUseStart(data) => self.tool(
                tool_id("main", &data.request_id, &data.call_id),
                &data.name,
                Value::Null,
                acp::ToolCallStatus::Pending,
                ToolFields::default(),
                &mut messages,
            ),
            EventContent::ToolUse(data) => self.tool(
                tool_id("main", &data.request_id, &data.call.id),
                &data.call.name,
                json!(data.call.input),
                acp::ToolCallStatus::Pending,
                ToolFields::default(),
                &mut messages,
            ),
            EventContent::ToolCall(data) => {
                for call in &data.calls {
                    self.tool(
                        tool_id("main", &data.request_id, &call.id),
                        &call.name,
                        json!(call.input),
                        acp::ToolCallStatus::Pending,
                        ToolFields::default(),
                        &mut messages,
                    );
                }
            }
            EventContent::ToolStart(data) => Self::tool_update(
                tool_id("main", &data.request_id, &data.call_id),
                acp::ToolCallStatus::InProgress,
                ToolFields::default(),
                &mut messages,
            ),
            EventContent::ToolResult(data) => {
                for call in &data.results {
                    let id = tool_id("main", &data.request_id, &call.call_id);
                    if !self.tools.contains(&id) {
                        self.tool(
                            id.clone(),
                            &call.call_id,
                            Value::Null,
                            acp::ToolCallStatus::Pending,
                            ToolFields::default(),
                            &mut messages,
                        );
                    }
                    Self::tool_update(
                        tool_id("main", &data.request_id, &call.call_id),
                        if call.result.is_error == Some(true) {
                            acp::ToolCallStatus::Failed
                        } else {
                            acp::ToolCallStatus::Completed
                        },
                        ToolFields::default().content(tool_content(&call.result.content)),
                        &mut messages,
                    );
                }
            }
            EventContent::ToolCompleted(data) => Self::tool_update(
                tool_id("main", &data.request_id, &data.call_id),
                acp::ToolCallStatus::InProgress,
                ToolFields::default().content(tool_content(&data.result.content)),
                &mut messages,
            ),
            EventContent::ExecStarted(data) => {
                let id = json!(["background", data.task_id]).to_string();
                self.tool(
                    id.clone(),
                    &data.description,
                    Value::Null,
                    acp::ToolCallStatus::InProgress,
                    ToolFields::default().lody(json!({"task":{"version":1,"taskId":data.task_id,"kind":"background","status":"in_progress","description":data.description,"parentToolCallId":tool_id("main",&data.request_id,&data.call_id),"startedAtEpochSeconds":event.create_at as f64/1000.}})),
                    &mut messages,
                );
            }
            EventContent::ExecEnded(data) => {
                let id = json!(["background", data.task_id]).to_string();
                let (status, state) = if matches!(data.status, ExecStatus::Completed) {
                    (acp::ToolCallStatus::Completed, "completed")
                } else {
                    (acp::ToolCallStatus::Failed, "failed")
                };
                let summary = preview(&data.result.content);
                self.tool(
                    id,
                    "Background task",
                    Value::Null,
                    status,
                    ToolFields::default()
                        .content(vec![text_content(&summary)])
                        .raw_output(json!({"status":data.status}))
                        .lody(json!({"task":{"version":1,"taskId":data.task_id,"kind":"background","status":state,"endedAtEpochSeconds":event.create_at as f64/1000.,"summary":summary}})),
                    &mut messages,
                );
            }
            EventContent::CompactionStart(data) => {
                let automatic = !matches!(data.trigger, CompactionTrigger::Manual);
                self.compactions.insert(event.sequence, automatic);
                self.tool(
                    json!(["compaction", event.sequence]).to_string(),
                    "Context compaction",
                    Value::Null,
                    acp::ToolCallStatus::InProgress,
                    ToolFields::default().lody(json!({"activity":{"version":1,"kind":"context_compaction","automatic":automatic}})),
                    &mut messages,
                );
            }
            EventContent::CompactionSummary(data) => {
                let automatic = data.start.and_then(|start| self.compactions.remove(&start));
                let mut activity = json!({"version":1,"kind":"context_compaction"});
                if let Some(automatic) = automatic {
                    activity["automatic"] = json!(automatic);
                }
                self.tool(
                    json!(["compaction", data.start.unwrap_or(event.sequence)]).to_string(),
                    "Context compaction",
                    Value::Null,
                    acp::ToolCallStatus::Completed,
                    ToolFields::default().lody(json!({"activity":activity})),
                    &mut messages,
                );
            }
            EventContent::Compaction(data) => {
                let automatic = data.start.and_then(|start| self.compactions.remove(&start));
                let (status, diagnostics, reason) = match &data.outcome {
                    ContextCompactionResult::Completed { diagnostics, .. } => (
                        acp::ToolCallStatus::Completed,
                        Some(diagnostics.as_ref()),
                        None,
                    ),
                    ContextCompactionResult::Skipped {
                        reason,
                        diagnostics,
                    } => (
                        acp::ToolCallStatus::Completed,
                        Some(diagnostics.as_ref()),
                        Some(format!("Skipped: {reason}")),
                    ),
                    ContextCompactionResult::Failed { error, diagnostics } => (
                        acp::ToolCallStatus::Failed,
                        diagnostics.as_deref(),
                        Some(error.clone()),
                    ),
                    ContextCompactionResult::Cancelled { reason } => (
                        acp::ToolCallStatus::Failed,
                        None,
                        Some(format!("Cancelled: {reason}")),
                    ),
                };
                let mut activity = json!({"version":1,"kind":"context_compaction"});
                if let Some(automatic) = automatic {
                    activity["automatic"] = json!(automatic);
                }
                let mut fields = ToolFields::default();
                if let Some(diagnostics) = diagnostics {
                    activity["usedTokensBefore"] = json!(diagnostics.estimated_input_tokens);
                    if let Some(after) = diagnostics.estimated_output_tokens {
                        activity["usedTokensAfter"] = json!(after);
                    }
                    activity["durationMs"] = json!(diagnostics.duration_ms);
                    fields = fields.raw_output(json!(diagnostics));
                }
                if let Some(reason) = reason {
                    if status == acp::ToolCallStatus::Failed {
                        activity["failureReason"] = json!(reason);
                    }
                    fields = fields.content(vec![text_content(&reason)]);
                }
                self.tool(
                    json!(["compaction", data.start.unwrap_or(event.sequence)]).to_string(),
                    "Context compaction",
                    Value::Null,
                    status,
                    fields.lody(json!({"activity":activity})),
                    &mut messages,
                );
            }
            EventContent::TurnEnd(_) => self.turn = None,
            EventContent::ModelEnd(data) => {
                self.closed_requests.insert(data.request_id.clone());
            }
            _ => {}
        }
        self.through = event.sequence;
        messages.updates
    }

    pub(crate) fn progress(&mut self, progress: Progress) -> Vec<Value> {
        let mut messages = Messages::new(self.turn.clone());
        if let Some(id) = progress.request_id {
            if self.closed_requests.contains(&id) {
                return messages.updates;
            }
            messages.chunk(
                acp::SessionUpdate::AgentMessageChunk,
                self.texts
                    .entry(id.clone())
                    .or_default()
                    .progress(progress.text_offset, progress.text),
            );
            messages.chunk(
                acp::SessionUpdate::AgentThoughtChunk,
                self.thoughts
                    .entry(id)
                    .or_default()
                    .progress(progress.thinking_offset, progress.thinking),
            );
        }
        messages.updates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(value: Value) -> Event {
        serde_json::from_value(value).unwrap()
    }

    fn replay(blocks: Value) -> Vec<Value> {
        let mut projection = Projection::default();
        assert!(
            projection
                .event(&event(json!({
                    "id":"input-1","sequence":9,"create_at":1,
                    "type":"input","name":"message",
                    "data":{"user":{"content":blocks}},
                })))
                .is_empty()
        );
        projection.event(&event(json!({
            "id":"consumed-1","sequence":10,"create_at":2,
            "type":"input","name":"consumed",
            "data":{"input_id":"input-1"},
        })))
    }

    #[test]
    fn a_loaded_skill_replays_as_one_line() {
        let updates = replay(json!([
            {"type":"text","text":"Explain the change"},
            {"type":"text","text":"<dim-skill name=\"commit\">\n# Commit\nwrite it\n</dim-skill>"},
        ]));
        let texts: Vec<&str> = updates
            .iter()
            .map(|update| update["content"]["text"].as_str().unwrap())
            .collect();
        assert_eq!(texts, ["Explain the change", "\nSkill loaded: commit"]);
    }

    /// A `/skill:<name>` token reaches the editor as one delimited unit, its
    /// text unchanged, so a client draws it as one piece rather than as prose.
    #[test]
    fn a_skill_token_replays_as_one_unit() {
        let updates = replay(json!([
            {"type":"text","text":"please /skill:commit it"},
        ]));
        let texts: Vec<&str> = updates
            .iter()
            .map(|update| update["content"]["text"].as_str().unwrap())
            .collect();
        assert_eq!(texts, ["please `/skill:commit` it"]);

        let untouched = replay(json!([
            {"type":"text","text":"a/skill:x stays prose"},
        ]));
        let texts: Vec<&str> = untouched
            .iter()
            .map(|update| update["content"]["text"].as_str().unwrap())
            .collect();
        assert_eq!(
            texts,
            ["a/skill:x stays prose"],
            "a slash inside a word is not a token"
        );
    }

    #[test]
    fn a_prompt_hooks_context_replays_as_no_chunk() {
        let updates = replay(json!([
            {"type":"text","text":"Explain the change"},
            {"type":"text","text":"<dim-hooks kind=\"prompt-submit\">\nread src/parser.rs\n</dim-hooks>"},
        ]));
        let texts: Vec<&str> = updates
            .iter()
            .map(|update| update["content"]["text"].as_str().unwrap())
            .collect();
        assert_eq!(
            texts,
            ["Explain the change"],
            "context a hook appended is the model's, not the operator's words"
        );
    }

    #[test]
    fn a_skill_marker_in_the_first_block_stays_text() {
        let updates = replay(json!([
            {"type":"text","text":"<dim-skill name=\"commit\">typed by hand"},
        ]));
        let texts: Vec<&str> = updates
            .iter()
            .map(|update| update["content"]["text"].as_str().unwrap())
            .collect();
        assert_eq!(texts, ["<dim-skill name=\"commit\">typed by hand"]);
    }
}
