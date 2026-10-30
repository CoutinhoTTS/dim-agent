use crate::projection::tool_id;
use agent_client_protocol::{Client, ConnectionTo, UntypedMessage};
use dim_agent_types::{
    ApprovalCallData, MEMBER_APPROVAL_ALLOW, MEMBER_APPROVAL_GET, MemberApprovalDetail,
};
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
    rpc: crate::rpc::Rpc,
) -> Option<Value> {
    let args = |decision: &str| json!({"sessionId":approval.session_id,"branchId":approval.branch_id,"requestId":approval.request_id,"decision":decision});
    let member = approval.call.name == MEMBER_APPROVAL_ALLOW;
    let detail_params = json!({"sessionId":approval.session_id,"branchId":approval.branch_id,"requestId":approval.request_id});
    let mut detail = if member {
        Some(
            rpc.typed::<MemberApprovalDetail>(MEMBER_APPROVAL_GET, detail_params.clone())
                .await,
        )
    } else {
        None
    };
    loop {
        let (title, input, can_allow) = match &detail {
            Some(Ok(MemberApprovalDetail::Pending { descriptor, .. })) => (
                format!(
                    "{} / {} / {}: {} — {}",
                    approval.branch_id,
                    descriptor.member.agent_id,
                    descriptor.member.branch_id,
                    descriptor.call.name,
                    descriptor.reason
                ),
                serde_json::to_value(descriptor).ok()?,
                true,
            ),
            Some(Ok(MemberApprovalDetail::Ended)) => return Some(args("deny")),
            Some(Err(error)) => (
                format!("Cannot review the original member action: {error}. Reject this request."),
                json!({"error":error.to_string()}),
                false,
            ),
            None => (
                approval.reason.clone(),
                Value::Object(approval.call.input.clone()),
                true,
            ),
        };
        let mut call = json!({"toolCallId":key("approval",&approval.session_id,&approval.branch_id,&approval.request_id,""),"title":title,"kind":"other","status":"pending","rawInput":input});
        if member {
            call["content"] = json!([{"type":"content","content":{"type":"text","text":serde_json::to_string_pretty(&input).ok()?}}]);
        }
        let mut options =
            vec![json!({"optionId":"reject_once","name":"Reject once","kind":"reject_once"})];
        if can_allow {
            options.insert(
                0,
                json!({"optionId":"allow_once","name":"Allow once","kind":"allow_once"}),
            );
        }
        let message = UntypedMessage::new(
            "session/request_permission",
            json!({"sessionId":approval.session_id,"toolCall":call,"options":options}),
        )
        .ok()?;
        let result = tokio::select! {
            () = stop.cancelled() => return None,
            result = outbox.request(&cx,&message.method,message.params) => match result {
                Ok(result) => result,
                Err(_) => return member.then(|| args("deny")),
            },
        };
        let decision = match result
            .get("outcome")
            .filter(|outcome| outcome["outcome"] == "selected")
            .and_then(|outcome| outcome["optionId"].as_str())
        {
            Some("allow_once") if can_allow => "allow",
            Some("reject_once") => "deny",
            _ => return member.then(|| args("deny")),
        };
        if !member || decision == "deny" {
            return Some(args(decision));
        }
        detail = Some(
            rpc.typed::<MemberApprovalDetail>(MEMBER_APPROVAL_GET, detail_params.clone())
                .await,
        );
        if matches!(detail, Some(Ok(MemberApprovalDetail::Pending { .. }))) {
            return Some(args("allow"));
        }
    }
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
            json!({"type":"array","items":{"anyOf":choices},"minItems":1,"maxItems":question.options.len()})
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
    let result: Option<Value> = tokio::select! {
        () = stop.cancelled() => return None,
        result = outbox.request(&cx,&message.method,message.params) => result.ok(),
    };
    let answers: Option<Vec<_>> = result
        .as_ref()
        .filter(|result| result["action"] == "accept")
        .and_then(|result| result["content"].as_object())
        .and_then(|content| {
            call.questions
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
                    if selected.iter().enumerate().any(|(index, value)| {
                        !question.options.iter().any(|choice| &choice.label == value)
                            || selected[..index].contains(value)
                    }) || (custom.is_some() && !question.allow_custom)
                        || (selected.is_empty() && custom.is_none())
                        || (!question.multi_select
                            && selected.len() + usize::from(custom.is_some()) > 1)
                    {
                        return None;
                    }
                    Some(Answer {
                        question_id: question.id.clone(),
                        selected,
                        custom,
                    })
                })
                .collect()
        });
    Some(question::respond(session, call, answers.as_deref()))
}
