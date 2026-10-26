use crate::projection::tool_id;
use agent_client_protocol::{Client, ConnectionTo, UntypedMessage};
use dim_agent_types::ApprovalCallData;
use dim_client::question::{self, Answer, Call};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Approval {
    pub session_id: String,
    pub branch_id: String,
    pub request_id: String,
    pub call: ApprovalCallData,
    pub reason: String,
}

pub(crate) fn key(kind: &str, session: &str, branch: &str, request: &str, call: &str) -> String {
    json!([kind, session, branch, request, call]).to_string()
}

pub(crate) async fn approval(
    cx: ConnectionTo<Client>,
    outbox: crate::outbox::Outbox,
    stop: CancellationToken,
    approval: Approval,
) -> Option<Value> {
    let message = UntypedMessage::new("session/request_permission",json!({"sessionId":approval.session_id,"toolCall":{"toolCallId":key("approval",&approval.session_id,&approval.branch_id,&approval.request_id,""),"title":approval.reason,"kind":"other","status":"pending","rawInput":approval.call.input},"options":[{"optionId":"allow_once","name":"Allow once","kind":"allow_once"},{"optionId":"reject_once","name":"Reject once","kind":"reject_once"}]})).ok()?;
    let result: Value = tokio::select! {
        () = stop.cancelled() => return None,
        result = outbox.request(&cx,&message.method,message.params) => result.ok()?,
    };
    let option = result.get("outcome")?;
    if option["outcome"] != "selected" {
        return None;
    }
    let decision = match option["optionId"].as_str()? {
        "allow_once" => "allow",
        "reject_once" => "deny",
        _ => return None,
    };
    Some(
        json!({"sessionId":approval.session_id,"branchId":approval.branch_id,"requestId":approval.request_id,"decision":decision}),
    )
}

pub(crate) async fn question(
    cx: ConnectionTo<Client>,
    outbox: crate::outbox::Outbox,
    stop: CancellationToken,
    session: &str,
    call: &Call,
) -> Option<Value> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (index, question) in call.questions.iter().enumerate() {
        let name = format!("q{index}");
        let choices: Vec<_> = question.options.iter().map(|choice|json!({"const":choice.label,"title":choice.label,"description":choice.description})).collect();
        let mut schema = if question.multi_select {
            json!({"type":"array","items":{"type":"string","oneOf":choices},"minItems":0,"maxItems":question.options.len()})
        } else {
            json!({"type":"string","oneOf":choices})
        };
        schema["title"] = json!(question.header.as_deref().unwrap_or(&question.question));
        schema["description"] = json!(question.question);
        properties.insert(name.clone(), schema);
        if question.allow_custom {
            properties.insert(
                format!("custom{index}"),
                json!({"type":"string","title":format!("Custom answer: {}",question.question)}),
            );
        } else {
            required.push(name);
        }
    }
    let message = UntypedMessage::new("elicitation/create",json!({"mode":"form","sessionId":session,"toolCallId":tool_id(&call.branch_id,&call.request_id,&call.call_id),"message":"Answer the agent's questions","requestedSchema":{"type":"object","properties":properties,"required":required}})).ok()?;
    let result: Value = tokio::select! {
        () = stop.cancelled() => return None,
        result = outbox.request(&cx,&message.method,message.params) => result.ok()?,
    };
    match result["action"].as_str()? {
        "decline" => Some(question::respond(session, call, None)),
        "accept" => {
            let content = result["content"].as_object()?;
            let answers: Option<Vec<_>> = call
                .questions
                .iter()
                .enumerate()
                .map(|(index, question)| {
                    let selection = content.get(&format!("q{index}"));
                    let selected: Vec<String> = match selection {
                        None => Vec::new(),
                        Some(Value::String(value)) if !question.multi_select => vec![value.clone()],
                        Some(Value::Array(values)) if question.multi_select => values
                            .iter()
                            .map(|value| value.as_str().map(str::to_owned))
                            .collect::<Option<_>>()?,
                        _ => return None,
                    };
                    let custom = content
                        .get(&format!("custom{index}"))
                        .and_then(Value::as_str)
                        .filter(|value| !value.trim().is_empty())
                        .map(str::to_owned);
                    if selected
                        .iter()
                        .any(|value| !question.options.iter().any(|choice| &choice.label == value))
                        || (custom.is_some() && !question.allow_custom)
                        || (selected.is_empty() && custom.is_none())
                    {
                        return None;
                    }
                    Some(Answer {
                        question_id: question.id.clone(),
                        selected,
                        custom,
                    })
                })
                .collect();
            Some(question::respond(session, call, Some(&answers?)))
        }
        _ => None,
    }
}
