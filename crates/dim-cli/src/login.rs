use crate::{
    Result, args::Options, client::Client, input::Input, model::session::ACCOUNT_NAMESPACE,
    sanitize::safe,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io::IsTerminal, time::Duration};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Provider {
    provider_id: String,
    display_name: String,
    login_methods: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Browser {
    attempt_id: String,
    authorization_url: String,
    expires_at: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Device {
    attempt_id: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    user_code: String,
}
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
enum State {
    Pending,
    Completed,
    Failed { error: String },
    Cancelled,
    Expired,
}

/// The Dim account directory of the entry the CLI was started from: the npm
/// launcher names its package in `DIMCODE_NPM_PACKAGE`, a standalone binary
/// goes by its command name. `dimagent` signs in at dimagent.com; `dimcode`
/// and anything else at dimagent.cn.
pub fn account_directory() -> &'static str {
    let entry = std::env::var("DIMCODE_NPM_PACKAGE").ok().or_else(|| {
        std::env::args_os()
            .next()
            .map(std::path::PathBuf::from)
            .and_then(|path| Some(path.file_stem()?.to_string_lossy().into_owned()))
    });
    match entry.as_deref() {
        Some("dimagent") => "https://dimagent.com",
        _ => "https://dimagent.cn",
    }
}

pub async fn login(client: &mut Client, options: &Options, provider_id: &str) -> Result<i32> {
    let providers: Vec<Provider> = client.typed("account.providers", json!({})).await?;
    let provider = providers
        .into_iter()
        .find(|provider| provider.provider_id == provider_id)
        .ok_or("Account provider is unavailable; install its runtime plugin first")?;
    let method = if options.device {
        "device"
    } else if provider
        .login_methods
        .iter()
        .any(|method| method == "browser")
    {
        "browser"
    } else {
        "device"
    };
    if !provider
        .login_methods
        .iter()
        .any(|available| available == method)
    {
        return Err("The provider does not support the requested login method".into());
    }
    println!("Sign in to {}", safe(&provider.display_name));
    let directory = options.directory.as_deref().unwrap_or(account_directory());
    let params = json!({"providerId":provider_id,"directory":directory});
    if method == "device" {
        let device: Device = client.typed("account.login.begin", params).await?;
        println!(
            "Open: {}\nCode: {}",
            safe(
                device
                    .verification_uri_complete
                    .as_deref()
                    .unwrap_or(&device.verification_uri)
            ),
            safe(&device.user_code)
        );
        return poll(client, provider_id, &device.attempt_id).await;
    }
    if !std::io::stdin().is_terminal() {
        return Err("Browser login requires a terminal to enter the callback URL; use --device if supported".into());
    }
    let mut params = params;
    params["redirectUri"] = json!(options.redirect_uri);
    let browser: Browser = client.typed("account.login.browser.begin", params).await?;
    println!("Open: {}", safe(&browser.authorization_url));
    crate::browser::open(&browser.authorization_url);
    let mut input = Input::new()?;
    let echo = input.hide_echo()?;
    println!("After authorization, paste the full callback URL. Input is hidden.");
    input.prompt("Callback URL: ")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;
    let callback = tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            signal?; drop(echo); println!(); cancel(client, provider_id, &browser.attempt_id).await?; return Ok(130);
        }
        result = input.line() => result?,
        _ = tokio::time::sleep(Duration::from_millis(browser.expires_at.saturating_sub(now))) => {
            drop(echo); println!(); cancel(client, provider_id, &browser.attempt_id).await?; return Err("Login expired; start login again".into());
        }
    };
    drop(echo);
    println!();
    let Some(callback) = callback.filter(|value| !value.trim().is_empty()) else {
        cancel(client, provider_id, &browser.attempt_id).await?;
        return Ok(130);
    };
    let complete = client.rpc(
        "account.login.browser.complete",
        json!({"providerId":provider_id,"attemptId":browser.attempt_id,"callbackUrl":callback}),
    );
    tokio::pin!(complete);
    tokio::select! {
        result = &mut complete => { result?; }
        signal = tokio::signal::ctrl_c() => {
            signal?;
            eprintln!("Login is still settling; use account {provider_id} to inspect the result");
            return Ok(130);
        }
    }
    println!("Signed in.");
    Ok(0)
}

async fn poll(client: &mut Client, provider_id: &str, attempt_id: &str) -> Result<i32> {
    loop {
        let status: State = client
            .typed(
                "account.login.status",
                json!({"providerId":provider_id,"attemptId":attempt_id}),
            )
            .await?;
        match status {
            State::Pending => {}
            State::Completed => {
                println!("Signed in.");
                return Ok(0);
            }
            State::Failed { error } => return Err(format!("Login failed: {}", safe(&error)).into()),
            State::Cancelled => return Err("Login was cancelled".into()),
            State::Expired => return Err("Login expired; start login again".into()),
        }
        tokio::select! {
            signal = tokio::signal::ctrl_c() => { signal?; cancel(client, provider_id, attempt_id).await?; return Ok(130); }
            _ = tokio::time::sleep(Duration::from_secs(1)) => {},
        }
    }
}

async fn cancel(client: &mut Client, provider_id: &str, attempt_id: &str) -> Result<()> {
    client
        .rpc(
            "account.login.cancel",
            json!({"providerId":provider_id,"attemptId":attempt_id}),
        )
        .await?;
    Ok(())
}

/// The params a namespace's account calls take. The managed account
/// namespace targets one provider; a native OAuth namespace has one account.
pub(crate) fn target(namespace: &str, provider_id: &str) -> Value {
    if namespace == ACCOUNT_NAMESPACE {
        json!({"providerId": provider_id})
    } else {
        json!({})
    }
}

/// The params a sign-in with `attempt_id` takes in `namespace`.
pub(crate) fn attempt(namespace: &str, provider_id: &str, attempt_id: &str) -> Value {
    let mut params = target(namespace, provider_id);
    params["attemptId"] = json!(attempt_id);
    params
}

/// The namespace's browser sign-in begin method.
pub(crate) fn browser_begin(namespace: &str) -> String {
    format!("{namespace}.login.browser.begin")
}

/// The namespace's device sign-in begin method.
pub(crate) fn device_begin(namespace: &str) -> String {
    format!("{namespace}.login.begin")
}
