use crate::{
    Result,
    args::{Action, Options},
    client::{Client, Message},
    display::{self, Display, Event, Progress},
    export,
    input::Input,
};
use dim_agent_types::{ApprovalCallData, ModelId};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashSet, VecDeque},
    future::pending,
    io::Write,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    session_id: String,
    model: ModelId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct History {
    events: Vec<Value>,
    through: u64,
    next_after: u64,
    done: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventNotice {
    session_id: String,
    branch_id: String,
    event: Event,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Approval {
    session_id: String,
    branch_id: String,
    request_id: String,
    call: ApprovalCallData,
    reason: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalCancelled {
    session_id: String,
    request_id: String,
}

fn decode<T: serde::de::DeserializeOwned>(value: Value, method: &str) -> Result<T> {
    serde_json::from_value(value).map_err(|_| format!("Invalid {method} notification").into())
}

pub async fn chat(client: &mut Client, options: &Options) -> Result<i32> {
    let interactive = options.prompt.is_none() && !matches!(options.action, Action::Exec);
    let mut input = if interactive {
        Some(Input::new()?)
    } else {
        None
    };
    let mut display = Display::new(interactive, options.json);
    let session_id = if let Some(id) = &options.resume {
        id.clone()
    } else {
        let mut params = json!({"cwd":options.cwd});
        if let (Some(provider), Some(model)) = (&options.provider, &options.model) {
            params["model"] = json!({"provider":provider,"modelId":model});
        }
        let created: Created = client.typed("session.create", params).await.map_err(|error| format!("Cannot create session: {error}. Configure a provider or use --model-config PATH with an API-key environment reference"))?;
        if interactive {
            println!(
                "dimcode · model: {}/{}",
                display::safe(&created.model.provider_id),
                display::safe(&created.model.model_id)
            );
        }
        created.session_id
    };
    // Print the handle even in headless mode so an uncertain submission can be resumed.
    display.record(&format!(
        "workspace: {}\nsession: {}",
        options.cwd.display(),
        session_id
    ))?;
    client
        .rpc("session.attach", json!({"sessionId":session_id}))
        .await?;
    let result = async {
        display.muted = !interactive;
        display.cursor = read_history(client, &session_id, 0, |value| {
            if options.json { Ok(()) } else { display.event(serde_json::from_value(value)?, true) }
        }).await?;
        display.failed = false;
        display.muted = false;
        let mut code = wait_turn(client, &session_id, None, &mut display, &mut input).await?;
        if code != 0 {
            return Ok(code);
        }
        if let Some(prompt) = &options.prompt {
            return wait_turn(client, &session_id, Some(prompt), &mut display, &mut input).await;
        }
        loop {
            let reader = input.as_mut().expect("interactive input");
            reader.prompt("\n> ")?;
            let text = tokio::select! {
                signal = tokio::signal::ctrl_c() => { signal?; println!(); break; }
                line = reader.line() => line?,
            };
            let Some(text) = text else {
                println!();
                break;
            };
            if text.trim().is_empty() {
                continue;
            }
            match text.split_whitespace().next() {
                Some("/exit") => {
                    if text.trim() == "/exit" {
                        break;
                    }
                    display.record("Usage: /exit")?;
                    continue;
                }
                Some("/export") => {
                    let result = async {
                        let rest = text.trim_start().strip_prefix("/export").expect("export command").trim_start();
                        let (format, path) = rest.split_once(char::is_whitespace)
                            .filter(|(_, path)| !path.trim_start().is_empty())
                            .ok_or("Usage: /export <markdown|json> PATH\nPATH is relative to the CLI current directory; ~ is literal.")?;
                        let path = path.trim_start();
                        export::write(client, &session_id, format.parse()?, Some(std::path::Path::new(path))).await
                    }.await;
                    if let Err(error) = result {
                        display.record(&format!("Error: {error}"))?;
                    }
                    continue;
                }
                _ => {}
            }
            display.failed = false;
            code = wait_turn(client, &session_id, Some(&text), &mut display, &mut input).await?;
            if code != 0 && code != 130 && code != 2 {
                return Ok(code);
            }
        }
        Ok(0)
    }
    .await;
    display.finish_text()?;
    let detached = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.rpc("session.detach", json!({"sessionId":session_id})),
    )
    .await;
    match (result, detached) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(_)) => Err("Session detach timed out".into()),
        (Ok(_), Ok(Err(error))) => Err(error),
        (Ok(code), Ok(Ok(_))) => Ok(code),
    }
}

pub async fn read_history(
    client: &mut Client,
    session_id: &str,
    mut after: u64,
    mut consume: impl FnMut(Value) -> Result<()>,
) -> Result<u64> {
    let mut through = None;
    loop {
        let mut params =
            json!({"sessionId":session_id,"branchId":"main","after":after,"limit":256});
        if let Some(through) = through {
            params["through"] = json!(through);
        }
        let page: History = client.typed("session.history", params).await?;
        if through.is_some_and(|through| through != page.through)
            || page.through > dim_agent_types::JS_MAX_SAFE_INTEGER
            || page.next_after < after
            || page.next_after > page.through
            || !page.done && page.next_after == after
        {
            return Err("Invalid history page cursor".into());
        }
        through = Some(page.through);
        for event in page.events {
            let sequence = event["sequence"]
                .as_u64()
                .ok_or("Invalid history event sequence")?;
            if sequence <= after || sequence > page.next_after {
                return Err("History event is outside its page".into());
            }
            consume(event)?;
        }
        after = page.next_after;
        if page.done {
            if after != page.through {
                return Err("History ended before its fixed watermark".into());
            }
            return Ok(after);
        }
    }
}

async fn wait_turn(
    client: &mut Client,
    session_id: &str,
    prompt: Option<&str>,
    display: &mut Display,
    input: &mut Option<Input>,
) -> Result<i32> {
    let mut submit = match prompt {
        Some(text) => Some(client.send("session.submit", json!({"sessionId":session_id,"branchId":"main","submissionId":uuid::Uuid::new_v4().to_string(),"text":text,"mode":"queue"})).await?),
        None => None,
    };
    let mut idle = if submit.is_none() {
        Some(
            client
                .send(
                    "session.idle",
                    json!({"sessionId":session_id,"branchId":"main"}),
                )
                .await?,
        )
    } else {
        None
    };
    let mut stop = None;
    let mut approval_replies = HashSet::new();
    let mut approvals: VecDeque<Approval> = VecDeque::new();
    let mut deferred = VecDeque::new();
    let mut prompted = false;
    let mut code = 0;
    let mut settled = None;
    loop {
        if let Some(sequence) = settled
            && submit.is_none()
            && idle.is_none()
            && stop.is_none()
            && approval_replies.is_empty()
        {
            while let Some(notice) = deferred.pop_front() {
                render(notice, session_id, display)?;
            }
            display.cursor = display.cursor.max(sequence);
            display.finish_text()?;
            return Ok(if code != 0 {
                code
            } else if display.failed {
                2
            } else {
                0
            });
        }
        if !approvals.is_empty() && input.is_some() && !prompted {
            let approval = approvals.front().expect("pending approval");
            display.record(&format!(
                "[approval] {}: {}",
                approval.call.name, approval.reason
            ))?;
            // Review executable code verbatim, but never dump arbitrary extension arguments.
            let keys: &[&str] = match approval.call.name.as_str() {
                "exec" | "execute" => &["command", "cwd", "description"],
                _ => &["path", "filePath", "description"],
            };
            for &key in keys {
                if let Some(Value::String(value)) = approval.call.input.get(key) {
                    display.record(&format!("  {key}: {value}"))?;
                }
            }
            input
                .as_mut()
                .expect("approval input")
                .prompt("Allow once? [y/N] ")?;
            prompted = true;
        }
        let event = tokio::select! {
            signal = tokio::signal::ctrl_c(), if stop.is_none() && settled.is_none() => {
                signal?;
                if !display.json { println!(); }
                display.record("[stopping] Waiting for resource settlement")?;
                code = 130;
                approvals.clear(); prompted = false;
                stop = Some(client.send("session.stop", json!({"sessionId":session_id,"branchId":"main","reason":"CLI interrupted"})).await?);
                continue;
            }
            line = async {
                match input.as_mut() { Some(reader) if prompted => reader.line().await, _ => pending().await }
            } => {
                let approval = approvals.pop_front().expect("prompted approval");
                let line = line?;
                let allow = line.as_ref().is_some_and(|line| matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"));
                approval_replies.insert(client.send("approval.respond", json!({"sessionId":session_id,"branchId":approval.branch_id,"requestId":approval.request_id,"decision":if allow {"allow"} else {"deny"}})).await?);
                prompted = false;
                if line.is_none() && stop.is_none() {
                    code = 130;
                    stop = Some(client.send("session.stop", json!({"sessionId":session_id,"branchId":"main","reason":"CLI input closed"})).await?);
                }
                continue;
            }
            message = client.next() => message?,
        };
        if display.json
            && let Message::Notice { method, params } = &event
            && params["sessionId"].as_str() == Some(session_id)
        {
            let mut output = std::io::stdout().lock();
            serde_json::to_writer(
                &mut output,
                &json!({"jsonrpc":"2.0","method":method,"params":params}),
            )?;
            writeln!(output)?;
            output.flush()?;
        }
        match event {
            Message::Response { id, result } if Some(id) == submit => {
                result?;
                submit = None;
                if stop.is_none() && settled.is_none() {
                    idle = Some(
                        client
                            .send(
                                "session.idle",
                                json!({"sessionId":session_id,"branchId":"main"}),
                            )
                            .await?,
                    );
                }
            }
            Message::Response { id, result } if Some(id) == idle || Some(id) == stop => {
                let result = result?;
                #[derive(Deserialize)]
                struct Barrier {
                    sequence: u64,
                }
                let barrier: Barrier =
                    serde_json::from_value(result).map_err(|_| "Invalid output barrier")?;
                if Some(id) == idle {
                    idle = None;
                    if stop.is_some() {
                        continue;
                    }
                } else {
                    stop = None;
                }
                settled = Some(barrier.sequence);
                approvals.clear();
                prompted = false;
            }
            Message::Response { id, result } if approval_replies.remove(&id) => {
                if code == 0 {
                    result?;
                }
            }
            Message::Response { .. } => return Err("Unexpected session response identity".into()),
            Message::Notice { method, params } if method == "approval.request" => {
                let approval: Approval = decode(params, &method)?;
                if approval.session_id != session_id {
                    continue;
                }
                if stop.is_some() || settled.is_some() {
                    continue;
                }
                if input.is_none() {
                    display.record("Interactive approval required; the operation was denied. Rerun in a terminal to review it")?;
                    approval_replies.insert(client.send("approval.respond", json!({"sessionId":session_id,"branchId":approval.branch_id,"requestId":approval.request_id,"decision":"deny"})).await?);
                    code = 3;
                    stop = Some(client.send("session.stop", json!({"sessionId":session_id,"branchId":"main","reason":"Interactive approval required"})).await?);
                } else {
                    approvals.push_back(approval);
                }
            }
            Message::Notice { method, params } if method == "approval.cancelled" => {
                let cancelled: ApprovalCancelled = decode(params, &method)?;
                if cancelled.session_id == session_id {
                    let current = approvals
                        .front()
                        .is_some_and(|approval| approval.request_id == cancelled.request_id);
                    approvals.retain(|approval| approval.request_id != cancelled.request_id);
                    if current {
                        prompted = false;
                        println!();
                        display.record("[approval cancelled]")?;
                    }
                }
            }
            notice @ Message::Notice { .. } if prompted => deferred.push_back(notice),
            notice @ Message::Notice { .. } => {
                while let Some(notice) = deferred.pop_front() {
                    render(notice, session_id, display)?;
                }
                render(notice, session_id, display)?;
            }
        }
    }
}

fn render(notice: Message, session_id: &str, display: &mut Display) -> Result<()> {
    let Message::Notice { method, params } = notice else {
        unreachable!()
    };
    if display.json {
        if params["sessionId"].as_str() == Some(session_id) {
            if method == "session.closed" {
                return Err("Session closed while attached".into());
            }
            let event = &params["event"];
            if method == "session.event"
                && params["branchId"] == "main"
                && ((event["type"] == "turn" && event["name"] == "end")
                    || (event["type"] == "step" && event["name"] == "model-output-end"))
                && event["data"]["status"] == "failed"
            {
                display.failed = true;
            }
        }
        return Ok(());
    }
    match method.as_str() {
        "session.event" => {
            let notice: EventNotice = decode(params, &method)?;
            if notice.session_id == session_id && notice.branch_id == "main" {
                display.event(notice.event, false)?;
            }
        }
        "session.model.progress" => {
            let notice: Progress = decode(params, &method)?;
            if notice.session_id == session_id && notice.branch_id == "main" {
                display.progress(notice)?;
            }
        }
        "session.closed" if params["sessionId"].as_str() == Some(session_id) => {
            return Err("Session closed while attached".into());
        }
        _ => {}
    }
    std::io::stdout().flush()?;
    Ok(())
}
