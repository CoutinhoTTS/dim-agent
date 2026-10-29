//! MCP sign-in and the `mcp` and `/mcp` commands of the line client: they
//! print to the terminal and read hidden input from it. The protocol parts
//! both clients share are in [`crate::mcp`].

use crate::{
    Result,
    args::Options,
    client::Client,
    input::Input,
    login::open_browser,
    mcp::{Capture, Catalog, Started, catalog_report, error, session_report, target},
    sanitize::safe,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    future::Future,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{io::AsyncWriteExt, time::Instant};
use url::Url;

pub async fn list(client: &mut Client, options: &Options) -> Result<()> {
    let (rows, notes) = catalog_report(client, options).await?;
    print_report(rows, notes);
    Ok(())
}

/// Writes a report: its rows on stdout, its notes on stderr.
pub fn print_report(rows: Vec<String>, notes: Vec<String>) {
    for row in rows {
        println!("{row}");
    }
    for note in notes {
        eprintln!("{note}");
    }
}

/// `mcp enable ID` and `mcp disable ID`: the request the standalone command
/// sends. Both `/mcp` clients call it too, so the form works where the
/// report names it.
pub async fn set_enabled(
    client: &mut Client,
    options: &Options,
    id: &str,
    enabled: bool,
) -> Result<(Vec<String>, Vec<String>)> {
    let catalog: Catalog = match client
        .typed("mcp.catalog", json!({"cwd":options.cwd}))
        .await
    {
        Ok(catalog) => catalog,
        Err(error) if !enabled && error.downcast_ref::<crate::client::RpcError>().is_some() => {
            client
                .rpc(
                    "mcp.set_enabled",
                    json!({"cwd":options.cwd,"id":id,"source":"project","expectedVersion":"","enabled":false}),
                )
                .await?;
            return Ok((
                vec![format!("Disabled {id:?} (project).")],
                vec![format!(
                    "The MCP catalog could not be read ({}); withdrew this workspace's enablement of {id:?}. A user entry with this ID is unchanged.",
                    safe(&error.to_string())
                )],
            ));
        }
        Err(error) => return Err(error),
    };
    let (source, listed) = match catalog.servers.into_iter().find(|server| server.id == id) {
        Some(server) => (server.source, true),
        None if !enabled && catalog.project_revision.is_some() => ("project".into(), false),
        None => return Err(format!("No MCP server {id:?} in mcp list").into()),
    };
    let expected = if source == "project" {
        catalog
            .project_revision
            .ok_or("Invalid mcp.catalog response")?
    } else {
        catalog.user_revision
    };
    client
        .rpc(
            "mcp.set_enabled",
            json!({"cwd":options.cwd,"id":id,"source":source,"expectedVersion":expected,"enabled":enabled}),
        )
        .await?;
    let mut notes = Vec::new();
    if enabled && source == "project" {
        notes.push(
            "It runs this workspace's .mcp.json entry, including later edits to it, in active and new sessions."
                .to_owned(),
        );
    }
    if !listed {
        notes.push(format!(
            "{id:?} is not in mcp list; withdrew any stored enablement of it in this workspace."
        ));
    }
    Ok((
        vec![format!(
            "{} {id:?} ({}).",
            if enabled { "Enabled" } else { "Disabled" },
            safe(&source)
        )],
        notes,
    ))
}

async fn settle<T>(
    request: impl Future<Output = Result<T>>,
    deadline: Option<Instant>,
) -> Result<(Result<T>, bool)> {
    tokio::pin!(request);
    let mut interrupted = false;
    loop {
        tokio::select! {
            biased;
            signal = tokio::signal::ctrl_c(), if !interrupted => {
                signal?;
                interrupted = true;
                eprintln!("Waiting for the MCP operation to settle...");
            }
            _ = async { match deadline { Some(deadline) => tokio::time::sleep_until(deadline).await, None => std::future::pending().await } }, if !interrupted => {
                interrupted = true;
                eprintln!("Sign-in expired; waiting for the submitted operation to settle...");
            }
            result = &mut request => return Ok((result, interrupted)),
        }
    }
}

async fn hidden(input: &mut Input, prompt: &str) -> Result<Option<String>> {
    let echo = input.hide_echo()?;
    let result = tokio::select! {
        biased;
        signal = tokio::signal::ctrl_c() => { signal?; None }
        line = async { input.prompt(prompt)?; input.line().await } => line?,
    };
    drop(echo);
    println!();
    Ok(result)
}

pub async fn sign_in(
    client: &mut Client,
    options: &Options,
    input: &mut Input,
    id: &str,
    no_browser: bool,
    secret: bool,
) -> Result<i32> {
    let (params, interrupted) = settle(target(client, options, id), None).await?;
    if interrupted {
        restart(id);
        return Ok(130);
    }
    let mut params = params.map_err(error)?;
    if secret {
        let Some(value) = hidden(input, "Client secret (hidden): ").await? else {
            restart(id);
            return Ok(130);
        };
        if value.is_empty() {
            return Err(
                "Client secret cannot be empty; omit --client-secret for public clients".into(),
            );
        }
        params["clientSecret"] = json!(value);
    }
    let (started, interrupted) =
        settle(client.typed::<Started>("mcp.auth.start", params), None).await?;
    if interrupted {
        restart(id);
        return Ok(130);
    }
    let started = started.map_err(error)?;
    let issuer = Url::parse(&started.issuer)?;
    println!(
        "MCP server: {id:?}\nResource: {}\nAuthorization server: {}",
        safe(&started.resource),
        safe(
            issuer
                .host_str()
                .ok_or("Authorization server has no host")?
        )
    );
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let deadline = Instant::now() + Duration::from_millis(started.expires_at.saturating_sub(now));
    if deadline <= Instant::now() {
        return Ok(stopped(id, deadline));
    }
    let local_browser = !no_browser
        && std::env::var_os("SSH_CONNECTION").is_none()
        && std::env::var_os("SSH_TTY").is_none();
    let mut capture = if !local_browser {
        None
    } else {
        Capture::bind(&started.redirect_uri).await
    };
    if local_browser && capture.is_none() {
        eprintln!(
            "Local callback listening is unavailable; copy the full URL from the browser address bar and paste it here."
        );
    }
    println!("Open: {}", safe(&started.authorization_url));
    if local_browser {
        open_browser(&started.authorization_url);
    }
    loop {
        let echo = input.hide_echo()?;
        if capture.is_some() {
            println!(
                "Waiting for the browser callback. You can also paste the full callback URL now; input is hidden. Ctrl+C stops waiting."
            );
        } else {
            println!("Paste the full callback URL. Input is hidden.");
        }
        let listener = capture.take();
        let browser = async move {
            match listener {
                Some(capture) => capture.receive().await,
                None => std::future::pending().await,
            }
        };
        let (callback, browser) = tokio::select! {
            biased;
            signal = tokio::signal::ctrl_c() => { signal?; return Ok(stopped(id, deadline)); },
            _ = tokio::time::sleep_until(deadline) => return Ok(stopped(id, deadline)),
            incoming = browser => match incoming {
                Ok((callback, stream)) => { println!(); (callback, Some(stream)) },
                Err(_) => { eprintln!("The local callback could not be read; paste it instead."); continue; }
            },
            line = async { input.prompt("Callback URL: ")?; input.line().await } => match line? {
                Some(callback) if !callback.trim().is_empty() => (callback, None),
                Some(_) => continue,
                None => return Ok(stopped(id, deadline)),
            },
        };
        drop(echo);
        println!();
        let (completed, interrupted) = settle(
            client.rpc(
                "mcp.auth.complete",
                json!({"attemptId":started.attempt_id,"callbackUrl":callback}),
            ),
            Some(deadline),
        )
        .await?;
        if let Some(mut stream) = browser {
            let page = if completed.is_ok() {
                "Sign-in completed. You can close this window."
            } else {
                "Return to the terminal to continue sign-in."
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{page}",
                page.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
        match completed {
            Ok(_) => {
                println!("Signed in to {id:?}.");
                return Ok(0);
            }
            Err(error) => {
                if interrupted {
                    return Ok(stopped(id, deadline));
                }
                if error
                    .downcast_ref::<crate::client::RpcError>()
                    .and_then(|error| error.data.as_ref())
                    .and_then(|data| data.get("appError"))
                    .and_then(Value::as_str)
                    == Some("mcp_auth_callback_invalid")
                {
                    eprintln!("Callback did not match this attempt; try again.");
                } else {
                    return Err(self::error(error));
                }
            }
        }
    }
}

fn stopped(id: &str, deadline: Instant) -> i32 {
    if Instant::now() >= deadline {
        eprintln!("Sign-in expired. Run mcp login {id:?} to start again.");
        1
    } else {
        restart(id);
        130
    }
}

fn restart(id: &str) {
    eprintln!("Stopped waiting. Run mcp login {id:?} to start again.");
}

pub async fn sign_out(client: &mut Client, options: &Options, id: &str) -> Result<i32> {
    #[derive(Deserialize)]
    struct LoggedOut {
        revoked: bool,
    }
    let params = target(client, options, id).await?;
    let (result, _) = settle(client.typed::<LoggedOut>("mcp.auth.logout", params), None).await?;
    let result = result.map_err(error)?;
    println!("Signed out of {id:?}. Revoked: {}.", result.revoked);
    Ok(0)
}

pub async fn session_status(client: &mut Client, session: &str) -> Result<()> {
    for line in session_report(client, session).await? {
        println!("{line}");
    }
    Ok(())
}

pub async fn command(
    client: &mut Client,
    options: &Options,
    input: &mut Input,
    session: &str,
    text: &str,
) -> Result<()> {
    let rest = text
        .trim()
        .strip_prefix("/mcp")
        .expect("MCP command")
        .trim();
    if rest.is_empty() || rest == "list" {
        list(client, options).await?;
        return session_status(client, session).await;
    }
    let (action, mut id) = rest.split_once(char::is_whitespace).ok_or(
        "Usage: /mcp [list|login [--no-browser] [--client-secret] ID|logout ID|enable ID|disable ID]",
    )?;
    id = id.trim_start();
    match action {
        "login" => {
            let mut no_browser = false;
            let mut secret = false;
            loop {
                let (flag, tail) = id.split_once(char::is_whitespace).unwrap_or((id, ""));
                match flag {
                    "--no-browser" => no_browser = true,
                    "--client-secret" => secret = true,
                    _ => break,
                }
                id = tail.trim_start();
            }
            if id.is_empty() || id.split_whitespace().any(|word| word.starts_with("--")) {
                return Err("Usage: /mcp login [--no-browser] [--client-secret] ID".into());
            }
            sign_in(client, options, input, id, no_browser, secret).await?;
            Ok(())
        }
        "logout" if !id.is_empty() && !id.split_whitespace().any(|word| word.starts_with("--")) => {
            sign_out(client, options, id).await?;
            Ok(())
        }
        "enable" | "disable"
            if !id.is_empty() && !id.split_whitespace().any(|word| word.starts_with("--")) =>
        {
            let (rows, notes) = set_enabled(client, options, id, action == "enable").await?;
            print_report(rows, notes);
            Ok(())
        }
        _ => Err(
            "Usage: /mcp [list|login [--no-browser] [--client-secret] ID|logout ID|enable ID|disable ID]"
                .into(),
        ),
    }
}
