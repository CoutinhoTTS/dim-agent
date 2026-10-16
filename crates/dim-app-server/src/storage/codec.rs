use super::sqlite::error;
use dim_agent_runtime::core::TraceStep;
use dim_agent_runtime::{Result, StoredEvent, TimelineTrace};
use dim_agent_types::{
    AgentEvent, EventDraft, EventKind, Message, ModelRequest, SystemPrompt, ToolDefinition,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireEvent {
    pub event: Value,
    refs: Option<References>,
    trace: Option<WireTrace>,
    pub batch_end: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct References {
    messages: Vec<String>,
    tools: Option<String>,
    system: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDraft {
    event: Value,
    refs: Option<References>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireTrace {
    event_id: String,
    original: WireDraft,
    steps: Vec<WireStep>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireStep {
    handler: String,
    result: Option<WireDraft>,
}

impl References {
    fn hashes(&self) -> impl Iterator<Item = &str> {
        self.messages
            .iter()
            .chain(&self.tools)
            .chain(&self.system)
            .map(String::as_str)
    }
}
impl WireEvent {
    /// The content rows this event and its before trace reference.
    pub(super) fn references(&self) -> impl Iterator<Item = &str> {
        let drafts = self.trace.iter().flat_map(|trace| {
            std::iter::once(&trace.original)
                .chain(trace.steps.iter().filter_map(|step| step.result.as_ref()))
        });
        self.refs
            .iter()
            .chain(drafts.filter_map(|draft| draft.refs.as_ref()))
            .flat_map(References::hashes)
    }
}

enum Content {
    Message(Arc<Message>),
    Tools(Arc<Vec<ToolDefinition>>),
    System(Arc<SystemPrompt>),
}
#[derive(Default)]
pub(super) struct ContentCache(HashMap<String, Content>);

pub(super) fn hash(kind: &str, body: &str) -> String {
    format!("{:x}", Sha256::digest(format!("{kind}\0{body}")))
}
fn store_content(
    connection: &Connection,
    session: &str,
    kind: &str,
    value: &impl Serialize,
) -> Result<String> {
    let body = serde_json::to_string(value).map_err(error)?;
    let hash = hash(kind, &body);
    connection
        .execute(
            "INSERT OR IGNORE INTO timeline_content VALUES (?1, ?2, ?3, ?4)",
            params![session, hash, kind, body],
        )
        .map_err(error)?;
    Ok(hash)
}
fn request(kind: &EventKind) -> Option<&Arc<ModelRequest>> {
    match kind {
        EventKind::ContextPrepare(data) => Some(&data.request),
        EventKind::ModelCall(data) => Some(&data.request),
        _ => None,
    }
}
fn encode_refs(
    connection: &Connection,
    session: &str,
    kind: &EventKind,
    event: &mut Value,
) -> Result<Option<References>> {
    let Some(request) = request(kind) else {
        return Ok(None);
    };
    let references = References {
        messages: request
            .messages
            .iter()
            .map(|message| store_content(connection, session, "message", message))
            .collect::<Result<_>>()?,
        tools: request
            .tools
            .as_ref()
            .map(|tools| store_content(connection, session, "tools", tools))
            .transpose()?,
        system: request
            .system
            .as_ref()
            .map(|system| store_content(connection, session, "system", system))
            .transpose()?,
    };
    let data = event["data"]["request"]
        .as_object_mut()
        .ok_or_else(|| error("model request is missing in event encoding"))?;
    data.insert("messages".into(), Value::Array(vec![]));
    data.remove("tools");
    data.remove("system");
    Ok(Some(references))
}
fn encode_draft(connection: &Connection, session: &str, draft: &EventDraft) -> Result<WireDraft> {
    let mut event = serde_json::to_value(draft).map_err(error)?;
    let refs = encode_refs(connection, session, &draft.kind, &mut event)?;
    Ok(WireDraft { event, refs })
}
pub(super) fn encode(
    connection: &Connection,
    session: &str,
    record: &StoredEvent,
) -> Result<WireEvent> {
    let mut event = serde_json::to_value(&record.event).map_err(error)?;
    let refs = encode_refs(connection, session, &record.event.kind, &mut event)?;
    let trace = record
        .trace
        .as_ref()
        .map(|trace| {
            Ok::<_, dim_agent_runtime::AgentError>(WireTrace {
                event_id: trace.event_id.clone(),
                original: encode_draft(connection, session, &trace.original)?,
                steps: trace
                    .steps
                    .iter()
                    .map(|step| {
                        Ok(WireStep {
                            handler: step.handler.clone(),
                            result: step
                                .result
                                .as_ref()
                                .map(|draft| encode_draft(connection, session, draft))
                                .transpose()?,
                        })
                    })
                    .collect::<Result<_>>()?,
            })
        })
        .transpose()?;
    Ok(WireEvent {
        event,
        refs,
        trace,
        batch_end: record.batch_end,
    })
}
impl ContentCache {
    fn load(
        &mut self,
        connection: &Connection,
        session: &str,
        reference: &str,
    ) -> Result<&Content> {
        if !self.0.contains_key(reference) {
            let (kind, body): (String, String) = connection
                .query_row(
                    "SELECT kind, body FROM timeline_content WHERE session_id = ?1 AND hash = ?2",
                    params![session, reference],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(error)?
                .ok_or_else(|| error(format!("missing request content {reference}")))?;
            if hash(&kind, &body) != reference {
                return Err(error(format!("request content hash mismatch {reference}")));
            }
            let content = match kind.as_str() {
                "message" => {
                    Content::Message(Arc::new(serde_json::from_str(&body).map_err(error)?))
                }
                "tools" => Content::Tools(Arc::new(serde_json::from_str(&body).map_err(error)?)),
                "system" => Content::System(Arc::new(serde_json::from_str(&body).map_err(error)?)),
                _ => return Err(error(format!("unknown request content kind {kind}"))),
            };
            self.0.insert(reference.into(), content);
        }
        Ok(&self.0[reference])
    }
    fn restore(
        &mut self,
        connection: &Connection,
        session: &str,
        kind: &mut EventKind,
        refs: Option<References>,
    ) -> Result<()> {
        let target = match kind {
            EventKind::ContextPrepare(data) => Some(&mut data.request),
            EventKind::ModelCall(data) => Some(&mut data.request),
            _ => None,
        };
        match (target, refs) {
            (Some(target), Some(refs)) => {
                let target = Arc::make_mut(target);
                target.messages = refs
                    .messages
                    .iter()
                    .map(
                        |reference| match self.load(connection, session, reference)? {
                            Content::Message(value) => Ok(value.clone()),
                            _ => Err(error("request message references wrong content kind")),
                        },
                    )
                    .collect::<Result<_>>()?;
                target.tools = refs
                    .tools
                    .map(
                        |reference| match self.load(connection, session, &reference)? {
                            Content::Tools(value) => Ok(value.clone()),
                            _ => Err(error("request tools reference wrong content kind")),
                        },
                    )
                    .transpose()?;
                target.system = refs
                    .system
                    .map(
                        |reference| match self.load(connection, session, &reference)? {
                            Content::System(value) => Ok(value.clone()),
                            _ => Err(error("request system references wrong content kind")),
                        },
                    )
                    .transpose()?;
                Ok(())
            }
            (None, None) => Ok(()),
            _ => Err(error("event request references do not match event kind")),
        }
    }
    fn draft(
        &mut self,
        connection: &Connection,
        session: &str,
        wire: WireDraft,
    ) -> Result<EventDraft> {
        let mut draft: EventDraft = serde_json::from_value(wire.event).map_err(error)?;
        self.restore(connection, session, &mut draft.kind, wire.refs)?;
        Ok(draft)
    }
    pub fn decode(
        &mut self,
        connection: &Connection,
        session: &str,
        wire: WireEvent,
    ) -> Result<StoredEvent> {
        let mut event: AgentEvent = serde_json::from_value(wire.event).map_err(error)?;
        self.restore(connection, session, &mut event.kind, wire.refs)?;
        let trace = wire
            .trace
            .map(|trace| {
                Ok::<_, dim_agent_runtime::AgentError>(TimelineTrace {
                    event_id: trace.event_id,
                    original: self.draft(connection, session, trace.original)?,
                    steps: trace
                        .steps
                        .into_iter()
                        .map(|step| {
                            Ok(TraceStep {
                                handler: step.handler,
                                result: step
                                    .result
                                    .map(|draft| self.draft(connection, session, draft))
                                    .transpose()?,
                            })
                        })
                        .collect::<Result<_>>()?,
                })
            })
            .transpose()?;
        Ok(StoredEvent {
            event: Arc::new(event),
            trace,
            batch_end: wire.batch_end,
        })
    }
}
