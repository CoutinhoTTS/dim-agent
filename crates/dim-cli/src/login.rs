use crate::{Result, args::Options, client::Client, display::safe, input::Input};
use serde::Deserialize;
use serde_json::json;
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
    let params = json!({"providerId":provider_id,"directory":options.directory});
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
    // Launching is best effort: the URL remains usable without desktop integration.
    println!("Open: {}", safe(&browser.authorization_url));
    open_browser(&browser.authorization_url);
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
            // The plugin owns an accepted exchange even if this waiter leaves.
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

fn open_browser(url: &str) {
    let valid = url.starts_with("https://") || url.starts_with("http://");
    if !valid {
        return;
    }
    #[cfg(target_os = "macos")]
    let command = Some(("open", vec![url]));
    #[cfg(target_os = "linux")]
    let command = Some(("xdg-open", vec![url]));
    #[cfg(windows)]
    let command = Some(("rundll32.exe", vec!["url.dll,FileProtocolHandler", url]));
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    let command: Option<(&str, Vec<&str>)> = None;
    if let Some((program, args)) = command {
        let _ = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}
