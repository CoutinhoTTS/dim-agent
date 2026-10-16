use crate::{
    Result, args::Options, client::Client, display::safe, input::Input, login::open_browser,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    future::Future,
    net::{Ipv4Addr, Ipv6Addr},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    time::Instant,
};
use url::Url;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub user_revision: String,
    pub project_revision: Option<String>,
    pub servers: Vec<Server>,
    user_application: Application,
    project_application: Option<Application>,
}
#[derive(Deserialize)]
pub struct Server {
    pub id: String,
    pub source: String,
    enabled: bool,
    transport: String,
    auth: Option<Auth>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Auth {
    status: String,
    missing_scopes: Option<Vec<String>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Application {
    error: Option<String>,
    cleanup_error: Option<String>,
}

pub async fn list(client: &mut Client, options: &Options) -> Result<()> {
    let catalog: Catalog = client
        .typed("mcp.catalog", json!({"cwd":options.cwd}))
        .await?;
    for server in catalog.servers {
        println!(
            "{:?}\t{}\t{}\t{}\t{}",
            server.id,
            safe(&server.source),
            safe(&server.transport),
            if server.enabled {
                "enabled"
            } else {
                "disabled"
            },
            safe(
                match server.auth.as_ref().map(|auth| auth.status.as_str()) {
                    Some("required") => "sign-in required",
                    Some("authorized") => "signed in",
                    Some("expired") => "sign-in expired",
                    Some(other) => other,
                    None => "—",
                }
            )
        );
        if let Some(scopes) = server.auth.and_then(|auth| auth.missing_scopes) {
            println!("  Missing scopes: {}", safe(&scopes.join(", ")));
        }
    }
    eprintln!(
        "Columns: ID, source, transport, enablement, authorization.\nAn enabled entry runs its command or endpoint. Project entries start disabled until you run mcp enable ID.\nSign in with mcp login ID; sign out with mcp logout ID."
    );
    for (source, application) in [
        ("user", Some(catalog.user_application)),
        ("project", catalog.project_application),
    ] {
        if let Some(application) = application {
            for error in [application.error, application.cleanup_error]
                .into_iter()
                .flatten()
            {
                eprintln!("{source} MCP configuration: {}", safe(&error));
            }
        }
    }
    Ok(())
}

async fn target(client: &mut Client, options: &Options, id: &str) -> Result<Value> {
    let catalog: Catalog = client
        .typed("mcp.catalog", json!({"cwd":options.cwd}))
        .await?;
    let server = catalog
        .servers
        .into_iter()
        .find(|server| server.id == id)
        .ok_or_else(|| format!("No MCP server {id:?} in mcp list"))?;
    Ok(json!({"id":id,"source":server.source,"cwd":options.cwd}))
}

pub fn error(error: crate::Error) -> crate::Error {
    let code = error
        .downcast_ref::<crate::client::RpcError>()
        .and_then(|error| error.data.as_ref())
        .map(|data| data.app_error.as_str());
    let action = match code {
        Some("mcp_auth_unsupported") => {
            "OAuth is unavailable for this entry; check its transport, static credentials and store format"
        }
        Some("mcp_auth_discovery_failed") => {
            "Check the server metadata or set oauth.authServerMetadataUrl"
        }
        Some("mcp_auth_client_required") => {
            "Set oauth.clientId; after an issuer change, sign out first"
        }
        Some("mcp_auth_attempt_closed") => {
            "Sign-in expired or was replaced; run mcp login ID again"
        }
        Some("mcp_auth_callback_invalid") => "Paste the full callback URL for this sign-in attempt",
        Some("mcp_auth_failed") => {
            "Authorization was refused; fix the cause and run mcp login ID again"
        }
        Some("mcp_auth_unavailable") => {
            "Authorization service or credential storage is unavailable; retry"
        }
        Some("mcp_unavailable") => {
            "Enable the MCP entry and retry; inspect mcp list for configuration errors"
        }
        _ => return error,
    };
    action.into()
}

// A cancelled interactive waiter still drains its admitted RPC before the next command.
async fn settle<T>(
    request: impl Future<Output = Result<T>>,
    deadline: Option<Instant>,
) -> Result<(Result<T>, bool)> {
    tokio::pin!(request);
    let mut interrupted = false;
    loop {
        tokio::select! {
            result = &mut request => return Ok((result, interrupted)),
            signal = tokio::signal::ctrl_c(), if !interrupted => {
                signal?;
                interrupted = true;
                eprintln!("Waiting for the MCP operation to settle...");
            }
            _ = async { match deadline { Some(deadline) => tokio::time::sleep_until(deadline).await, None => std::future::pending().await } }, if !interrupted => {
                interrupted = true;
                eprintln!("Sign-in expired; waiting for the submitted operation to settle...");
            }
        }
    }
}

async fn hidden(
    input: &mut Input,
    prompt: &str,
    deadline: Option<Instant>,
) -> Result<Option<String>> {
    let echo = input.hide_echo()?;
    input.prompt(prompt)?;
    let result = tokio::select! {
        signal = tokio::signal::ctrl_c() => { signal?; None }
        line = input.line() => line?,
        _ = async { match deadline { Some(deadline) => tokio::time::sleep_until(deadline).await, None => std::future::pending().await } } => None,
    };
    drop(echo);
    println!();
    Ok(result)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    attempt_id: String,
    authorization_url: String,
    redirect_uri: String,
    resource: String,
    issuer: String,
    expires_at: u64,
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
        let Some(value) = hidden(input, "Client secret (hidden): ", None)
            .await?
            .filter(|value| !value.is_empty())
        else {
            restart(id);
            return Ok(130);
        };
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
        restart(id);
        return Ok(130);
    }
    let local_browser = !no_browser
        && std::env::var_os("SSH_CONNECTION").is_none()
        && std::env::var_os("SSH_TTY").is_none();
    let capture = if !local_browser {
        None
    } else {
        Capture::bind(&started.redirect_uri).await
    };
    println!("Open: {}", safe(&started.authorization_url));
    if local_browser {
        open_browser(&started.authorization_url);
    }
    let mut incoming = if let Some(capture) = capture {
        println!("Waiting for the browser callback. Ctrl+C stops waiting.");
        tokio::select! {
            signal = tokio::signal::ctrl_c() => { signal?; restart(id); return Ok(130) }
            _ = tokio::time::sleep_until(deadline) => { restart(id); return Ok(130) }
            callback = capture.receive() => match callback {
                Ok(callback) => Some(callback),
                Err(_) => { eprintln!("The local callback could not be read; paste it instead."); None }
            }
        }
    } else {
        None
    };
    loop {
        let (callback, browser) = match incoming.take() {
            Some((callback, stream)) => (callback, Some(stream)),
            None => {
                println!("Paste the full callback URL. Input is hidden.");
                let Some(callback) = hidden(input, "Callback URL: ", Some(deadline))
                    .await?
                    .filter(|value| !value.trim().is_empty())
                else {
                    restart(id);
                    return Ok(130);
                };
                (callback, None)
            }
        };
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
                return Ok(if interrupted { 130 } else { 0 });
            }
            Err(error) => {
                if interrupted {
                    restart(id);
                    return Ok(130);
                }
                // Check the owner-provided code before mapping its action text.
                if error
                    .downcast_ref::<crate::client::RpcError>()
                    .and_then(|error| error.data.as_ref())
                    .is_some_and(|data| data.app_error == "mcp_auth_callback_invalid")
                {
                    eprintln!("Callback did not match this attempt; try again.");
                } else {
                    return Err(self::error(error));
                }
            }
        }
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
    let (result, interrupted) =
        settle(client.typed::<LoggedOut>("mcp.auth.logout", params), None).await?;
    let result = result.map_err(error)?;
    println!("Signed out of {id:?}. Revoked: {}.", result.revoked);
    Ok(if interrupted { 130 } else { 0 })
}

pub async fn session_status(client: &mut Client, session: &str) -> Result<()> {
    let state: State = client
        .typed("session.mcp", json!({"sessionId":session}))
        .await?;
    for server in state.servers {
        println!("{:?}\t{}", server.id, safe(&server.state));
        if server.state == "auth_required" {
            println!(
                "  Sign in with mcp login {:?} or /mcp login {:?}.",
                server.id, server.id
            );
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub session_id: String,
    pub servers: Vec<Status>,
}
#[derive(Deserialize)]
pub struct Status {
    pub id: String,
    pub state: String,
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
    let (action, mut id) = rest
        .split_once(char::is_whitespace)
        .ok_or("Usage: /mcp [list|login [--no-browser] [--client-secret] ID|logout ID]")?;
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
            if id.is_empty() {
                return Err("Usage: /mcp login [--no-browser] [--client-secret] ID".into());
            }
            sign_in(client, options, input, id, no_browser, secret).await?;
            Ok(())
        }
        "logout" if !id.is_empty() => {
            sign_out(client, options, id).await?;
            Ok(())
        }
        _ => Err("Usage: /mcp [list|login [--no-browser] [--client-secret] ID|logout ID]".into()),
    }
}

struct Capture {
    ipv4: TcpListener,
    ipv6: TcpListener,
    redirect: Url,
}
impl Capture {
    async fn bind(redirect: &str) -> Option<Self> {
        let redirect = Url::parse(redirect).ok()?;
        if redirect.scheme() != "http"
            || !matches!(
                redirect.host_str(),
                Some("localhost" | "127.0.0.1" | "[::1]")
            )
        {
            return None;
        }
        let port = redirect.port_or_known_default()?;
        let ipv4 = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await.ok()?;
        let ipv6 = TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await.ok()?;
        Some(Self {
            ipv4,
            ipv6,
            redirect,
        })
    }
    async fn receive(self) -> Result<(String, TcpStream)> {
        let (mut stream, _) = tokio::select! { value = self.ipv4.accept() => value?, value = self.ipv6.accept() => value? };
        drop(self.ipv4);
        drop(self.ipv6);
        let mut request = Vec::new();
        let mut reader = BufReader::new((&mut stream).take(16385));
        loop {
            let start = request.len();
            if reader.read_until(b'\n', &mut request).await? == 0 || request.len() > 16384 {
                return Err("Incomplete or oversized local callback request".into());
            }
            if &request[start..] == b"\r\n" {
                break;
            }
        }
        drop(reader);
        let line = std::str::from_utf8(
            request
                .split_inclusive(|byte| *byte == b'\n')
                .next()
                .ok_or("Missing callback request")?,
        )?;
        let fields: Vec<_> = line.split_ascii_whitespace().collect();
        if line.len() > 8192
            || !line.ends_with('\n')
            || fields.len() != 3
            || fields[0] != "GET"
            || !fields[1].starts_with('/')
            || fields[1].starts_with("//")
            || !matches!(fields[2], "HTTP/1.0" | "HTTP/1.1")
        {
            let _ = stream
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            return Err("Invalid local callback request".into());
        }
        Ok((
            format!(
                "{}{}",
                self.redirect.origin().ascii_serialization(),
                fields[1]
            ),
            stream,
        ))
    }
}
