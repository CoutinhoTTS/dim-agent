//! Presentation derived only from committed product facts and native progress.

use dim_agent_types::{
    CompactionTrigger, ContentBlock, ContextCompactionResult, ExecStatus, ToolContent, UserInput,
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
}

pub(crate) fn tool_id(branch: &str, request: &str, call: &str) -> String {
    json!([branch, request, call]).to_string()
}

fn chunk(kind: &str, text: String, updates: &mut Vec<Value>) {
    if !text.is_empty() {
        updates.push(json!({"sessionUpdate":kind,"content":{"type":"text","text":text}}));
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

impl Projection {
    fn tool(
        &mut self,
        id: String,
        name: &str,
        input: Value,
        status: &str,
        updates: &mut Vec<Value>,
    ) {
        if self.tools.insert(id.clone()) {
            updates.push(json!({"sessionUpdate":"tool_call","toolCallId":id,"title":name,"name":name,"kind":"other","status":status,"rawInput":input}));
        } else {
            updates.push(json!({"sessionUpdate":"tool_call_update","toolCallId":id,"status":status,"rawInput":input}));
        }
    }

    pub(crate) fn event(&mut self, event: &Event) -> Vec<Value> {
        let mut updates = Vec::new();
        let turn = match &event.content {
            EventContent::TurnStart(data) => Some(data.turn_id.clone()),
            _ => self.turn.clone(),
        };
        match &event.content {
            EventContent::TurnStart(data) => self.turn = Some(data.turn_id.clone()),
            EventContent::Input(data) => {
                if let Some(user) = &data.user { self.inputs.insert(event.id.clone(), user.clone()); }
            }
            EventContent::InputConsumed(data) => {
                let user = self.inputs.remove(&data.input_id);
                let silent = self.silent_inputs.remove(&data.input_id);
                if !silent && let Some(user) = user {
                    for content in user.content {
                        match content {
                            ContentBlock::Text { text } => chunk("user_message_chunk", text, &mut updates),
                            ContentBlock::Image { source } => {
                                if let Some(data) = source.data {
                                    updates.push(json!({"sessionUpdate":"user_message_chunk","content":{"type":"image","mimeType":source.media_type,"data":data}}));
                                }
                            }
                        }
                    }
                }
            }
            EventContent::InputDeleted(data) => {
                self.inputs.remove(&data.input_id);
                self.silent_inputs.remove(&data.input_id);
            }
            EventContent::Text(data) => chunk("agent_message_chunk", self.texts.entry(data.request_id.clone()).or_default().committed(&data.delta), &mut updates),
            EventContent::Thinking(data) => chunk("agent_thought_chunk", self.thoughts.entry(data.request_id.clone()).or_default().committed(&data.delta), &mut updates),
            EventContent::ToolUseStart(data) => self.tool(tool_id("main", &data.request_id, &data.call_id), &data.name, Value::Null, "pending", &mut updates),
            EventContent::ToolUse(data) => self.tool(tool_id("main", &data.request_id, &data.call.id), &data.call.name, json!(data.call.input), "pending", &mut updates),
            EventContent::ToolCall(data) => {
                for call in &data.calls { self.tool(tool_id("main", &data.request_id, &call.id), &call.name, json!(call.input), "pending", &mut updates); }
            }
            EventContent::ToolStart(data) => updates.push(json!({"sessionUpdate":"tool_call_update","toolCallId":tool_id("main", &data.request_id, &data.call_id),"status":"in_progress"})),
            EventContent::ToolResult(data) => {
                for call in &data.results {
                    updates.push(json!({"sessionUpdate":"tool_call_update","toolCallId":tool_id("main", &data.request_id, &call.call_id),"status":if call.result.is_error == Some(true) {"failed"} else {"completed"},"content":[{"type":"content","content":{"type":"text","text":preview(&call.result.content)}}]}));
                }
            }
            EventContent::ToolCompleted(data) => updates.push(json!({"sessionUpdate":"tool_call_update","toolCallId":tool_id("main", &data.request_id, &data.call_id),"status":"in_progress","content":[{"type":"content","content":{"type":"text","text":preview(&data.result.content)}}]})),
            EventContent::ExecStarted(data) => {
                let id = json!(["background",data.task_id]).to_string();
                self.tool(id.clone(),&data.description,Value::Null,"in_progress",&mut updates);
                if let Some(update) = updates.last_mut() { update["_meta"] = json!({"lody":{"task":{"version":1,"taskId":data.task_id,"kind":"background","status":"in_progress","description":data.description,"parentToolCallId":tool_id("main",&data.request_id,&data.call_id),"startedAtEpochSeconds":event.create_at as f64/1000.}}}); }
            }
            EventContent::ExecEnded(data) => {
                let id = json!(["background",data.task_id]).to_string();
                let status = if matches!(data.status,ExecStatus::Completed) {"completed"} else {"failed"};
                self.tool(id,"Background task",Value::Null,status,&mut updates);
                if let Some(update) = updates.last_mut() {
                    update["content"] = json!([{"type":"content","content":{"type":"text","text":preview(&data.result.content)}}]);
                    update["_meta"] = json!({"lody":{"task":{"version":1,"taskId":data.task_id,"kind":"background","status":status,"endedAtEpochSeconds":event.create_at as f64/1000.,"summary":preview(&data.result.content)}}});
                    update["rawOutput"] = json!({"status":data.status});
                }
            }
            EventContent::CompactionStart(data) => {
                let automatic = !matches!(data.trigger,CompactionTrigger::Manual);
                self.compactions.insert(event.sequence,automatic);
                self.tool(json!(["compaction",event.sequence]).to_string(),"Context compaction",Value::Null,"in_progress",&mut updates);
                if let Some(update) = updates.last_mut() { update["_meta"] = json!({"lody":{"activity":{"version":1,"kind":"context_compaction","automatic":automatic}}}); }
            }
            EventContent::Compaction(data) => {
                let automatic = data.start.and_then(|start|self.compactions.remove(&start));
                let (status,diagnostics,reason) = match &data.outcome {
                    ContextCompactionResult::Completed { diagnostics,.. } => ("completed",Some(diagnostics.as_ref()),None),
                    ContextCompactionResult::Skipped { reason,diagnostics } => ("completed",Some(diagnostics.as_ref()),Some(format!("Skipped: {reason}"))),
                    ContextCompactionResult::Failed { error,diagnostics } => ("failed",diagnostics.as_deref(),Some(error.clone())),
                    ContextCompactionResult::Cancelled { reason } => ("failed",None,Some(format!("Cancelled: {reason}"))),
                };
                self.tool(json!(["compaction",data.start.unwrap_or(event.sequence)]).to_string(),"Context compaction",Value::Null,status,&mut updates);
                if let Some(update) = updates.last_mut() {
                    let mut activity = json!({"version":1,"kind":"context_compaction"});
                    if let Some(automatic) = automatic { activity["automatic"] = json!(automatic); }
                    if let Some(diagnostics) = diagnostics {
                        activity["usedTokensBefore"] = json!(diagnostics.estimated_input_tokens);
                        if let Some(after) = diagnostics.estimated_output_tokens { activity["usedTokensAfter"] = json!(after); }
                        activity["durationMs"] = json!(diagnostics.duration_ms);
                        update["rawOutput"] = json!(diagnostics);
                    }
                    if let Some(reason) = reason { if status == "failed" { activity["failureReason"] = json!(reason); } update["content"] = json!([{"type":"content","content":{"type":"text","text":reason}}]); }
                    update["_meta"] = json!({"lody":{"activity":activity}});
                }
            }
            EventContent::TurnEnd(_) => self.turn = None,
            _ => {}
        }
        if let Some(turn) = turn {
            for update in &mut updates {
                update["_meta"]["lody"]["turnId"] = json!(turn);
            }
        }
        self.through = event.sequence;
        updates
    }

    pub(crate) fn progress(&mut self, progress: Progress) -> Vec<Value> {
        let mut updates = Vec::new();
        if let Some(id) = progress.request_id {
            chunk(
                "agent_message_chunk",
                self.texts
                    .entry(id.clone())
                    .or_default()
                    .progress(progress.text_offset, progress.text),
                &mut updates,
            );
            chunk(
                "agent_thought_chunk",
                self.thoughts
                    .entry(id)
                    .or_default()
                    .progress(progress.thinking_offset, progress.thinking),
                &mut updates,
            );
        }
        if let Some(turn) = &self.turn {
            for update in &mut updates {
                update["_meta"]["lody"]["turnId"] = json!(turn);
            }
        }
        updates
    }
}
