//! `/goal` at the line-mode prompt: shows the goal in force on `main`, and
//! writes it through the methods Goal registers (cli-commands.md).
use crate::{Result, client::Client, display::Display, goal, session::read_history};

#[cfg(unix)]
type Interrupts = tokio::signal::unix::Signal;
#[cfg(windows)]
type Interrupts = tokio::signal::windows::CtrlC;

/// Runs one `/goal`; `text` is the whole command line.
pub async fn command(
    client: &mut Client,
    session_id: &str,
    text: &str,
    display: &mut Display,
    interrupts: &mut Interrupts,
) -> Result<()> {
    let words = text.trim_start().strip_prefix("/goal").unwrap_or(text);
    let Some(request) = goal::request(words) else {
        display.record(goal::USAGE)?;
        return Ok(());
    };
    let call = goal::call(&request, display.goal.goal().as_ref());
    let operation = async {
        if !goal::available(client, session_id).await {
            return Ok::<bool, crate::Error>(false);
        }
        let Some(call) = call else { return Ok(true) };
        let mut params = serde_json::json!({"sessionId": session_id, "branchId": "main"});
        if let Some(objective) = goal::objective(&request) {
            params["objective"] = serde_json::json!(objective);
        }
        if let Err(error) = client.rpc(call.method(), params).await {
            return match crate::client::app_error(&error) {
                Some("unknown_method") => Ok(false),
                _ => Err(error),
            };
        }
        read_history(client, session_id, display.cursor, |value| {
            display.event(serde_json::from_value(value)?, true)
        })
        .await?;
        Ok(true)
    };
    if !settle(interrupts, operation).await? {
        display.record(goal::NOT_AVAILABLE)?;
        return Ok(());
    }
    for line in goal::report("main", call, display.goal.goal().as_ref()) {
        display.record(&line)?;
    }
    Ok(())
}

/// Waits for an operation; interrupts are reported and consumed until it
/// settles.
async fn settle<T>(
    interrupts: &mut Interrupts,
    operation: impl std::future::Future<Output = T>,
) -> T {
    tokio::pin!(operation);
    loop {
        tokio::select! {
            biased;
            _ = interrupts.recv() => {
                println!("\nWaiting for the goal command before accepting more input.");
            }
            result = &mut operation => break result,
        }
    }
}
